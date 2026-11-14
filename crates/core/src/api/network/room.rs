use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::num::NonZeroU8;

use crate::api::game::board::{
    BoardCode, BotConfig, CharacterId, ExplosionMode, Game, GameError, MemberId, TeamId,
};
use serde::{Deserialize, Serialize};

use super::Uuid;
use super::protocol::{LobbySetup, ReplayStep};

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
        // global: bool, //lan/wan
        open: bool,
        // password
        // max_members
        // ...
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoomError {
    NotFound,
    NotOwner,
    PermissionDenied,
    MembersNotReady,
    CannotKickOwner,
    CannotDemoteOwner,
    MemberNotFound,
    InvalidBoardSize,
    /// Лобби-снимок не годится: неверный размер доски или команда места.
    InvalidLobby,
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
    /// Счётчик изменений комнаты: растёт с каждым успешным изменением.
    /// Клиенты отбрасывают устаревшие снапшоты по этому счётчику.
    seq: u64,
    /// Шаги последнего действия (старт или ход с ответами ботов): по ним
    /// клиент повторяет чужие ходы с анимацией волна за волной.
    last_steps: Vec<ReplayStep>,
    /// Отображаемые номера ботов («Бот 1», «Бот 2», ...).
    bot_labels: HashMap<CharacterId, u8>,
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
            seq: 0,
            last_steps: Vec::new(),
            bot_labels: HashMap::new(),
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

    /// Счётчик изменений комнаты.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Шаги последнего действиядля серверного «реплея» на клиенте.
    pub fn last_steps(&self) -> &[ReplayStep] {
        &self.last_steps
    }

    /// Отображаемые номера ботов по идентификаторам персонажей.
    pub fn bot_labels(&self) -> &HashMap<CharacterId, u8> {
        &self.bot_labels
    }

    pub fn game(&self) -> &Game {
        &self.game
    }

    /// Whether `Game::start` spreads the starting cells automatically.
    pub fn automatic_distribution(&self) -> bool {
        self.automatic_distribution
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
        let id = self.game.add_local_member(actor, team_id)?;
        self.touch();
        Ok(id)
    }

    /// Переназначает контроллера места: любой игрок комнаты или бот.
    /// Требует права на изменение участников (по умолчанию — только
    /// администратор) и разрешено только до старта партии.
    pub fn set_member_controller(
        &mut self,
        actor: Uuid,
        member_id: CharacterId,
        controller: MemberId,
    ) -> Result<(), RoomError> {
        self.ensure_characters(actor)?;
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted.into());
        }
        if !self.game.characters().contains_key(&member_id) {
            return Err(GameError::CharacterNotFound.into());
        }
        if let MemberId::Player(player) = controller {
            // Контроллером может стать только участник комнаты.
            if !self.members.contains_key(&player) {
                return Err(RoomError::MemberNotFound);
            }
        }
        self.game.set_controller(member_id, controller)?;
        match controller {
            MemberId::Bot => {
                self.bots.insert(member_id, BotConfig::STANDARD);
                if !self.bot_labels.contains_key(&member_id) {
                    let label = self.lowest_free_bot_label();
                    self.bot_labels.insert(member_id, label);
                }
            }
            MemberId::Player(_) => {
                self.bots.remove(&member_id);
                self.bot_labels.remove(&member_id);
            }
        }
        self.lobby_changed();
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
        self.game.set_board_size(size)?;
        self.touch();
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
        self.touch();
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
        self.game.set_explosion_mode(mode)?;
        self.touch();
        Ok(())
    }

    /// Sets the squared safe radius for start cells. The value is clamped to
    /// [`Game::max_safe_radius`]; the stored value is returned.
    pub fn set_safe_radius(&mut self, actor: Uuid, radius_sq: u8) -> Result<u8, RoomError> {
        self.ensure_game_rule(actor)?;
        let clamped = self.game.set_safe_radius(radius_sq)?;
        self.touch();
        Ok(clamped)
    }

    /// Replaces the room-wide policy. Only administrators may do this.
    pub fn set_game_rights(&mut self, actor: Uuid, rights: GameRights) -> Result<(), RoomError> {
        self.ensure_owner(actor)?;
        self.rights = rights;
        self.touch();
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
        self.touch();
        Ok(())
    }

    /// Сбрасывает завершённую партию: чистит доску и статистику, возвращает
    /// участников в лобби не готовыми. Состав мест, команды и настройки
    /// комнаты не меняются. Запросить может любой участник комнаты.
    pub fn rematch(&mut self, actor: Uuid) -> Result<(), RoomError> {
        if !self.members.contains_key(&actor) {
            return Err(RoomError::PermissionDenied);
        }
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted.into());
        }
        if !self
            .game
            .characters()
            .values()
            .any(|character| character.stats.wins > 0)
        {
            return Ok(());
        }
        self.game.stop();
        self.game.reset_stats();
        for member in self.members.values_mut() {
            member.status = Status::NotReady;
        }
        self.touch();
        Ok(())
    }

    /// Проигрывает все подряд идущие ходы ботов напрямую, записывая шаги в
    /// `steps` — по ним клиенты повторяют ходы с пошаговой анимацией.
    fn run_bot_turns(&self, game: &mut Game, steps: &mut Vec<ReplayStep>) -> Result<(), GameError> {
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
            match game.bot_move(member_id, &config) {
                Some(pos) => {
                    game.increase(member_id, pos)?;
                    steps.push(ReplayStep::Move {
                        member: member_id,
                        pos,
                        increase: true,
                    });
                }
                None => {
                    game.skip(member_id)?;
                    steps.push(ReplayStep::Skip { member: member_id });
                }
            }
        }
        Ok(())
    }

    pub fn set_open(&mut self, actor: Uuid, open: bool) -> Result<(), RoomError> {
        self.ensure_owner(actor)?;
        match &mut self.mode {
            RoomMode::Offline => Err(RoomError::Offline),
            RoomMode::Online { open: current, .. } => {
                *current = open;
                self.touch();
                Ok(())
            }
        }
    }

    pub fn join(&mut self, player: Uuid, team_id: TeamId) -> Result<(), RoomError> {
        match self.mode {
            RoomMode::Offline => return Err(RoomError::Offline),
            // RoomMode::Online { global: true, .. } => {
            //     return Err(RoomError::GlobalTransportUnavailable);
            // }
            RoomMode::Online { open: false, .. } => return Err(RoomError::Closed),
            RoomMode::Online { open: true } => {}
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
            self.touch();
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
        self.lobby_changed();
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
        self.lobby_changed();
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
        self.touch();
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
        let label = self.lowest_free_bot_label();
        self.bots.insert(bot_id, config);
        self.bot_labels.insert(bot_id, label);
        self.lobby_changed();
        Ok(bot_id)
    }

    /// Наименьший свободный отображаемый номер бота.
    fn lowest_free_bot_label(&self) -> u8 {
        let used: BTreeSet<u8> = self.bot_labels.values().copied().collect();
        (1..=u8::MAX)
            .find(|label| !used.contains(label))
            .unwrap_or(u8::MAX)
    }

    /// Удаляет место. Удаляемо любое место — бот, игрок и даже собственное
    /// место администратора. При этом игрок-контролёр остаётся членом
    /// комнаты (оператором без места): выйти из комнаты может только сам.
    pub fn kick_member(&mut self, actor: Uuid, member_id: CharacterId) -> Result<(), RoomError> {
        self.ensure_characters(actor)?;
        let controller = self
            .game
            .characters()
            .get(&member_id)
            .ok_or(GameError::CharacterNotFound)?
            .controller;
        self.game.kick_member(member_id)?;
        self.bot_labels.remove(&member_id);
        if controller == MemberId::Bot {
            self.bots.remove(&member_id);
        }
        // Игрок остаётся членом комнаты даже без мест: выйти из комнаты он
        // может только сам. Удаляется именно место, а не участник.
        self.compact_teams();
        self.lobby_changed();
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
        self.start_unchecked(automatic_distribution)
    }

    /// Запускает партию без проверки прав вызывающего (для автостарта из
    /// [`Self::lobby_changed`]). Все игроки, сидящие за местами, должны быть
    /// готовы. После успешного старта готовность сбрасывается, чтобы после
    /// конца партии любой leave/kick не перезапустил матч мгновенно.
    fn start_unchecked(&mut self, automatic_distribution: bool) -> Result<(), RoomError> {
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
        let mut steps = vec![ReplayStep::Start {
            automatic: automatic_distribution,
        }];
        let mut game = self.game.clone();
        game.start(automatic_distribution)?;

        self.run_bot_turns(&mut game, &mut steps)?;

        self.game = game;
        for member in self.members.values_mut() {
            member.status = Status::NotReady;
        }
        self.seq = self.seq.wrapping_add(1);
        self.last_steps = steps;
        Ok(())
    }

    /// Принудительное завершение партии администратором. Победитель
    /// определяется по текущей расстановке (клиенты увидят его в статистике).
    pub fn end_game(&mut self, actor: Uuid) -> Result<(), RoomError> {
        self.ensure_owner(actor)?;
        self.game.end_game()?;
        self.touch();
        Ok(())
    }

    /// Меняет местами позиции двух участников в последовательности ходов.
    /// Требует права на изменение правил (по умолчанию — только администратор)
    /// и разрешено только до старта партии.
    pub fn swap_turn_order(
        &mut self,
        actor: Uuid,
        a: CharacterId,
        b: CharacterId,
    ) -> Result<(), RoomError> {
        self.ensure_game_rule(actor)?;
        self.game.swap_turn_order(a, b)?;
        self.lobby_changed();
        Ok(())
    }

    /// Общая проверка хода живого участника: только владелец персонажа
    /// может им играть. Сам ход выполняется на копии, боты доигрывают
    /// свои ходы, и только тогда комната принимает новое состояние. Каждый
    /// шаг (ход и ответы ботов) записывается в `last_steps`, чтобы клиенты
    /// повторили чужие ходы с анимацией волна за волной.
    fn player_action(
        &mut self,
        actor: Uuid,
        member_id: CharacterId,
        apply: impl FnOnce(
            &mut Game,
            CharacterId,
            &mut Vec<ReplayStep>,
        ) -> Result<Option<TeamId>, GameError>,
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

        let mut steps = Vec::new();
        let mut game = self.game.clone();
        let result = apply(&mut game, member_id, &mut steps)?;
        self.run_bot_turns(&mut game, &mut steps)?;
        self.game = game;
        self.seq = self.seq.wrapping_add(1);
        self.last_steps = steps;
        Ok(result)
    }

    pub fn increase(
        &mut self,
        actor: Uuid,
        member_id: CharacterId,
        pos: BoardCode,
    ) -> Result<Option<TeamId>, RoomError> {
        self.player_action(actor, member_id, |game, member, steps| {
            game.increase(member, pos).map(|winner| {
                steps.push(ReplayStep::Move {
                    member,
                    pos,
                    increase: true,
                });
                winner
            })
        })
    }

    pub fn decrease(
        &mut self,
        actor: Uuid,
        member_id: CharacterId,
        pos: BoardCode,
    ) -> Result<Option<TeamId>, RoomError> {
        self.player_action(actor, member_id, |game, member, steps| {
            game.decrease(member, pos).map(|winner| {
                steps.push(ReplayStep::Move {
                    member,
                    pos,
                    increase: false,
                });
                winner
            })
        })
    }

    pub fn skip(
        &mut self,
        actor: Uuid,
        member_id: CharacterId,
    ) -> Result<Option<TeamId>, RoomError> {
        self.player_action(actor, member_id, |game, member, steps| {
            game.skip(member).map(|winner| {
                steps.push(ReplayStep::Skip { member });
                winner
            })
        })
    }

    pub fn surrender(
        &mut self,
        actor: Uuid,
        member_id: CharacterId,
    ) -> Result<Option<TeamId>, RoomError> {
        self.player_action(actor, member_id, |game, member, steps| {
            game.surrender(member).map(|winner| {
                steps.push(ReplayStep::Surrender { member });
                winner
            })
        })
    }

    /// Воссоздаёт на сервере лобби игрока при переключении Оффлайн/Онлайн:
    /// размер доски, правила и места (боты с номерами, локальные игроки на
    /// своих командах). Вызывается только на свежесозданной комнате.
    pub fn apply_lobby_setup(&mut self, actor: Uuid, setup: &LobbySetup) -> Result<(), RoomError> {
        self.ensure_owner(actor)?;
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted.into());
        }
        let size = NonZeroU8::new(setup.width)
            .zip(NonZeroU8::new(setup.height))
            .and_then(|(w, h)| BoardCode::from_size(w, h))
            .ok_or(RoomError::InvalidLobby)?;
        self.game.set_board_size(size)?;
        self.game.set_explosion_mode(if setup.lossless {
            ExplosionMode::Lossless
        } else {
            ExplosionMode::Lossy
        })?;
        self.automatic_distribution = setup.automatic;

        // Место создателя: на своей команде или без места — оператор комнаты.
        let owner_seat = self
            .game
            .characters()
            .iter()
            .find(|(_, character)| character.controller == MemberId::Player(actor))
            .map(|(&id, _)| id)
            .ok_or(RoomError::MemberNotFound)?;
        match setup.my_team {
            Some(raw) => {
                let team = TeamId::new(raw).ok_or(RoomError::InvalidLobby)?;
                self.game.change_team(owner_seat, team)?;
            }
            None => self.kick_member(actor, owner_seat)?,
        }

        for seat in &setup.seats {
            let team = TeamId::new(seat.team).ok_or(RoomError::InvalidLobby)?;
            if seat.bot {
                let bot_id = self.add_bot_with_config(actor, team, BotConfig::STANDARD)?;
                // Сохраняем запрошенный номер, если он ещё свободен.
                let requested_free = seat.label > 0
                    && !self
                        .bot_labels
                        .iter()
                        .any(|(&id, &label)| id != bot_id && label == seat.label);
                if requested_free {
                    self.bot_labels.insert(bot_id, seat.label);
                }
            } else {
                // Hot-seat место под контролем создателя комнаты.
                self.game.add_local_member(actor, team)?;
            }
        }
        // Радиус ставим последним: предел зависит от числа команд.
        let _ = self.game.set_safe_radius(setup.radius_sq);
        self.touch();
        Ok(())
    }

    /// Отмечает изменение комнаты: повышает счётчик и сбрасывает шаги
    /// последнего действия. Вызывается только на успешных путях мутаторов.
    fn touch(&mut self) {
        self.seq = self.seq.wrapping_add(1);
        self.last_steps.clear();
    }

    /// Изменение состава лобби (готовность, места, участники). Если партия
    /// уже идёт — обычный `touch`. Если все члены комнаты готовы — партия
    /// стартует автоматически. Требуем готовность всех членов, а не только
    /// сидящих за местами: иначе настройка ботов в свежей комната (где
    /// владелец ещё NotReady) мгновенно стартовала бы партию.
    /// После успешного старта `touch` не зовётся: он очистил бы шаги
    /// начавшейся партии и клиенты не анимировали бы её.
    fn lobby_changed(&mut self) {
        if self.game.is_started() {
            self.touch();
            return;
        }
        if !self
            .members
            .values()
            .all(|member| member.status == Status::Ready)
            || self.start_unchecked(self.automatic_distribution).is_err()
        {
            self.touch();
        }
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

    /// Drops the room when the last member has left. Unlike [`Self::remove`]
    /// this needs no actor: it is called from leave/kick handling on the server.
    pub fn remove_if_empty(&mut self, id: RoomId) {
        if self
            .rooms
            .get(&id)
            .is_some_and(|room| room.members().is_empty())
        {
            self.rooms.remove(&id);
        }
    }

    pub fn list_open_local(&self) -> impl Iterator<Item = &Room> {
        self.rooms
            .values()
            .filter(|room| matches!(room.mode, RoomMode::Online { open: true }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::network::protocol::{LobbySeat, RoomStateResponse};

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
        let room_id = directory.create(owner, RoomMode::Online { open: true });

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
    fn remove_if_empty_drops_only_empty_rooms() {
        let owner = uid(10);
        let guest = uid(20);
        let mut directory = RoomDirectory::default();
        let room_id = directory.create(owner, RoomMode::Online { open: true });

        directory.remove_if_empty(room_id);
        assert!(directory.get(room_id).is_some(), "owner still inside");

        directory.join(room_id, guest, tid(1)).unwrap();
        directory.leave(room_id, owner).unwrap();
        directory.remove_if_empty(room_id);
        assert!(directory.get(room_id).is_some(), "guest still inside");

        directory.leave(room_id, guest).unwrap();
        directory.remove_if_empty(room_id);
        assert!(directory.get(room_id).is_none(), "last member left");
    }

    #[test]
    fn room_owns_game_management_permissions_and_member_sync() {
        let owner = uid(10);
        let guest = uid(20);
        let mut room = Room::new(owner, RoomMode::Online { open: true });
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
            room.skip(guest, owner_character),
            Err(RoomError::PermissionDenied)
        );
        room.set_game_rights(owner, GameRights::new(Rights::Nope, false, false))
            .unwrap();
        assert_eq!(
            room.change_team(guest, guest_character, tid(4)),
            Err(RoomError::PermissionDenied)
        );
        // Администратор может удалить и собственное место: он остаётся
        // в комнате оператором без места.
        room.kick_member(owner, owner_character).unwrap();
        assert!(room.members().contains_key(&owner));
        assert!(!room.game().characters().contains_key(&owner_character));

        // Удаление места игрока не удаляет его из комнаты: он остаётся
        // участником без места и может занять новое.
        room.kick_member(owner, guest_character).unwrap();
        assert_eq!(room.members().len(), 2);
        assert!(room.members().contains_key(&owner));
        assert!(room.members().contains_key(&guest));
        assert!(!room.game().characters().contains_key(&guest_character));

        let bot_id = room.add_bot(owner, tid(2)).unwrap();
        assert_eq!(room.bots().get(&bot_id), Some(&BotConfig::STANDARD));
        assert_eq!(room.members().len(), 2);
        assert_eq!(room.bot_labels().get(&bot_id), Some(&1));
    }

    #[test]
    fn rights_default_to_own_team_changes_for_members() {
        let owner = uid(10);
        let guest = uid(20);
        let mut room = Room::new(owner, RoomMode::Online { open: true });
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
        let mut room = Room::new(owner, RoomMode::Online { open: true });
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
        let mut room = Room::new(owner, RoomMode::Online { open: true });
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
        let mut room = Room::new(owner, RoomMode::Online { open: true });
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
        let mut room = Room::new(owner, RoomMode::Online { open: true });
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

    fn seat_of(room: &Room, player: Uuid) -> CharacterId {
        room.game()
            .characters()
            .iter()
            .find(|(_, character)| character.controller == MemberId::Player(player))
            .map(|(&id, _)| id)
            .expect("every room member controls a seat")
    }

    #[test]
    fn seat_controllers_can_be_reassigned_to_members_and_bots() {
        let owner = uid(10);
        let guest = uid(20);
        let mut room = Room::new(owner, RoomMode::Online { open: true });
        room.join(guest, tid(2)).unwrap();
        let guest_seat = seat_of(&room, guest);

        // Место гостя передаётся владельцу — запрет кика больше не действует.
        room.set_member_controller(owner, guest_seat, MemberId::Player(owner))
            .unwrap();
        assert_eq!(
            room.game()
                .characters()
                .get(&guest_seat)
                .unwrap()
                .controller,
            MemberId::Player(owner)
        );

        // То же место становится бота-местом — конфиг бота появляется.
        room.set_member_controller(owner, guest_seat, MemberId::Bot)
            .unwrap();
        assert_eq!(
            room.game()
                .characters()
                .get(&guest_seat)
                .unwrap()
                .controller,
            MemberId::Bot
        );
        assert!(room.bots().contains_key(&guest_seat));

        // Обычному участнику права не хватает.
        assert_eq!(
            room.set_member_controller(guest, guest_seat, MemberId::Player(guest)),
            Err(RoomError::PermissionDenied)
        );

        // Контроллером может стать только участник комнаты.
        assert_eq!(
            room.set_member_controller(owner, guest_seat, MemberId::Player(uid(99))),
            Err(RoomError::MemberNotFound)
        );
    }

    #[test]
    fn seat_controllers_cannot_be_reassigned_after_the_start() {
        let owner = uid(10);
        let guest = uid(20);
        let mut room = Room::new(owner, RoomMode::Online { open: true });
        room.join(guest, tid(2)).unwrap();
        room.set_ready(owner, true).unwrap();
        // Последний готовый игрок стартует партию автоматически.
        room.set_ready(guest, true).unwrap();
        assert!(room.game().is_started());

        assert_eq!(
            room.set_member_controller(owner, seat_of(&room, guest), MemberId::Bot),
            Err(RoomError::Game(GameError::GameAlreadyStarted))
        );
    }

    #[test]
    fn surrender_out_of_turn_costs_the_seat_and_keeps_the_turn() {
        let owner = uid(10);
        let guest = uid(20);
        let mut room = Room::new(owner, RoomMode::Online { open: true });
        room.join(guest, tid(2)).unwrap();
        // Третья команда — бот: после сдачи гостя партия продолжается.
        room.add_bot(owner, tid(3)).unwrap();
        room.set_ready(owner, true).unwrap();
        room.set_ready(guest, true).unwrap();
        assert!(room.game().is_started());

        let guest_seat = seat_of(&room, guest);

        // Первым ходит владелец: гость сдаётся чужим ходом — поражение
        // засчитывается сразу, очередь хода остаётся владельцу.
        room.surrender(guest, guest_seat).unwrap();
        assert_eq!(
            room.game()
                .characters()
                .get(&guest_seat)
                .unwrap()
                .stats
                .losses,
            1
        );
        assert_eq!(room.game().current_turn(), Some(0));
        assert!(room.game().is_started());
    }

    #[test]
    fn admin_can_force_end_a_started_game() {
        let owner = uid(10);
        let guest = uid(20);
        let mut room = Room::new(owner, RoomMode::Online { open: true });
        room.join(guest, tid(2)).unwrap();
        room.set_ready(owner, true).unwrap();
        room.set_ready(guest, true).unwrap();
        assert!(room.game().is_started());

        // Обычному участнику досрочное завершение недоступно.
        assert_eq!(room.end_game(guest), Err(RoomError::NotOwner));

        room.end_game(owner).unwrap();
        assert!(!room.game().is_started());
        // Итог зафиксирован в статистике: у команды-победителя есть Win.
        assert!(
            room.game()
                .characters()
                .values()
                .any(|character| character.stats.wins > 0)
        );
        assert_eq!(
            room.end_game(owner),
            Err(RoomError::Game(GameError::GameNotStarted))
        );
    }

    #[test]
    fn turn_order_swap_requires_admin_rights_before_start() {
        let owner = uid(10);
        let guest = uid(20);
        let mut room = Room::new(owner, RoomMode::Online { open: true });
        room.join(guest, tid(2)).unwrap();
        let owner_seat = seat_of(&room, owner);
        let guest_seat = seat_of(&room, guest);

        // Порядок ходов по умолчанию: владелец, затем гость.
        let order: Vec<CharacterId> = room.game().characters().keys().copied().collect();
        assert_eq!(order, vec![owner_seat, guest_seat]);

        // Обычному участнику перестановка мест недоступна.
        assert_eq!(
            room.swap_turn_order(guest, owner_seat, guest_seat),
            Err(RoomError::PermissionDenied)
        );

        room.swap_turn_order(owner, guest_seat, owner_seat).unwrap();
        let order: Vec<CharacterId> = room.game().characters().keys().copied().collect();
        assert_eq!(order, vec![guest_seat, owner_seat]);

        // После старта партии порядок менять нельзя.
        room.set_ready(owner, true).unwrap();
        room.set_ready(guest, true).unwrap();
        assert!(room.game().is_started());
        assert_eq!(
            room.swap_turn_order(owner, guest_seat, owner_seat),
            Err(RoomError::Game(GameError::GameAlreadyStarted))
        );
    }

    #[test]
    fn turn_order_survives_the_wire_round_trip() {
        let owner = uid(10);
        let guest = uid(20);
        let mut room = Room::new(owner, RoomMode::Online { open: true });
        room.join(guest, tid(2)).unwrap();
        let owner_seat = seat_of(&room, owner);
        let guest_seat = seat_of(&room, guest);

        room.swap_turn_order(owner, guest_seat, owner_seat).unwrap();

        // Снапшот проходит по проводам как есть: postcard — последовательный
        // формат, порядок записей IndexMap сохраняется, а не сортируется.
        let response = RoomStateResponse {
            id: RoomId::new(1).unwrap(),
            me: owner,
            room: room.clone(),
            names: HashMap::new(),
        };
        let bytes = postcard::to_allocvec(&response).unwrap();
        let back: RoomStateResponse = postcard::from_bytes(&bytes).unwrap();

        let order: Vec<CharacterId> = back.room.game().characters().keys().copied().collect();
        assert_eq!(order, vec![guest_seat, owner_seat]);
    }

    #[test]
    fn leaving_mid_game_costs_the_seat() {
        let owner = uid(10);
        let guest = uid(20);
        let mut room = Room::new(owner, RoomMode::Online { open: true });
        room.join(guest, tid(2)).unwrap();
        room.set_ready(owner, true).unwrap();
        room.set_ready(guest, true).unwrap();
        assert!(room.game().is_started());

        let guest_seat = seat_of(&room, guest);

        // Выйти из партии можно в любой момент — это поражение.
        room.leave(guest).unwrap();
        assert_eq!(
            room.game()
                .characters()
                .get(&guest_seat)
                .unwrap()
                .stats
                .losses,
            1
        );
    }

    #[test]
    fn player_moves_are_recorded_as_replay_steps() {
        let owner = uid(10);
        let mut room = Room::new(owner, RoomMode::Online { open: true });
        room.add_bot(owner, tid(2)).unwrap();
        // Готовность единственного игрока автостартует партию (авторасстановка
        // в комнате включена по умолчанию).
        room.set_ready(owner, true).unwrap();
        assert!(room.game().is_started());

        assert_eq!(
            room.last_steps().first(),
            Some(&ReplayStep::Start { automatic: true })
        );
        assert!(room.seq() > 0);
        let seq = room.seq();

        let owner_seat = seat_of(&room, owner);
        let team = room.game().characters().get(&owner_seat).unwrap().team_id;
        let pos = room
            .game()
            .board()
            .iter()
            .find(|(_, cell)| cell.owner == Some(team))
            .map(|(code, _)| code)
            .expect("automatic distribution places a starting cell");

        room.increase(owner, owner_seat, pos).unwrap();
        let steps = room.last_steps();
        assert_eq!(
            steps.first(),
            Some(&ReplayStep::Move {
                member: owner_seat,
                pos,
                increase: true,
            })
        );
        // За игроком бот отвечает собственным шагом — по нему клиент
        // повторяет ответ с пошаговой анимацией.
        assert!(steps.iter().skip(1).any(|step| {
            matches!(
                step,
                ReplayStep::Move { member, .. }
                    | ReplayStep::Skip { member }
                    if *member != owner_seat
            )
        }));
        assert!(room.seq() > seq);

        // Изменение вне партии сбрасывает шаги, но повышает счётчик.
        room.set_game_rights(owner, GameRights::default()).unwrap();
        assert!(room.last_steps().is_empty());
        assert!(room.seq() > seq + 1);
    }

    #[test]
    fn lobby_setup_creates_the_same_lobby_on_the_server() {
        let me = uid(10);
        let mut room = Room::new(me, RoomMode::Online { open: true });

        room.apply_lobby_setup(
            me,
            &LobbySetup {
                width: 5,
                height: 7,
                automatic: false,
                lossless: false,
                radius_sq: 2,
                my_team: None,
                seats: vec![
                    LobbySeat {
                        bot: false,
                        label: 0,
                        team: 1,
                    },
                    LobbySeat {
                        bot: true,
                        label: 5,
                        team: 2,
                    },
                    LobbySeat {
                        bot: true,
                        label: 2,
                        team: 3,
                    },
                ],
            },
        )
        .unwrap();

        let size = room.game().board().size();
        assert_eq!((size.width().get(), size.height().get()), (5, 7));
        assert_eq!(room.game().explosion_mode(), ExplosionMode::Lossy);
        assert!(!room.automatic_distribution());
        assert_eq!(room.game().characters().len(), 3);
        assert!(room.members().contains_key(&me));

        let local = room
            .game()
            .characters()
            .values()
            .find(|character| character.controller == MemberId::Player(me))
            .unwrap();
        assert_eq!(local.team_id, tid(1));

        let team2_seat = room
            .game()
            .characters()
            .iter()
            .find(|(_, character)| character.team_id == tid(2))
            .map(|(&id, _)| id)
            .unwrap();
        let team3_seat = room
            .game()
            .characters()
            .iter()
            .find(|(_, character)| character.team_id == tid(3))
            .map(|(&id, _)| id)
            .unwrap();
        assert_eq!(room.bots().get(&team2_seat), Some(&BotConfig::STANDARD));
        assert_eq!(room.bots().get(&team3_seat), Some(&BotConfig::STANDARD));
        assert_eq!(room.bot_labels().get(&team2_seat), Some(&5));
        assert_eq!(room.bot_labels().get(&team3_seat), Some(&2));
    }

    #[test]
    fn start_records_the_first_bot_turns() {
        let me = uid(10);
        let mut room = Room::new(me, RoomMode::Online { open: true });
        room.apply_lobby_setup(
            me,
            &LobbySetup {
                width: 5,
                height: 5,
                automatic: true,
                lossless: false,
                radius_sq: 2,
                my_team: None,
                seats: vec![
                    LobbySeat {
                        bot: true,
                        label: 1,
                        team: 2,
                    },
                    LobbySeat {
                        bot: true,
                        label: 2,
                        team: 3,
                    },
                ],
            },
        )
        .unwrap();
        room.set_ready(me, true).unwrap();
        // Владелец-оператор без места: его готовность автостартует бот-партию,
        // и боты доигрывают её целиком.

        // Партия только из ботов разыгрывается на сервере целиком: шаги
        // начинаются со старта и продолжаются ходами ботов.
        assert_eq!(
            room.last_steps().first(),
            Some(&ReplayStep::Start { automatic: true })
        );
        assert!(room.last_steps().len() > 1);
    }

    #[test]
    fn readying_every_member_starts_the_game_automatically() {
        let owner = uid(10);
        let guest = uid(20);
        let mut room = Room::new(owner, RoomMode::Online { open: true });
        room.join(guest, tid(2)).unwrap();

        room.set_ready(owner, true).unwrap();
        assert!(
            !room.game().is_started(),
            "the second player is not ready yet"
        );

        room.set_ready(guest, true).unwrap();
        assert!(room.game().is_started());
        // Готовность сбрасывается вместе со стартом: после естественного конца
        // партии не готов ни один участник, и последующие изменения лобби
        // (leave/kick/готовность) не перезапускают матч мгновенно.
        assert!(
            room.members()
                .values()
                .all(|member| member.status() != Status::Ready)
        );

        // Повторная готовность в идущей партии отклоняется.
        assert_eq!(
            room.set_ready(owner, true),
            Err(RoomError::Game(GameError::GameAlreadyStarted))
        );
    }

    #[test]
    fn unseated_members_gate_the_autostart_too() {
        let owner = uid(10);
        let guest = uid(20);
        let mut room = Room::new(owner, RoomMode::Online { open: true });
        room.join(guest, tid(2)).unwrap();
        // Владелец удаляет собственное место и остаётся оператором-зрителем;
        // гость и бот образуют играбельный стол.
        room.add_bot(owner, tid(3)).unwrap();
        room.kick_member(owner, seat_of(&room, owner)).unwrap();
        assert!(room.members().contains_key(&owner));

        room.set_ready(guest, true).unwrap();
        assert!(
            !room.game().is_started(),
            "the spectator owner is not ready"
        );

        // Гость уже готов: партия стартует последним нажатием владельца.
        room.set_ready(owner, true).unwrap();
        assert!(room.game().is_started());
    }

    #[test]
    fn rematch_resets_the_finished_match() {
        let me = uid(10);
        let mut room = Room::new(me, RoomMode::Online { open: true });
        room.apply_lobby_setup(
            me,
            &LobbySetup {
                width: 5,
                height: 5,
                automatic: true,
                lossless: false,
                radius_sq: 2,
                my_team: None,
                seats: vec![
                    LobbySeat {
                        bot: true,
                        label: 1,
                        team: 2,
                    },
                    LobbySeat {
                        bot: true,
                        label: 2,
                        team: 3,
                    },
                ],
            },
        )
        .unwrap();
        room.set_ready(me, true).unwrap();
        // Боты доиграли партию целиком.
        assert!(!room.game().is_started());
        assert!(
            room.game()
                .characters()
                .values()
                .any(|character| character.stats.wins > 0)
        );
        let seats_before: Vec<CharacterId> = room.game().characters().keys().copied().collect();

        // Чужак не может попросить переигровку.
        assert_eq!(room.rematch(uid(99)), Err(RoomError::PermissionDenied));

        room.rematch(me).unwrap();
        // Доска чистая, статистика обнулена, состав мест не изменился.
        assert!(!room.game().is_started());
        assert!(
            room.game()
                .board()
                .iter()
                .all(|(_, cell)| cell.value == 0 && cell.owner.is_none())
        );
        assert!(
            room.game()
                .characters()
                .values()
                .all(|character| character.stats.wins == 0 && character.stats.losses == 0)
        );
        assert_eq!(
            room.game().characters().keys().copied().collect::<Vec<_>>(),
            seats_before
        );
        assert!(
            room.members()
                .values()
                .all(|member| member.status() == Status::NotReady)
        );

        // Повторная переигровка без сыгранной партии — no-op без ошибки.
        room.rematch(me).unwrap();

        // Гость занимает место: его ход удерживает партию идущей —
        // в этот момент переигровка отклоняется.
        let guest = uid(20);
        room.join(guest, tid(4)).unwrap();
        room.set_ready(guest, true).unwrap();
        room.set_ready(me, true).unwrap();
        assert!(room.game().is_started());
        assert_eq!(
            room.rematch(me),
            Err(RoomError::Game(GameError::GameAlreadyStarted))
        );
    }
}
