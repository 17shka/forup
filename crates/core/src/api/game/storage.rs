use std::sync::Arc;
use std::{
    num::NonZeroU64,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::board::{Game, GameError, GameSnapshot, TeamId, Timeline};

const GAMES: TableDefinition<u64, &[u8]> = TableDefinition::new("games");
const TIMELINE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("game_timeline");
const METADATA: TableDefinition<&str, u64> = TableDefinition::new("metadata");
const GAME_METADATA: TableDefinition<u64, &[u8]> = TableDefinition::new("game_metadata");
const CHECKPOINTS: TableDefinition<u64, &[u8]> = TableDefinition::new("game_checkpoints");

pub type GameId = NonZeroU64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryError {
    Database(String),
    Serialization(String),
    GameNotStarted,
    GameAlreadyAdvanced,
    GameNotFound,
    SequenceOverflow,
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(message) => write!(f, "database error: {message}"),
            Self::Serialization(message) => write!(f, "serialization error: {message}"),
            Self::GameNotStarted => write!(f, "cannot persist a game before it starts"),
            Self::GameAlreadyAdvanced => {
                write!(
                    f,
                    "cannot use a game after its first action as the initial snapshot"
                )
            }
            Self::GameNotFound => write!(f, "game history not found"),
            Self::SequenceOverflow => write!(f, "game history sequence overflow"),
        }
    }
}

impl std::error::Error for HistoryError {}

#[derive(Clone)]
pub struct GameStore {
    database: Arc<Database>,
}

impl std::fmt::Debug for GameStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameStore").finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredGame {
    room_id: u32,
    snapshot: GameSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameMetadata {
    pub game_id: GameId,
    pub room_id: u32,
    pub saved_at: u64,
    pub is_completed: bool,
    pub winner_team: Option<TeamId>,
    pub turn_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameHistory {
    pub game_id: GameId,
    pub room_id: u32,
    pub snapshot: GameSnapshot,
    pub timeline: Vec<Timeline>,
}

impl GameHistory {
    pub fn replay(&self) -> Result<Game, GameError> {
        let mut game = self.snapshot.restore();
        for event in &self.timeline {
            apply_event(&mut game, event)?;
        }
        Ok(game)
    }

    pub fn replay_steps(&self) -> Result<Vec<Game>, GameError> {
        let mut game = self.snapshot.restore();
        let mut steps = Vec::with_capacity(self.timeline.len() + 1);
        steps.push(game.clone());
        for event in &self.timeline {
            apply_event(&mut game, event)?;
            steps.push(game.clone());
        }
        Ok(steps)
    }
}

fn apply_event(game: &mut Game, event: &Timeline) -> Result<(), GameError> {
    match *event {
        Timeline::Action(action) => {
            let member_id = game
                .current_turn
                .and_then(|idx| game.characters.get_index(idx))
                .map(|(id, _)| *id)
                .ok_or(GameError::GameNotStarted)?;
            game.apply_action(member_id, action).map(|_| ())
        }
        Timeline::End => game.end_game(),
    }
}

impl GameStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, HistoryError> {
        let database =
            Database::create(path).map_err(|error| HistoryError::Database(error.to_string()))?;
        Ok(Self {
            database: Arc::new(database),
        })
    }

    pub fn start_game(
        &self,
        room_id: u32,
        game: &Game,
        automatic_distribution: bool,
    ) -> Result<GameId, HistoryError> {
        if !game.is_started() {
            return Err(HistoryError::GameNotStarted);
        }
        if !game.timeline.is_empty() {
            return Err(HistoryError::GameAlreadyAdvanced);
        }

        let serialized_game = encode(&StoredGame {
            room_id,
            snapshot: game.snapshot(automatic_distribution),
        })?;

        let saved_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| HistoryError::Database(error.to_string()))?
            .as_secs();

        let write = self
            .database
            .begin_write()
            .map_err(|error| HistoryError::Database(error.to_string()))?;
        let game_id = {
            let mut metadata_table = write
                .open_table(METADATA)
                .map_err(|error| HistoryError::Database(error.to_string()))?;
            let next_id = metadata_table
                .get("next_game_id")
                .map_err(|error| HistoryError::Database(error.to_string()))?
                .map_or(NonZeroU64::MIN, |value| {
                    NonZeroU64::new(value.value()).unwrap_or(NonZeroU64::MIN)
                });
            let following_id = next_id
                .get()
                .checked_add(1)
                .ok_or(HistoryError::SequenceOverflow)?;
            metadata_table
                .insert("next_game_id", following_id)
                .map_err(|error| HistoryError::Database(error.to_string()))?;
            next_id
        };
        {
            let mut games = write
                .open_table(GAMES)
                .map_err(|error| HistoryError::Database(error.to_string()))?;
            games
                .insert(game_id.get(), serialized_game.as_slice())
                .map_err(|error| HistoryError::Database(error.to_string()))?;
        }
        {
            let mut meta_table = write
                .open_table(GAME_METADATA)
                .map_err(|error| HistoryError::Database(error.to_string()))?;
            let metadata = GameMetadata {
                game_id,
                room_id,
                saved_at,
                is_completed: false,
                winner_team: None,
                turn_count: 0,
            };
            let serialized_metadata = encode(&metadata)?;
            meta_table
                .insert(game_id.get(), serialized_metadata.as_slice())
                .map_err(|error| HistoryError::Database(error.to_string()))?;
        }
        write
            .commit()
            .map_err(|error| HistoryError::Database(error.to_string()))?;

        Ok(game_id)
    }

    /// Saves a full mid-game checkpoint that can be loaded later to resume play.
    pub fn save_checkpoint(&self, room_id: u32, game: &Game) -> Result<GameId, HistoryError> {
        let serialized_game = encode(game)?;
        let saved_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| HistoryError::Database(error.to_string()))?
            .as_secs();

        let is_completed = !game.is_started();
        let winner_team = compute_winner_team(game);
        let turn_count = game.timeline.len() as u64;

        let write = self
            .database
            .begin_write()
            .map_err(|error| HistoryError::Database(error.to_string()))?;
        let game_id = {
            let mut metadata_table = write
                .open_table(METADATA)
                .map_err(|error| HistoryError::Database(error.to_string()))?;
            let next_id = metadata_table
                .get("next_game_id")
                .map_err(|error| HistoryError::Database(error.to_string()))?
                .map_or(NonZeroU64::MIN, |value| {
                    NonZeroU64::new(value.value()).unwrap_or(NonZeroU64::MIN)
                });
            let following_id = next_id
                .get()
                .checked_add(1)
                .ok_or(HistoryError::SequenceOverflow)?;
            metadata_table
                .insert("next_game_id", following_id)
                .map_err(|error| HistoryError::Database(error.to_string()))?;
            next_id
        };
        {
            let mut checkpoints = write
                .open_table(CHECKPOINTS)
                .map_err(|error| HistoryError::Database(error.to_string()))?;
            checkpoints
                .insert(game_id.get(), serialized_game.as_slice())
                .map_err(|error| HistoryError::Database(error.to_string()))?;
        }
        {
            let mut meta_table = write
                .open_table(GAME_METADATA)
                .map_err(|error| HistoryError::Database(error.to_string()))?;
            let metadata = GameMetadata {
                game_id,
                room_id,
                saved_at,
                is_completed,
                winner_team,
                turn_count,
            };
            let serialized_metadata = encode(&metadata)?;
            meta_table
                .insert(game_id.get(), serialized_metadata.as_slice())
                .map_err(|error| HistoryError::Database(error.to_string()))?;
        }
        write
            .commit()
            .map_err(|error| HistoryError::Database(error.to_string()))?;

        Ok(game_id)
    }

    pub fn append_timeline(
        &self,
        game_id: GameId,
        events: &[Timeline],
    ) -> Result<(), HistoryError> {
        if events.is_empty() {
            return Ok(());
        }

        let write = self
            .database
            .begin_write()
            .map_err(|error| HistoryError::Database(error.to_string()))?;
        {
            let games = write
                .open_table(GAMES)
                .map_err(|error| HistoryError::Database(error.to_string()))?;
            games
                .get(game_id.get())
                .map_err(|error| HistoryError::Database(error.to_string()))?
                .ok_or(HistoryError::GameNotFound)?;
        }

        {
            let mut timeline = write
                .open_table(TIMELINE)
                .map_err(|error| HistoryError::Database(error.to_string()))?;
            let start = timeline_key(game_id, 0);
            let end = timeline_key(game_id, u64::MAX);
            let mut entries = timeline
                .range(start.as_slice()..=end.as_slice())
                .map_err(|error| HistoryError::Database(error.to_string()))?;
            let mut next_sequence = if let Some(entry) = entries.next_back() {
                let (key, _) = entry.map_err(|error| HistoryError::Database(error.to_string()))?;
                timeline_sequence(key.value())?
                    .checked_add(1)
                    .ok_or(HistoryError::SequenceOverflow)?
            } else {
                0
            };

            for event in events {
                let key = timeline_key(game_id, next_sequence);
                let value = encode(event)?;
                timeline
                    .insert(key.as_slice(), value.as_slice())
                    .map_err(|error| HistoryError::Database(error.to_string()))?;
                next_sequence = next_sequence
                    .checked_add(1)
                    .ok_or(HistoryError::SequenceOverflow)?;
            }
        }

        write
            .commit()
            .map_err(|error| HistoryError::Database(error.to_string()))?;
        Ok(())
    }

    pub fn games_for_room(&self, room_id: u32) -> Result<Vec<GameId>, HistoryError> {
        let read = self
            .database
            .begin_read()
            .map_err(|error| HistoryError::Database(error.to_string()))?;
        let games = read
            .open_table(GAMES)
            .map_err(|error| HistoryError::Database(error.to_string()))?;
        let mut game_ids = Vec::new();
        for entry in games
            .iter()
            .map_err(|error| HistoryError::Database(error.to_string()))?
        {
            let (id, value) = entry.map_err(|error| HistoryError::Database(error.to_string()))?;
            let game = decode::<StoredGame>(value.value())?;
            if game.room_id == room_id {
                game_ids.push(NonZeroU64::new(id.value()).expect("game ids are non-zero"));
            }
        }
        Ok(game_ids)
    }

    /// Lists saved games for a room with metadata, ordered by save time.
    pub fn list_games(&self, room_id: u32) -> Result<Vec<GameMetadata>, HistoryError> {
        let read = self
            .database
            .begin_read()
            .map_err(|error| HistoryError::Database(error.to_string()))?;
        let metadata_table = read
            .open_table(GAME_METADATA)
            .map_err(|error| HistoryError::Database(error.to_string()))?;
        let mut games = Vec::new();
        for entry in metadata_table
            .iter()
            .map_err(|error| HistoryError::Database(error.to_string()))?
        {
            let (_, value) = entry.map_err(|error| HistoryError::Database(error.to_string()))?;
            let metadata: GameMetadata = decode(value.value())?;
            if metadata.room_id == room_id {
                games.push(metadata);
            }
        }
        games.sort_by_key(|g| g.saved_at);
        Ok(games)
    }

    /// Loads a full mid-game checkpoint so play can be resumed from that state.
    pub fn load_checkpoint(&self, game_id: GameId) -> Result<Option<Game>, HistoryError> {
        let read = self
            .database
            .begin_read()
            .map_err(|error| HistoryError::Database(error.to_string()))?;
        let checkpoints = read
            .open_table(CHECKPOINTS)
            .map_err(|error| HistoryError::Database(error.to_string()))?;
        let Some(value) = checkpoints
            .get(game_id.get())
            .map_err(|error| HistoryError::Database(error.to_string()))?
        else {
            return Ok(None);
        };
        Ok(Some(decode::<Game>(value.value())?))
    }

    pub fn load(&self, game_id: GameId) -> Result<Option<GameHistory>, HistoryError> {
        let read = self
            .database
            .begin_read()
            .map_err(|error| HistoryError::Database(error.to_string()))?;
        let stored_game = {
            let games = read
                .open_table(GAMES)
                .map_err(|error| HistoryError::Database(error.to_string()))?;
            let Some(value) = games
                .get(game_id.get())
                .map_err(|error| HistoryError::Database(error.to_string()))?
            else {
                return Ok(None);
            };
            decode::<StoredGame>(value.value())?
        };

        let mut timeline = Vec::new();
        let mut expected_sequence = 0;
        {
            let events = read
                .open_table(TIMELINE)
                .map_err(|error| HistoryError::Database(error.to_string()))?;
            let start = timeline_key(game_id, 0);
            let end = timeline_key(game_id, u64::MAX);
            for entry in events
                .range(start.as_slice()..=end.as_slice())
                .map_err(|error| HistoryError::Database(error.to_string()))?
            {
                let (key, value) =
                    entry.map_err(|error| HistoryError::Database(error.to_string()))?;
                if timeline_sequence(key.value())? != expected_sequence {
                    return Err(HistoryError::Database(
                        "stored timeline sequence is not contiguous".into(),
                    ));
                }
                timeline.push(decode::<Timeline>(value.value())?);
                expected_sequence = expected_sequence
                    .checked_add(1)
                    .ok_or(HistoryError::SequenceOverflow)?;
            }
        }

        Ok(Some(GameHistory {
            game_id,
            room_id: stored_game.room_id,
            snapshot: stored_game.snapshot,
            timeline,
        }))
    }
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, HistoryError> {
    postcard::to_allocvec(value).map_err(|error| HistoryError::Serialization(error.to_string()))
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, HistoryError> {
    postcard::from_bytes(bytes).map_err(|error| HistoryError::Serialization(error.to_string()))
}

fn timeline_key(game_id: GameId, sequence: u64) -> [u8; 16] {
    let mut key = [0; 16];
    key[..8].copy_from_slice(&game_id.get().to_be_bytes());
    key[8..].copy_from_slice(&sequence.to_be_bytes());
    key
}

fn timeline_sequence(key: &[u8]) -> Result<u64, HistoryError> {
    let sequence = key
        .get(8..16)
        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
        .ok_or_else(|| HistoryError::Database("invalid timeline key".into()))?;
    Ok(u64::from_be_bytes(sequence))
}

fn compute_winner_team(game: &Game) -> Option<TeamId> {
    game.characters
        .iter()
        .find_map(|(_, member)| (member.stats.wins > 0).then_some(member.team_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::game::board::{BoardCode, CharacterId, GameAction, TeamId};
    use crate::api::network::{Uuid, UuidExt};

    fn pos(slot: u8) -> BoardCode {
        BoardCode::from_slot(slot)
    }

    #[test]
    fn stores_a_start_snapshot_and_replayable_timeline() {
        let path = std::env::temp_dir().join(format!("forup-history-{}.redb", Uuid::generate()));
        let store = GameStore::open(&path).unwrap();
        let mut game = Game::standard();
        let member_id = game
            .add_local_member(Uuid::new(7).unwrap(), TeamId::MIN)
            .unwrap();
        game.add_local_member(Uuid::new(8).unwrap(), TeamId::new(2).unwrap())
            .unwrap();
        game.start(false).unwrap();

        let game_id = store.start_game(42, &game, false).unwrap();
        game.apply_action(member_id, GameAction::Increase(pos(0)))
            .unwrap();
        store.append_timeline(game_id, &game.timeline).unwrap();
        let first_event_count = game.timeline.len();

        game.apply_action(CharacterId::new(2).unwrap(), GameAction::Increase(pos(1)))
            .unwrap();
        store
            .append_timeline(game_id, &game.timeline[first_event_count..])
            .unwrap();

        let history = store.load(game_id).unwrap().unwrap();
        assert_eq!(history.room_id, 42);
        assert_eq!(history.timeline.len(), 2);
        assert_eq!(
            history.timeline[0],
            Timeline::Action(GameAction::Increase(pos(0))),
        );

        let replayed = history.replay().unwrap();
        let steps = history.replay_steps().unwrap();
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].board, history.snapshot.restore().board);
        assert_eq!(steps[2].board, game.board);
        assert_eq!(replayed.board, game.board);
        assert_eq!(replayed.current_turn, game.current_turn);
        assert_eq!(replayed.characters, game.characters);
        assert_eq!(store.load(GameId::MAX).unwrap(), None);

        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn save_checkpoint_and_load_roundtrip() {
        let path = std::env::temp_dir().join(format!("forup-checkpoint-{}.redb", Uuid::generate()));
        let store = GameStore::open(&path).unwrap();
        let mut game = Game::standard();
        let member_id = game
            .add_local_member(Uuid::new(7).unwrap(), TeamId::MIN)
            .unwrap();
        game.add_local_member(Uuid::new(8).unwrap(), TeamId::new(2).unwrap())
            .unwrap();
        game.start(false).unwrap();

        // Save initial checkpoint before any moves
        let board_before = game.board.clone();
        let checkpoint_id = store.save_checkpoint(42, &game).unwrap();

        // Make some moves
        game.apply_action(member_id, GameAction::Increase(pos(0)))
            .unwrap();
        game.apply_action(CharacterId::new(2).unwrap(), GameAction::Increase(pos(1)))
            .unwrap();

        // Save another checkpoint after moves
        let checkpoint_id2 = store.save_checkpoint(42, &game).unwrap();
        assert_ne!(checkpoint_id, checkpoint_id2);

        // Load first checkpoint - should have state before moves
        let loaded = store.load_checkpoint(checkpoint_id).unwrap().unwrap();
        assert_eq!(loaded.board, board_before);
        assert_eq!(loaded.timeline.len(), 0);

        // Load second checkpoint - should have state after moves
        let loaded2 = store.load_checkpoint(checkpoint_id2).unwrap().unwrap();
        assert_eq!(loaded2.board, game.board);
        assert_eq!(loaded2.current_turn, game.current_turn);
        assert_eq!(loaded2.characters, game.characters);
        assert_eq!(loaded2.timeline.len(), game.timeline.len());

        // List games for room
        let games = store.list_games(42).unwrap();
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].room_id, 42);
        assert_eq!(games[0].game_id, checkpoint_id);
        assert_eq!(games[1].game_id, checkpoint_id2);
        assert!(games[0].saved_at <= games[1].saved_at);

        // Loading nonexistent checkpoint returns None
        assert_eq!(store.load_checkpoint(GameId::MAX).unwrap(), None);

        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn start_game_saves_metadata() {
        let path = std::env::temp_dir().join(format!("forup-metadata-{}.redb", Uuid::generate()));
        let store = GameStore::open(&path).unwrap();
        let mut game = Game::standard();
        let _member_id = game
            .add_local_member(Uuid::new(7).unwrap(), TeamId::MIN)
            .unwrap();
        game.add_local_member(Uuid::new(8).unwrap(), TeamId::new(2).unwrap())
            .unwrap();
        game.start(false).unwrap();

        let game_id = store.start_game(42, &game, false).unwrap();

        let games = store.list_games(42).unwrap();
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].game_id, game_id);
        assert_eq!(games[0].room_id, 42);
        assert!(!games[0].is_completed);
        assert_eq!(games[0].turn_count, 0);

        drop(store);
        std::fs::remove_file(path).unwrap();
    }
}
