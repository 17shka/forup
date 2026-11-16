use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::api::{auth::PlayerId, game::circle::prelude::*};

use super::code::BoardCode;
use super::field::Board;
use super::ids::{
    CharacterData, CharacterId, IdSet, MemberId, MemberStat, Score, TeamId, first_available_id,
};

/// Правило распределения массы при взрыве клетки.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExplosionMode {
    Lossy,
    #[default]
    Lossless,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameError {
    GameNotStarted,
    CharacterNotFound,
    CharacterEliminated,
    NotYourTurn,
    PositionOutOfBounds,
    FirstMoveRequiresEmptyCell,
    CellNotOwnedByTeam,
    CellCannotBeDecreased,
    NoMembers,
    NotEnoughMembers,
    NotEnoughTeams,
    GameAlreadyStarted,
    CharacterIdSpaceExhausted,
    BotIdSpaceExhausted,
    /// Стартовая клетка ближе к занятой клетке, чем разрешает [`Game::safe_radius`].
    StartCellTooClose,
}

/// Партия целиком: участники, доска и правила. Поля закрыты — доступ
/// только через методы, чтобы состояние нельзя было изменить в обход проверок.
#[derive(Clone, Debug, Default, Eq, Serialize, Deserialize)]
pub struct Game {
    characters: IndexMap<CharacterId, CharacterData>,
    current_turn: Option<usize>,
    pub(super) board: Board,
    pub(super) explosion_mode: ExplosionMode,
    /// Квадрат «безопасного радиуса»: минимальное расстояние от стартовой клетки
    /// до уже занятых клеток. Меняется только через [`Game::set_safe_radius`],
    /// которая не даёт превысить [`Game::max_safe_radius`].
    #[serde(default)]
    safe_radius: u8,
}

/// Порядок персонажей — часть состояния партии (последовательность
/// ходов). Производное `PartialEq` для `IndexMap` сравнивает пары
/// «ключ–значение» без учёта порядка, из-за чего снапшот с изменённой
/// очерёдностью выглядел бы равным прежнему состоянию клиента и не
/// применялся. Поэтому записи сверяются позиционно.
impl PartialEq for Game {
    fn eq(&self, other: &Self) -> bool {
        self.current_turn == other.current_turn
            && self.board == other.board
            && self.explosion_mode == other.explosion_mode
            && self.safe_radius == other.safe_radius
            && self.characters.len() == other.characters.len()
            && self
                .characters
                .iter()
                .zip(other.characters.iter())
                .all(|((left_id, left), (right_id, right))| left_id == right_id && left == right)
    }
}

impl Game {
    pub fn standard() -> Self {
        Self::new(Board::standard())
    }

    fn next_character_id(&self) -> Option<CharacterId> {
        first_available_id(self.characters.keys().copied())
    }

    pub fn new(board: Board) -> Self {
        Self {
            board,
            ..Default::default()
        }
    }

    pub fn is_started(&self) -> bool {
        self.current_turn.is_some()
    }

    /// Turn order: characters in the order they make moves.
    pub fn characters(&self) -> &IndexMap<CharacterId, CharacterData> {
        &self.characters
    }

    /// Index of the character making the current move, if the game is running.
    pub fn current_turn(&self) -> Option<usize> {
        self.current_turn
    }

    pub fn board(&self) -> &Board {
        &self.board
    }

    pub fn explosion_mode(&self) -> ExplosionMode {
        self.explosion_mode
    }

    /// Меняет режим взрывов. Только до старта партии.
    pub fn set_explosion_mode(&mut self, mode: ExplosionMode) -> Result<(), GameError> {
        if self.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        self.explosion_mode = mode;
        Ok(())
    }

    /// Меняет размер доски. Только до старта партии.
    pub fn set_board_size(&mut self, size: BoardCode) -> Result<(), GameError> {
        if self.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        self.board.set_size(size);
        Ok(())
    }

    /// Передаёт персонажа другому контроллеру (игрок или бот). Только до
    /// старта партии.
    pub fn set_controller(
        &mut self,
        member_id: CharacterId,
        controller: MemberId,
    ) -> Result<(), GameError> {
        if self.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        let character = self
            .characters
            .get_mut(&member_id)
            .ok_or(GameError::CharacterNotFound)?;
        character.controller = controller;
        Ok(())
    }

    /// Обнуляет статистику всех персонажей (возврат в лобби).
    pub fn reset_stats(&mut self) {
        for character in self.characters.values_mut() {
            character.stats = MemberStat::default();
        }
    }

    /// Количество разных команд среди участников.
    pub fn team_count(&self) -> u8 {
        let mut teams = IdSet::default();
        for member in self.characters.values() {
            teams.insert(member.team_id.get());
        }
        teams.len() as u8
    }

    /// Наибольший допустимый [`Game::safe_radius`] для текущего размера доски и
    /// количества команд: при нём последняя команда гарантированно найдёт свободную
    /// стартовую клетку. Команд считается не меньше двух, иначе ограничения нет.
    pub fn max_safe_radius(&self) -> u8 {
        max_radius_sq_for_players(self.board.size(), self.team_count().max(2))
    }

    /// Действующий квадрат безопасного радиуса для стартовых клеток.
    /// Если поле или команды изменились, значение урезается до
    /// [`Game::max_safe_radius`].
    pub fn safe_radius(&self) -> u8 {
        self.safe_radius.min(self.max_safe_radius())
    }

    /// Задаёт квадрат безопасного радиуса, урезая его до
    /// [`Game::max_safe_radius`]. Возвращает сохранённое значение.
    pub fn set_safe_radius(&mut self, radius_sq: u8) -> Result<u8, GameError> {
        if self.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        self.safe_radius = radius_sq.min(self.max_safe_radius());
        Ok(self.safe_radius)
    }

    /// Свободная клетка дальше `safe_radius` от всех занятых клеток.
    fn is_safe_start_cell(&self, pos: BoardCode, safe_radius: Score) -> bool {
        self.board[pos].owner.is_none()
            && self
                .board
                .occupied_positions()
                .all(|other| self.board.distance_sq(pos, other) > safe_radius)
    }

    /// Можно ли выбрать `pos` стартовой клеткой. Если из-за безопасного радиуса
    /// не осталось ни одной подходящей клетки, разрешается любая свободная.
    pub fn can_start_at(&self, pos: BoardCode) -> bool {
        if !self.board.contains(pos) || self.board[pos].owner.is_some() {
            return false;
        }
        let safe_radius = Score::from(self.safe_radius());
        self.is_safe_start_cell(pos, safe_radius)
            || !self
                .board
                .positions()
                .any(|other| self.is_safe_start_cell(other, safe_radius))
    }

    /// Все клетки, которые сейчас можно выбрать стартовыми.
    pub(super) fn start_cells(&self) -> Vec<BoardCode> {
        let safe_radius = Score::from(self.safe_radius());
        let spaced: Vec<_> = self
            .board
            .positions()
            .filter(|&pos| self.is_safe_start_cell(pos, safe_radius))
            .collect();
        if !spaced.is_empty() {
            return spaced;
        }
        self.board
            .iter()
            .filter(|(_, cell)| cell.owner.is_none())
            .map(|(pos, _)| pos)
            .collect()
    }

    /// Клетки, в которые сейчас можно сходить этим участником: стартовые
    /// (первый ход команды) или уже принадлежащие его команде. Пусто, если
    /// ход не его. Одна точка входа для движка, ботов и клиента: правила
    /// хода описаны в одном месте.
    pub fn legal_targets(&self, member_id: CharacterId) -> Vec<BoardCode> {
        let Ok(team) = self.check_turn(member_id) else {
            return Vec::new();
        };
        if self.is_starting_action(member_id) {
            self.start_cells()
        } else {
            self.board
                .occupied_positions()
                .filter(|&pos| self.board[pos].owner == Some(team))
                .collect()
        }
    }

    pub fn change_team(
        &mut self,
        target_member: CharacterId,
        new_team: TeamId,
    ) -> Result<(), GameError> {
        if self.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }

        match self.characters.get_mut(&target_member) {
            Some(character) => {
                character.team_id = new_team;
                Ok(())
            }
            None => Err(GameError::CharacterNotFound),
        }
    }

    /// Adds another seat controlled by the same local player/device.
    pub fn add_local_member(
        &mut self,
        controller: PlayerId,
        team_id: TeamId,
    ) -> Result<CharacterId, GameError> {
        if self.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        let member_id = self
            .next_character_id()
            .ok_or(GameError::CharacterIdSpaceExhausted)?;

        let member = CharacterData::new(MemberId::Player(controller), team_id);
        self.characters.insert(member_id, member);
        Ok(member_id)
    }

    pub fn join(&mut self, player: PlayerId, team_id: TeamId) -> Result<(), GameError> {
        if self.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        let controller = MemberId::Player(player);
        if self
            .characters
            .values()
            .any(|member| member.controller == controller)
        {
            return Ok(());
        }
        let character_id = self
            .next_character_id()
            .ok_or(GameError::CharacterIdSpaceExhausted)?;
        self.characters
            .insert(character_id, CharacterData::new(controller, team_id));
        Ok(())
    }

    pub fn leave(&mut self, member_id: CharacterId) {
        if self.is_started() {
            // Выход из партии — поражение независимо от того, чей сейчас ход:
            // покидающий игру не должен блокировать чужие ходы.
            self.eliminate(member_id);
            let _ = self.outcome();
            return;
        }

        self.characters.shift_remove(&member_id);
    }

    pub fn add_bot(&mut self, team_id: TeamId) -> Result<CharacterId, GameError> {
        if self.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        let member_id = self
            .next_character_id()
            .ok_or(GameError::CharacterIdSpaceExhausted)?;
        self.characters
            .insert(member_id, CharacterData::new(MemberId::Bot, team_id));
        Ok(member_id)
    }

    pub fn kick_member(&mut self, member_id: CharacterId) -> Result<(), GameError> {
        if self.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        if !self.characters.contains_key(&member_id) {
            return Err(GameError::CharacterNotFound);
        }
        self.characters.shift_remove(&member_id);

        if self.is_turn(member_id) {
            self.advance_turn();
        }
        Ok(())
    }

    pub fn start(&mut self, automatic_distribution: bool) -> Result<(), GameError> {
        if self.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        if self.characters.is_empty() {
            return Err(GameError::NoMembers);
        }
        if self.characters.len() < 2 {
            return Err(GameError::NotEnoughMembers);
        }
        let mut teams = IdSet::default();
        for member in self.characters.values() {
            teams.insert(member.team_id.get());
        }
        if teams.len() < 2 {
            return Err(GameError::NotEnoughTeams);
        }

        for (_, character) in self.characters.iter_mut() {
            character.stats = MemberStat::default();
        }
        self.board.clear_cells();

        self.safe_radius = self.safe_radius();
        self.current_turn = Some(0);

        if automatic_distribution {
            self.distribute_start_cells();
        }
        Ok(())
    }

    fn distribute_start_cells(&mut self) {
        let mut teams = IdSet::default();
        for m in self.characters.values() {
            teams.insert(m.team_id.get());
        }

        let mut placed: Vec<BoardCode> = Vec::with_capacity(teams.len());
        for team in teams.iter() {
            let mut best: Option<(Score, BoardCode)> = None;

            for pos in self.board.positions() {
                if placed.contains(&pos) {
                    continue;
                }
                let dist = placed
                    .iter()
                    .map(|&p| self.board.distance_sq(p, pos))
                    .min()
                    .unwrap_or(Score::MAX);

                if best.is_none_or(|(d, _)| dist > d) {
                    best = Some((dist, pos));
                }
            }

            if let Some((_, pos)) = best {
                placed.push(pos);
                let cell = &mut self.board[pos];
                cell.owner = Some(team);
                cell.value = 1;
            }
        }
    }

    pub fn stop(&mut self) {
        self.current_turn = None;
        self.board.clear_cells();
    }

    /// Принудительно завершает партию: победителем становится команда-
    /// лидер по массе, а при пустой доске — команда первого участника
    /// в порядке ходов. Возвращает команду-победителя.
    pub fn end_game(&mut self) -> Result<TeamId, GameError> {
        if !self.is_started() {
            return Err(GameError::GameNotStarted);
        }
        let winner = self
            .board
            .mass_leader()
            .or_else(|| self.characters.values().next().map(|c| c.team_id))
            .ok_or(GameError::NoMembers)?;

        let losers: Vec<CharacterId> = self
            .characters
            .iter()
            .filter(|(id, m)| m.team_id != winner && !self.is_eliminated(**id))
            .map(|(id, _)| *id)
            .collect();

        for id in losers {
            self.eliminate(id);
        }
        self.current_turn = None;
        self.award_win(winner);
        Ok(winner)
    }

    /// Меняет местами позиции двух участников в последовательности ходов.
    /// Доступно только до начала партии.
    pub fn swap_turn_order(&mut self, a: CharacterId, b: CharacterId) -> Result<(), GameError> {
        if self.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        if !self.characters.contains_key(&a) || !self.characters.contains_key(&b) {
            return Err(GameError::CharacterNotFound);
        }
        if a == b {
            return Ok(());
        }
        let ia = self
            .characters
            .get_index_of(&a)
            .expect("contains_key проверен выше");
        let ib = self
            .characters
            .get_index_of(&b)
            .expect("contains_key проверен выше");
        self.characters.swap_indices(ia, ib);
        Ok(())
    }

    fn is_turn(&self, member_id: CharacterId) -> bool {
        self.current_turn
            .and_then(|idx| self.characters.get_index(idx))
            .map(|(id, _)| *id)
            == Some(member_id)
    }

    fn is_alive(&self, member_id: CharacterId) -> bool {
        self.characters.contains_key(&member_id) && !self.is_eliminated(member_id)
    }

    fn is_eliminated(&self, member_id: CharacterId) -> bool {
        self.characters
            .get(&member_id)
            .is_some_and(|c| c.stats.losses > 0)
    }

    fn advance_turn(&mut self) {
        let Some(idx) = self.current_turn else {
            return;
        };
        let len = self.characters.len();
        self.current_turn = (1..=len)
            .map(|step| (idx + step) % len)
            .find(|&next| self.is_alive(*self.characters.get_index(next).unwrap().0));
    }

    fn active_team(&self, member_id: CharacterId) -> Result<TeamId, GameError> {
        if !self.is_started() {
            return Err(GameError::GameNotStarted);
        }
        let member = self
            .characters
            .get(&member_id)
            .ok_or(GameError::CharacterNotFound)?;

        if self.is_eliminated(member_id) {
            return Err(GameError::CharacterEliminated);
        }
        Ok(member.team_id)
    }

    pub fn check_turn(&self, member_id: CharacterId) -> Result<TeamId, GameError> {
        let team_id = self.active_team(member_id)?;
        if !self.is_turn(member_id) {
            return Err(GameError::NotYourTurn);
        }
        Ok(team_id)
    }

    /// Пропуск хода. Первый ход пропустить нельзя — нужно занять клетку.
    /// Возвращает победителя, если партия этим ходом завершилась.
    pub fn skip(&mut self, member_id: CharacterId) -> Result<Option<TeamId>, GameError> {
        if !self.is_started() {
            return Err(GameError::GameNotStarted);
        }
        if self.is_starting_action(member_id) {
            return Err(GameError::FirstMoveRequiresEmptyCell);
        }
        self.check_turn(member_id)?;
        let character = self.characters.get_mut(&member_id).unwrap();
        character.stats.skipped_moves = character.stats.skipped_moves.saturating_add(1);
        self.advance_turn();
        self.outcome()
    }

    /// Сдача: участник получает поражение, партия продолжается, пока
    /// остаются другие команды. Сдаться можно в любой момент, даже не
    /// своим ходом: очередь хода при этом не меняется.
    pub fn surrender(&mut self, member_id: CharacterId) -> Result<Option<TeamId>, GameError> {
        if !self.is_started() {
            return Err(GameError::GameNotStarted);
        }
        self.active_team(member_id)?;
        self.eliminate(member_id);
        self.outcome()
    }

    pub fn is_starting_action(&self, member_id: CharacterId) -> bool {
        self.characters.get(&member_id).is_some_and(|member| {
            member.stats.total_moves() == 0 && !self.board.owns_cells(member.team_id)
        })
    }

    /// Ход-увеличение клетки: применяется изменение и доигрывается вся
    /// цепочка взрывов. Возвращает победителя, если партия завершилась.
    pub fn increase(
        &mut self,
        member_id: CharacterId,
        pos: BoardCode,
    ) -> Result<Option<TeamId>, GameError> {
        self.play_move(member_id, pos, true)
    }

    /// Ход-уменьшение клетки: применяется изменение и доигрывается вся
    /// цепочка взрывов.
    pub fn decrease(
        &mut self,
        member_id: CharacterId,
        pos: BoardCode,
    ) -> Result<Option<TeamId>, GameError> {
        self.play_move(member_id, pos, false)
    }

    fn play_move(
        &mut self,
        member_id: CharacterId,
        pos: BoardCode,
        increase: bool,
    ) -> Result<Option<TeamId>, GameError> {
        if !self.is_started() {
            return Err(GameError::GameNotStarted);
        }
        self.start_move(member_id, pos, increase)?;
        self.explode_all();
        self.finish_move()
    }

    /// Начало хода: применяет изменение клетки без доигрывания взрывов.
    /// Все проверки проходят до каких-либо изменений состояния. Дальше
    /// волны взрывов доигрываются по одной через [`Self::explode_wave`],
    /// после последней ход завершается [`Self::finish_move`].
    pub fn start_move(
        &mut self,
        member_id: CharacterId,
        pos: BoardCode,
        increase: bool,
    ) -> Result<(), GameError> {
        let team = self.check_turn(member_id)?;
        if !self.board.contains(pos) {
            return Err(GameError::PositionOutOfBounds);
        }

        let owner = self.board[pos].owner;
        // Стартовый ход занимает пустую клетку; Decrease при старте только занимает
        // её, не меняя значения, Increase добавляет первый заряд.
        let starting = !self.board.owns_cells(team);
        if !starting {
            if owner != Some(team) {
                return Err(GameError::CellNotOwnedByTeam);
            }
        } else if owner.is_some() {
            return Err(GameError::FirstMoveRequiresEmptyCell);
        } else if !self.can_start_at(pos) {
            return Err(GameError::StartCellTooClose);
        }

        // Стартовое уменьшение значение не трогает, поэтому проверка минимума
        // к нему не применяется.
        if !increase && !starting && self.board[pos].value <= self.board[pos].ability.min_value() {
            return Err(GameError::CellCannotBeDecreased);
        }

        {
            let character = self.characters.get_mut(&member_id).unwrap();
            let counter = if increase {
                &mut character.stats.increase_moves
            } else {
                &mut character.stats.decrease_moves
            };
            *counter = counter.saturating_add(1);
        }

        let delta = if starting && !increase {
            0
        } else if increase {
            1
        } else {
            -1
        };
        self.board.apply_delta(pos, team, delta);
        Ok(())
    }

    /// Доигрывает одну волну взрывов. Возвращает `true`, если волна
    /// применилась и цепочку стоит продолжать; `false` — взрывов больше
    /// нет либо исход партии решён, и ход пора завершать через
    /// [`Self::finish_move`]. При раннем выходе отметки `activated`
    /// сохраняются, поэтому прерванная цепочка доигрывается позже.
    pub fn explode_wave(&mut self) -> bool {
        if !self.board.explode_activated(self.explosion_mode) {
            return false;
        }
        Self::sole_surviving_team(&self.board, &self.characters).is_none()
            && !self.board.is_saturated()
    }

    /// Доигрывает все волны взрывов без пошаговой анимации.
    fn explode_all(&mut self) {
        while self.explode_wave() {}
    }

    /// Завершает ход после изменения и взрывов: возвращает победителя,
    /// если партия решилась, иначе передаёт ход дальше. Насыщенная доска
    /// ход не передаёт — исход решается по массе.
    pub fn finish_move(&mut self) -> Result<Option<TeamId>, GameError> {
        if let Some(winner) = Self::sole_surviving_team(&self.board, &self.characters) {
            self.eliminate_teams_without_cells();
            self.current_turn = None;
            self.award_win(winner);
            return Ok(Some(winner));
        }
        if self.board.is_saturated() {
            return self.outcome();
        }
        self.advance_turn();
        self.outcome()
    }

    /// Общий итог хода: одна из проверок в его процессе могла завершить
    /// партию.
    fn outcome(&mut self) -> Result<Option<TeamId>, GameError> {
        if !self.is_started() {
            return Ok(None);
        }
        self.check_game_status()
    }

    pub fn check_game_status(&mut self) -> Result<Option<TeamId>, GameError> {
        if !self.is_started() {
            return Err(GameError::GameNotStarted);
        }

        if let Some(winner) = Self::sole_surviving_team(&self.board, &self.characters) {
            self.eliminate_teams_without_cells();
            self.current_turn = None;
            self.award_win(winner);
            return Ok(Some(winner));
        }

        if self.board.is_saturated() {
            return Ok(self.finish_by_mass());
        }

        self.eliminate_teams_without_cells();
        Ok(self.finish_by_last_team())
    }

    fn sole_surviving_team(
        board: &Board,
        characters: &IndexMap<CharacterId, CharacterData>,
    ) -> Option<TeamId> {
        // Команды, реально владеющие клетками, собираются один раз: раньше
        // `owns_cells` звал полный обход доски для каждого участника и каждой
        // волны взрывов.
        let mut owning = IdSet::default();
        for pos in board.occupied_positions() {
            if let Some(owner) = board[pos].owner {
                owning.insert(owner.get());
            }
        }

        let mut all = IdSet::default();
        let mut alive = IdSet::default();

        for member in characters.values() {
            all.insert(member.team_id.get());
            let has_moved = member.stats.total_moves() > 0;
            let is_eliminated = member.stats.losses > 0;
            if !is_eliminated && (!has_moved || owning.contains(member.team_id.get())) {
                alive.insert(member.team_id.get());
            }
        }

        if all.len() < 2 || alive.len() != 1 {
            return None;
        }
        alive.iter().next()
    }

    fn finish_by_mass(&mut self) -> Option<TeamId> {
        self.current_turn = None;
        let winner = self.board.mass_leader()?;

        self.award_win(winner);

        let losers: Vec<CharacterId> = self
            .characters
            .iter()
            .filter(|(id, m)| m.team_id != winner && !self.is_eliminated(**id))
            .map(|(id, _)| *id)
            .collect();

        for id in losers {
            self.eliminate(id);
        }
        Some(winner)
    }

    fn eliminate_teams_without_cells(&mut self) {
        let mut alive = IdSet::default();
        for (_, cell) in self.board.iter() {
            if let Some(owner) = cell.owner {
                alive.insert(owner.get());
            }
        }

        let dead: Vec<CharacterId> = self
            .characters
            .iter()
            .filter(|(id, m)| {
                !self.is_eliminated(**id)
                    && m.stats.total_moves() > 0
                    && !alive.contains(m.team_id.get())
            })
            .map(|(id, _)| *id)
            .collect();

        for id in dead {
            self.eliminate(id);
        }
    }

    fn finish_by_last_team(&mut self) -> Option<TeamId> {
        let mut all = IdSet::default();
        let mut alive = IdSet::default();

        for (&member_id, member) in &self.characters {
            all.insert(member.team_id.get());
            if !self.is_eliminated(member_id) {
                alive.insert(member.team_id.get());
            }
        }

        if all.len() < 2 || alive.len() != 1 {
            return None;
        }

        let winner = alive.iter().next()?;
        self.current_turn = None;
        self.award_win(winner);
        Some(winner)
    }

    fn award_win(&mut self, team: TeamId) {
        for (_, member) in self.characters.iter_mut() {
            if member.team_id == team {
                member.stats.wins = member.stats.wins.saturating_add(1);
            }
        }
    }

    fn eliminate(&mut self, member_id: CharacterId) {
        if let Some(character) = self.characters.get_mut(&member_id)
            && character.stats.losses == 0
        {
            character.stats.losses = 1;
        }
        if self.is_turn(member_id) {
            self.advance_turn();
        }
    }
}

#[cfg(test)]
mod game_rule_tests {
    use super::*;
    use std::num::NonZeroU8;

    use super::super::CellData;

    fn cid(id: u8) -> CharacterId {
        CharacterId::new(id).unwrap()
    }

    fn tid(id: u8) -> TeamId {
        TeamId::new(id).unwrap()
    }

    fn uid(id: u32) -> PlayerId {
        PlayerId::new(id).unwrap()
    }

    fn size(w: u8, h: u8) -> BoardCode {
        let (w, h) = (NonZeroU8::new(w).unwrap(), NonZeroU8::new(h).unwrap());
        BoardCode::from_size(w, h).unwrap()
    }

    fn pos(slot: u8) -> BoardCode {
        BoardCode::from_slot(slot)
    }

    #[test]
    fn game_requires_two_members_on_different_teams() {
        let mut game = Game::standard();
        game.add_local_member(uid(1), tid(1)).unwrap();
        assert_eq!(game.start(false), Err(GameError::NotEnoughMembers));

        game.add_local_member(uid(2), tid(1)).unwrap();
        assert_eq!(game.start(false), Err(GameError::NotEnoughTeams));
    }

    #[test]
    fn safe_radius_is_clamped_to_the_table_maximum() {
        let mut game = Game::new(Board::new(size(10, 10)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.add_local_member(uid(3), tid(3)).unwrap();
        game.add_local_member(uid(4), tid(4)).unwrap();

        assert_eq!(game.max_safe_radius(), 9);
        assert_eq!(game.set_safe_radius(u8::MAX), Ok(9));
        assert_eq!(game.safe_radius(), 9);

        // Меньшее поле снижает предел, и действующее значение следует за ним.
        game.board.set_size(size(3, 3));
        assert!(game.safe_radius() <= game.max_safe_radius());

        game.start(false).unwrap();
        assert_eq!(game.set_safe_radius(0), Err(GameError::GameAlreadyStarted));
    }

    #[test]
    fn legal_targets_follow_the_start_then_owned_cell_rules() {
        let mut game = Game::new(Board::new(size(8, 8)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.start(false).unwrap();

        // До первого хода доступна любая свободная клетка-старт.
        let targets = game.legal_targets(cid(1));
        assert!(!targets.is_empty());
        assert!(targets.iter().all(|&p| game.board[p].owner.is_none()));

        game.increase(cid(1), pos(0)).unwrap();
        // Ход перешёл к команде два: у первой клеток уже нет, у второй — старт.
        assert!(game.legal_targets(cid(1)).is_empty());
        assert!(!game.legal_targets(cid(2)).is_empty());

        game.increase(cid(2), pos(5)).unwrap();
        game.increase(cid(1), pos(0)).unwrap();
        // Своя клетка остаётся единственной легальной целью.
        assert_eq!(game.legal_targets(cid(2)), vec![pos(5)]);
    }

    #[test]
    fn start_cells_keep_the_safe_radius() {
        let mut game = Game::new(Board::new(size(8, 8)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        let safe_radius = game.set_safe_radius(8).unwrap();
        assert_eq!(safe_radius, 8);
        game.start(false).unwrap();

        game.increase(cid(1), pos(0)).unwrap();
        let near = game.board.position(2, 2).unwrap();
        let far = game.board.position(3, 3).unwrap();
        assert!(!game.can_start_at(near));
        assert_eq!(
            game.increase(cid(2), near),
            Err(GameError::StartCellTooClose)
        );
        assert!(game.can_start_at(far));
        game.increase(cid(2), far).unwrap();
    }

    #[test]
    fn starting_decrease_claims_the_cell_without_a_value() {
        let mut game = Game::new(Board::new(size(6, 6)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.start(false).unwrap();

        // Уменьшение числится как ход, но клетка остаётся нулевой.
        game.decrease(cid(1), pos(0)).unwrap();
        let cell = game.board[pos(0)];
        assert_eq!(cell.owner, Some(tid(1)));
        assert_eq!(cell.value, 0);
        assert_eq!(game.characters[&cid(1)].stats.decrease_moves, 1);
        assert_eq!(game.current_turn, Some(1));

        // Команда уже владеет клеткой нулевого значения — обычное уменьшение
        // требует запас до минимального значения и отклоняется.
        game.increase(cid(2), pos(35)).unwrap();
        assert_eq!(
            game.decrease(cid(1), pos(0)),
            Err(GameError::CellCannotBeDecreased)
        );
    }

    #[test]
    fn every_team_finds_a_start_cell_at_the_maximum_safe_radius() {
        for teams in 2..=crate::api::game::circle::MAX_CACHED_PLAYERS {
            for ordinal in 0..BoardCode::SIZE_COUNT {
                let board_size = BoardCode::from_ordinal(ordinal).unwrap();
                if board_size.cell_count() < usize::from(teams) {
                    continue;
                }
                let mut game = Game::new(Board::new(board_size));
                for team in 1..=teams {
                    game.add_bot(tid(team)).unwrap();
                }
                game.set_safe_radius(u8::MAX).unwrap();
                game.start(false).unwrap();
                let safe_radius = Score::from(game.safe_radius());

                for team in 1..=teams {
                    let pos = game
                        .board
                        .positions()
                        .find(|&pos| game.is_safe_start_cell(pos, safe_radius))
                        .unwrap_or_else(|| {
                            panic!("{board_size:?} {teams} teams: no cell for team {team}")
                        });
                    let cell = &mut game.board[pos];
                    cell.owner = Some(tid(team));
                    cell.value = 1;
                }
            }
        }
    }

    #[test]
    fn elimination_status_is_kept_in_member_stats() {
        let mut game = Game::new(Board::new(size(3, 3)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.add_local_member(uid(3), tid(3)).unwrap();
        game.start(false).unwrap();

        game.surrender(cid(1)).unwrap();

        assert_eq!(game.characters[&cid(1)].stats.losses, 1);
        assert_eq!(game.current_turn, Some(1));
        assert_eq!(game.check_turn(cid(1)), Err(GameError::CharacterEliminated));
    }

    #[test]
    fn surrender_is_allowed_even_out_of_turn() {
        let mut game = Game::new(Board::new(size(3, 3)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.add_local_member(uid(3), tid(3)).unwrap();
        game.start(false).unwrap();

        // Первый ход за cid(1): сдача чужого места больше не требует очереди.
        game.surrender(cid(2)).unwrap();
        assert_eq!(game.characters[&cid(2)].stats.losses, 1);
        // Очередь хода остаётся текущему игроку.
        assert_eq!(game.current_turn, Some(0));
        // Своим ходом сдаться по-прежнему можно.
        game.surrender(cid(1)).unwrap();
        assert_eq!(game.characters[&cid(1)].stats.losses, 1);
    }

    #[test]
    fn end_game_requires_a_started_game() {
        let mut game = Game::new(Board::new(size(3, 3)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();

        assert_eq!(game.end_game(), Err(GameError::GameNotStarted));
    }

    #[test]
    fn end_game_crowns_the_mass_leader() {
        let mut game = Game::new(Board::new(size(3, 3)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.start(false).unwrap();
        game.board[pos(0)] = CellData {
            value: 3,
            owner: Some(tid(1)),
            ..CellData::default()
        };
        game.board[pos(1)] = CellData {
            value: 1,
            owner: Some(tid(2)),
            ..CellData::default()
        };

        let winner = game.end_game().unwrap();

        assert_eq!(winner, tid(1));
        assert!(!game.is_started());
        assert_eq!(game.characters[&cid(1)].stats.wins, 1);
        assert_eq!(game.characters[&cid(2)].stats.losses, 1);
    }

    #[test]
    fn end_game_on_empty_board_crowns_the_first_seat() {
        let mut game = Game::new(Board::new(size(3, 3)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.start(false).unwrap();

        let winner = game.end_game().unwrap();

        assert_eq!(winner, tid(1));
        assert!(!game.is_started());
        assert_eq!(game.characters[&cid(1)].stats.wins, 1);
        assert_eq!(game.characters[&cid(2)].stats.losses, 1);
    }

    #[test]
    fn swapped_turn_order_survives_start() {
        let mut game = Game::new(Board::new(size(3, 3)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.add_local_member(uid(3), tid(3)).unwrap();

        game.swap_turn_order(cid(2), cid(3)).unwrap();
        let order: Vec<CharacterId> = game.characters().keys().copied().collect();
        assert_eq!(order, vec![cid(1), cid(3), cid(2)]);

        game.start(false).unwrap();
        // Порядок не пересортировывается при старте.
        let order: Vec<CharacterId> = game.characters().keys().copied().collect();
        assert_eq!(order, vec![cid(1), cid(3), cid(2)]);
        assert_eq!(game.current_turn, Some(0));

        game.increase(cid(1), pos(0)).unwrap();
        // Вторым за столом теперь ходит cid(3).
        assert_eq!(game.current_turn, Some(1));
        assert!(game.check_turn(cid(3)).is_ok());

        // После старта порядок менять нельзя.
        assert_eq!(
            game.swap_turn_order(cid(1), cid(2)),
            Err(GameError::GameAlreadyStarted)
        );
    }

    #[test]
    fn game_equality_accounts_for_turn_order() {
        let mut game = Game::new(Board::new(size(3, 3)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();

        let mut swapped = game.clone();
        swapped.swap_turn_order(cid(1), cid(2)).unwrap();
        // Один и тот же состав в другом порядке — разные партии: иначе
        // клиент принял бы снапшот со свапом за подтверждение старого.
        assert_ne!(game, swapped);

        swapped.swap_turn_order(cid(1), cid(2)).unwrap();
        assert_eq!(game, swapped);
    }

    #[test]
    fn leaving_mid_game_counts_as_a_loss_out_of_turn() {
        let mut game = Game::new(Board::new(size(3, 3)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.add_local_member(uid(3), tid(3)).unwrap();
        game.start(false).unwrap();

        // Выходит не тот участник, чей ход: поражение всё равно фиксируется.
        game.leave(cid(2));

        assert_eq!(game.characters[&cid(2)].stats.losses, 1);
        assert_eq!(game.current_turn, Some(0));
        assert!(game.is_started());
    }

    #[test]
    fn explosions_stop_as_soon_as_one_team_is_left() {
        let mut game = Game::new(Board::new(size(3, 3)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.start(false).unwrap();
        game.characters
            .get_mut(&cid(1))
            .unwrap()
            .stats
            .increase_moves = 1;
        game.characters
            .get_mut(&cid(2))
            .unwrap()
            .stats
            .increase_moves = 1;
        game.board[pos(0)] = CellData {
            value: 3,
            owner: Some(tid(1)),
            ..CellData::default()
        };
        game.board[pos(1)] = CellData {
            value: 1,
            owner: Some(tid(2)),
            ..CellData::default()
        };
        game.board[pos(3)] = CellData {
            value: 3,
            owner: Some(tid(1)),
            ..CellData::default()
        };

        let winner = game.increase(cid(1), pos(0)).unwrap();

        assert_eq!(winner, Some(tid(1)));
        assert_eq!(game.board[pos(3)].value, 4);
        assert!(!game.is_started());
    }

    fn board_mass(board: &Board) -> i32 {
        board.cells().iter().map(|cell| i32::from(cell.value)).sum()
    }

    #[test]
    fn lossless_border_explosion_keeps_the_missing_shares() {
        let mut board = Board::new(size(3, 3));
        board[pos(0)] = CellData {
            value: 3,
            owner: Some(tid(1)),
            ..CellData::default()
        };
        // The corner cell becomes 4, explodes and has only two of its four
        // cardinal targets on the board.
        board.apply_move(pos(0), tid(1), 1, ExplosionMode::Lossless);

        assert_eq!(board[pos(0)].value, 2);
        assert_eq!(board[pos(1)].value, 1);
        assert_eq!(board[pos(3)].value, 1);
        assert_eq!(board_mass(&board), 4);
    }

    #[test]
    fn lossy_border_explosion_dissipates_the_missing_shares() {
        let mut board = Board::new(size(3, 3));
        board[pos(0)] = CellData {
            value: 3,
            owner: Some(tid(1)),
            ..CellData::default()
        };
        board.apply_move(pos(0), tid(1), 1, ExplosionMode::Lossy);

        assert_eq!(board[pos(0)].value, 0);
        assert_eq!(board[pos(1)].value, 1);
        assert_eq!(board[pos(3)].value, 1);
        assert_eq!(board_mass(&board), 2);
    }

    #[test]
    fn decreasing_a_zero_cell_does_not_consume_the_turn() {
        let mut game = Game::standard();
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.start(true).unwrap();

        let pos = game
            .board
            .iter()
            .find(|(_, cell)| cell.owner == Some(tid(1)))
            .map(|(pos, _)| pos)
            .expect("team 1 starts with a cell");

        // The first decrease takes the cell from 1 to 0 and advances the turn.
        game.decrease(cid(1), pos).unwrap();
        assert_eq!(game.board[pos].value, 0);

        // Put the same member back on turn and try to decrease the empty cell
        // again: it must be rejected without advancing the turn.
        game.current_turn = Some(0);
        let before = game.current_turn;

        assert_eq!(
            game.decrease(cid(1), pos),
            Err(GameError::CellCannotBeDecreased)
        );
        assert_eq!(game.current_turn, before);
        assert_eq!(game.board[pos].value, 0);
    }
}
