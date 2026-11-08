use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::num::NonZeroU8;

use glam::U8Vec2;

use crate::api::game::board::{
    Board, BotConfig, CharacterId, Game, GameAction, GameError, MemberId, TeamId, Timeline,
};
use crate::api::game::storage::{GameId, GameMetadata, GameStore, HistoryError};
use serde::{Deserialize, Serialize};

use super::{Uuid, UuidExt};

pub type RoomId = Uuid;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Status {
    #[default]
    Inactive,
    NotReady,
    Ready,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomMode {
    Offline,
    Online {
        global: bool,
        open: bool,
        // password
        // max_members
        // ...
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoomError {
    NotFound,
    NotOwner,
    PermissionDenied,
    MembersNotReady,
    CannotKickOwner,
    Closed,
    Offline,
    GlobalTransportUnavailable,
    Game(GameError),
    History(HistoryError),
}

impl From<GameError> for RoomError {
    fn from(value: GameError) -> Self {
        Self::Game(value)
    }
}

#[derive(Clone, Debug)]
pub struct Room {
    id: RoomId,
    owner: Uuid,
    mode: RoomMode,
    members: HashMap<Uuid, Status>,
    bots: HashMap<CharacterId, BotConfig>,
    game: Game,
    has_started: bool,
    history_store: Option<GameStore>,
    game_id: Option<GameId>,
}

impl Room {
    pub fn new(owner: Uuid, mode: RoomMode) -> Self {
        Self::with_id(owner, owner, mode)
    }

    fn with_id(id: RoomId, owner: Uuid, mode: RoomMode) -> Self {
        let mut game = Game::standard();
        game.join(owner, TeamId::MIN)
            .expect("a new room must have space for its owner");
        Self {
            id,
            owner,
            mode,
            members: HashMap::from([(owner, Status::NotReady)]),
            bots: HashMap::new(),
            game,
            has_started: false,
            history_store: None,
            game_id: None,
        }
    }

    pub fn id(&self) -> RoomId {
        self.id
    }

    fn with_history_store(mut self, store: GameStore) -> Self {
        self.history_store = Some(store);
        self
    }

    pub fn game_id(&self) -> Option<GameId> {
        self.game_id
    }

    pub fn owner(&self) -> Uuid {
        self.owner
    }

    pub fn mode(&self) -> RoomMode {
        self.mode
    }

    pub fn members(&self) -> &HashMap<Uuid, Status> {
        &self.members
    }

    pub fn bots(&self) -> &HashMap<CharacterId, BotConfig> {
        &self.bots
    }

    pub fn game(&self) -> &Game {
        &self.game
    }

    pub(crate) fn has_started(&self) -> bool {
        self.has_started
    }

    /// Lowest team id not yet used by any seat.
    pub fn next_team_id(&self) -> TeamId {
        let used_teams: BTreeSet<_> = self
            .game
            .characters
            .values()
            .map(|member| member.team_id)
            .collect();
        (1..=u8::MAX)
            .filter_map(TeamId::new)
            .find(|team_id| !used_teams.contains(team_id))
            .unwrap_or(TeamId::MIN)
    }

    /// Adds an extra hot-seat seat controlled by the owner. Local seats are not
    /// room members: they never gate `start` on readiness.
    pub fn add_local_member(&mut self, actor: Uuid) -> Result<CharacterId, RoomError> {
        self.ensure_owner(actor)?;
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted.into());
        }
        let team_id = self.next_team_id();
        Ok(self.game.add_local_member(actor, team_id)?)
    }

    /// Switches a seat between human control and a bot. Only before the game starts.
    pub fn set_member_kind(
        &mut self,
        actor: Uuid,
        member_id: CharacterId,
        is_bot: bool,
    ) -> Result<(), RoomError> {
        self.ensure_owner(actor)?;
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted.into());
        }
        let member = self
            .game
            .characters
            .get_mut(&member_id)
            .ok_or(GameError::CharacterNotFound)?;
        if is_bot {
            member.controller = MemberId::Bot;
            self.bots.insert(member_id, BotConfig::STANDARD);
        } else {
            member.controller = MemberId::Player(actor);
            self.bots.remove(&member_id);
        }
        Ok(())
    }

    pub fn set_board_size(&mut self, actor: Uuid, width: u8, height: u8) -> Result<(), RoomError> {
        self.ensure_owner(actor)?;
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted.into());
        }
        self.game.board.set_size(U8Vec2::new(width, height));
        Ok(())
    }

    pub fn set_automatic_distribution(
        &mut self,
        actor: Uuid,
        automatic: bool,
    ) -> Result<(), RoomError> {
        self.ensure_owner(actor)?;
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted.into());
        }
        self.game.automatic_distribution = automatic;
        Ok(())
    }

    pub fn reset(&mut self) {
        let size = self.game.board.size();
        let mut game = Game::new(Board::new_rect(size));
        game.join(self.owner, TeamId::MIN)
            .expect("a reset room must have space for its owner");
        self.game = game;
        self.bots.clear();
        self.has_started = false;
        self.game_id = None;
    }

    /// Plays out every consecutive bot turn directly on the game.
    fn run_bot_turns(&self, game: &mut Game) -> Result<(), GameError> {
        while game.is_started() {
            let Some(member_id) = game
                .current_turn
                .and_then(|turn| game.turn_order.get(turn).copied())
            else {
                break;
            };
            let Some(config) = self.bots.get(&member_id).copied() else {
                break;
            };
            let action = game.bot_action(member_id, &config);
            game.apply_action(member_id, action)?;
        }
        Ok(())
    }

    pub fn set_open(&mut self, actor: Uuid, open: bool) -> Result<(), RoomError> {
        self.ensure_owner(actor)?;
        match &mut self.mode {
            RoomMode::Offline => Err(RoomError::Offline),
            RoomMode::Online { open: current, .. } => {
                *current = open;
                Ok(())
            }
        }
    }

    pub fn join(&mut self, player: Uuid, team_id: TeamId) -> Result<(), RoomError> {
        match self.mode {
            RoomMode::Offline => return Err(RoomError::Offline),
            RoomMode::Online { global: true, .. } => {
                return Err(RoomError::GlobalTransportUnavailable);
            }
            RoomMode::Online { open: false, .. } => return Err(RoomError::Closed),
            RoomMode::Online {
                global: false,
                open: true,
            } => {}
        }
        if !self.members.contains_key(&player) {
            self.game.join(player, team_id)?;
            self.members.insert(player, Status::NotReady);
        }
        Ok(())
    }

    pub fn leave(&mut self, player: Uuid) -> Result<(), RoomError> {
        let controller = MemberId::Player(player);
        let mut game = self.game.clone();
        let first_new_event = game.timeline.len();
        let characters: Vec<_> = game
            .characters
            .iter()
            .filter(|(_, data)| data.controller == controller)
            .map(|(id, _)| *id)
            .collect();
        for id in characters {
            game.leave(id);
        }

        self.persist_timeline(&game.timeline[first_new_event..])?;

        self.members.remove(&player);
        self.game = game;
        self.compact_teams();
        Ok(())
    }

    pub fn set_ready(&mut self, actor: Uuid, ready: bool) -> Result<(), RoomError> {
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted.into());
        }
        let status = self
            .members
            .get_mut(&actor)
            .ok_or(RoomError::PermissionDenied)?;
        *status = if ready {
            Status::Ready
        } else {
            Status::NotReady
        };
        Ok(())
    }

    pub fn set_allow_team_change(&mut self, actor: Uuid, allow: bool) -> Result<(), RoomError> {
        self.ensure_owner(actor)?;
        self.game.set_allow_team_change(allow);
        Ok(())
    }

    pub fn change_team(
        &mut self,
        actor: Uuid,
        target_member: CharacterId,
        new_team: TeamId,
    ) -> Result<(), RoomError> {
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted.into());
        }

        let is_owner = actor == self.owner;
        let is_self = self
            .game
            .characters
            .get(&target_member)
            .is_some_and(|character| character.controller == MemberId::Player(actor));
        if !is_owner && (!is_self || !self.game.allow_team_change) {
            return Err(RoomError::PermissionDenied);
        }

        self.game.change_team(target_member, new_team)?;
        Ok(())
    }

    pub fn add_bot(&mut self, actor: Uuid, team_id: TeamId) -> Result<CharacterId, RoomError> {
        self.add_bot_with_config(actor, team_id, BotConfig::STANDARD)
    }

    pub fn add_bot_with_config(
        &mut self,
        actor: Uuid,
        team_id: TeamId,
        config: BotConfig,
    ) -> Result<CharacterId, RoomError> {
        self.ensure_owner(actor)?;
        let bot_id = self.game.add_bot(team_id)?;
        self.bots.insert(bot_id, config);
        Ok(bot_id)
    }

    pub fn kick_member(&mut self, actor: Uuid, member_id: CharacterId) -> Result<(), RoomError> {
        self.ensure_owner(actor)?;
        // In online rooms the owner's seat is the owner's only presence, so it
        // cannot be removed. Offline hot-seat rooms may drop every human seat
        // to watch bots play.
        if matches!(self.mode, RoomMode::Online { .. })
            && self
                .game
                .characters
                .get(&member_id)
                .is_some_and(|character| character.controller == MemberId::Player(self.owner))
        {
            return Err(RoomError::CannotKickOwner);
        }

        let controller = self
            .game
            .characters
            .get(&member_id)
            .ok_or(GameError::CharacterNotFound)?
            .controller;
        self.game.kick_member(member_id)?;
        match controller {
            MemberId::Player(player) => {
                if !self
                    .game
                    .characters
                    .values()
                    .any(|character| character.controller == controller)
                {
                    self.members.remove(&player);
                }
            }
            MemberId::Bot => {
                self.bots.remove(&member_id);
            }
        }
        self.compact_teams();
        Ok(())
    }

    /// Уплотняет номера команд до 1..K. Только до старта: после старта
    /// номера команд уже записаны в клетках доски.
    fn compact_teams(&mut self) {
        if self.game.is_started() {
            return;
        }
        let used: Vec<TeamId> = self
            .game
            .characters
            .values()
            .map(|member| member.team_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for member in self.game.characters.values_mut() {
            let index = used
                .iter()
                .position(|team| *team == member.team_id)
                .expect("the member's team is among used teams") as u8;
            member.team_id = NonZeroU8::new(index + 1).expect("team ids fit into 1..=255");
        }
    }

    pub fn start(&mut self, actor: Uuid, automatic_distribution: bool) -> Result<(), RoomError> {
        self.ensure_owner(actor)?;
        if self.members.values().any(|status| *status != Status::Ready) {
            return Err(RoomError::MembersNotReady);
        }
        let mut game = self.game.clone();
        game.start(automatic_distribution)?;

        if let Some(store) = &self.history_store {
            let game_id = store
                .start_game(self.id.get(), &game)
                .map_err(RoomError::History)?;
            self.game_id = Some(game_id);
        }

        self.run_bot_turns(&mut game)?;
        self.persist_timeline(&game.timeline)?;

        self.game = game;
        self.has_started = true;
        Ok(())
    }

    pub fn end_game(&mut self, actor: Uuid) -> Result<(), RoomError> {
        self.ensure_owner(actor)?;
        let mut game = self.game.clone();
        let first_new_event = game.timeline.len();
        game.end_game()?;

        self.persist_timeline(&game.timeline[first_new_event..])?;

        self.game = game;
        Ok(())
    }

    pub fn apply_action(
        &mut self,
        actor: Uuid,
        member_id: CharacterId,
        action: GameAction,
    ) -> Result<Option<TeamId>, RoomError> {
        let character = self
            .game
            .characters
            .get(&member_id)
            .ok_or(GameError::CharacterNotFound)?;
        if character.controller != MemberId::Player(actor) {
            return Err(RoomError::PermissionDenied);
        }

        let mut game = self.game.clone();
        let first_new_event = game.timeline.len();
        let result = game.apply_action(member_id, action)?;
        self.run_bot_turns(&mut game)?;
        self.persist_timeline(&game.timeline[first_new_event..])?;
        self.game = game;
        Ok(result)
    }

    /// Saves the current game state as a checkpoint so it can be resumed later.
    /// Requires the room to have a history store configured.
    pub fn save_game(&mut self) -> Result<GameId, RoomError> {
        let Some(store) = &self.history_store else {
            return Err(RoomError::History(HistoryError::Database(
                "no history store configured".into(),
            )));
        };
        let game_id = store
            .save_checkpoint(self.id.get(), &self.game)
            .map_err(RoomError::History)?;
        self.game_id = Some(game_id);
        Ok(game_id)
    }

    /// Returns metadata for all saved games in this room, ordered by save time.
    /// Requires the room to have a history store configured.
    pub fn past_games(&self) -> Result<Vec<GameMetadata>, RoomError> {
        let Some(store) = &self.history_store else {
            return Err(RoomError::History(HistoryError::Database(
                "no history store configured".into(),
            )));
        };
        store.list_games(self.id.get()).map_err(RoomError::History)
    }

    fn persist_timeline(&self, events: &[Timeline]) -> Result<(), RoomError> {
        if let (Some(store), Some(game_id)) = (&self.history_store, self.game_id) {
            store
                .append_timeline(game_id, events)
                .map_err(RoomError::History)?;
        }
        Ok(())
    }

    fn ensure_owner(&self, actor: Uuid) -> Result<(), RoomError> {
        if actor == self.owner {
            Ok(())
        } else {
            Err(RoomError::NotOwner)
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RoomDirectory {
    rooms: BTreeMap<RoomId, Room>,
    history_store: Option<GameStore>,
}

impl RoomDirectory {
    pub fn with_history_store(history_store: GameStore) -> Self {
        Self {
            rooms: BTreeMap::new(),
            history_store: Some(history_store),
        }
    }

    pub fn create(&mut self, owner: Uuid, mode: RoomMode) -> RoomId {
        let id = loop {
            let candidate = Uuid::generate();
            if !self.rooms.contains_key(&candidate) {
                break candidate;
            }
        };
        let room = Room::with_id(id, owner, mode);
        let room = if let Some(store) = &self.history_store {
            room.with_history_store(store.clone())
        } else {
            room
        };
        self.rooms.insert(id, room);
        id
    }

    pub fn get(&self, id: RoomId) -> Option<&Room> {
        self.rooms.get(&id)
    }

    pub fn get_mut(&mut self, id: RoomId) -> Option<&mut Room> {
        self.rooms.get_mut(&id)
    }

    pub fn join(&mut self, id: RoomId, player: Uuid, team_id: TeamId) -> Result<(), RoomError> {
        self.rooms
            .get_mut(&id)
            .ok_or(RoomError::NotFound)?
            .join(player, team_id)
    }

    pub fn leave(&mut self, id: RoomId, player: Uuid) -> Result<(), RoomError> {
        self.rooms
            .get_mut(&id)
            .ok_or(RoomError::NotFound)?
            .leave(player)
    }

    pub fn remove(&mut self, id: RoomId, actor: Uuid) -> Result<Room, RoomError> {
        let room = self.rooms.get(&id).ok_or(RoomError::NotFound)?;
        room.ensure_owner(actor)?;
        Ok(self.rooms.remove(&id).expect("room checked above"))
    }

    pub fn list_open_local(&self) -> impl Iterator<Item = &Room> {
        self.rooms.values().filter(|room| {
            matches!(
                room.mode,
                RoomMode::Online {
                    global: false,
                    open: true
                }
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid(id: u32) -> Uuid {
        Uuid::new(id).unwrap()
    }

    fn tid(id: u8) -> TeamId {
        TeamId::new(id).unwrap()
    }

    #[test]
    fn owner_can_open_close_and_players_can_join_only_when_open() {
        let owner = uid(10);
        let guest = uid(20);
        let mut directory = RoomDirectory::default();
        let room_id = directory.create(
            owner,
            RoomMode::Online {
                global: false,
                open: true,
            },
        );

        directory.join(room_id, guest, tid(1)).unwrap();
        assert_eq!(directory.get(room_id).unwrap().members().len(), 2);
        directory
            .get_mut(room_id)
            .unwrap()
            .set_open(owner, false)
            .unwrap();
        assert_eq!(
            directory.join(room_id, uid(30), tid(2)),
            Err(RoomError::Closed)
        );
        assert_eq!(
            directory.get_mut(room_id).unwrap().set_open(guest, true),
            Err(RoomError::NotOwner)
        );
        directory
            .get_mut(room_id)
            .unwrap()
            .set_open(owner, true)
            .unwrap();
        directory.join(room_id, uid(30), tid(2)).unwrap();
    }

    #[test]
    fn room_owns_game_management_permissions_and_member_sync() {
        let owner = uid(10);
        let guest = uid(20);
        let mut room = Room::new(
            owner,
            RoomMode::Online {
                global: false,
                open: true,
            },
        );
        room.join(guest, tid(1)).unwrap();
        let guest_character = room
            .game()
            .characters
            .iter()
            .find(|(_, character)| character.controller == MemberId::Player(guest))
            .map(|(&id, _)| id)
            .unwrap();
        let owner_character = room
            .game()
            .characters
            .iter()
            .find(|(_, character)| character.controller == MemberId::Player(owner))
            .map(|(&id, _)| id)
            .unwrap();

        assert_eq!(room.add_bot(guest, tid(2)), Err(RoomError::NotOwner));
        assert_eq!(room.change_team(guest, guest_character, tid(3)), Ok(()));
        assert_eq!(
            room.apply_action(guest, owner_character, GameAction::Skip),
            Err(RoomError::PermissionDenied)
        );
        room.set_allow_team_change(owner, false).unwrap();
        assert_eq!(
            room.change_team(guest, guest_character, tid(4)),
            Err(RoomError::PermissionDenied)
        );
        assert_eq!(
            room.kick_member(owner, owner_character),
            Err(RoomError::CannotKickOwner)
        );

        room.kick_member(owner, guest_character).unwrap();
        assert_eq!(room.members().len(), 1);
        assert!(room.members().contains_key(&owner));
        assert!(!room.members().contains_key(&guest));
        assert!(!room.game().characters.contains_key(&guest_character));

        let bot_id = room.add_bot(owner, tid(2)).unwrap();
        assert_eq!(room.bots().get(&bot_id), Some(&BotConfig::STANDARD));
        assert_eq!(room.members().len(), 1);
    }

    #[test]
    fn room_start_and_actions_are_persisted_for_replay() {
        use crate::api::game::storage::GameStore;
        use glam::U8Vec2;

        let path =
            std::env::temp_dir().join(format!("forup-room-history-{}.redb", Uuid::generate()));
        let store = GameStore::open(&path).unwrap();
        let mut directory = RoomDirectory::with_history_store(store.clone());
        let owner = uid(10);
        let guest = uid(20);
        let room_id = directory.create(
            owner,
            RoomMode::Online {
                global: false,
                open: true,
            },
        );
        let other_room_id = directory.create(
            owner,
            RoomMode::Online {
                global: false,
                open: true,
            },
        );
        assert_ne!(room_id, other_room_id);

        directory.join(room_id, guest, tid(2)).unwrap();
        let owner_character = directory
            .get(room_id)
            .unwrap()
            .game()
            .characters
            .iter()
            .find(|(_, character)| character.controller == MemberId::Player(owner))
            .map(|(&id, _)| id)
            .unwrap();
        assert_eq!(
            directory.get_mut(room_id).unwrap().start(owner, false),
            Err(RoomError::MembersNotReady)
        );

        directory
            .get_mut(room_id)
            .unwrap()
            .set_ready(owner, true)
            .unwrap();
        directory
            .get_mut(room_id)
            .unwrap()
            .set_ready(guest, true)
            .unwrap();
        directory
            .get_mut(room_id)
            .unwrap()
            .start(owner, false)
            .unwrap();

        let game_id = directory.get(room_id).unwrap().game_id().unwrap();
        directory
            .get_mut(room_id)
            .unwrap()
            .apply_action(owner, owner_character, GameAction::Increase(U8Vec2::ZERO))
            .unwrap();
        directory.get_mut(room_id).unwrap().end_game(owner).unwrap();

        assert_eq!(store.games_for_room(room_id.get()).unwrap(), vec![game_id]);
        let history = store.load(game_id).unwrap().unwrap();
        assert_eq!(history.snapshot.characters.len(), 2);

        assert_eq!(history.timeline.len(), 2);
        assert_eq!(
            history.timeline[0],
            Timeline::Action {
                member_id: owner_character,
                action: GameAction::Increase(U8Vec2::ZERO),
            }
        );
        assert_eq!(history.timeline[1], Timeline::End);
        let replayed = history.replay().unwrap();
        assert_eq!(replayed.board, directory.get(room_id).unwrap().game().board);
        assert_eq!(replayed.current_turn, None);

        drop(directory);
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn offline_room_cannot_be_joined_or_opened() {
        let mut directory = RoomDirectory::default();
        let room_id = directory.create(uid(10), RoomMode::Offline);
        assert_eq!(
            directory.join(room_id, uid(20), tid(1)),
            Err(RoomError::Offline)
        );
        assert_eq!(
            directory.get_mut(room_id).unwrap().set_open(uid(10), true),
            Err(RoomError::Offline)
        );
    }

    fn offline_lobby() -> Room {
        let mut room = Room::new(uid(10), RoomMode::Offline);
        room.set_board_size(uid(10), 8, 8).unwrap();
        room
    }

    #[test]
    fn lobby_settings_and_members_are_backed_by_the_game_model() {
        let mut room = Room::new(uid(10), RoomMode::Offline);
        assert_eq!((room.view_state().width, room.view_state().height), (8, 8));

        room.set_board_size(uid(10), 6, 5).unwrap();
        room.set_automatic_distribution(uid(10), false).unwrap();
        room.add_local_member(uid(10)).unwrap();
        room.add_bot(uid(10), room.next_team_id()).unwrap();
        let lobby = room.view_state();
        assert_eq!((lobby.width, lobby.height), (6, 5));
        assert!(!lobby.automatic_distribution);
        assert_eq!(lobby.members.len(), 3);
        assert_eq!(lobby.teams.len(), 3);
        assert!(lobby.members.iter().any(|member| member.is_bot));
        assert_eq!(room.set_board_size(uid(20), 6, 5), Err(RoomError::NotOwner));
    }

    #[test]
    fn starts_with_or_without_automatic_cell_distribution() {
        let mut distributed = offline_lobby();
        distributed
            .add_bot(uid(10), distributed.next_team_id())
            .unwrap();
        distributed.set_ready(uid(10), true).unwrap();
        distributed.start(uid(10), true).unwrap();
        let started = distributed.view_state();
        assert!(started.is_started);
        assert_eq!(started.teams[0].cell_count, 1);
        assert_eq!(started.teams[1].cell_count, 1);
        assert!(!started.legal_moves.is_empty());

        let mut empty = offline_lobby();
        empty.set_automatic_distribution(uid(10), false).unwrap();
        empty.add_local_member(uid(10)).unwrap();
        empty.set_ready(uid(10), true).unwrap();
        empty.start(uid(10), false).unwrap();
        let started = empty.view_state();
        assert!(!started.automatic_distribution);
        assert!(!started.can_decrease);
        assert!(!started.can_skip);
        assert!(started.teams.iter().all(|team| team.cell_count == 0));
        assert_eq!(started.legal_moves.len(), 64);
        assert_eq!(
            empty.apply_action(uid(10), CharacterId::MIN, GameAction::Skip),
            Err(RoomError::Game(GameError::FirstMoveRequiresEmptyCell))
        );
    }

    #[test]
    fn player_actions_run_bots_and_support_skip_and_surrender() {
        let mut room = offline_lobby();
        room.add_bot(uid(10), room.next_team_id()).unwrap();
        room.set_ready(uid(10), true).unwrap();
        room.start(uid(10), true).unwrap();
        let player_id = room.view_state().current_member_id.unwrap();
        let after_skip = {
            room.apply_action(uid(10), player_id, GameAction::Skip)
                .unwrap();
            room.view_state()
        };

        assert!(after_skip.is_started);
        assert_eq!(after_skip.current_member_id, Some(player_id));
        assert_eq!(room.game().stats[&player_id].skipped_moves, 1);
        assert_eq!(room.game().timeline.len(), 2);

        room.apply_action(uid(10), player_id, GameAction::Surrender)
            .unwrap();
        let game_over = room.view_state();
        assert!(game_over.is_game_over);
        assert_eq!(game_over.winner_team, Some(tid(2)));
    }

    #[test]
    fn members_can_change_teams_and_kinds_and_be_removed() {
        let mut room = offline_lobby();
        let second_player = room.add_local_member(uid(10)).unwrap();
        let bot = room.add_bot(uid(10), room.next_team_id()).unwrap();

        room.change_team(uid(10), second_player, tid(2)).unwrap();
        assert_eq!(
            room.view_state()
                .members
                .iter()
                .find(|member| member.id == second_player)
                .unwrap()
                .team_id,
            tid(2)
        );
        // Все игроки могут быть удалены - остаётся только бот для наблюдения.
        room.kick_member(uid(10), second_player).unwrap();
        room.kick_member(uid(10), CharacterId::MIN).unwrap();
        let bots_only = room.view_state();
        assert_eq!(bots_only.members.len(), 1);
        assert!(bots_only.members.iter().all(|member| member.is_bot));

        // Переключение вида участника делает бота игроком и наоборот.
        room.set_member_kind(uid(10), bot, false).unwrap();
        assert!(
            !room
                .view_state()
                .members
                .iter()
                .find(|member| member.id == bot)
                .unwrap()
                .is_bot
        );
        room.set_member_kind(uid(10), bot, true).unwrap();
        assert!(
            room.view_state()
                .members
                .iter()
                .find(|member| member.id == bot)
                .unwrap()
                .is_bot
        );
        assert_eq!(
            room.set_member_kind(uid(10), CharacterId::new(200).unwrap(), false),
            Err(RoomError::Game(GameError::CharacterNotFound))
        );

        // После удаления всех игроков игра стартовать не может.
        assert_eq!(
            room.start(uid(10), true),
            Err(RoomError::Game(GameError::NotEnoughMembers))
        );
    }

    #[test]
    fn reset_clears_the_game_but_keeps_the_board_size() {
        let mut room = offline_lobby();
        room.add_bot(uid(10), room.next_team_id()).unwrap();
        room.set_ready(uid(10), true).unwrap();
        room.start(uid(10), true).unwrap();
        room.reset();

        let state = room.view_state();
        assert!(!state.is_started);
        assert!(!state.is_game_over);
        assert_eq!((state.width, state.height), (8, 8));
        assert_eq!(state.members.len(), 1);
        assert!(!state.members[0].is_bot);
        assert_eq!(state.teams.len(), 1);
    }

    #[test]
    fn save_game_and_past_games() {
        use crate::api::game::storage::GameStore;
        use glam::U8Vec2;

        let path = std::env::temp_dir().join(format!("forup-room-save-{}.redb", Uuid::generate()));
        let store = GameStore::open(&path).unwrap();
        let mut directory = RoomDirectory::with_history_store(store.clone());

        let owner = uid(10);
        let room_id = directory.create(
            owner,
            RoomMode::Online {
                global: false,
                open: true,
            },
        );

        directory.join(room_id, uid(20), tid(2)).unwrap();
        directory
            .get_mut(room_id)
            .unwrap()
            .set_ready(owner, true)
            .unwrap();
        directory
            .get_mut(room_id)
            .unwrap()
            .set_ready(uid(20), true)
            .unwrap();
        directory
            .get_mut(room_id)
            .unwrap()
            .start(owner, true)
            .unwrap();

        let room = directory.get(room_id).unwrap();
        let player_id = room.view_state().current_member_id.unwrap();
        let _ = room;

        // Make a move
        directory
            .get_mut(room_id)
            .unwrap()
            .apply_action(owner, player_id, GameAction::Increase(U8Vec2::ZERO))
            .unwrap();

        // Save checkpoint
        let checkpoint_id = directory.get_mut(room_id).unwrap().save_game().unwrap();

        // List past games - start() also saves metadata, so there are 2 games
        let past = directory.get(room_id).unwrap().past_games().unwrap();
        assert_eq!(past.len(), 2);
        // The latest save should be our checkpoint
        assert_eq!(past[1].game_id, checkpoint_id);
        assert_eq!(past[1].room_id, room_id.get());
        assert!(!past[1].is_completed);

        // Load checkpoint and verify state
        let loaded = store.load_checkpoint(checkpoint_id).unwrap().unwrap();
        assert_eq!(loaded.board, directory.get(room_id).unwrap().game().board);

        drop(directory);
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn save_game_without_history_store_errors() {
        let mut directory = RoomDirectory::default();
        let room_id = directory.create(
            uid(10),
            RoomMode::Online {
                global: false,
                open: true,
            },
        );
        assert_eq!(
            directory.get_mut(room_id).unwrap().save_game(),
            Err(RoomError::History(
                crate::api::game::storage::HistoryError::Database(
                    "no history store configured".into(),
                )
            ))
        );
    }
}
