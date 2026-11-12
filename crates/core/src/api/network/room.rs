use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::num::NonZeroU8;

use crate::api::game::board::{
    Board, BoardCode, BotConfig, CharacterId, ExplosionMode, Game, GameAction, GameError, MemberId,
    TeamId,
};
use serde::{Deserialize, Serialize};

use super::Uuid;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
}

impl From<GameError> for RoomError {
    fn from(value: GameError) -> Self {
        Self::Game(value)
    }
}

/// Комната целиком: режим, участники, права и партия. Сервер хранит комнаты
/// в `BTreeMap<RoomId, Room>` и сохраняет карту в базу; идентичность комнаты —
/// её ключ в этой карте, поэтому отдельного поля идентификатора нет.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Room {
    mode: RoomMode,
    members: HashMap<Uuid, MemberData>,
    /// Room-wide policy used by members without an override.
    rights: GameRights,
    bots: HashMap<CharacterId, BotConfig>,
    game: Game,
    automatic_distribution: bool,
}

impl Room {
    pub fn new(owner: Uuid, mode: RoomMode) -> Self {
        let mut game = Game::standard();
        game.join(owner, TeamId::MIN)
            .expect("a new room must have space for its owner");
        Self {
            mode,
            members: HashMap::from([(owner, MemberData::admin(Status::NotReady))]),
            rights: GameRights::default(),
            bots: HashMap::new(),
            game,
            automatic_distribution: true,
        }
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

    /// Lowest team id not yet used by any seat.
    pub fn next_team_id(&self) -> TeamId {
        let used_teams: BTreeSet<_> = self
            .game
            .characters()
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
        let controller = if is_bot {
            self.bots.insert(member_id, BotConfig::STANDARD);
            MemberId::Bot
        } else {
            self.bots.remove(&member_id);
            MemberId::Player(actor)
        };
        Ok(self.game.set_controller(member_id, controller)?)
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
        Ok(self.game.set_board_size(size)?)
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
        let mode = if lossless {
            ExplosionMode::Lossless
        } else {
            ExplosionMode::Lossy
        };
        Ok(self.game.set_explosion_mode(mode)?)
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
        let size = self.game.board().size();
        let explosion_mode = self.game.explosion_mode();
        let safe_radius = self.game.safe_radius();
        let automatic = self.automatic_distribution;
        let mut game = Game::new(Board::new(size));
        game.set_explosion_mode(explosion_mode)
            .expect("a fresh game accepts any explosion mode");

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
    }

    /// Plays out every consecutive bot turn directly on the game.
    fn run_bot_turns(&self, game: &mut Game) -> Result<(), GameError> {
        while game.is_started() {
            let Some(member_id) = game
                .current_turn()
                .and_then(|turn| game.characters().get_index(turn).map(|(id, _)| *id))
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
        let characters: Vec<_> = self
            .game
            .characters()
            .iter()
            .filter(|(_, data)| data.controller == controller)
            .map(|(id, _)| *id)
            .collect();
        for id in characters {
            self.game.leave(id);
        }

        self.members.remove(&player);
        self.promote_admin_if_needed();
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
            .characters()
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
                .characters()
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
            .characters()
            .get(&member_id)
            .ok_or(GameError::CharacterNotFound)?
            .controller;
        self.game.kick_member(member_id)?;
        match controller {
            MemberId::Player(player) => {
                if !self
                    .game
                    .characters()
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
            .characters()
            .values()
            .map(|member| member.team_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let remap: Vec<(CharacterId, u8)> = self
            .game
            .characters()
            .iter()
            .map(|(&id, member)| {
                let index = used
                    .iter()
                    .position(|team| *team == member.team_id)
                    .expect("the member's team is among used teams");
                (id, index as u8 + 1)
            })
            .collect();
        for (member, team_number) in remap {
            if let Some(new_team) = NonZeroU8::new(team_number) {
                self.game
                    .change_team(member, new_team)
                    .expect("compaction only renumbers existing members");
            }
        }
    }

    pub fn start(&mut self, actor: Uuid, automatic_distribution: bool) -> Result<(), RoomError> {
        self.ensure_owner(actor)?;
        // Only members who actually control a seat gate the start; an operator
        // without seats (e.g. after kicking everyone in a hot-seat room) does
        // not block the game from reporting the real problem.
        let seated: BTreeSet<Uuid> = self
            .game
            .characters()
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

        self.run_bot_turns(&mut game)?;

        self.game = game;
        Ok(())
    }

    pub fn end_game(&mut self, actor: Uuid) -> Result<(), RoomError> {
        self.ensure_owner(actor)?;
        self.game.end_game()?;
        Ok(())
    }

    pub fn apply_action(
        &mut self,
        actor: Uuid,
        member_id: CharacterId,
        action: GameAction,
    ) -> Result<Option<TeamId>, RoomError> {
        let controller = self
            .game
            .characters()
            .get(&member_id)
            .ok_or(GameError::CharacterNotFound)?
            .controller;
        if controller != MemberId::Player(actor) {
            return Err(RoomError::PermissionDenied);
        }

        let mut game = self.game.clone();
        let result = game.apply_action(member_id, action)?;
        self.run_bot_turns(&mut game)?;
        self.game = game;
        Ok(result)
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
}

impl RoomDirectory {
    /// Выбирает следующий свободный порядковый идентификатор комнаты.
    fn next_room_id(&self) -> RoomId {
        let taken = |id: u32| Uuid::new(id).filter(|&id| !self.rooms.contains_key(&id));
        let next = self
            .rooms
            .last_key_value()
            .map_or(1, |(&id, _)| id.get().saturating_add(1));
        taken(next)
            .or_else(|| (1..=u32::MAX).find_map(taken))
            .expect("room id space exhausted")
    }

    pub fn create(&mut self, owner: Uuid, mode: RoomMode) -> RoomId {
        let id = self.next_room_id();
        self.rooms.insert(id, Room::new(owner, mode));
        id
    }

    /// Восстанавливает комнату под конкретным ключом (например, из базы).
    pub fn insert(&mut self, id: RoomId, room: Room) {
        self.rooms.insert(id, room);
    }

    pub fn rooms(&self) -> &BTreeMap<RoomId, Room> {
        &self.rooms
    }

    pub fn is_empty(&self) -> bool {
        self.rooms.is_empty()
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
            .characters()
            .iter()
            .find(|(_, character)| character.controller == MemberId::Player(guest))
            .map(|(&id, _)| id)
            .unwrap();
        let owner_character = room
            .game()
            .characters()
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
        assert!(!room.game().characters().contains_key(&guest_character));

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
            .characters()
            .iter()
            .find(|(_, character)| character.controller == MemberId::Player(owner))
            .map(|(&id, _)| id)
            .unwrap();
        let guest_character = room
            .game()
            .characters()
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
            .characters()
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
            .characters()
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
}
