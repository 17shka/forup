use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::num::NonZeroU8;

use crate::api::game::board::{
    Board, BoardCode, BotConfig, CharacterId, ExplosionMode, Game, GameAction, GameError, MemberId,
    TeamId, Timeline,
};
use crate::api::game::storage::{GameId, GameMetadata, GameStore, HistoryError};
use serde::{Deserialize, Serialize};

use super::{Uuid, UuidExt};

pub type RoomId = Uuid;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Rights {
    #[default]
    Nope,
    Own,
    All,
}

impl Rights {
    /// Whether this right allows acting on a character, given that it belongs
    /// to the acting player.
    pub fn allows(self, is_own: bool) -> bool {
        match self {
            Rights::Nope => false,
            Rights::Own => is_own,
            Rights::All => true,
        }
    }
}

/// Per-member overrides. An unset field inherits the room-wide [`GameRights`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PlayerRights {
    teams: Option<Rights>,
    // добавлять/удалять/изменять владельца
    characters: Option<bool>,
    game_rule: Option<bool>,
}

impl PlayerRights {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_teams(mut self, teams: Rights) -> Self {
        self.teams = Some(teams);
        self
    }

    pub fn with_characters(mut self, allowed: bool) -> Self {
        self.characters = Some(allowed);
        self
    }

    pub fn with_game_rule(mut self, allowed: bool) -> Self {
        self.game_rule = Some(allowed);
        self
    }

    pub fn teams(&self) -> Option<Rights> {
        self.teams
    }

    pub fn characters(&self) -> Option<bool> {
        self.characters
    }

    pub fn game_rule(&self) -> Option<bool> {
        self.game_rule
    }
}

/// Room-wide game rights. Members may override them individually through
/// [`PlayerRights`], while administrators ([`MemberRights::All`]) always have
/// every right.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GameRights {
    teams: Rights,
    characters: bool,
    game_rule: bool,
}

impl Default for GameRights {
    fn default() -> Self {
        // By default everyone may move their own characters, while the roster
        // and the rules stay in the administrator's hands.
        Self {
            teams: Rights::Own,
            characters: false,
            game_rule: false,
        }
    }
}

impl GameRights {
    pub fn new(teams: Rights, characters: bool, game_rule: bool) -> Self {
        Self {
            teams,
            characters,
            game_rule,
        }
    }

    pub fn teams(&self) -> Rights {
        self.teams
    }

    pub fn characters(&self) -> bool {
        self.characters
    }

    pub fn game_rule(&self) -> bool {
        self.game_rule
    }

    pub fn with_teams(mut self, teams: Rights) -> Self {
        self.teams = teams;
        self
    }

    pub fn with_characters(mut self, characters: bool) -> Self {
        self.characters = characters;
        self
    }

    pub fn with_game_rule(mut self, game_rule: bool) -> Self {
        self.game_rule = game_rule;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MemberRights {
    /// Administrator: every right, regardless of the room policy.
    All,
    /// Regular member: inherits the room policy and may override it.
    Custom(PlayerRights),
}

impl Default for MemberRights {
    fn default() -> Self {
        MemberRights::Custom(PlayerRights::default())
    }
}

impl MemberRights {
    pub fn is_admin(self) -> bool {
        matches!(self, MemberRights::All)
    }

    /// Resolves the member's rights against the room-wide policy.
    pub fn resolve(self, game: &GameRights) -> GameRights {
        match self {
            MemberRights::All => GameRights::new(Rights::All, true, true),
            MemberRights::Custom(player) => GameRights {
                teams: player.teams.unwrap_or(game.teams),
                characters: player.characters.unwrap_or(game.characters),
                game_rule: player.game_rule.unwrap_or(game.game_rule),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MemberData {
    status: Status,
    rights: MemberRights,
}

impl MemberData {
    pub fn new(status: Status, rights: MemberRights) -> Self {
        Self { status, rights }
    }

    /// The room creator and any promoted administrator.
    pub fn admin(status: Status) -> Self {
        Self::new(status, MemberRights::All)
    }

    /// A regular member that inherits the room-wide policy.
    pub fn player(status: Status) -> Self {
        Self::new(status, MemberRights::Custom(PlayerRights::default()))
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn rights(&self) -> MemberRights {
        self.rights
    }

    pub fn is_admin(&self) -> bool {
        self.rights.is_admin()
    }
}

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
        global: bool, //lan/wan
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
    CannotDemoteOwner,
    MemberNotFound,
    InvalidBoardSize,
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
    mode: RoomMode,
    members: HashMap<Uuid, MemberData>,
    /// Room-wide policy used by members without an override.
    rights: GameRights,
    bots: HashMap<CharacterId, BotConfig>,
    game: Game,
    automatic_distribution: bool,
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
            mode,
            members: HashMap::from([(owner, MemberData::admin(Status::NotReady))]),
            rights: GameRights::default(),
            bots: HashMap::new(),
            game,
            automatic_distribution: true,
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

    pub fn owner(&self) -> Option<Uuid> {
        self.members
            .iter()
            .filter(|(_, data)| data.is_admin())
            .map(|(id, _)| *id)
            .min()
    }

    pub fn mode(&self) -> RoomMode {
        self.mode
    }

    pub fn members(&self) -> &HashMap<Uuid, MemberData> {
        &self.members
    }

    pub fn member(&self, id: Uuid) -> Option<&MemberData> {
        self.members.get(&id)
    }

    pub fn member_rights(&self, id: Uuid) -> Option<MemberRights> {
        self.members.get(&id).map(MemberData::rights)
    }

    /// The room-wide policy shared by members without an override.
    pub fn rights(&self) -> GameRights {
        self.rights
    }

    /// Rights a member actually has after applying their personal overrides.
    pub fn effective_rights(&self, id: Uuid) -> Option<GameRights> {
        self.members
            .get(&id)
            .map(|member| member.rights.resolve(&self.rights))
    }

    pub fn is_admin(&self, id: Uuid) -> bool {
        self.members.get(&id).is_some_and(MemberData::is_admin)
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
        self.ensure_characters(actor)?;
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
        self.ensure_characters(actor)?;
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
        self.ensure_game_rule(actor)?;
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted.into());
        }
        let size = NonZeroU8::new(width)
            .zip(NonZeroU8::new(height))
            .and_then(|(w, h)| BoardCode::from_size(w, h))
            .ok_or(RoomError::InvalidBoardSize)?;
        self.game.board.set_size(size);
        Ok(())
    }

    pub fn set_automatic_distribution(
        &mut self,
        actor: Uuid,
        automatic: bool,
    ) -> Result<(), RoomError> {
        self.ensure_game_rule(actor)?;
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted.into());
        }
        self.automatic_distribution = automatic;
        Ok(())
    }

    pub fn set_lossless_explosions(
        &mut self,
        actor: Uuid,
        lossless: bool,
    ) -> Result<(), RoomError> {
        self.ensure_game_rule(actor)?;
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted.into());
        }
        self.game.explosion_mode = if lossless {
            ExplosionMode::Lossless
        } else {
            ExplosionMode::Lossy
        };
        Ok(())
    }

    /// Sets the squared safe radius for start cells. The value is clamped to
    /// [`Game::max_safe_radius`]; the stored value is returned.
    pub fn set_safe_radius(&mut self, actor: Uuid, radius_sq: u8) -> Result<u8, RoomError> {
        self.ensure_game_rule(actor)?;
        Ok(self.game.set_safe_radius(radius_sq)?)
    }

    /// Replaces the room-wide policy. Only administrators may do this.
    pub fn set_game_rights(&mut self, actor: Uuid, rights: GameRights) -> Result<(), RoomError> {
        self.ensure_owner(actor)?;
        self.rights = rights;
        Ok(())
    }

    /// Overrides a single member's rights. Only administrators may do this and
    /// the last administrator cannot demote themselves.
    pub fn set_member_rights(
        &mut self,
        actor: Uuid,
        target: Uuid,
        rights: MemberRights,
    ) -> Result<(), RoomError> {
        self.ensure_owner(actor)?;
        let target_is_admin = self.members.get(&target).is_some_and(MemberData::is_admin);
        if target_is_admin && !rights.is_admin() && self.admin_count() <= 1 {
            return Err(RoomError::CannotDemoteOwner);
        }
        let member = self
            .members
            .get_mut(&target)
            .ok_or(RoomError::MemberNotFound)?;
        member.rights = rights;
        Ok(())
    }

    pub fn reset(&mut self) {
        let size = self.game.board.size();
        let explosion_mode = self.game.explosion_mode;
        let safe_radius = self.game.safe_radius();
        let automatic = self.automatic_distribution;
        let mut game = Game::new(Board::new(size));
        game.explosion_mode = explosion_mode;

        // Re-seat every member on their own team so the lobby stays consistent
        // with the member list after the game is cleared.
        let mut member_ids: Vec<_> = self.members.keys().copied().collect();
        member_ids.sort_unstable();
        for (index, member) in member_ids.into_iter().enumerate() {
            let team_id = TeamId::new((index as u8).saturating_add(1)).unwrap_or(TeamId::MIN);
            game.join(member, team_id)
                .expect("a reset room must have space for every member");
        }
        for member in self.members.values_mut() {
            member.status = Status::NotReady;
        }

        // Clamp against the re-seated teams: the limit depends on their count.
        let _ = game.set_safe_radius(safe_radius);
        self.game = game;
        self.automatic_distribution = automatic;
        self.bots.clear();
        self.has_started = false;
        self.game_id = None;
    }

    /// Plays out every consecutive bot turn directly on the game.
    fn run_bot_turns(&self, game: &mut Game) -> Result<(), GameError> {
        while game.is_started() {
            let Some(member_id) = game
                .current_turn
                .and_then(|turn| game.characters.get_index(turn).map(|(id, _)| *id))
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
            // The first member of an unowned room takes over as administrator.
            let rights = if self.members.values().any(MemberData::is_admin) {
                MemberRights::Custom(PlayerRights::default())
            } else {
                MemberRights::All
            };
            self.members
                .insert(player, MemberData::new(Status::NotReady, rights));
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
        self.promote_admin_if_needed();
        self.game = game;
        self.compact_teams();
        Ok(())
    }

    pub fn set_ready(&mut self, actor: Uuid, ready: bool) -> Result<(), RoomError> {
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted.into());
        }
        let member = self
            .members
            .get_mut(&actor)
            .ok_or(RoomError::PermissionDenied)?;
        member.status = if ready {
            Status::Ready
        } else {
            Status::NotReady
        };
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

        let is_own = self
            .game
            .characters
            .get(&target_member)
            .is_some_and(|character| character.controller == MemberId::Player(actor));
        let may_change = self
            .effective_rights(actor)
            .is_some_and(|rights| rights.teams.allows(is_own));
        if !may_change {
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
        self.ensure_characters(actor)?;
        let bot_id = self.game.add_bot(team_id)?;
        self.bots.insert(bot_id, config);
        Ok(bot_id)
    }

    pub fn kick_member(&mut self, actor: Uuid, member_id: CharacterId) -> Result<(), RoomError> {
        self.ensure_characters(actor)?;
        // In online rooms a seat is its player's only presence, so an
        // administrator's seat cannot be removed by another player. Offline
        // hot-seat rooms may drop every human seat to watch bots play.
        if matches!(self.mode, RoomMode::Online { .. }) {
            let is_owner_seat = self
                .game
                .characters
                .get(&member_id)
                .is_some_and(|character| match character.controller {
                    MemberId::Player(player) => self.is_admin(player),
                    MemberId::Bot => false,
                });
            if is_owner_seat {
                return Err(RoomError::CannotKickOwner);
            }
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
                    && !self.is_admin(player)
                {
                    self.members.remove(&player);
                    self.promote_admin_if_needed();
                }
            }
            MemberId::Bot => {
                self.bots.remove(&member_id);
            }
        }
        self.compact_teams();
        Ok(())
    }

    /// Keeps at least one administrator in a non-empty room, e.g. after the
    /// owner leaves or is removed. The lowest member id is promoted so the
    /// result is deterministic.
    fn promote_admin_if_needed(&mut self) {
        if self.members.is_empty() || self.members.values().any(MemberData::is_admin) {
            return;
        }
        let Some(id) = self.members.keys().copied().min() else {
            return;
        };
        if let Some(member) = self.members.get_mut(&id) {
            member.rights = MemberRights::All;
        }
    }

    fn admin_count(&self) -> usize {
        self.members
            .values()
            .filter(|member| member.is_admin())
            .count()
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
        // Only members who actually control a seat gate the start; an operator
        // without seats (e.g. after kicking everyone in a hot-seat room) does
        // not block the game from reporting the real problem.
        let seated: BTreeSet<Uuid> = self
            .game
            .characters
            .values()
            .filter_map(|character| match character.controller {
                MemberId::Player(player) => Some(player),
                MemberId::Bot => None,
            })
            .collect();
        if self
            .members
            .iter()
            .any(|(id, member)| seated.contains(id) && member.status != Status::Ready)
        {
            return Err(RoomError::MembersNotReady);
        }
        let mut game = self.game.clone();
        game.start(automatic_distribution)?;

        if let Some(store) = &self.history_store {
            let game_id = store
                .start_game(self.id.get(), &game, automatic_distribution)
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

    /// Requires the actor to be an administrator (full rights).
    fn ensure_owner(&self, actor: Uuid) -> Result<(), RoomError> {
        if self.is_admin(actor) {
            Ok(())
        } else {
            Err(RoomError::NotOwner)
        }
    }

    fn resolved_rights(&self, actor: Uuid) -> Result<GameRights, RoomError> {
        self.effective_rights(actor)
            .ok_or(RoomError::PermissionDenied)
    }

    fn ensure_game_rule(&self, actor: Uuid) -> Result<(), RoomError> {
        if self.resolved_rights(actor)?.game_rule {
            Ok(())
        } else {
            Err(RoomError::PermissionDenied)
        }
    }

    fn ensure_characters(&self, actor: Uuid) -> Result<(), RoomError> {
        if self.resolved_rights(actor)?.characters {
            Ok(())
        } else {
            Err(RoomError::PermissionDenied)
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

    fn pos(slot: u8) -> BoardCode {
        BoardCode::from_slot(slot)
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

        assert_eq!(
            room.add_bot(guest, tid(2)),
            Err(RoomError::PermissionDenied)
        );
        assert_eq!(room.change_team(guest, guest_character, tid(3)), Ok(()));
        assert_eq!(
            room.apply_action(guest, owner_character, GameAction::Skip),
            Err(RoomError::PermissionDenied)
        );
        room.set_game_rights(owner, GameRights::new(Rights::Nope, false, false))
            .unwrap();
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
    fn rights_default_to_own_team_changes_for_members() {
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
        let owner_character = room
            .game()
            .characters
            .iter()
            .find(|(_, character)| character.controller == MemberId::Player(owner))
            .map(|(&id, _)| id)
            .unwrap();
        let guest_character = room
            .game()
            .characters
            .iter()
            .find(|(_, character)| character.controller == MemberId::Player(guest))
            .map(|(&id, _)| id)
            .unwrap();

        assert!(room.is_admin(owner));
        assert!(!room.is_admin(guest));
        assert_eq!(room.rights(), GameRights::default());
        assert_eq!(room.effective_rights(guest), Some(GameRights::default()));

        // Guests may move their own seats but not someone else's.
        assert_eq!(room.change_team(guest, guest_character, tid(3)), Ok(()));
        assert_eq!(
            room.change_team(guest, owner_character, tid(3)),
            Err(RoomError::PermissionDenied)
        );

        // The roster and the rules stay in the administrator's hands.
        assert_eq!(
            room.add_bot(guest, tid(2)),
            Err(RoomError::PermissionDenied)
        );
        assert_eq!(
            room.set_board_size(guest, 5, 5),
            Err(RoomError::PermissionDenied)
        );
        assert_eq!(
            room.set_game_rights(guest, GameRights::new(Rights::All, true, true)),
            Err(RoomError::NotOwner)
        );
    }

    #[test]
    fn member_overrides_extend_the_room_policy() {
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

        room.set_member_rights(
            owner,
            guest,
            MemberRights::Custom(
                PlayerRights::new()
                    .with_teams(Rights::All)
                    .with_characters(true)
                    .with_game_rule(true),
            ),
        )
        .unwrap();

        assert_eq!(
            room.effective_rights(guest),
            Some(GameRights::new(Rights::All, true, true))
        );
        assert_eq!(room.change_team(guest, guest_character, tid(2)), Ok(()));
        assert!(room.add_bot(guest, tid(3)).is_ok());
        assert_eq!(room.set_board_size(guest, 5, 5), Ok(()));
    }

    #[test]
    fn global_rights_tighten_members_but_never_administrators() {
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

        room.set_game_rights(owner, GameRights::new(Rights::Nope, true, false))
            .unwrap();

        // The policy denies team changes but grants roster management.
        assert_eq!(
            room.change_team(guest, guest_character, tid(2)),
            Err(RoomError::PermissionDenied)
        );
        assert!(room.add_bot(guest, tid(3)).is_ok());
        assert_eq!(
            room.set_board_size(guest, 5, 5),
            Err(RoomError::PermissionDenied)
        );

        // Administrators ignore the room policy.
        assert_eq!(room.change_team(owner, guest_character, tid(2)), Ok(()));
    }

    #[test]
    fn the_last_administrator_is_preserved() {
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

        assert_eq!(
            room.set_member_rights(owner, owner, MemberRights::Custom(PlayerRights::default())),
            Err(RoomError::CannotDemoteOwner)
        );

        // Once there are two administrators, one of them may step down.
        room.set_member_rights(owner, guest, MemberRights::All)
            .unwrap();
        assert_eq!(room.admin_count(), 2);
        room.set_member_rights(owner, owner, MemberRights::Custom(PlayerRights::default()))
            .unwrap();
        assert_eq!(room.owner(), Some(guest));
    }

    #[test]
    fn leaving_promotes_the_next_member_to_administrator() {
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

        room.leave(owner).unwrap();
        assert_eq!(room.owner(), Some(guest));
        assert!(room.is_admin(guest));
    }

    #[test]
    fn room_start_and_actions_are_persisted_for_replay() {
        use crate::api::game::storage::GameStore;

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
            .apply_action(owner, owner_character, GameAction::Increase(pos(0)))
            .unwrap();
        directory.get_mut(room_id).unwrap().end_game(owner).unwrap();

        assert_eq!(store.games_for_room(room_id.get()).unwrap(), vec![game_id]);
        let history = store.load(game_id).unwrap().unwrap();
        assert_eq!(history.snapshot.characters.len(), 2);

        assert_eq!(history.timeline.len(), 2);
        assert_eq!(
            history.timeline[0],
            Timeline::Action(GameAction::Increase(pos(0))),
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
        // Ширина и высота нормализуются: 6 × 5 и 5 × 6 — один размер.
        assert_eq!((lobby.width, lobby.height), (5, 6));
        assert_eq!(lobby.members.len(), 3);
        assert_eq!(lobby.teams.len(), 3);
        assert!(lobby.members.iter().any(|member| member.is_bot));
        assert_eq!(
            room.set_board_size(uid(20), 6, 5),
            Err(RoomError::PermissionDenied)
        );
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
        assert_eq!(room.game().characters[&player_id].stats.skipped_moves, 1);
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
            .apply_action(owner, player_id, GameAction::Increase(pos(0)))
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
