use std::collections::{BTreeSet, HashMap};

use forup_core::api::{
    auth::PlayerId,
    game::prelude::*,
    network::{LobbySeat, LobbySetup, ReplayStep, Room, Status},
};

use super::views::{CharacterView, TeamParticipantView, TeamView};

/// Deterministic identity for the first human seat so server-rendered markup
/// matches the hydrated one.
const FIRST_OWNER: u32 = 1;
/// Hue of the first team; the rest are distributed evenly around the wheel.
const TEAM_HUE_OFFSET: u16 = 0;

pub type ParticipantId = u32;

/// Who controls a participant. Bots are only labelled by number for the UI;
/// the engine treats every bot as the same `MemberId::Bot`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Controller {
    Human,
    Bot(u8),
    /// Сетевой участник: идентификатор его аккаунта на сервере.
    Remote(PlayerId),
}

impl Controller {
    /// Stable key used by the controller selector.
    pub fn key(self) -> String {
        match self {
            Self::Human => "human".to_string(),
            Self::Bot(number) => format!("bot{number}"),
            Self::Remote(id) => format!("remote{id}"),
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        if key == "human" {
            return Some(Self::Human);
        }
        key.strip_prefix("bot")
            .and_then(|number| number.parse::<u8>().ok())
            .filter(|number| *number > 0)
            .map(Self::Bot)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Participant {
    pub id: ParticipantId,
    pub controller: Controller,
    pub characters: Vec<CharacterId>,
    /// Отображаемое имя сетевого участника (только для онлайн-комнат).
    pub name: Option<String>,
    /// Готовность сетевого участника; офлайн всегда `None`.
    pub ready: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::Display, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum GamePhase {
    /// Waiting for the local player to press Ready.
    Lobby,
    Playing,
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::Display, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum MoveMode {
    Increase,
    Decrease,
}

/// Компактный набор слотов клеток доски (не больше 256, см. `BoardCode`).
/// Хранит легальные клетки текущего хода: маска считается один раз на
/// состояние партии, а проверка отдельной клетки — одно обращение по индексу.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlotMask([u64; 4]);

impl SlotMask {
    pub fn insert(&mut self, slot: u8) {
        self.0[usize::from(slot >> 6)] |= 1u64 << (slot & 63);
    }

    pub fn contains(self, slot: u8) -> bool {
        self.0[usize::from(slot >> 6)] & (1u64 << (slot & 63)) != 0
    }
}

/// Снимок данных, от которых зависит отрисовка клеток доски. Настройки
/// (безопасный радиус и т.п.) сюда не входят: их смена не трогает доску,
/// поэтому равенство снимка сохраняется и клетки не перерисовываются.
#[derive(Clone, Debug, PartialEq)]
pub struct BoardView {
    pub board: Board,
    pub phase: GamePhase,
    pub current_team: Option<TeamId>,
    pub explosion_mode: ExplosionMode,
    pub animation_tick: u64,
    changes: Vec<i16>,
}

impl BoardView {
    pub fn cell(&self, pos: BoardCode) -> Option<&CellData> {
        self.board.get(pos)
    }

    /// Изменение значения клетки за последний шаг хода (для анимации).
    pub fn change_at(&self, pos: BoardCode) -> i16 {
        self.changes
            .get(usize::from(pos.slot()))
            .copied()
            .unwrap_or(0)
    }
}

/// Полностью локальная партия. Ход применяется в два приема: сначала
/// [`Self::make_move`] применяет изменение клетки, затем волны взрывов
/// доигрываются по одной через [`Self::animation_step`], чтобы UI
/// анимировал их с настраиваемой скоростью.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalGame {
    pub game: Game,
    pub participants: Vec<Participant>,
    next_participant_id: ParticipantId,
    pub phase: GamePhase,
    pub winner: Option<TeamId>,
    pub automatic_distribution: bool,
    /// Ход доигрывается взрывами: ввод заблокирован до конца анимации.
    exploding: bool,
    /// Изменение значения каждой клетки за последний шаг хода (сам ход или
    /// одна волна взрывов): анимируется только изменившаяся на этом шаге
    /// клетка.
    changes: Vec<i16>,
    /// Чередующееся значение для перезапуска CSS-анимаций при повторных
    /// изменениях.
    pub animation_tick: u64,
    /// Идентичность игрока в онлайн-комнате; `None` — партия офлайн.
    me: Option<PlayerId>,
    /// Игроки онлайн-комнаты для панели над списком команд; офлайн пуст.
    pub players: Vec<RoomPlayer>,
    /// Шаги серверного «реплея» (чужие ходы), доигрываемые по одному,
    /// чтобы взрывы анимировались волна за волной, как в офлайне.
    replay: Vec<ReplayStep>,
    /// Водяной знак снапшотов: номер последнего принятого состояния комнаты.
    sync_seq: u64,
    /// Номер снапшота, к которому приведёт доигрывание текущего реплея.
    replay_seq: u64,
    /// Предсказанный ход: локально применённое действие, которое сервер
    /// подтвердит (или отклонит — тогда реплей вернёт ход заново).
    predicted: Option<ReplayStep>,
}

impl LocalGame {
    pub fn new() -> Self {
        let game = Game::standard();
        let changes = vec![0; game.board().cells().len()];
        let mut local = Self {
            game,
            participants: Vec::new(),
            next_participant_id: 1,
            phase: GamePhase::Lobby,
            winner: None,
            automatic_distribution: true,
            exploding: false,
            changes,
            animation_tick: 0,
            me: None,
            players: Vec::new(),
            replay: Vec::new(),
            sync_seq: 0,
            replay_seq: 0,
            predicted: None,
        };
        local
            .add_participant_on_team(Controller::Human, TeamId::MIN)
            .expect("a fresh game has free character ids");
        local
            .add_participant_on_team(
                Controller::Bot(1),
                TeamId::new(2).expect("team 2 is a valid team id"),
            )
            .expect("a fresh game has free character ids");
        local
    }

    /// Восстанавливает локальное представление из снапшота серверной комнаты:
    /// одно место = один участник, сетевые места помечаются `Remote`.
    /// На native вызывается только тестами: сетевой клиент существует лишь
    /// в wasm-сборке.
    #[cfg_attr(
        not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))),
        allow(dead_code)
    )]
    pub fn from_room(room: &Room, me: PlayerId, names: &HashMap<PlayerId, String>) -> Self {
        let game = room.game().clone();
        let phase = if game.is_started() {
            GamePhase::Playing
        } else if game.characters().values().any(|data| data.stats.wins > 0) {
            GamePhase::Finished
        } else {
            GamePhase::Lobby
        };
        let winner = (phase == GamePhase::Finished)
            .then(|| Self::winner_from_stats(&game))
            .flatten();

        let mut players = Vec::new();
        for (id, data) in room.members() {
            players.push(RoomPlayer {
                id: *id,
                name: names.get(id).cloned(),
                ready: data.status() == Status::Ready,
                admin: data.is_admin(),
            });
        }
        players.sort_unstable_by_key(|player| player.id);

        let mut participants = Vec::new();
        let mut max_id = 0;
        for (character, data) in game.characters() {
            let (controller, name, ready) = match data.controller {
                MemberId::Player(player) => {
                    let ready = room
                        .member(player)
                        .map(|member| member.status() == Status::Ready);
                    let name = names.get(&player).cloned();
                    if player == me {
                        (Controller::Human, name, ready)
                    } else {
                        (Controller::Remote(player), name, ready)
                    }
                }
                MemberId::Bot => (Controller::Bot(character.get()), None, None),
            };
            max_id = max_id.max(u32::from(character.get()));
            participants.push(Participant {
                id: u32::from(character.get()),
                controller,
                characters: vec![*character],
                name,
                ready,
            });
        }

        let changes = vec![0; game.board().cells().len()];
        Self {
            game,
            participants,
            next_participant_id: max_id.saturating_add(1),
            phase,
            winner,
            automatic_distribution: room.automatic_distribution(),
            exploding: false,
            changes,
            animation_tick: 0,
            me: Some(me),
            players,
            replay: Vec::new(),
            sync_seq: room.seq(),
            replay_seq: 0,
            predicted: None,
        }
    }

    /// Принимает следующий снапшот комнаты, сохраняя контекст анимации:
    /// дельты клеток считаются относительно предыдущего состояния, чтобы
    /// чужие ходы пульсировали так же, как локальные. Неизменившийся снапшот
    /// не перезапускает анимации (защита от спама опросом).
    #[cfg_attr(
        not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))),
        allow(dead_code)
    )]
    pub fn absorb(&mut self, next: LocalGame) {
        let before = self.cell_values();
        let board_changed = before.len() != next.game.board().cells().len();
        let state_changed = self.game != next.game;
        let tick = self.animation_tick.wrapping_add(1);
        *self = next;
        if !board_changed && state_changed {
            self.snapshot_deltas(&before);
            self.animation_tick = tick;
        }
    }

    /// Равны ли метаданные тому, что копирует [`Self::confirm`]: контекст
    /// анимации (`changes`, `animation_tick`, `exploding`) и очередь реплея
    /// не сравниваются, иначе равные снапшоты опроса перезапускали бы
    /// уведомления после каждого хода.
    #[cfg_attr(
        not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))),
        allow(dead_code)
    )]
    pub fn same_snapshot_meta(&self, other: &LocalGame) -> bool {
        self.participants == other.participants
            && self.players == other.players
            && self.phase == other.phase
            && self.winner == other.winner
            && self.automatic_distribution == other.automatic_distribution
            && self.me == other.me
            && self.next_participant_id == other.next_participant_id
            && self.sync_seq == other.sync_seq
    }

    /// Тихо принимает серверный снапшот с той же игрой (предсказание
    /// подтвердилось): копируются только метаданные, локальный контекст
    /// анимации не трогается, чтобы волны взрывов не мигали.
    #[cfg_attr(
        not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))),
        allow(dead_code)
    )]
    pub fn confirm(&mut self, next: LocalGame) {
        self.sync_seq = next.sync_seq;
        self.participants = next.participants;
        self.players = next.players;
        self.phase = next.phase;
        self.winner = next.winner;
        self.automatic_distribution = next.automatic_distribution;
        self.me = next.me;
        self.next_participant_id = next.next_participant_id;
        self.replay.clear();
        self.replay_seq = 0;
        self.predicted = None;
    }

    /// Отключает партию от сети, сохраняя стол: сетевые места становятся
    /// локальными игроками, а очередь реплея сбрасывается. Владелец комнаты
    /// остаётся оператором лобби даже без своего места.
    pub fn detach(&mut self) {
        self.me = None;
        self.players.clear();
        self.replay.clear();
        self.replay_seq = 0;
        self.sync_seq = 0;
        self.predicted = None;
        for participant in &mut self.participants {
            if let Controller::Bot(_) = participant.controller {
                continue;
            }
            participant.controller = Controller::Human;
            participant.name = None;
            participant.ready = None;
        }
    }

    /// Начинает серверный реплей чужих ходов. Сначала шаги проверяются на
    /// копии движка: любое расхождение с целевым состоянием (`target`)
    /// отклоняет реплей целиком — партия рассинхронизирована и требует
    /// полного ресинка. Наш предсказанный ход из очереди вырезается: он уже
    /// применён локально. Возвращает `true`, если реплей принят.
    #[cfg_attr(
        not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))),
        allow(dead_code)
    )]
    pub fn start_replay(&mut self, steps: &[ReplayStep], target: &Game, seq: u64) -> bool {
        let mut steps = steps.to_vec();
        if steps.first() == self.predicted.as_ref() {
            steps.remove(0);
        }
        self.predicted = None;

        let mut probe = self.game.clone();
        for step in &steps {
            let applied: Result<(), GameError> = match *step {
                ReplayStep::Start { automatic } => probe.start(automatic).map(|_| ()),
                ReplayStep::Move {
                    member,
                    pos,
                    increase,
                } => probe.start_move(member, pos, increase).and_then(|()| {
                    while probe.explode_wave() {}
                    probe.finish_move().map(|_| ())
                }),
                ReplayStep::Skip { member } => probe.skip(member).map(|_| ()),
                ReplayStep::Surrender { member } => probe.surrender(member).map(|_| ()),
            };
            if applied.is_err() {
                return false;
            }
        }
        if probe != *target {
            return false;
        }

        self.replay = steps;
        self.replay_seq = seq;
        let _ = self.begin_replay_step();
        true
    }

    // --- lobby ------------------------------------------------------------

    #[allow(dead_code)]
    pub fn add_human(&mut self) -> Result<ParticipantId, GameError> {
        self.add_participant(Controller::Human)
    }

    /// Adds a new participant controlled by a fresh bot number.
    pub fn add_bot(&mut self) -> Result<ParticipantId, GameError> {
        let controller = Controller::Bot(self.next_bot_number());
        self.add_participant(controller)
    }

    fn add_participant(&mut self, controller: Controller) -> Result<ParticipantId, GameError> {
        let team = self.next_team_id();
        self.add_participant_on_team(controller, team)
    }

    /// Участник по идентификатору.
    fn participant(&self, id: ParticipantId) -> Option<&Participant> {
        self.participants
            .iter()
            .find(|participant| participant.id == id)
    }

    /// Индекс участника по идентификатору.
    fn participant_index(&self, id: ParticipantId) -> Option<usize> {
        self.participants
            .iter()
            .position(|participant| participant.id == id)
    }

    fn add_participant_on_team(
        &mut self,
        controller: Controller,
        team: TeamId,
    ) -> Result<ParticipantId, GameError> {
        self.ensure_editable()?;

        let id = self.next_participant_id;
        self.next_participant_id = self.next_participant_id.saturating_add(1);
        let character = match controller {
            Controller::Human => self.game.add_local_member(Self::human_player_id()?, team)?,
            Controller::Bot(_) => self.game.add_bot(team)?,
            // Сетевые места появляются только из серверных снапшотов.
            Controller::Remote(_) => return Err(GameError::CharacterNotFound),
        };

        self.participants.push(Participant {
            id,
            controller,
            characters: vec![character],
            name: None,
            ready: None,
        });
        Ok(id)
    }

    fn human_player_id() -> Result<PlayerId, GameError> {
        PlayerId::new(FIRST_OWNER).ok_or(GameError::CharacterIdSpaceExhausted)
    }

    fn used_bot_numbers(&self) -> BTreeSet<u8> {
        self.participants
            .iter()
            .filter_map(|participant| match participant.controller {
                Controller::Bot(number) => Some(number),
                Controller::Human | Controller::Remote(_) => None,
            })
            .collect()
    }

    /// Наименьший свободный номер из 1..=255 в занятом наборе.
    fn lowest_free_number(used: &BTreeSet<u8>) -> u8 {
        (1..=u8::MAX)
            .find(|number| !used.contains(number))
            .unwrap_or(u8::MAX)
    }

    fn next_bot_number(&self) -> u8 {
        Self::lowest_free_number(&self.used_bot_numbers())
    }

    /// Adds another character to a participant. The lobby UI works with one
    /// participant per seat, so this is only kept for engine-level use.
    #[allow(dead_code)]
    pub fn add_character(
        &mut self,
        participant_id: ParticipantId,
    ) -> Result<CharacterId, GameError> {
        self.ensure_editable()?;

        let (controller, team) = {
            let participant = self
                .participant(participant_id)
                .ok_or(GameError::CharacterNotFound)?;
            let team = participant
                .characters
                .iter()
                .find_map(|character| {
                    self.game
                        .characters()
                        .get(character)
                        .map(|data| data.team_id)
                })
                .unwrap_or_else(|| self.next_team_id());
            (participant.controller, team)
        };

        let character = match controller {
            Controller::Human => self.game.add_local_member(Self::human_player_id()?, team)?,
            Controller::Bot(_) => self.game.add_bot(team)?,
            Controller::Remote(_) => return Err(GameError::CharacterNotFound),
        };

        if let Some(index) = self.participant_index(participant_id) {
            self.participants[index].characters.push(character);
        }
        Ok(character)
    }

    pub fn remove_participant(&mut self, participant_id: ParticipantId) -> Result<(), GameError> {
        self.ensure_editable()?;

        let Some(index) = self.participant_index(participant_id) else {
            return Err(GameError::CharacterNotFound);
        };
        let participant = self.participants.remove(index);
        for character in participant.characters {
            let _ = self.game.kick_member(character);
        }
        Ok(())
    }

    /// Позиция участника в последовательности ходов (с нуля): индекс
    /// его первого персонажа в порядке ходов движка.
    pub fn turn_position(&self, participant_id: ParticipantId) -> Option<usize> {
        self.participant(participant_id)?
            .characters
            .iter()
            .find_map(|character| self.game.characters().get_index_of(character))
    }

    /// Персонаж, занимающий позицию `position` в последовательности ходов.
    pub fn character_at_turn_position(&self, position: usize) -> Option<CharacterId> {
        self.game
            .characters()
            .get_index(position)
            .map(|(id, _)| *id)
    }

    /// Меняет участника местами с тем, кто стоит на позиции
    /// `target_position` (с нуля) в последовательности ходов. Менять
    /// порядок можно только до старта партии.
    pub fn swap_turn_position(
        &mut self,
        participant_id: ParticipantId,
        target_position: usize,
    ) -> Result<(), GameError> {
        self.ensure_editable()?;
        if self.turn_position(participant_id) == Some(target_position) {
            return Ok(());
        }
        let a = self
            .participant(participant_id)
            .and_then(|participant| participant.characters.first().copied())
            .ok_or(GameError::CharacterNotFound)?;
        let b = self
            .game
            .characters()
            .get_index(target_position)
            .map(|(id, _)| *id)
            .ok_or(GameError::CharacterNotFound)?;
        self.game.swap_turn_order(a, b)
    }

    /// Removes one character from a participant. Not exposed by the lobby UI.
    #[allow(dead_code)]
    pub fn remove_character(
        &mut self,
        participant_id: ParticipantId,
        character: CharacterId,
    ) -> Result<(), GameError> {
        self.ensure_editable()?;

        let belongs_to_participant = self
            .participant(participant_id)
            .is_some_and(|participant| participant.characters.contains(&character));
        if !belongs_to_participant {
            return Err(GameError::CharacterNotFound);
        }

        self.game.kick_member(character)?;
        if let Some(index) = self.participant_index(participant_id) {
            self.participants[index]
                .characters
                .retain(|candidate| *candidate != character);
        }
        Ok(())
    }

    /// Switches who controls every character of a participant. Bot numbers are
    /// UI labels, so selecting a bot controller creates it implicitly and no
    /// separate "add bot" action is required.
    pub fn set_participant_controller(
        &mut self,
        participant_id: ParticipantId,
        controller: Controller,
    ) -> Result<(), GameError> {
        self.ensure_editable()?;
        let Some(index) = self.participant_index(participant_id) else {
            return Err(GameError::CharacterNotFound);
        };
        if self.participants[index].controller == controller {
            return Ok(());
        }
        // Сетевые места не назначаются локально: источник истины — сервер.
        if let Controller::Remote(_) = controller {
            return Ok(());
        }

        let member = match controller {
            Controller::Human => MemberId::Player(Self::human_player_id()?),
            Controller::Bot(_) => MemberId::Bot,
            Controller::Remote(_) => return Ok(()),
        };

        let characters = self.participants[index].characters.clone();
        for character in characters {
            self.game.set_controller(character, member)?;
        }

        self.participants[index].controller = controller;
        Ok(())
    }

    /// Every controller the lobby can assign: the occupied bot numbers plus the
    /// lowest free one, so a new bot can be created from the participant row
    /// itself without the list growing on every selection.
    pub fn controller_options(&self) -> Vec<Controller> {
        let mut numbers = self.used_bot_numbers();
        let next = Self::lowest_free_number(&numbers);
        numbers.insert(next);

        let mut options = Vec::with_capacity(numbers.len() + 1);
        options.push(Controller::Human);
        options.extend(numbers.into_iter().map(Controller::Bot));
        options
    }

    pub fn participant_controller(&self, participant_id: ParticipantId) -> Option<Controller> {
        self.participant(participant_id)
            .map(|participant| participant.controller)
    }

    /// Ключ контроллера онлайн-места для селектора в панели участников:
    /// `player{N}` для аккаунтов и `bot{N}` для ботов — как в офлайне,
    /// чтобы место можно было передать конкретному боту с номером.
    pub fn seat_controller_key(&self, participant_id: ParticipantId) -> String {
        match self.participant_controller(participant_id) {
            Some(Controller::Bot(number)) => format!("bot{number}"),
            Some(Controller::Human) => match self.me {
                Some(me) => format!("player{}", me.get()),
                None => String::new(),
            },
            Some(Controller::Remote(id)) => format!("player{id}"),
            None => String::new(),
        }
    }

    /// Номера ботов для селектора онлайн-места: занятые плюс наименьший
    /// свободный, чтобы список не рос при каждом выборе (как офлайн-вариант
    /// [`Self::controller_options`], но без локального игрока).
    #[cfg_attr(
        not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))),
        allow(dead_code)
    )]
    pub fn seat_bot_options(&self) -> Vec<u8> {
        let mut numbers = self.used_bot_numbers();
        numbers.insert(Self::lowest_free_number(&numbers));
        numbers.into_iter().collect()
    }

    pub fn participant_team(&self, participant_id: ParticipantId) -> Option<TeamId> {
        self.participant(participant_id)?
            .characters
            .iter()
            .find_map(|character| {
                self.game
                    .characters()
                    .get(character)
                    .map(|data| data.team_id)
            })
    }

    pub fn participant_stats(&self, participant_id: ParticipantId) -> (u16, u16, u16) {
        let Some(participant) = self.participant(participant_id) else {
            return (0, 0, 0);
        };
        self.character_stats(&participant.characters)
    }

    /// Moves every character of a participant to the chosen team. The lobby UI
    /// works with participants, not individual characters, so a seat always
    /// belongs to exactly one team.
    pub fn set_participant_team(
        &mut self,
        participant_id: ParticipantId,
        team: TeamId,
    ) -> Result<(), GameError> {
        self.ensure_editable()?;
        let Some(participant) = self.participant(participant_id) else {
            return Err(GameError::CharacterNotFound);
        };
        let characters = participant.characters.clone();
        for character in characters {
            self.game.change_team(character, team)?;
        }
        Ok(())
    }

    /// Снимок лобби для сервера: по нему комната воссоздаётся при
    /// переключении Оффлайн → Онлайн, чтобы стол (доска, правила, участники,
    /// команды) не перестраивался. Первое человеческое место — место
    /// создателя, остальные — локальные игроки горячего сиденья.
    #[cfg_attr(
        not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))),
        allow(dead_code)
    )]
    pub fn lobby_setup(&self) -> LobbySetup {
        let size = self.game.board().size();
        let mut my_team = None;
        let mut seats = Vec::new();
        for participant in &self.participants {
            let Some(team) = self.participant_team(participant.id) else {
                continue;
            };
            if my_team.is_none() && participant.controller == Controller::Human {
                my_team = Some(team.get());
                continue;
            }
            let bot = matches!(participant.controller, Controller::Bot(_));
            seats.push(LobbySeat {
                bot,
                team: team.get(),
            });
        }
        LobbySetup {
            width: size.width().get(),
            height: size.height().get(),
            automatic: self.automatic_distribution,
            explosion_mode: self.game.explosion_mode(),
            radius_sq: self.safe_radius(),
            my_team,
            seats,
        }
    }

    // --- match settings ---------------------------------------------------

    pub fn board_size(&self) -> BoardCode {
        self.game.board().size()
    }

    /// Resizes the board. Only allowed outside a running match.
    pub fn set_board_size(&mut self, size: BoardCode) -> Result<(), GameError> {
        self.ensure_editable()?;
        self.game.set_board_size(size)?;
        self.reset_animation();
        Ok(())
    }

    pub fn automatic_distribution(&self) -> bool {
        self.automatic_distribution
    }

    pub fn set_automatic_distribution(&mut self, value: bool) {
        self.enter_edit_mode();
        if !self.game.is_started() {
            self.automatic_distribution = value;
        }
    }

    pub fn explosion_mode(&self) -> ExplosionMode {
        self.game.explosion_mode()
    }

    pub fn set_explosion_mode(&mut self, mode: ExplosionMode) {
        self.enter_edit_mode();
        if !self.game.is_started() {
            let _ = self.game.set_explosion_mode(mode);
        }
    }

    /// Квадрат безопасного радиуса для стартовых клеток.
    pub fn safe_radius(&self) -> u8 {
        self.game.safe_radius()
    }

    /// Верхняя граница [`Self::safe_radius`] для текущей доски и команд.
    pub fn max_safe_radius(&self) -> u8 {
        self.game.max_safe_radius()
    }

    pub fn set_safe_radius(&mut self, radius_sq: u8) -> Result<(), GameError> {
        self.enter_edit_mode();
        self.game.set_safe_radius(radius_sq).map(|_| ())
    }

    /// Ходящий сейчас участник ещё не выбрал стартовую клетку.
    pub fn is_choosing_start_cell(&self) -> bool {
        self.phase == GamePhase::Playing
            && self
                .current_character()
                .is_some_and(|character| self.game.is_starting_action(character))
    }

    // --- game -------------------------------------------------------------

    pub fn current_character(&self) -> Option<CharacterId> {
        self.game
            .current_turn()
            .and_then(|index| self.game.characters().get_index(index).map(|(id, _)| *id))
    }

    pub fn current_is_bot(&self) -> bool {
        self.current_character().is_some_and(|character| {
            self.game
                .characters()
                .get(&character)
                .is_some_and(|data| data.controller == MemberId::Bot)
        })
    }

    pub fn is_human_turn(&self) -> bool {
        if self.phase != GamePhase::Playing || self.current_character().is_none() {
            return false;
        }
        match self.me {
            // В онлайн-комнате ход наш, только если сервер назначил очередь
            // на наше место (удалённые места и боты чужие).
            Some(me) => self
                .current_character()
                .and_then(|character| self.game.characters().get(&character))
                .is_some_and(|data| data.controller == MemberId::Player(me)),
            None => !self.current_is_bot(),
        }
    }

    /// Готовность собственного места в онлайн-комнате; офлайн — `None`.
    pub fn own_ready(&self) -> Option<bool> {
        self.me?;
        self.participants
            .iter()
            .find(|participant| participant.controller == Controller::Human)
            .and_then(|participant| participant.ready)
    }

    pub fn current_team(&self) -> Option<TeamId> {
        self.current_character()
            .and_then(|character| self.game.characters().get(&character))
            .map(|data| data.team_id)
    }

    pub fn character_participant_id(&self, character: CharacterId) -> Option<ParticipantId> {
        self.participants
            .iter()
            .find(|participant| participant.characters.contains(&character))
            .map(|participant| participant.id)
    }

    pub fn current_participant_id(&self) -> Option<ParticipantId> {
        self.current_character()
            .and_then(|character| self.character_participant_id(character))
    }

    /// Ходящий сейчас участник.
    pub fn current_participant(&self) -> Option<&Participant> {
        self.current_participant_id()
            .and_then(|id| self.participant(id))
    }

    /// The next distinct participant in turn order, skipping eliminated seats.
    pub fn next_participant_id(&self) -> Option<ParticipantId> {
        let current = self.current_participant_id()?;
        let start = self.game.current_turn()?;
        let len = self.game.characters().len();
        if len == 0 {
            return None;
        }
        (1..=len).find_map(|step| {
            let character = self
                .game
                .characters()
                .get_index((start + step) % len)
                .map(|(id, _)| *id)?;
            if self.is_eliminated(character) {
                return None;
            }
            let participant = self.character_participant_id(character)?;
            (participant != current).then_some(participant)
        })
    }

    /// Старт требует минимум две разные команды, в каждой есть участник
    /// (бот или игрок — не важно).
    pub fn can_start(&self) -> bool {
        self.phase != GamePhase::Playing
            && self.game.characters().len() >= 2
            && self.teams().len() >= 2
    }

    pub fn can_ready(&self) -> bool {
        matches!(self.phase, GamePhase::Lobby | GamePhase::Finished) && self.can_start()
    }

    pub fn can_skip(&self) -> bool {
        !self.exploding
            && self.is_human_turn()
            && self
                .current_character()
                .is_some_and(|character| !self.game.is_starting_action(character))
    }

    pub fn can_decrease(&self) -> bool {
        self.can_skip()
            && self.current_team().is_some_and(|team| {
                self.game
                    .board()
                    .iter()
                    .any(|(_, cell)| cell.owner == Some(team) && cell.value > 0)
            })
    }

    pub fn can_surrender(&self) -> bool {
        !self.exploding && self.surrender_character().is_some()
    }

    /// Есть ли у игрока своё место за играющей партией: у наблюдателя
    /// (например, два бота друг против друга) кнопки сдачи нет вовсе.
    /// В отличие от [`Self::can_surrender`] не зависит от анимации взрывов,
    /// поэтому видимость кнопки не мерцает при каждом ходе.
    pub fn has_playable_seat(&self) -> bool {
        self.phase == GamePhase::Playing && self.surrender_character().is_some()
    }

    /// Место, которым игрок сдаётся: своим ходом — текущий персонаж, иначе —
    /// первое живое место под нашим контролем (онлайн — наше сетевое место,
    /// офлайн — первый человек за столом hot-seat).
    pub fn surrender_character(&self) -> Option<CharacterId> {
        if self.phase != GamePhase::Playing {
            return None;
        }
        if self.is_human_turn()
            && let Some(character) = self.current_character()
            && !self.is_eliminated(character)
        {
            return Some(character);
        }
        self.participants
            .iter()
            .filter(|participant| participant.controller == Controller::Human)
            .find_map(|participant| {
                participant
                    .characters
                    .iter()
                    .find(|character| !self.is_eliminated(**character))
            })
            .copied()
    }

    /// Легальные клетки хода человека одной маской: стартовые клетки, пока
    /// команда не заняла поле, иначе — уже её клетки. Считается один раз на
    /// состояние: поклеточная проверка пересчитывала бы стартовые клетки для
    /// каждой клетки доски и была квадратичной.
    pub fn legal_targets(&self) -> SlotMask {
        let mut mask = SlotMask::default();
        if !self.is_human_turn() {
            return mask;
        }
        let Some(character) = self.current_character() else {
            return mask;
        };
        for pos in self.game.legal_targets(character) {
            mask.insert(pos.slot());
        }
        mask
    }

    /// Ready up, or start a rematch with the same participants.
    pub fn ready(&mut self) -> Result<(), GameError> {
        if !self.can_ready() {
            return Ok(());
        }
        // A finished match may still be "started" inside the engine; reset it
        // before starting the rematch.
        self.enter_edit_mode();
        self.game.start(self.automatic_distribution)?;
        self.phase = GamePhase::Playing;
        self.winner = None;
        self.exploding = false;
        self.reset_animation();
        // Первое место может принадлежать боту (в том числе когда ботов все):
        // его ход запускается сразу, дальше партию гоняет анимация.
        self.begin_bot_turn()
    }

    /// Переигровка завершённой партии: чистая доска и статистика, тот же
    /// состав участников. Работает только из Finished, иначе — no-op.
    pub fn rematch(&mut self) -> Result<(), GameError> {
        self.enter_edit_mode();
        Ok(())
    }

    /// Ход живого участника: применяет изменение клетки и переходит в
    /// режим пошаговой анимации взрывов (см. [`Self::animation_step`]).
    /// Во время анимации новые ходы игнорируются.
    pub fn make_move(&mut self, pos: BoardCode, mode: MoveMode) -> Result<(), GameError> {
        if self.exploding {
            return Ok(());
        }
        let character = self.require_human_turn()?;
        let before = self.cell_values();
        self.game
            .start_move(character, pos, mode == MoveMode::Increase)?;
        // Предсказание: этот же шаг вернётся с сервера и вырежется из реплея.
        self.predicted = Some(ReplayStep::Move {
            member: character,
            pos,
            increase: mode == MoveMode::Increase,
        });
        self.begin_animation_burst();
        self.snapshot_deltas(&before);
        self.exploding = true;
        Ok(())
    }

    pub fn skip(&mut self) -> Result<(), GameError> {
        if self.exploding {
            return Ok(());
        }
        let character = self.require_human_turn()?;
        let winner = self.game.skip(character)?;
        self.predicted = Some(ReplayStep::Skip { member: character });
        self.settle(winner);
        self.begin_bot_turn()?;
        Ok(())
    }

    /// Сдача. Сдаться можно в любой момент партии, даже не своим ходом
    /// (место выбирает [`Self::surrender_character`]).
    pub fn surrender(&mut self) -> Result<(), GameError> {
        if self.exploding {
            return Ok(());
        }
        let Some(character) = self.surrender_character() else {
            return Err(GameError::GameNotStarted);
        };
        let winner = self.game.surrender(character)?;
        self.predicted = Some(ReplayStep::Surrender { member: character });
        self.settle(winner);
        self.begin_bot_turn()?;
        Ok(())
    }

    /// Принудительное завершение партии администратором: победителя
    /// определяет движок по текущей расстановке, доска сохраняется
    /// на экране результата. Идущая анимация взрывов обрывается: партия
    /// окончена, кадры не доигрываются.
    pub fn end_game(&mut self) -> Result<(), GameError> {
        if self.phase != GamePhase::Playing {
            return Err(GameError::GameNotStarted);
        }
        let winner = self.game.end_game()?;
        self.exploding = false;
        self.replay.clear();
        self.predicted = None;
        self.reset_animation();
        self.settle(Some(winner));
        Ok(())
    }

    /// Доигрывает взрывы текущего хода на одну волну. Возвращает `true`,
    /// пока анимация продолжается. Каждая волна обновляет `changes` и
    /// `animation_tick`, поэтому клетки перерисовываются постепенно;
    /// скорость задает только вызывающая сторона, движок ничего не ждет.
    pub fn animation_step(&mut self) -> Result<bool, GameError> {
        if self.phase != GamePhase::Playing || !self.exploding {
            return Ok(false);
        }

        let before = self.cell_values();
        let more = self.game.explode_wave();
        // Кадр волны: показать, даже если она оказалась последней
        // (исход партии решен) и цепочка больше не продолжится.
        // Бомбовая волна может не менять значений (урон зажат в ноль
        // на минимуме), но тик всё равно растёт: он чередует CSS-классы,
        // и клетка, меняющаяся в нескольких волнах, каждый раз
        // перезапускает пульсацию заново.
        if more || self.cell_values() != before {
            self.animation_tick = self.animation_tick.wrapping_add(1);
            self.snapshot_deltas(&before);
        }

        if more {
            return Ok(true);
        }

        self.exploding = false;
        let winner = self.game.finish_move()?;
        self.settle(winner);
        // Доигрываем очередь серверного реплея, а ботов — только когда
        // реплей иссяк: их ходы в онлайне предсказываются локально.
        if !self.replay.is_empty() {
            self.begin_replay_step()?;
        } else {
            self.begin_bot_turn()?;
        }
        Ok(self.exploding)
    }

    /// Ход доигрывается взрывами: ввод заблокирован до конца анимации.
    pub fn is_animating(&self) -> bool {
        self.exploding
    }

    /// Водяной знак последнего принятого снапшота комнаты: сетевой
    /// клиент отбрасывает устаревшие снапшоты по нему.
    #[cfg_attr(
        not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))),
        allow(dead_code)
    )]
    pub fn sync_seq(&self) -> u64 {
        self.sync_seq
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    // --- board / team view helpers ---------------------------------------

    pub fn teams(&self) -> Vec<TeamId> {
        self.game
            .characters()
            .values()
            .map(|character| character.team_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Hues are spread evenly around the wheel, so every team is as far as
    /// possible from the others. Saturation and lightness are fixed in CSS.
    pub fn team_hue(&self, team: TeamId) -> u16 {
        self.team_hues()
            .into_iter()
            .find(|&(candidate, _)| candidate == team)
            .map(|(_, hue)| hue)
            .unwrap_or(0)
    }

    /// Пара «команда — оттенок» для всех команд. Считается один раз за
    /// перерисовку: поклеточный вызов [`Self::team_hue`] перестраивал бы набор
    /// команд для каждой раскрашенной клетки.
    pub fn team_hues(&self) -> Vec<(TeamId, u16)> {
        let teams = self.teams();
        let count = teams.len().max(1) as u32;
        teams
            .iter()
            .enumerate()
            .map(|(index, team)| {
                let hue = ((u32::from(TEAM_HUE_OFFSET) + index as u32 * 360 / count) % 360) as u16;
                (*team, hue)
            })
            .collect()
    }

    /// Occupied teams plus the lowest free one, so every current value stays
    /// selectable while the list never grows unnecessarily.
    pub fn team_options(&self) -> Vec<TeamId> {
        let mut numbers = self.used_team_numbers();
        numbers.insert(Self::lowest_free_number(&numbers));
        numbers.into_iter().filter_map(TeamId::new).collect()
    }

    #[allow(dead_code)]
    pub fn character_views(&self, participant_id: ParticipantId) -> Vec<CharacterView> {
        self.participant(participant_id)
            .map(|participant| {
                participant
                    .characters
                    .iter()
                    .filter_map(|character| {
                        self.game
                            .characters()
                            .get(character)
                            .map(|data| CharacterView {
                                id: *character,
                                team: data.team_id,
                            })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn team_views(&self) -> Vec<TeamView> {
        // Оттенки и суммы считаются один раз на всю панель: покомандные вызовы
        // `team_hue`/`team_totals` заново обходили бы доску и команды.
        let hues = self.team_hues();
        let totals = self.team_totals_all();
        self.teams()
            .into_iter()
            .map(|team| {
                let (cells, mass) = totals.get(&team).copied().unwrap_or((0, 0));
                let hue = hues
                    .iter()
                    .find(|&&(candidate, _)| candidate == team)
                    .map(|&(_, hue)| hue)
                    .unwrap_or(0);
                let participants: Vec<_> = self
                    .participants
                    .iter()
                    .enumerate()
                    .filter_map(|(index, participant)| {
                        let characters: Vec<CharacterId> = participant
                            .characters
                            .iter()
                            .copied()
                            .filter(|character| {
                                self.game
                                    .characters()
                                    .get(character)
                                    .is_some_and(|data| data.team_id == team)
                            })
                            .collect();
                        if characters.is_empty() {
                            return None;
                        }

                        // Участник проиграл, когда выбыли все его персонажи в этой
                        // команде: карточка тускнеет, это видно и во время
                        // партии, и после её завершения (до правок в лобби).
                        let lost = characters
                            .iter()
                            .all(|character| self.is_eliminated(*character));

                        let (wins, losses, moves) = self.character_stats(&characters);

                        Some(TeamParticipantView {
                            id: participant.id,
                            index: index + 1,
                            controller: participant.controller,
                            characters,
                            name: participant.name.clone(),
                            ready: participant.ready,
                            wins,
                            losses,
                            moves,
                            lost,
                        })
                    })
                    .collect();
                let lost =
                    !participants.is_empty() && participants.iter().all(|member| member.lost);
                TeamView {
                    team,
                    hue,
                    cells,
                    mass,
                    participants,
                    lost,
                }
            })
            .collect()
    }

    /// Снимок доски и хода для отрисовки клеток. Клиент держит его в мемо:
    /// смена настроек, не трогающих доску (безопасный радиус), не меняет снимок
    /// и не перерисовывает ни одну клетку.
    pub fn board_view(&self) -> BoardView {
        BoardView {
            board: self.game.board().clone(),
            phase: self.phase,
            current_team: self.current_team(),
            explosion_mode: self.game.explosion_mode(),
            animation_tick: self.animation_tick,
            changes: self.changes.clone(),
        }
    }

    /// Занятые клетки и масса каждой команды одним проходом по доске:
    /// покомандный обход был бы `O(команд × клеток)`.
    fn team_totals_all(&self) -> HashMap<TeamId, (u16, i32)> {
        let mut totals: HashMap<TeamId, (u16, i32)> = HashMap::new();
        for (_, cell) in self.game.board().iter() {
            if let Some(team) = cell.owner {
                let entry = totals.entry(team).or_insert((0, 0));
                entry.0 = entry.0.saturating_add(1);
                entry.1 = entry.1.saturating_add(i32::from(cell.value));
            }
        }
        totals
    }

    // --- internals --------------------------------------------------------

    fn require_human_turn(&self) -> Result<CharacterId, GameError> {
        if self.phase != GamePhase::Playing {
            return Err(GameError::GameNotStarted);
        }
        if !self.is_human_turn() {
            return Err(GameError::NotYourTurn);
        }
        self.current_character().ok_or(GameError::NotYourTurn)
    }

    fn is_eliminated(&self, character: CharacterId) -> bool {
        self.game
            .characters()
            .get(&character)
            .map(|c| c.stats.losses > 0)
            .unwrap_or(false)
    }

    fn next_team_id(&self) -> TeamId {
        TeamId::new(Self::lowest_free_number(&self.used_team_numbers())).unwrap_or(TeamId::MIN)
    }

    /// Занятые номера команд.
    fn used_team_numbers(&self) -> BTreeSet<u8> {
        self.game
            .characters()
            .values()
            .map(|character| character.team_id.get())
            .collect()
    }

    /// Editing after a finished match returns to a clean lobby while keeping
    /// the participant list.
    fn enter_edit_mode(&mut self) {
        if self.phase != GamePhase::Finished {
            return;
        }
        self.game.stop();
        self.game.reset_stats();
        self.phase = GamePhase::Lobby;
        self.winner = None;
        self.exploding = false;
        self.reset_animation();
    }

    /// Правки лобби возможны только до старта партии: возврат из завершённой
    /// партии в чистое лобби и отказ, если партия уже идёт.
    fn ensure_editable(&mut self) -> Result<(), GameError> {
        self.enter_edit_mode();
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        Ok(())
    }

    /// Суммарная статистика персонажей: победы, поражения, ходы.
    fn character_stats(&self, characters: &[CharacterId]) -> (u16, u16, u16) {
        let mut wins = 0u16;
        let mut losses = 0u16;
        let mut moves = 0u16;
        for character in characters {
            if let Some(data) = self.game.characters().get(character) {
                let stat = data.stats;
                wins = wins.saturating_add(u16::from(stat.wins));
                losses = losses.saturating_add(u16::from(stat.losses));
                moves = moves.saturating_add(stat.total_moves());
            }
        }
        (wins, losses, moves)
    }

    fn begin_animation_burst(&mut self) {
        self.animation_tick = self.animation_tick.wrapping_add(1);
        let len = self.game.board().cells().len();
        self.changes.clear();
        self.changes.resize(len, 0);
    }

    /// Сбрасывает кадры анимации: без дельт клетки не анимируются.
    /// Используется и мгновенным режимом (задержка 0) после доигрывания
    /// хода целиком: кадры не показывались — воспроизводить их не нужно.
    pub fn reset_animation(&mut self) {
        self.changes.clear();
        self.changes.resize(self.game.board().cells().len(), 0);
        self.animation_tick = 0;
    }

    /// Проигрывает ходы ботов подряд, пока очередь не вернется к человеку.
    /// Ход со взрывами переводит бота в режим анимации, как и человека.
    fn begin_bot_turn(&mut self) -> Result<(), GameError> {
        while self.phase == GamePhase::Playing && !self.exploding && self.current_is_bot() {
            let bot = self
                .current_character()
                .expect("a bot turn always has a current character");
            match self.game.bot_move(bot, &BotConfig::STANDARD) {
                Some(pos) => {
                    let before = self.cell_values();
                    self.game.start_move(bot, pos, true)?;
                    self.begin_animation_burst();
                    self.snapshot_deltas(&before);
                    self.exploding = true;
                }
                None => {
                    let winner = self.game.skip(bot)?;
                    self.settle(winner);
                }
            }
        }
        Ok(())
    }

    /// Доигрывает очередь серверного реплея до первого хода со взрывами:
    /// мгновенные шаги (старт, пропуск, сдача) применяются подряд, ход
    /// клетки переводит партию в режим анимации — волна за волной, как
    /// локальный ход. Шаги уже проверены в [`Self::start_replay`], поэтому
    /// ошибки движка здесь невозможны.
    fn begin_replay_step(&mut self) -> Result<(), GameError> {
        while !self.exploding && !self.replay.is_empty() {
            let step = self.replay.remove(0);
            match step {
                ReplayStep::Start { automatic } => {
                    self.game.start(automatic)?;
                    self.phase = GamePhase::Playing;
                    self.winner = None;
                    self.reset_animation();
                }
                ReplayStep::Move {
                    member,
                    pos,
                    increase,
                } => {
                    let before = self.cell_values();
                    self.game.start_move(member, pos, increase)?;
                    self.begin_animation_burst();
                    self.snapshot_deltas(&before);
                    self.exploding = true;
                }
                ReplayStep::Skip { member } => {
                    let winner = self.game.skip(member)?;
                    self.settle(winner);
                }
                ReplayStep::Surrender { member } => {
                    let winner = self.game.surrender(member)?;
                    self.settle(winner);
                }
            }
        }
        if self.replay.is_empty() {
            // Реплей иссяк: локальная партия догнала сервер, дальше боты
            // (если ход опять их) предсказываются локально.
            self.sync_seq = self.replay_seq;
            self.replay_seq = 0;
            self.begin_bot_turn()?;
        } else if self.phase != GamePhase::Playing {
            // Очередь не пуста, но партия закончилась: следующих ходов
            // не будет, хвост реплея не имеет смысла.
            self.sync_seq = self.replay_seq;
            self.replay_seq = 0;
            self.replay.clear();
        }
        Ok(())
    }

    /// Фиксирует результат завершившегося хода: победителя или конец
    /// партии.
    fn settle(&mut self, winner: Option<TeamId>) {
        if let Some(team) = winner {
            self.finish(team);
        } else if !self.game.is_started() {
            let team = Self::winner_from_stats(&self.game);
            self.phase = GamePhase::Finished;
            self.winner = team;
        }
    }

    fn cell_values(&self) -> Vec<i16> {
        self.game
            .board()
            .cells()
            .iter()
            .map(|cell| i16::from(cell.value))
            .collect()
    }

    /// Записывает в `changes` изменение значений за последний шаг. Значения
    /// заменяются, а не суммируются за ход: клетка анимируется только в той
    /// волне, где изменилась, — уже взорвавшаяся клетка не пульсирует дальше.
    fn snapshot_deltas(&mut self, before: &[i16]) {
        for (index, cell) in self.game.board().cells().iter().enumerate() {
            let Some(previous) = before.get(index) else {
                continue;
            };
            if let Some(slot) = self.changes.get_mut(index) {
                *slot = i16::from(cell.value) - *previous;
            }
        }
    }

    fn finish(&mut self, team: TeamId) {
        self.phase = GamePhase::Finished;
        self.winner = Some(team);
    }

    /// Команда-победитель по статистике завершившегося матча.
    fn winner_from_stats(game: &Game) -> Option<TeamId> {
        game.characters()
            .iter()
            .find_map(|(&_character, data)| (data.stats.wins > 0).then_some(data.team_id))
    }
}

impl Default for LocalGame {
    fn default() -> Self {
        Self::new()
    }
}

/// Игрок онлайн-комнаты для панели игроков над списком команд.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))),
    allow(dead_code)
)]
pub struct RoomPlayer {
    pub id: PlayerId,
    pub name: Option<String>,
    pub ready: bool,
    pub admin: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU8;

    fn started() -> LocalGame {
        let mut game = LocalGame::new();
        game.ready().expect("a local match starts");
        game
    }

    fn current_human_cell(game: &LocalGame) -> BoardCode {
        let team = game.current_team().expect("a human team owns the turn");
        game.game
            .board()
            .iter()
            .find(|(_, cell)| cell.owner == Some(team))
            .map(|(pos, _)| pos)
            .expect("the automatic distribution gives every team one cell")
    }

    #[test]
    fn ready_starts_the_match_with_a_human_turn() {
        let game = started();
        assert_eq!(game.phase, GamePhase::Playing);
        assert!(game.is_human_turn());
        assert!(!game.is_animating());
        assert_eq!(game.animation_tick, 0);
    }

    #[test]
    fn human_move_plays_explosions_wave_by_wave_and_the_bot_answers() {
        let mut game = started();
        let pos = current_human_cell(&game);

        game.make_move(pos, MoveMode::Increase).unwrap();

        // The change is applied immediately and the burst starts animating.
        assert!(game.is_animating());
        assert_eq!(game.animation_tick, 1);
        assert!(game.changes.iter().any(|delta| *delta != 0));

        // Waves play one per step, then the turn settles and the bot answers.
        while game.animation_step().unwrap() {}
        assert!(!game.is_animating());
        assert!(game.phase == GamePhase::Finished || game.is_human_turn());
    }

    #[test]
    fn invalid_move_is_not_forwarded_and_leaves_the_state_untouched() {
        let mut game = started();
        let before = game.game.clone();
        let bot_team = game
            .teams()
            .into_iter()
            .find(|team| Some(*team) != game.current_team())
            .expect("the default match has two teams");
        let pos = game
            .game
            .board()
            .iter()
            .find(|(_, cell)| cell.owner == Some(bot_team))
            .map(|(pos, _)| pos)
            .expect("the bot starts with a cell");

        assert!(game.make_move(pos, MoveMode::Increase).is_err());

        assert_eq!(game.game, before);
        assert!(!game.is_animating());
        assert_eq!(game.animation_tick, 0);
    }

    #[test]
    fn participants_and_characters_can_be_added_and_removed() {
        let mut game = LocalGame::new();
        let initial_participants = game.participants.len();

        let human = game.add_human().unwrap();
        assert_eq!(game.participants.len(), initial_participants + 1);
        let characters_before = game.character_views(human).len();
        let character = game.add_character(human).unwrap();
        assert_eq!(game.character_views(human).len(), characters_before + 1);

        game.remove_character(human, character).unwrap();
        assert_eq!(game.character_views(human).len(), characters_before);

        game.remove_participant(human).unwrap();
        assert!(
            game.participants
                .iter()
                .all(|participant| participant.id != human)
        );
    }

    #[test]
    fn turn_order_can_be_swapped_before_start() {
        let mut game = LocalGame::new();
        // Третий участник нужен, чтобы проверить переустановку позиций 1..3.
        game.add_bot().unwrap();
        let ids: Vec<ParticipantId> = game.participants.iter().map(|p| p.id).collect();
        assert_eq!(ids.len(), 3);
        // Стартовый порядок повторяет список участников.
        assert_eq!(game.turn_position(ids[0]), Some(0));
        assert_eq!(game.turn_position(ids[1]), Some(1));
        assert_eq!(game.turn_position(ids[2]), Some(2));

        // Игрок 2 меняется местами с игроком 3: тот, кто стоял третьим,
        // становится вторым.
        game.swap_turn_position(ids[1], 2).unwrap();
        assert_eq!(game.turn_position(ids[1]), Some(2));
        assert_eq!(game.turn_position(ids[2]), Some(1));
        assert_eq!(game.turn_position(ids[0]), Some(0));

        // Свой номер — no-op.
        game.swap_turn_position(ids[2], 1).unwrap();
        assert_eq!(game.turn_position(ids[2]), Some(1));

        game.ready().unwrap();
        // После старта порядок зафиксирован и не пересортировывается.
        assert_eq!(game.turn_position(ids[0]), Some(0));
        assert_eq!(game.turn_position(ids[1]), Some(2));
        assert_eq!(game.turn_position(ids[2]), Some(1));
        assert_eq!(
            game.swap_turn_position(ids[1], 0),
            Err(GameError::GameAlreadyStarted)
        );
    }

    #[test]
    fn turn_order_swap_updates_an_adopted_snapshot() {
        let me = PlayerId::new(1).unwrap();
        let guest = PlayerId::new(2).unwrap();
        let mut room = Room::new(me, true);
        room.join(guest, room.next_team_id()).unwrap();
        let ids: Vec<CharacterId> = room.game().characters().keys().copied().collect();

        let mut state = LocalGame::from_room(&room, me, &HashMap::new());
        assert_eq!(state.turn_position(u32::from(ids[0].get())), Some(0));
        assert_eq!(state.turn_position(u32::from(ids[1].get())), Some(1));

        // Админ меняет порядок ходов: сервер отдаёт новый снапшот.
        room.swap_turn_order(me, ids[0], ids[1]).unwrap();
        let next = LocalGame::from_room(&room, me, &HashMap::new());

        // Порядок — часть состояния партии: снапшот не равен прежнему
        // состоянию, поэтому ветка «предсказание подтвердилось» его не съест.
        assert!(state.game != next.game);

        // Путь absorb применяет новый порядок — селекторы позиций
        // обновятся после ответа сервера.
        state.absorb(next);
        assert_eq!(state.turn_position(u32::from(ids[0].get())), Some(1));
        assert_eq!(state.turn_position(u32::from(ids[1].get())), Some(0));
    }

    #[test]
    fn admin_end_game_finishes_the_match() {
        let mut game = LocalGame::new();
        game.ready().unwrap();
        assert_eq!(game.phase, GamePhase::Playing);
        assert!(game.has_playable_seat());

        game.end_game().unwrap();

        assert_eq!(game.phase, GamePhase::Finished);
        assert!(game.winner.is_some());
        // Повторное завершение уже законченной партии отклоняется.
        assert_eq!(game.end_game(), Err(GameError::GameNotStarted));
    }

    #[test]
    fn bot_only_match_has_no_playable_seat() {
        let mut game = LocalGame::new();
        let human = game
            .participants
            .iter()
            .find(|participant| participant.controller == Controller::Human)
            .map(|participant| participant.id)
            .unwrap();
        game.set_participant_controller(human, Controller::Bot(3))
            .unwrap();
        game.ready().unwrap();

        // Два бота друг против друга: местная сдача недоступна, но
        // администратор может завершить партию принудительно.
        assert_eq!(game.phase, GamePhase::Playing);
        assert!(!game.has_playable_seat());
        assert!(!game.can_surrender());

        game.end_game().unwrap();
        assert_eq!(game.phase, GamePhase::Finished);
    }

    #[test]
    fn characters_can_be_assigned_to_teams() {
        let mut game = LocalGame::new();
        let participant = game.participants[0].id;
        let character = game.character_views(participant)[0].id;
        let team = TeamId::new(3).unwrap();

        game.set_participant_team(participant, team).unwrap();

        assert_eq!(
            game.game
                .characters()
                .get(&character)
                .map(|data| data.team_id),
            Some(team)
        );
        assert!(game.teams().contains(&team));
    }

    #[test]
    fn a_fresh_lobby_puts_the_player_on_team_one_and_the_bot_on_team_two() {
        let game = LocalGame::new();
        let player = game
            .participants
            .iter()
            .find(|participant| participant.controller == Controller::Human)
            .unwrap();
        let bot = game
            .participants
            .iter()
            .find(|participant| matches!(participant.controller, Controller::Bot(1)))
            .unwrap();

        assert_eq!(
            game.character_views(player.id)[0].team,
            TeamId::new(1).unwrap()
        );
        assert_eq!(
            game.character_views(bot.id)[0].team,
            TeamId::new(2).unwrap()
        );
    }

    #[test]
    fn match_settings_are_applied_on_start() {
        let mut game = LocalGame::new();
        let (w, h) = (NonZeroU8::new(5).unwrap(), NonZeroU8::new(7).unwrap());
        let size = BoardCode::from_size(w, h).unwrap();
        game.set_board_size(size).unwrap();
        game.set_automatic_distribution(false);

        assert_eq!(game.board_size(), size);
        assert!(!game.automatic_distribution());

        game.ready().unwrap();

        assert!(!game.automatic_distribution());
        assert!(
            game.game
                .board()
                .iter()
                .all(|(_, cell)| cell.owner.is_none())
        );
    }

    fn tid(id: u8) -> TeamId {
        TeamId::new(id).unwrap()
    }

    #[test]
    fn team_options_keep_the_current_team_available() {
        let mut game = LocalGame::new();
        assert_eq!(game.team_options(), vec![tid(1), tid(2), tid(3)]);

        let bot = game
            .participants
            .iter()
            .find(|participant| matches!(participant.controller, Controller::Bot(1)))
            .map(|participant| participant.id)
            .unwrap();
        game.set_participant_team(bot, tid(5)).unwrap();

        assert_eq!(game.participant_team(bot), Some(tid(5)));
        assert_eq!(game.team_options(), vec![tid(1), tid(2), tid(5)]);
    }

    #[test]
    fn safe_radius_is_limited_by_the_board_and_teams() {
        let mut game = LocalGame::new();
        let max = game.max_safe_radius();
        assert!(max > 0);

        game.set_safe_radius(u8::MAX).unwrap();
        assert_eq!(game.safe_radius(), max);

        game.ready().unwrap();
        assert_eq!(game.set_safe_radius(0), Err(GameError::GameAlreadyStarted));
    }

    #[test]
    fn explosion_mode_is_a_match_setting() {
        let mut game = LocalGame::new();
        assert_eq!(game.explosion_mode(), ExplosionMode::BorderKeep(false));

        game.set_explosion_mode(ExplosionMode::BorderLoss(false));
        assert_eq!(game.explosion_mode(), ExplosionMode::BorderLoss(false));

        game.ready().unwrap();
        game.set_explosion_mode(ExplosionMode::CriticalMass(true));
        // Игра уже началась, поэтому режим не меняется.
        assert_eq!(game.explosion_mode(), ExplosionMode::BorderLoss(false));
    }

    #[test]
    fn participants_can_switch_between_human_and_bot() {
        let mut game = LocalGame::new();
        let human = game
            .participants
            .iter()
            .find(|participant| participant.controller == Controller::Human)
            .map(|participant| participant.id)
            .unwrap();
        let character = game.character_views(human)[0].id;

        game.set_participant_controller(human, Controller::Bot(1))
            .unwrap();
        assert_eq!(
            game.game
                .characters()
                .get(&character)
                .map(|data| data.controller),
            Some(MemberId::Bot)
        );

        game.set_participant_controller(human, Controller::Human)
            .unwrap();
        assert!(matches!(
            game.game
                .characters()
                .get(&character)
                .map(|data| data.controller),
            Some(MemberId::Player(_))
        ));
    }

    #[test]
    fn controller_options_reuse_free_bot_numbers() {
        let mut game = LocalGame::new();
        assert_eq!(
            game.controller_options(),
            vec![Controller::Human, Controller::Bot(1), Controller::Bot(2)]
        );

        let bot = game
            .participants
            .iter()
            .find(|participant| matches!(participant.controller, Controller::Bot(1)))
            .map(|participant| participant.id)
            .unwrap();

        // Picking the free number must not create another free number.
        game.set_participant_controller(bot, Controller::Bot(2))
            .unwrap();
        assert_eq!(
            game.controller_options(),
            vec![Controller::Human, Controller::Bot(1), Controller::Bot(2)]
        );

        game.set_participant_controller(bot, Controller::Bot(1))
            .unwrap();
        assert_eq!(
            game.controller_options(),
            vec![Controller::Human, Controller::Bot(1), Controller::Bot(2)]
        );
    }

    #[test]
    fn adding_a_bot_uses_the_lowest_free_number() {
        let mut game = LocalGame::new();
        let added = game.add_bot().unwrap();
        let added = game
            .participants
            .iter()
            .find(|participant| participant.id == added)
            .unwrap();
        assert_eq!(added.controller, Controller::Bot(2));
    }

    #[test]
    fn team_colors_are_evenly_spread() {
        let game = LocalGame::new();
        let first = TeamId::new(1).unwrap();
        let second = TeamId::new(2).unwrap();

        let first_hue = game.team_hue(first);
        let second_hue = game.team_hue(second);

        assert_ne!(first_hue, second_hue);
        assert_eq!((first_hue + 180) % 360, second_hue);
    }

    #[test]
    fn the_match_requires_two_characters_on_two_teams() {
        let mut game = LocalGame::new();
        assert!(game.can_start());

        let bot = game
            .participants
            .iter()
            .find(|participant| matches!(participant.controller, Controller::Bot(_)))
            .map(|participant| participant.id)
            .unwrap();
        game.remove_participant(bot).unwrap();

        assert!(!game.can_start());
    }

    #[test]
    fn the_match_requires_two_occupied_teams() {
        let mut game = LocalGame::new();
        let human = game
            .participants
            .iter()
            .find(|participant| participant.controller == Controller::Human)
            .map(|participant| participant.id)
            .unwrap();

        // Оба участника в одной команде — старт невозможен.
        game.set_participant_team(human, tid(2)).unwrap();
        assert!(!game.can_start());
        assert!(game.ready().is_ok());
        assert_eq!(game.phase, GamePhase::Lobby);
    }

    #[test]
    fn an_all_bot_lobby_can_start_the_match() {
        let mut game = LocalGame::new();
        let human = game
            .participants
            .iter()
            .find(|participant| participant.controller == Controller::Human)
            .map(|participant| participant.id)
            .unwrap();
        game.set_participant_controller(human, Controller::Bot(2))
            .unwrap();

        assert!(game.can_start());

        game.ready().unwrap();
        assert_eq!(game.phase, GamePhase::Playing);
        assert!(!game.is_human_turn());
        // Ход первого бота уже сделан и доигрывается анимацией.
        assert!(game.is_animating());
    }

    #[test]
    fn surrendered_participant_and_team_are_marked_lost() {
        let mut game = started();
        let human = game
            .participants
            .iter()
            .find(|participant| participant.controller == Controller::Human)
            .map(|participant| participant.id)
            .unwrap();

        game.surrender().unwrap();
        assert_eq!(game.phase, GamePhase::Finished);

        let views = game.team_views();
        let human_team_view = views
            .iter()
            .find(|view| view.participants.iter().any(|member| member.id == human))
            .unwrap();
        assert!(
            human_team_view
                .participants
                .iter()
                .all(|member| member.lost)
        );
        assert!(human_team_view.lost);
    }

    #[test]
    fn surrender_finishes_the_match() {
        let mut game = started();

        game.surrender().unwrap();

        assert_eq!(game.phase, GamePhase::Finished);
        assert!(game.winner.is_some());
    }

    #[test]
    fn surrender_works_out_of_turn_for_our_own_seat() {
        let owner = PlayerId::new(11).unwrap();
        let guest = PlayerId::new(12).unwrap();
        let mut room = Room::new(owner, true);
        let team = room.next_team_id();
        room.join(guest, team).unwrap();
        // Третья команда — бот: после сдачи гостя партия продолжается.
        room.add_bot(owner, room.next_team_id()).unwrap();
        room.set_ready(owner, true).unwrap();
        room.set_ready(guest, true).unwrap();
        assert!(room.game().is_started());

        let mut game = LocalGame::from_room(&room, guest, &HashMap::new());
        assert_eq!(game.phase, GamePhase::Playing);
        // Сейчас ход владельца, а не наш.
        assert!(!game.is_human_turn());
        assert!(game.can_surrender());

        game.surrender().unwrap();

        // Своё место выбывает, ход остаётся владельцу, партия продолжается.
        assert_eq!(game.phase, GamePhase::Playing);
        let guest_seat = game
            .participants
            .iter()
            .find(|participant| participant.controller == Controller::Human)
            .unwrap()
            .characters[0];
        assert!(game.is_eliminated(guest_seat));
        assert!(!game.is_human_turn());

        // Тот же результат у сервера: предсказание совпало.
        room.surrender(guest, guest_seat).unwrap();
        let server = LocalGame::from_room(&room, guest, &HashMap::new());
        assert_eq!(game.game, server.game);
    }

    #[test]
    fn rematch_returns_the_finished_match_to_the_lobby() {
        let mut game = started();
        game.surrender().unwrap();
        assert_eq!(game.phase, GamePhase::Finished);

        let participants = game.participants.clone();
        game.rematch().unwrap();

        // Чистое лобби: доска обнулена, статистика сброшена, тот же состав.
        assert_eq!(game.phase, GamePhase::Lobby);
        assert!(game.winner.is_none());
        assert_eq!(game.participants, participants);
        assert!(
            game.game
                .board()
                .iter()
                .all(|(_, cell)| cell.value == 0 && cell.owner.is_none())
        );
        assert!(
            game.game
                .characters()
                .values()
                .all(|character| character.stats.wins == 0 && character.stats.losses == 0)
        );

        // Новая партия стартует той же кнопкой готовности.
        game.ready().unwrap();
        assert_eq!(game.phase, GamePhase::Playing);

        // Из идущей партии «Заново» недоступно: no-op без ошибки.
        game.rematch().unwrap();
        assert_eq!(game.phase, GamePhase::Playing);
    }

    #[test]
    fn from_room_builds_participants_from_the_snapshot() {
        let me = PlayerId::new(7).unwrap();
        let guest = PlayerId::new(9).unwrap();
        let mut room = Room::new(me, true);
        let team = room.next_team_id();
        room.join(guest, team).unwrap();
        room.set_ready(guest, true).unwrap();

        let mut names = HashMap::new();
        names.insert(me, "Alice".to_string());
        names.insert(guest, "Robin".to_string());

        let game = LocalGame::from_room(&room, me, &names);
        assert_eq!(game.me, Some(me));
        assert_eq!(game.phase, GamePhase::Lobby);

        let own = game
            .participants
            .iter()
            .find(|participant| participant.controller == Controller::Human)
            .unwrap();
        assert_eq!(own.ready, Some(false));
        assert_eq!(own.name.as_deref(), Some("Alice"));

        let remote = game
        .participants
        .iter()
        .find(|participant| matches!(participant.controller, Controller::Remote(id) if id == guest))
        .unwrap();
        assert_eq!(remote.name.as_deref(), Some("Robin"));
        assert_eq!(remote.ready, Some(true));

        assert_eq!(game.players.len(), 2);
        assert_eq!(game.players[0].id, me);
        assert_eq!(game.players[0].name.as_deref(), Some("Alice"));
        assert!(!game.players[0].ready);
        assert!(game.players[0].admin);
        assert_eq!(game.players[1].id, guest);
        assert_eq!(game.players[1].name.as_deref(), Some("Robin"));
        assert!(game.players[1].ready);
        assert!(!game.players[1].admin);
    }

    #[test]
    fn seat_controller_keys_identify_each_seat() {
        let me = PlayerId::new(7).unwrap();
        let guest = PlayerId::new(9).unwrap();
        let mut room = Room::new(me, true);
        room.join(guest, room.next_team_id()).unwrap();
        let bot = room.add_bot(me, room.next_team_id()).unwrap();

        let game = LocalGame::from_room(&room, me, &HashMap::new());

        assert_eq!(
            game.seat_controller_key(u32::from(bot.get())),
            format!("bot{}", bot.get())
        );
        for (character, data) in room.game().characters() {
            let key = game.seat_controller_key(u32::from(character.get()));
            match data.controller {
                MemberId::Player(player) if player == me => {
                    assert_eq!(key, "player7");
                }
                MemberId::Player(player) if player == guest => {
                    assert_eq!(key, "player9");
                }
                _ => {}
            }
        }
        assert_eq!(game.seat_controller_key(9999), String::new());
    }

    #[test]
    fn absorb_animates_remote_moves() {
        let me = PlayerId::new(7).unwrap();
        let mut room = Room::new(me, true);
        let team = room.next_team_id();
        room.add_bot(me, team).unwrap();
        // Готовность единственного игрока автостартует партию на сервере.
        room.set_ready(me, true).unwrap();
        assert!(room.game().is_started());

        let mut game = LocalGame::from_room(&room, me, &HashMap::new());
        assert_eq!(game.phase, GamePhase::Playing);
        assert!(game.is_human_turn());

        // Находим свою клетку и ходим через серверную комнату.
        let pos = game
            .game
            .board()
            .iter()
            .find(|(_, cell)| cell.owner == game.current_team())
            .map(|(pos, _)| pos)
            .expect("the automatic distribution gives every team one cell");
        let character = game.current_character().unwrap();
        room.increase(me, character, pos).unwrap();

        let next = LocalGame::from_room(&room, me, &HashMap::new());
        game.absorb(next);
        assert_ne!(game.board_view().change_at(pos), 0);
    }

    #[test]
    fn identical_snapshots_stay_equal() {
        let me = PlayerId::new(7).unwrap();
        let mut room = Room::new(me, true);
        room.join(PlayerId::new(9).unwrap(), room.next_team_id())
            .unwrap();
        let names = HashMap::new();
        let first = LocalGame::from_room(&room, me, &names);
        let second = LocalGame::from_room(&room, me, &names);
        assert_eq!(first, second);
    }

    #[test]
    fn remote_moves_replay_wave_by_wave() {
        let me = PlayerId::new(7).unwrap();
        let mut room = Room::new(me, true);
        let team = room.next_team_id();
        room.add_bot(me, team).unwrap();
        // Готовность единственного игрока автостартует партию на сервере.
        room.set_ready(me, true).unwrap();

        let mut game = LocalGame::from_room(&room, me, &HashMap::new());
        assert_eq!(game.phase, GamePhase::Playing);
        assert!(game.is_human_turn());

        // Ходим через серверную комнату (как чужой игрок) и реплеим его.
        let pos = game
            .game
            .board()
            .iter()
            .find(|(_, cell)| cell.owner == game.current_team())
            .map(|(pos, _)| pos)
            .expect("the automatic distribution gives every team one cell");
        let character = game.current_character().unwrap();
        let seq = room.seq();
        room.increase(me, character, pos).unwrap();

        let target = LocalGame::from_room(&room, me, &HashMap::new());
        assert!(game.start_replay(room.last_steps(), &target.game, room.seq()));
        // Ход уже виден на доске и доигрывается волнами.
        assert!(game.is_animating());
        assert_ne!(game.board_view().change_at(pos), 0);

        while game.animation_step().unwrap() {}
        assert!(!game.is_animating());
        assert_eq!(game.game, target.game);
        assert_eq!(game.sync_seq, room.seq());
        assert!(seq < room.seq());
    }

    #[test]
    fn start_replay_rejects_diverging_steps() {
        let me = PlayerId::new(7).unwrap();
        let mut room = Room::new(me, true);
        let team = room.next_team_id();
        room.add_bot(me, team).unwrap();
        // Готовность единственного игрока автостартует партию на сервере.
        room.set_ready(me, true).unwrap();

        let mut game = LocalGame::from_room(&room, me, &HashMap::new());
        let pos = current_human_cell(&game);
        let character = game.current_character().unwrap();
        room.increase(me, character, pos).unwrap();

        let steps = room.last_steps().to_vec();
        let target = room.game().clone();
        assert!(game.start_replay(&steps, &target, room.seq()));
        while game.animation_step().unwrap() {}

        // Те же шаги повторно к доигранному состоянию не применимы.
        assert!(!game.start_replay(&steps, &target, room.seq()));
    }

    #[test]
    fn predicted_own_move_is_trimmed_from_the_replay() {
        let me = PlayerId::new(7).unwrap();
        let mut room = Room::new(me, true);
        let team = room.next_team_id();
        room.add_bot(me, team).unwrap();
        // Готовность единственного игрока автостартует партию на сервере.
        room.set_ready(me, true).unwrap();

        let mut game = LocalGame::from_room(&room, me, &HashMap::new());
        assert!(game.is_human_turn());
        let pos = current_human_cell(&game);

        // Ходим локально (предсказание) и доигрываем его вручную без
        // локального ответа бота — как если бы ответ сервера обгонял
        // локальное предсказание хода бота.
        game.make_move(pos, MoveMode::Increase).unwrap();
        while game.game.explode_wave() {}
        let winner = game.game.finish_move().unwrap();
        game.settle(winner);
        game.exploding = false;

        // Сервер применяет тот же ход и доигрывает ответ бота.
        let predicted_member = match game.predicted {
            Some(ReplayStep::Move { member, .. }) => member,
            other => panic!("expected a predicted move, got {other:?}"),
        };
        room.increase(me, predicted_member, pos).unwrap();
        assert_eq!(room.last_steps().len(), 2);

        // Реплей отрезает наш предсказанный ход и анимирует только бота.
        assert!(game.start_replay(room.last_steps(), room.game(), room.seq()));
        assert!(game.is_animating());
        while game.animation_step().unwrap() {}
        assert!(!game.is_animating());
        assert_eq!(game.game, *room.game());
        assert_eq!(game.sync_seq, room.seq());
    }

    #[test]
    fn detach_turns_network_players_into_local() {
        let me = PlayerId::new(7).unwrap();
        let guest = PlayerId::new(9).unwrap();
        let mut room = Room::new(me, true);
        let team = room.next_team_id();
        room.join(guest, team).unwrap();
        room.set_ready(guest, true).unwrap();
        let names = HashMap::new();

        let mut game = LocalGame::from_room(&room, me, &names);
        assert!(game.players.len() == 2);

        game.detach();

        assert!(game.me.is_none());
        assert!(game.players.is_empty());
        assert!(game.replay.is_empty());
        assert_eq!(game.sync_seq, 0);
        for participant in &game.participants {
            assert!(matches!(
                participant.controller,
                Controller::Human | Controller::Bot(_)
            ));
            assert!(participant.name.is_none());
            assert!(participant.ready.is_none());
        }
    }

    #[test]
    fn lobby_setup_round_trips_the_lobby() {
        let mut game = LocalGame::new();
        game.add_bot().unwrap();
        let (w, h) = (NonZeroU8::new(5).unwrap(), NonZeroU8::new(7).unwrap());
        game.set_board_size(BoardCode::from_size(w, h).unwrap())
            .unwrap();

        let setup = game.lobby_setup();
        assert_eq!(setup.my_team, Some(1));
        assert_eq!(setup.width, 5);
        assert_eq!(setup.height, 7);
        assert_eq!(setup.seats.len(), 2);

        // Восстанавливаем комнату по снимку и сравниваем).
        let me = PlayerId::new(7).unwrap();
        let mut room = Room::new(me, true);
        room.apply_lobby_setup(me, &setup).unwrap();

        let restored = LocalGame::from_room(&room, me, &HashMap::new());
        assert_eq!(restored.board_size(), BoardCode::from_size(w, h).unwrap());
        let controllers: Vec<Controller> = restored
            .participants
            .iter()
            .map(|participant| participant.controller)
            .collect();
        assert_eq!(
            controllers,
            vec![Controller::Human, Controller::Bot(2), Controller::Bot(3)]
        );
        // Номер бота — его CharacterId; боты остаются в своих командах.
        assert_eq!(restored.participant_team(2), Some(tid(2)));
    }

    #[test]
    fn seat_bot_options_keep_the_current_bot_available() {
        let mut game = LocalGame::new();
        assert_eq!(game.seat_bot_options(), vec![1, 2]);

        game.add_bot().unwrap();
        assert_eq!(game.seat_bot_options(), vec![1, 2, 3]);
    }
}
