use std::collections::BTreeSet;

use forup_core::api::game::board::prelude::*;
use forup_core::api::network::Uuid;
use glam::U8Vec2;
use leptos::prelude::*;

/// Deterministic identity for the first human seat so server-rendered markup
/// matches the hydrated one.
const FIRST_OWNER: u32 = 1;
/// Hue of the first team; the rest are distributed evenly around the wheel.
const TEAM_HUE_OFFSET: u16 = 210;

pub type ParticipantId = u32;

/// Who controls a participant. Bots are only labelled by number for the UI;
/// the engine treats every bot as the same `MemberId::Bot`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Controller {
    Human,
    Bot(u8),
}

impl Controller {
    /// Stable key used by the controller selector.
    pub fn key(self) -> String {
        match self {
            Self::Human => "human".to_string(),
            Self::Bot(number) => format!("bot{number}"),
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

#[derive(Clone, Debug)]
pub struct Participant {
    pub id: ParticipantId,
    pub controller: Controller,
    pub characters: Vec<CharacterId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GamePhase {
    /// Waiting for the local player to press Ready.
    Lobby,
    Playing,
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveMode {
    Increase,
    Decrease,
}

impl MoveMode {
    pub fn action(self, pos: U8Vec2) -> GameAction {
        match self {
            Self::Increase => GameAction::Increase(pos),
            Self::Decrease => GameAction::Decrease(pos),
        }
    }
}

/// A fully local match. Every action is replayed on this local replica first;
/// only actions the local engine accepts are recorded in
/// [`Self::validated_actions`], which is the payload an online client forwards
/// to the server.
#[derive(Clone, Debug)]
pub struct LocalGame {
    pub game: Game,
    pub participants: Vec<Participant>,
    next_participant_id: ParticipantId,
    pub phase: GamePhase,
    pub winner: Option<TeamId>,
    pub validated_actions: Vec<GameAction>,
    /// Net mass change of every board slot during the current action burst.
    changes: Vec<i16>,
    /// Alternating value used to restart CSS animations on repeated changes.
    pub animation_tick: u64,
}

impl LocalGame {
    pub fn new() -> Self {
        let game = Game::standard();
        let changes = vec![0; game.board.cells().len()];
        let mut local = Self {
            game,
            participants: Vec::new(),
            next_participant_id: 1,
            phase: GamePhase::Lobby,
            winner: None,
            validated_actions: Vec::new(),
            changes,
            animation_tick: 0,
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

    fn add_participant_on_team(
        &mut self,
        controller: Controller,
        team: TeamId,
    ) -> Result<ParticipantId, GameError> {
        // Editing a finished match first returns to a clean lobby, so the
        // engine no longer considers the game started.
        self.enter_edit_mode();
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }

        let id = self.next_participant_id;
        self.next_participant_id = self.next_participant_id.saturating_add(1);
        let character = match controller {
            Controller::Human => self.game.add_local_member(Self::human_uuid()?, team)?,
            Controller::Bot(_) => self.game.add_bot(team)?,
        };

        self.participants.push(Participant {
            id,
            controller,
            characters: vec![character],
        });
        Ok(id)
    }

    fn human_uuid() -> Result<Uuid, GameError> {
        Uuid::new(FIRST_OWNER).ok_or(GameError::CharacterIdSpaceExhausted)
    }

    fn next_bot_number(&self) -> u8 {
        let max = self
            .participants
            .iter()
            .filter_map(|participant| match participant.controller {
                Controller::Bot(number) => Some(number),
                Controller::Human => None,
            })
            .max()
            .unwrap_or(0);
        max.saturating_add(1).max(1)
    }

    pub fn add_character(
        &mut self,
        participant_id: ParticipantId,
    ) -> Result<CharacterId, GameError> {
        self.enter_edit_mode();
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }

        let (controller, team) = {
            let participant = self
                .participants
                .iter()
                .find(|participant| participant.id == participant_id)
                .ok_or(GameError::CharacterNotFound)?;
            let team = participant
                .characters
                .iter()
                .find_map(|character| self.game.characters.get(character).map(|data| data.team_id))
                .unwrap_or_else(|| self.next_team_id());
            (participant.controller, team)
        };

        let character = match controller {
            Controller::Human => self.game.add_local_member(Self::human_uuid()?, team)?,
            Controller::Bot(_) => self.game.add_bot(team)?,
        };

        if let Some(participant) = self
            .participants
            .iter_mut()
            .find(|participant| participant.id == participant_id)
        {
            participant.characters.push(character);
        }
        Ok(character)
    }

    pub fn remove_participant(&mut self, participant_id: ParticipantId) -> Result<(), GameError> {
        self.enter_edit_mode();
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }

        let Some(index) = self
            .participants
            .iter()
            .position(|participant| participant.id == participant_id)
        else {
            return Err(GameError::CharacterNotFound);
        };
        let participant = self.participants.remove(index);
        for character in participant.characters {
            let _ = self.game.kick_member(character);
        }
        Ok(())
    }

    pub fn remove_character(
        &mut self,
        participant_id: ParticipantId,
        character: CharacterId,
    ) -> Result<(), GameError> {
        self.enter_edit_mode();
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }

        let belongs_to_participant = self.participants.iter().any(|participant| {
            participant.id == participant_id && participant.characters.contains(&character)
        });
        if !belongs_to_participant {
            return Err(GameError::CharacterNotFound);
        }

        self.game.kick_member(character)?;
        if let Some(participant) = self
            .participants
            .iter_mut()
            .find(|participant| participant.id == participant_id)
        {
            participant
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
        self.enter_edit_mode();
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        let Some(index) = self
            .participants
            .iter()
            .position(|participant| participant.id == participant_id)
        else {
            return Err(GameError::CharacterNotFound);
        };
        if self.participants[index].controller == controller {
            return Ok(());
        }

        let member = match controller {
            Controller::Human => MemberId::Player(Self::human_uuid()?),
            Controller::Bot(_) => MemberId::Bot,
        };

        let characters = self.participants[index].characters.clone();
        for character in characters {
            if let Some(data) = self.game.characters.get_mut(&character) {
                data.controller = member;
            }
        }

        self.participants[index].controller = controller;
        Ok(())
    }

    /// Every controller the lobby can assign, including one fresh bot number so
    /// a new bot can be created from the participant row itself.
    pub fn controller_options(&self) -> Vec<Controller> {
        let max = self
            .participants
            .iter()
            .filter_map(|participant| match participant.controller {
                Controller::Bot(number) => Some(number),
                Controller::Human => None,
            })
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(3);

        let mut options = Vec::with_capacity(usize::from(max) + 1);
        options.push(Controller::Human);
        options.extend((1..=max).map(Controller::Bot));
        options
    }

    /// Moves every character of a participant to the chosen team. The lobby UI
    /// works with participants, not individual characters, so a seat always
    /// belongs to exactly one team.
    pub fn set_participant_team(
        &mut self,
        participant_id: ParticipantId,
        team: TeamId,
    ) -> Result<(), GameError> {
        self.enter_edit_mode();
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        let Some(participant) = self
            .participants
            .iter()
            .find(|participant| participant.id == participant_id)
        else {
            return Err(GameError::CharacterNotFound);
        };
        let characters = participant.characters.clone();
        for character in characters {
            self.game.change_team(character, team)?;
        }
        Ok(())
    }

    // --- match settings ---------------------------------------------------

    pub fn board_size(&self) -> U8Vec2 {
        self.game.board.size()
    }

    /// Resizes the board. Only allowed outside a running match; the engine
    /// clamps the size to its supported range.
    pub fn set_board_size(&mut self, size: U8Vec2) -> Result<(), GameError> {
        self.enter_edit_mode();
        if self.game.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        self.game.board.set_size(size);
        self.reset_animation();
        Ok(())
    }

    pub fn automatic_distribution(&self) -> bool {
        self.game.automatic_distribution
    }

    pub fn set_automatic_distribution(&mut self, value: bool) {
        self.enter_edit_mode();
        if !self.game.is_started() {
            self.game.automatic_distribution = value;
        }
    }

    // --- game -------------------------------------------------------------

    pub fn current_character(&self) -> Option<CharacterId> {
        self.game
            .current_turn
            .and_then(|index| self.game.turn_order.get(index).copied())
    }

    pub fn current_is_bot(&self) -> bool {
        self.current_character().is_some_and(|character| {
            self.game
                .characters
                .get(&character)
                .is_some_and(|data| data.controller == MemberId::Bot)
        })
    }

    pub fn is_human_turn(&self) -> bool {
        self.phase == GamePhase::Playing
            && self.current_character().is_some()
            && !self.current_is_bot()
    }

    pub fn current_team(&self) -> Option<TeamId> {
        self.current_character()
            .and_then(|character| self.game.characters.get(&character))
            .map(|data| data.team_id)
    }

    pub fn character_participant(&self, character: CharacterId) -> Option<(usize, Controller)> {
        self.participants
            .iter()
            .enumerate()
            .find_map(|(index, participant)| {
                participant
                    .characters
                    .contains(&character)
                    .then_some((index + 1, participant.controller))
            })
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

    /// The next distinct participant in turn order, skipping eliminated seats.
    pub fn next_participant_id(&self) -> Option<ParticipantId> {
        let current = self.current_participant_id()?;
        let start = self.game.current_turn?;
        let len = self.game.turn_order.len();
        if len == 0 {
            return None;
        }
        (1..=len).find_map(|step| {
            let character = *self.game.turn_order.get((start + step) % len)?;
            if self.is_eliminated(character) {
                return None;
            }
            let participant = self.character_participant_id(character)?;
            (participant != current).then_some(participant)
        })
    }

    pub fn can_start(&self) -> bool {
        self.phase != GamePhase::Playing
            && self.game.characters.len() >= 2
            && self.teams().len() >= 2
            && self
                .participants
                .iter()
                .any(|participant| participant.controller == Controller::Human)
    }

    pub fn can_ready(&self) -> bool {
        matches!(self.phase, GamePhase::Lobby | GamePhase::Finished) && self.can_start()
    }

    pub fn can_skip(&self) -> bool {
        self.is_human_turn()
            && self
                .current_character()
                .is_some_and(|character| !self.game.is_starting_action(character))
    }

    pub fn can_decrease(&self) -> bool {
        self.can_skip()
            && self.current_team().is_some_and(|team| {
                self.game
                    .board
                    .iter()
                    .any(|(_, cell)| cell.owner == Some(team) && cell.value > 0)
            })
    }

    pub fn can_surrender(&self) -> bool {
        self.is_human_turn()
            && self
                .current_character()
                .is_some_and(|character| !self.is_eliminated(character))
    }

    /// A legal target for the current human seat's next action.
    pub fn can_move_to(&self, pos: U8Vec2) -> bool {
        if !self.is_human_turn() || !self.game.board.contains(pos) {
            return false;
        }
        let (Some(character), Some(team), Some(cell)) = (
            self.current_character(),
            self.current_team(),
            self.game.board.get(pos),
        ) else {
            return false;
        };
        if self.game.is_starting_action(character) {
            cell.owner.is_none()
        } else {
            cell.owner == Some(team)
        }
    }

    /// Ready up, or start a rematch with the same participants.
    pub fn ready(&mut self) -> Result<(), GameError> {
        if !self.can_ready() {
            return Ok(());
        }
        // A finished match may still be "started" inside the engine; reset it
        // before starting the rematch.
        self.enter_edit_mode();
        self.game.start(self.game.automatic_distribution)?;
        self.phase = GamePhase::Playing;
        self.winner = None;
        self.validated_actions.clear();
        self.reset_animation();
        Ok(())
    }

    pub fn make_move(&mut self, pos: U8Vec2, mode: MoveMode) -> Result<(), GameError> {
        let character = self.require_human_turn()?;
        self.submit(character, mode.action(pos))
    }

    pub fn skip(&mut self) -> Result<(), GameError> {
        let character = self.require_human_turn()?;
        self.submit(character, GameAction::Skip)
    }

    pub fn surrender(&mut self) -> Result<(), GameError> {
        let character = self.require_human_turn()?;
        self.submit(character, GameAction::Surrender)
    }

    /// Validates and applies a human action, then immediately plays every
    /// consecutive bot turn. There is no artificial delay: the resulting board
    /// changes are animated by the UI through [`Self::change_at`].
    ///
    /// Online flow: call this locally, and only if it returns `Ok` forward the
    /// recorded human action to the server, which re-validates it against the
    /// authoritative game.
    pub fn submit(&mut self, character: CharacterId, action: GameAction) -> Result<(), GameError> {
        if self.phase != GamePhase::Playing {
            return Err(GameError::GameNotStarted);
        }

        self.begin_animation_burst();
        self.apply_single(character, action)?;
        self.validated_actions.push(action);

        while self.phase == GamePhase::Playing && self.current_is_bot() {
            let bot = self
                .current_character()
                .expect("a bot turn always has a current character");
            let bot_action = self.game.bot_action(bot, &BotConfig::STANDARD);
            self.apply_single(bot, bot_action)?;
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Drains the locally validated human actions so a network layer can
    /// forward them to the server. Nothing is removed before validation.
    #[allow(dead_code)] // used by the future multiplayer transport
    pub fn take_validated_actions(&mut self) -> Vec<GameAction> {
        std::mem::take(&mut self.validated_actions)
    }

    // --- board / team view helpers ---------------------------------------

    pub fn teams(&self) -> Vec<TeamId> {
        self.game
            .characters
            .values()
            .map(|character| character.team_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Hues are spread evenly around the wheel, so every team is as far as
    /// possible from the others. Saturation and lightness are fixed in CSS.
    pub fn team_hue(&self, team: TeamId) -> u16 {
        let teams = self.teams();
        let count = teams.len().max(1) as u32;
        let index = teams
            .iter()
            .position(|candidate| *candidate == team)
            .unwrap_or(0) as u32;
        ((u32::from(TEAM_HUE_OFFSET) + index * 360 / count) % 360) as u16
    }

    pub fn team_options(&self) -> Vec<TeamId> {
        let max = self
            .game
            .characters
            .values()
            .map(|character| character.team_id.get())
            .max()
            .unwrap_or(0);
        let max = max.saturating_add(1).min(u8::MAX).max(2);
        (1..=max).filter_map(TeamId::new).collect()
    }

    #[allow(dead_code)]
    pub fn character_views(&self, participant_id: ParticipantId) -> Vec<CharacterView> {
        self.participants
            .iter()
            .find(|participant| participant.id == participant_id)
            .map(|participant| {
                participant
                    .characters
                    .iter()
                    .filter_map(|character| {
                        self.game
                            .characters
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
        self.teams()
            .into_iter()
            .map(|team| {
                let (cells, mass) = team_totals(&self.game, team);
                let participants = self
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
                                    .characters
                                    .get(character)
                                    .is_some_and(|data| data.team_id == team)
                            })
                            .collect();
                        if characters.is_empty() {
                            return None;
                        }

                        let mut wins = 0u16;
                        let mut losses = 0u16;
                        let mut moves = 0u16;
                        for character in &characters {
                            if let Some(stat) = self.game.stats.get(character) {
                                wins = wins.saturating_add(u16::from(stat.wins));
                                losses = losses.saturating_add(u16::from(stat.losses));
                                moves = moves.saturating_add(stat.total_moves());
                            }
                        }

                        Some(TeamParticipantView {
                            id: participant.id,
                            index: index + 1,
                            controller: participant.controller,
                            characters,
                            wins,
                            losses,
                            moves,
                        })
                    })
                    .collect();
                TeamView {
                    team,
                    hue: self.team_hue(team),
                    cells,
                    mass,
                    participants,
                }
            })
            .collect()
    }

    pub fn change_at(&self, pos: U8Vec2) -> i16 {
        let width = usize::from(self.game.board.size().x);
        let index = usize::from(pos.y) * width + usize::from(pos.x);
        self.changes.get(index).copied().unwrap_or(0)
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
            .stats
            .get(&character)
            .is_some_and(|stat| stat.losses > 0)
    }

    fn next_team_id(&self) -> TeamId {
        let used: BTreeSet<u8> = self
            .game
            .characters
            .values()
            .map(|character| character.team_id.get())
            .collect();
        (1..=u8::MAX)
            .filter_map(TeamId::new)
            .find(|team| !used.contains(&team.get()))
            .unwrap_or(TeamId::MIN)
    }

    /// Editing after a finished match returns to a clean lobby while keeping
    /// the participant list.
    fn enter_edit_mode(&mut self) {
        if self.phase != GamePhase::Finished {
            return;
        }
        self.game.stop();
        for stat in self.game.stats.values_mut() {
            *stat = MemberStat::default();
        }
        self.phase = GamePhase::Lobby;
        self.winner = None;
        self.validated_actions.clear();
        self.reset_animation();
    }

    fn begin_animation_burst(&mut self) {
        self.animation_tick = self.animation_tick.wrapping_add(1);
        let len = self.game.board.cells().len();
        self.changes.clear();
        self.changes.resize(len, 0);
    }

    fn reset_animation(&mut self) {
        self.changes.clear();
        self.changes.resize(self.game.board.cells().len(), 0);
        self.animation_tick = 0;
    }

    fn apply_single(
        &mut self,
        character: CharacterId,
        action: GameAction,
    ) -> Result<(), GameError> {
        let before: Vec<i16> = self
            .game
            .board
            .cells()
            .iter()
            .map(|cell| i16::from(cell.value))
            .collect();

        let mut next = self.game.clone();
        let winner = next.apply_action(character, action)?;
        self.game = next;

        for (index, cell) in self.game.board.cells().iter().enumerate() {
            let Some(previous) = before.get(index) else {
                continue;
            };
            let delta = i16::from(cell.value) - *previous;
            if delta == 0 {
                continue;
            }
            if let Some(slot) = self.changes.get_mut(index) {
                *slot = slot.saturating_add(delta);
            }
        }

        if let Some(team) = winner {
            self.finish(team);
        } else if !self.game.is_started() {
            let team = self.winner_from_stats();
            self.phase = GamePhase::Finished;
            self.winner = team;
        }
        Ok(())
    }

    fn finish(&mut self, team: TeamId) {
        self.phase = GamePhase::Finished;
        self.winner = Some(team);
    }

    fn winner_from_stats(&self) -> Option<TeamId> {
        self.game.characters.iter().find_map(|(&character, data)| {
            self.game
                .stats
                .get(&character)
                .is_some_and(|stat| stat.wins > 0)
                .then_some(data.team_id)
        })
    }
}

impl Default for LocalGame {
    fn default() -> Self {
        Self::new()
    }
}

pub fn team_totals(game: &Game, team: TeamId) -> (u16, i32) {
    game.board.iter().fold((0, 0), |(cells, mass), (_, cell)| {
        if cell.owner == Some(team) {
            (
                cells.saturating_add(1),
                mass.saturating_add(i32::from(cell.value)),
            )
        } else {
            (cells, mass)
        }
    })
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CharacterView {
    pub id: CharacterId,
    pub team: TeamId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeamParticipantView {
    pub id: ParticipantId,
    /// 1-based index of the participant in the lobby list.
    pub index: usize,
    pub controller: Controller,
    pub characters: Vec<CharacterId>,
    pub wins: u16,
    pub losses: u16,
    pub moves: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeamView {
    pub team: TeamId,
    pub hue: u16,
    pub cells: u16,
    pub mass: i32,
    pub participants: Vec<TeamParticipantView>,
}

/// Reactive handle shared by the game page, the team panels and the lobby.
#[derive(Clone, Copy)]
pub struct GameSession {
    pub state: RwSignal<LocalGame>,
    pub error: RwSignal<Option<GameError>>,
    pub mode: RwSignal<MoveMode>,
}

impl GameSession {
    pub fn provide() -> Self {
        let session = Self {
            state: RwSignal::new(LocalGame::new()),
            error: RwSignal::new(None),
            mode: RwSignal::new(MoveMode::Increase),
        };
        provide_context(session);
        session
    }

    pub fn expect() -> Self {
        expect_context::<Self>()
    }

    /// Runs a local update, surfacing engine errors in the UI.
    pub fn dispatch(&self, apply: impl FnOnce(&mut LocalGame) -> Result<(), GameError>) {
        if let Some(outcome) = self.state.try_update(apply) {
            self.error.set(outcome.err());
        }
    }

    pub fn reset(&self) {
        self.state.update(LocalGame::reset);
        self.error.set(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started() -> LocalGame {
        let mut game = LocalGame::new();
        game.ready().expect("a local match starts");
        game
    }

    fn current_human_cell(game: &LocalGame) -> U8Vec2 {
        let team = game.current_team().expect("a human team owns the turn");
        game.game
            .board
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
        assert!(game.validated_actions.is_empty());
        assert_eq!(game.animation_tick, 0);
    }

    #[test]
    fn human_move_is_validated_and_the_bot_answers_immediately() {
        let mut game = started();
        let pos = current_human_cell(&game);

        game.make_move(pos, MoveMode::Increase).unwrap();

        // Only the human action is forwarded; the bot is a local prediction.
        assert_eq!(game.validated_actions, vec![GameAction::Increase(pos)]);
        assert_eq!(game.animation_tick, 1);
        assert!(game.changes.iter().any(|delta| *delta != 0));
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
            .board
            .iter()
            .find(|(_, cell)| cell.owner == Some(bot_team))
            .map(|(pos, _)| pos)
            .expect("the bot starts with a cell");

        assert!(game.make_move(pos, MoveMode::Increase).is_err());

        assert_eq!(game.game, before);
        assert!(game.validated_actions.is_empty());
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
    fn characters_can_be_assigned_to_teams() {
        let mut game = LocalGame::new();
        let participant = game.participants[0].id;
        let character = game.character_views(participant)[0].id;
        let team = TeamId::new(3).unwrap();

        game.set_participant_team(participant, team).unwrap();

        assert_eq!(
            game.game
                .characters
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
        game.set_board_size(U8Vec2::new(5, 7)).unwrap();
        game.set_automatic_distribution(false);

        assert_eq!(game.board_size(), U8Vec2::new(5, 7));
        assert!(!game.automatic_distribution());

        game.ready().unwrap();

        assert!(!game.automatic_distribution());
        assert!(game.game.board.iter().all(|(_, cell)| cell.owner.is_none()));
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
                .characters
                .get(&character)
                .map(|data| data.controller),
            Some(MemberId::Bot)
        );

        game.set_participant_controller(human, Controller::Human)
            .unwrap();
        assert!(matches!(
            game.game
                .characters
                .get(&character)
                .map(|data| data.controller),
            Some(MemberId::Player(_))
        ));
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
    fn surrender_finishes_the_match() {
        let mut game = started();

        game.surrender().unwrap();

        assert_eq!(game.phase, GamePhase::Finished);
        assert!(game.winner.is_some());
    }
}
