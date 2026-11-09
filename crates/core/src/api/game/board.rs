use std::cmp::Reverse;
use std::collections::HashMap;
use std::num::NonZeroU8;
use std::ops::{Index, IndexMut};

use serde::{Deserialize, Serialize};

use crate::api::circle::prelude::*;
use crate::api::network::Uuid; // NonZeroU32

pub mod prelude {
    pub use super::{
        Board, BoardSize, BotConfig, CellAbility, CellData, CharacterData, CharacterId,
        ExplosionMode, Game, GameAction, GameError, GameSnapshot, MemberId, MemberStat, Score,
        TeamId, Timeline,
    };
}

/// Ненулевые идентификаторы: `Option<CharacterId>` и `Option<TeamId>` занимают 1 байт.
pub type CharacterId = NonZeroU8;
pub type TeamId = NonZeroU8;
pub type Score = i32;

/// Компактный идентификатор доски размером в один байт.
///
/// Одно и то же значение типа используется в двух ролях:
/// * **размер доски** — строится через [`BoardSize::new`]; ширина и высота
///   нормализуются так, что `w <= h`, поэтому `4 × 8` и `8 × 4` — один размер;
/// * **клетка** — строится через [`BoardSize::from_slot`], где первая клетка
///   кодируется как `BoardSize(0)`.
///
/// В одном байте помещается 256 слотов клеток, поэтому максимальная доска —
/// `16 × 16`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardSize(u8);

impl BoardSize {
    pub const MIN_BOARD_SIZE: (u8, u8) = (3, 3);
    /// Один байт даёт ровно 256 слотов клеток, поэтому максимальная доска —
    /// `16 × 16`. Кодов размеров (канонических пар `3..=16`) нужно всего 105,
    /// так что ограничивает именно количество клеток, а не кодировка размера.
    pub const MAX_BOARD_SIZE: (u8, u8) = (16, 16);
    pub const STANDARD_BOARD_SIZE: (u8, u8) = (8, 8);

    /// Минимальная доска `3 × 3`.
    pub const MIN: Self = match Self::encode(3, 3) {
        Some(id) => Self::from_id(id),
        None => panic!("the minimum board size must be valid"),
    };
    /// Стандартная доска `8 × 8`.
    pub const STANDARD: Self = match Self::encode(8, 8) {
        Some(id) => Self::from_id(id),
        None => panic!("the standard board size must be valid"),
    };
    /// Количество допустимых канонических размеров. Идентификаторы размеров
    /// занимают диапазон `1..=SIZE_COUNT`.
    pub const SIZE_COUNT: u8 = match Self::encode(Self::MAX_BOARD_SIZE.0, Self::MAX_BOARD_SIZE.1) {
        Some(id) => id,
        None => panic!("the maximum board size must be valid"),
    };

    /// Размер по его порядковому номеру в перечислении канонических размеров
    /// (`0..SIZE_COUNT`). Порядок перечисления — по возрастанию высоты, затем ширины:
    /// `3 × 3`, `3 × 4`, `4 × 4`, `3 × 5`, ...
    pub const fn from_ordinal(ordinal: u8) -> Option<Self> {
        if ordinal < Self::SIZE_COUNT {
            Some(Self::from_id(ordinal + 1))
        } else {
            None
        }
    }

    /// Порядковый номер размера в перечислении (`0..SIZE_COUNT`).
    pub const fn ordinal(self) -> Option<u8> {
        if self.0 >= 1 && self.0 <= Self::SIZE_COUNT {
            Some(self.0 - 1)
        } else {
            None
        }
    }

    const fn encode(w: u8, h: u8) -> Option<u8> {
        let (w, h) = if w > h { (h, w) } else { (w, h) };
        if w < Self::MIN_BOARD_SIZE.0
            || h < Self::MIN_BOARD_SIZE.1
            || w > Self::MAX_BOARD_SIZE.0
            || h > Self::MAX_BOARD_SIZE.1
        {
            return None;
        }
        let prev_h = (h - Self::MIN_BOARD_SIZE.1) as u16;
        let base_id = prev_h * (prev_h + 1) / 2;
        let id = base_id + (w - Self::MIN_BOARD_SIZE.0 + 1) as u16;
        Some(id as u8)
    }

    const fn from_id(id: u8) -> Self {
        Self(id)
    }

    pub const fn new(w: u8, h: u8) -> Option<Self> {
        match Self::encode(w, h) {
            Some(id) => Some(Self::from_id(id)),
            None => None,
        }
    }

    /// Идентификатор клетки по её слоту: первая клетка — `BoardSize(0)`.
    pub const fn from_slot(slot: u8) -> Self {
        Self(slot)
    }

    /// Слот клетки (с нуля), обратный к [`BoardSize::from_slot`].
    pub const fn slot(self) -> u8 {
        self.0
    }

    pub const fn dimensions(self) -> (u8, u8) {
        let id = self.0 as u16;
        let min_size = Self::MIN_BOARD_SIZE.1 as u16;
        let mut h = min_size;
        loop {
            let n = h - min_size;
            let max_id = (n + 1) * (n + 2) / 2;
            if id <= max_id {
                let prev_max = n * (n + 1) / 2;
                let w = id - prev_max + min_size - 1;
                return (w as u8, h as u8);
            }
            h += 1;
        }
    }
    pub const fn w(self) -> u8 {
        self.dimensions().0
    }
    pub const fn h(self) -> u8 {
        self.dimensions().1
    }
    pub const fn cell_count(self) -> usize {
        let (w, h) = self.dimensions();
        w as usize * h as usize
    }
}

/// Правило распределения массы при взрыве клетки.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExplosionMode {
    Lossy,
    #[default]
    Lossless,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct IdSet([u64; 4]);

impl IdSet {
    fn insert(&mut self, id: u8) {
        self.0[usize::from(id >> 6)] |= 1u64 << (id & 63);
    }

    fn contains(&self, id: u8) -> bool {
        (self.0[usize::from(id >> 6)] & (1u64 << (id & 63))) != 0
    }

    fn first_missing(&self) -> Option<CharacterId> {
        for (index, word) in self.0.iter().enumerate() {
            // Бит 0 считается занятым: id 0 не существует у NonZero-типов.
            let reserved = (index == 0) as u64;
            let available = !*word & !reserved;
            if available != 0 {
                let slot = index * u64::BITS as usize + available.trailing_zeros() as usize;
                if let Some(id) = CharacterId::new(slot as u8) {
                    return Some(id);
                }
            }
        }
        None
    }

    fn len(&self) -> usize {
        self.0.iter().map(|word| word.count_ones() as usize).sum()
    }

    fn iter(self) -> impl Iterator<Item = CharacterId> {
        (1..=u8::MAX).filter_map(move |id| CharacterId::new(id).filter(|_| self.contains(id)))
    }
}

fn first_available_id(ids: impl IntoIterator<Item = CharacterId>) -> Option<CharacterId> {
    let mut used = IdSet::default();
    for id in ids {
        used.insert(id.get());
    }
    used.first_missing()
}

#[cfg(test)]
mod id_set_tests {
    use super::first_available_id;

    #[test]
    fn returns_the_lowest_unused_id() {
        let ids = [1, 2, 4].map(|id| super::CharacterId::new(id).unwrap());
        assert_eq!(first_available_id(ids), super::CharacterId::new(3));
    }

    #[test]
    fn returns_none_when_all_ids_are_used() {
        let ids = (1..=u8::MAX).map(|id| super::CharacterId::new(id).unwrap());
        assert_eq!(first_available_id(ids), None);
    }
}

#[cfg(test)]
mod board_size_tests {
    use super::BoardSize;

    #[test]
    fn orientation_is_normalized() {
        assert_eq!(BoardSize::new(4, 8), BoardSize::new(8, 4));
    }

    #[test]
    fn cell_slots_are_zero_based() {
        assert_eq!(BoardSize::from_slot(0).slot(), 0);
        assert_eq!(BoardSize::from_slot(u8::MAX).slot(), u8::MAX);
    }

    #[test]
    fn max_board_uses_every_cell_slot() {
        let (w, h) = BoardSize::MAX_BOARD_SIZE;
        let max = BoardSize::new(w, h).unwrap();
        assert_eq!((max.w(), max.h()), (w, h));
        assert_eq!(max.cell_count(), usize::from(u8::MAX) + 1);
        assert_eq!(BoardSize::new(w + 1, h + 1), None);
    }

    #[test]
    fn the_ordinal_enumerates_every_canonical_size() {
        assert_eq!(
            BoardSize::from_ordinal(0).unwrap().dimensions(),
            BoardSize::MIN_BOARD_SIZE
        );
        assert_eq!(
            BoardSize::from_ordinal(BoardSize::SIZE_COUNT - 1)
                .unwrap()
                .dimensions(),
            BoardSize::MAX_BOARD_SIZE
        );
        assert_eq!(BoardSize::from_ordinal(BoardSize::SIZE_COUNT), None);

        let sizes: Vec<_> = (0..BoardSize::SIZE_COUNT)
            .filter_map(BoardSize::from_ordinal)
            .map(|size| size.dimensions())
            .collect();
        assert!(sizes.contains(&(3, 6)));
        assert!(sizes.contains(&(4, 9)));
        assert!(sizes.contains(&(5, 16)));
        for (ordinal, size) in sizes.into_iter().enumerate() {
            assert!(size.0 >= 3 && size.0 <= size.1 && size.1 <= 16);
            assert_eq!(
                BoardSize::new(size.0, size.1).unwrap().ordinal(),
                Some(ordinal as u8)
            );
        }
    }
}

#[cfg(test)]
mod game_rule_tests {
    use super::*;

    fn cid(id: u8) -> CharacterId {
        CharacterId::new(id).unwrap()
    }

    fn tid(id: u8) -> TeamId {
        TeamId::new(id).unwrap()
    }

    fn uid(id: u32) -> Uuid {
        Uuid::new(id).unwrap()
    }

    fn size(w: u8, h: u8) -> BoardSize {
        BoardSize::new(w, h).unwrap()
    }

    fn pos(slot: u8) -> BoardSize {
        BoardSize::from_slot(slot)
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
    fn compact_snapshot_reconstructs_initial_distribution_and_turn() {
        let mut game = Game::new(Board::new_rect(size(6, 6)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.start(true).unwrap();

        let snapshot = game.snapshot();
        let restored = snapshot.restore();

        assert_eq!(restored.board, game.board);
        assert_eq!(restored.turn_order, game.turn_order);
        assert_eq!(restored.current_turn, game.current_turn);
    }

    #[test]
    fn snapshot_keeps_the_explosion_mode() {
        let mut game = Game::standard();
        game.explosion_mode = ExplosionMode::Lossless;

        assert_eq!(
            game.snapshot().restore().explosion_mode,
            ExplosionMode::Lossless
        );
    }

    #[test]
    fn elimination_status_is_kept_in_member_stats() {
        let mut game = Game::new(Board::new_rect(size(3, 3)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.add_local_member(uid(3), tid(3)).unwrap();
        game.start(false).unwrap();

        game.apply_action(cid(1), GameAction::Surrender).unwrap();

        assert_eq!(game.stats[&cid(1)].losses, 1);
        assert_eq!(game.current_turn, Some(1));
        assert_eq!(game.check_turn(cid(1)), Err(GameError::CharacterEliminated));
    }

    #[test]
    fn explosions_stop_as_soon_as_one_team_is_left() {
        let mut game = Game::new(Board::new_rect(size(3, 3)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.start(false).unwrap();
        game.stats.get_mut(&cid(1)).unwrap().increase_moves = 1;
        game.stats.get_mut(&cid(2)).unwrap().increase_moves = 1;
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

        let winner = game
            .apply_action(cid(1), GameAction::Increase(pos(0)))
            .unwrap();

        assert_eq!(winner, Some(tid(1)));
        assert_eq!(game.board[pos(3)].value, 4);
        assert!(!game.is_started());
    }

    #[test]
    fn border_absorbs_explosion_mass_and_the_chain_stabilizes() {
        let mut game = Game::new(Board::new_rect(size(3, 3)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.start(false).unwrap();
        for cell in &mut game.board.cells {
            *cell = CellData {
                value: 3,
                owner: Some(tid(1)),
                ..CellData::default()
            };
        }

        let winner = game
            .apply_action(cid(1), GameAction::Increase(pos(0)))
            .unwrap();

        assert_eq!(winner, None);
        assert!(game.is_started());
        assert!(game.board.cells.iter().all(|cell| !cell.can_explode()));
        let remaining_mass: i32 = game
            .board
            .cells
            .iter()
            .map(|cell| i32::from(cell.value))
            .sum();
        assert!(remaining_mass < 28);
    }

    fn board_mass(board: &Board) -> i32 {
        board.cells().iter().map(|cell| i32::from(cell.value)).sum()
    }

    #[test]
    fn lossless_border_explosion_keeps_the_missing_shares() {
        let mut board = Board::new_rect(size(3, 3));
        board[pos(0)] = CellData {
            value: 3,
            owner: Some(tid(1)),
            ability: CellAbility::None,
        };
        let mut blasts = Vec::new();

        // The corner cell becomes 4, explodes and has only two of its four
        // cardinal targets on the board.
        board.apply_move(pos(0), tid(1), 1, &mut blasts, ExplosionMode::Lossless);

        assert_eq!(board[pos(0)].value, 2);
        assert_eq!(board[pos(1)].value, 1);
        assert_eq!(board[pos(3)].value, 1);
        assert_eq!(board_mass(&board), 4);
    }

    #[test]
    fn lossy_border_explosion_dissipates_the_missing_shares() {
        let mut board = Board::new_rect(size(3, 3));
        board[pos(0)] = CellData {
            value: 3,
            owner: Some(tid(1)),
            ability: CellAbility::None,
        };
        let mut blasts = Vec::new();

        board.apply_move(pos(0), tid(1), 1, &mut blasts, ExplosionMode::Lossy);

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
        game.apply_action(cid(1), GameAction::Decrease(pos))
            .unwrap();
        assert_eq!(game.board[pos].value, 0);

        // Put the same member back on turn and try to decrease the empty cell
        // again: it must be rejected without advancing the turn.
        game.current_turn = Some(0);
        let before = game.current_turn;

        assert_eq!(
            game.apply_action(cid(1), GameAction::Decrease(pos)),
            Err(GameError::CellCannotBeDecreased)
        );
        assert_eq!(game.current_turn, before);
        assert_eq!(game.board[pos].value, 0);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Timeline {
    Action {
        member_id: CharacterId,
        action: GameAction,
    },
    End,
}

impl Timeline {
    pub fn new(member_id: CharacterId, action: GameAction) -> Self {
        Self::Action { member_id, action }
    }

    fn game_ended() -> Self {
        Self::End
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameAction {
    Increase(BoardSize),
    Decrease(BoardSize),
    Skip,
    Surrender,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Game {
    pub characters: HashMap<CharacterId, CharacterData>,
    pub stats: HashMap<CharacterId, MemberStat>,
    pub turn_order: Vec<CharacterId>,
    pub current_turn: Option<usize>,
    pub timeline: Vec<Timeline>,
    pub board: Board,
    pub automatic_distribution: bool,
    pub explosion_mode: ExplosionMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameSnapshot {
    pub characters: Vec<(CharacterId, CharacterData)>,
    pub board_size: BoardSize,
    pub automatic_distribution: bool,
    pub explosion_mode: ExplosionMode,
}

impl GameSnapshot {
    pub fn restore(&self) -> Game {
        let mut game = Game::new(Board::new_rect(self.board_size));
        game.characters = self.characters.iter().copied().collect();
        game.stats = game
            .characters
            .keys()
            .map(|&id| (id, MemberStat::default()))
            .collect();
        game.turn_order = game.characters.keys().copied().collect();
        game.turn_order.sort_unstable();
        game.current_turn = Some(0);
        game.automatic_distribution = self.automatic_distribution;
        game.explosion_mode = self.explosion_mode;
        if self.automatic_distribution {
            game.distribute_start_cells();
        }
        game
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
            automatic_distribution: true,
            ..Default::default()
        }
    }

    pub fn is_started(&self) -> bool {
        self.current_turn.is_some()
    }

    pub fn snapshot(&self) -> GameSnapshot {
        let mut characters: Vec<_> = self
            .characters
            .iter()
            .map(|(&id, &character)| (id, character))
            .collect();
        characters.sort_unstable_by_key(|(id, _)| *id);
        GameSnapshot {
            characters,
            board_size: self.board.size(),
            automatic_distribution: self.automatic_distribution,
            explosion_mode: self.explosion_mode,
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

    fn log(&mut self, member_id: CharacterId, action: GameAction) {
        self.timeline.push(Timeline::new(member_id, action));
    }

    /// Adds another seat controlled by the same local player/device.
    pub fn add_local_member(
        &mut self,
        controller: Uuid,
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
        self.stats.insert(member_id, MemberStat::default());
        Ok(member_id)
    }

    pub fn join(&mut self, uuid: Uuid, team_id: TeamId) -> Result<(), GameError> {
        if self.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        let controller = MemberId::Player(uuid);
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
        self.stats.insert(character_id, MemberStat::default());
        Ok(())
    }

    pub fn leave(&mut self, member_id: CharacterId) {
        if self.is_started() {
            let _ = self.apply_action(member_id, GameAction::Surrender);
            return;
        }

        self.characters.remove(&member_id);
        self.stats.remove(&member_id);
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
        self.stats.insert(member_id, MemberStat::default());
        Ok(member_id)
    }

    pub fn kick_member(&mut self, member_id: CharacterId) -> Result<(), GameError> {
        if self.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        if !self.characters.contains_key(&member_id) {
            return Err(GameError::CharacterNotFound);
        }
        self.characters.remove(&member_id);
        self.stats.remove(&member_id);

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

        self.timeline.clear();
        for stat in self.stats.values_mut() {
            *stat = MemberStat::default();
        }
        self.board.clear_cells();
        self.automatic_distribution = automatic_distribution;

        self.turn_order.clear();
        self.turn_order.extend(self.characters.keys().copied());
        self.turn_order.sort_unstable();
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

        let mut placed: Vec<BoardSize> = Vec::with_capacity(teams.len());
        for team in teams.iter() {
            let mut best: Option<(Score, BoardSize)> = None;

            for pos in self.board.ids() {
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
        self.turn_order.clear();
        self.board.clear_cells();
    }

    pub fn end_game(&mut self) -> Result<(), GameError> {
        if !self.is_started() {
            return Err(GameError::GameNotStarted);
        }
        self.current_turn = None;
        self.timeline.push(Timeline::game_ended());
        Ok(())
    }

    fn is_turn(&self, member_id: CharacterId) -> bool {
        self.current_turn.and_then(|idx| self.turn_order.get(idx)) == Some(&member_id)
    }

    fn is_alive(&self, member_id: CharacterId) -> bool {
        self.characters.contains_key(&member_id) && !self.is_eliminated(member_id)
    }

    fn is_eliminated(&self, member_id: CharacterId) -> bool {
        self.stats
            .get(&member_id)
            .is_some_and(|stat| stat.losses > 0)
    }

    fn advance_turn(&mut self) {
        let Some(idx) = self.current_turn else {
            return;
        };
        let len = self.turn_order.len();
        self.current_turn = (1..=len)
            .map(|step| (idx + step) % len)
            .find(|&next| self.is_alive(self.turn_order[next]));
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

    pub fn skip(&mut self, member_id: CharacterId) -> Result<(), GameError> {
        self.check_turn(member_id)?;
        let stat = self.stats.entry(member_id).or_default();
        stat.skipped_moves = stat.skipped_moves.saturating_add(1);
        self.advance_turn();
        Ok(())
    }

    pub fn surrender(&mut self, member_id: CharacterId) -> Result<(), GameError> {
        self.active_team(member_id)?;
        self.eliminate(member_id);
        Ok(())
    }

    pub fn is_starting_action(&self, member_id: CharacterId) -> bool {
        self.characters.get(&member_id).is_some_and(|member| {
            self.stats
                .get(&member_id)
                .is_none_or(|stat| stat.total_moves() == 0)
                && !self.board.owns_cells(member.team_id)
        })
    }

    pub fn make_move(
        &mut self,
        member_id: CharacterId,
        pos: BoardSize,
        increase: bool,
    ) -> Result<(), GameError> {
        self.make_move_with_result(member_id, pos, increase)
            .map(|_| ())
    }

    fn make_move_with_result(
        &mut self,
        member_id: CharacterId,
        pos: BoardSize,
        increase: bool,
    ) -> Result<Option<TeamId>, GameError> {
        let team = self.check_turn(member_id)?;
        if !self.board.contains(pos) {
            return Err(GameError::PositionOutOfBounds);
        }

        let owner = self.board[pos].owner;
        if self.board.owns_cells(team) {
            if owner != Some(team) {
                return Err(GameError::CellNotOwnedByTeam);
            }
        } else if owner.is_some() {
            return Err(GameError::FirstMoveRequiresEmptyCell);
        }

        if !increase && self.board[pos].value <= self.board[pos].ability.min_value() {
            return Err(GameError::CellCannotBeDecreased);
        }

        let stat = self.stats.entry(member_id).or_default();
        let counter = if increase {
            &mut stat.increase_moves
        } else {
            &mut stat.decrease_moves
        };
        *counter = counter.saturating_add(1);

        let delta = if increase { 1 } else { -1 };
        let mut blasts = Vec::new();
        let (characters, stats) = (&self.characters, &self.stats);
        self.board.apply_move_until(
            pos,
            team,
            delta,
            &mut blasts,
            self.explosion_mode,
            |board| {
                Self::sole_surviving_team(board, characters, stats).is_some()
                    || board.is_saturated()
            },
        );

        if let Some(winner) = Self::sole_surviving_team(&self.board, &self.characters, &self.stats)
        {
            self.eliminate_teams_without_cells();
            self.current_turn = None;
            self.award_win(winner);
            return Ok(Some(winner));
        }
        if self.board.is_saturated() {
            return Ok(None);
        }

        self.advance_turn();
        Ok(None)
    }

    pub fn apply_action(
        &mut self,
        member_id: CharacterId,
        action: GameAction,
    ) -> Result<Option<TeamId>, GameError> {
        if !self.is_started() {
            return Err(GameError::GameNotStarted);
        }
        if self.is_starting_action(member_id)
            && !matches!(action, GameAction::Increase(_) | GameAction::Surrender)
        {
            return Err(GameError::FirstMoveRequiresEmptyCell);
        }

        let move_winner = match action {
            GameAction::Increase(pos) => self.make_move_with_result(member_id, pos, true)?,
            GameAction::Decrease(pos) => self.make_move_with_result(member_id, pos, false)?,
            GameAction::Skip => {
                self.skip(member_id)?;
                None
            }
            GameAction::Surrender => {
                self.surrender(member_id)?;
                None
            }
        };

        self.log(member_id, action);

        if move_winner.is_some() {
            return Ok(move_winner);
        }
        if !self.is_started() {
            return Ok(None);
        }
        self.check_game_status()
    }

    pub fn check_game_status(&mut self) -> Result<Option<TeamId>, GameError> {
        if !self.is_started() {
            return Err(GameError::GameNotStarted);
        }

        if let Some(winner) = Self::sole_surviving_team(&self.board, &self.characters, &self.stats)
        {
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
        characters: &HashMap<CharacterId, CharacterData>,
        stats: &HashMap<CharacterId, MemberStat>,
    ) -> Option<TeamId> {
        let mut all = IdSet::default();
        let mut alive = IdSet::default();

        for (&member_id, member) in characters {
            all.insert(member.team_id.get());
            let stat = stats.get(&member_id);
            let has_moved = stat.is_some_and(|stat| stat.total_moves() > 0);
            let is_eliminated = stat.is_some_and(|stat| stat.losses > 0);
            if !is_eliminated && (!has_moved || board.owns_cells(member.team_id)) {
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
                    && self
                        .stats
                        .get(*id)
                        .is_some_and(|stat| stat.total_moves() > 0)
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
        for (&member_id, member) in &self.characters {
            if member.team_id == team {
                let stat = self.stats.entry(member_id).or_default();
                stat.wins = stat.wins.saturating_add(1);
            }
        }
    }

    fn eliminate(&mut self, member_id: CharacterId) {
        if self.characters.contains_key(&member_id) {
            let stat = self.stats.entry(member_id).or_default();
            if stat.losses == 0 {
                stat.losses = 1;
            }
        }
        if self.is_turn(member_id) {
            self.advance_turn();
        }
    }

    pub fn move_weights(&self, member_id: CharacterId, cfg: &BotConfig) -> Vec<(BoardSize, Score)> {
        let Ok(team) = self.check_turn(member_id) else {
            return Vec::new();
        };

        let mut scratch = Board::default();
        scratch.clone_from(&self.board);
        let mut blasts = Vec::new();

        self.legal_cells(team)
            .into_iter()
            .map(|pos| {
                scratch.clone_from(&self.board);
                let steps = scratch.apply_move(pos, team, 1, &mut blasts, self.explosion_mode);
                let chain = (steps.saturating_sub(1) as Score) * cfg.chain_bonus;
                (pos, scratch.evaluate(team, cfg) + chain)
            })
            .collect()
    }

    pub fn bot_action(&self, member_id: CharacterId, cfg: &BotConfig) -> GameAction {
        cfg.pick_best_action(&self.move_weights(member_id, cfg))
    }

    fn legal_cells(&self, team: TeamId) -> Vec<BoardSize> {
        let has_cells = self.board.owns_cells(team);

        self.board
            .iter()
            .filter(|(_, c)| {
                if has_cells {
                    c.owner == Some(team)
                } else {
                    c.owner.is_none()
                }
            })
            .map(|(pos, _)| pos)
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BotConfig {
    pub own_cell_bonus: Score,
    pub own_value_weight: Score,
    pub enemy_value_weight: Score,
    pub threat_penalty: Score,
    pub chain_bonus: Score,
}

impl BotConfig {
    pub const STANDARD: Self = Self {
        own_cell_bonus: 1,
        own_value_weight: 1,
        enemy_value_weight: 1,
        threat_penalty: 3,
        chain_bonus: 2,
    };

    pub fn pick_best_action(&self, weights: &[(BoardSize, Score)]) -> GameAction {
        weights
            .iter()
            .max_by_key(|&&(pos, score)| (score, Reverse(pos.slot())))
            .map_or(GameAction::Skip, |&(pos, _)| GameAction::Increase(pos))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Board {
    cells: Vec<CellData>,
    size: BoardSize,
}

impl Default for Board {
    fn default() -> Self {
        let size = BoardSize::MIN;
        Self {
            cells: vec![CellData::default(); size.cell_count()],
            size,
        }
    }
}

impl Index<BoardSize> for Board {
    type Output = CellData;

    #[inline]
    fn index(&self, pos: BoardSize) -> &CellData {
        debug_assert!(self.contains(pos));
        &self.cells[usize::from(pos.slot())]
    }
}

impl IndexMut<BoardSize> for Board {
    #[inline]
    fn index_mut(&mut self, pos: BoardSize) -> &mut CellData {
        debug_assert!(self.contains(pos));
        &mut self.cells[usize::from(pos.slot())]
    }
}

impl Board {
    pub fn standard() -> Self {
        Self::new_rect(BoardSize::STANDARD)
    }

    pub fn new_rect(size: BoardSize) -> Self {
        let mut board = Self::default();
        board.set_size(size);
        board
    }

    pub fn set_size(&mut self, size: BoardSize) {
        // Re-validate through the dimension code: a deserialized value may not
        // describe a playable board, because the same byte type also encodes
        // cell slots.
        let (w, h) = size.dimensions();
        let size = BoardSize::new(w, h).unwrap_or(BoardSize::MIN);
        self.size = size;
        self.cells.resize(size.cell_count(), CellData::default());
        self.clear_cells();
    }

    pub fn size(&self) -> BoardSize {
        self.size
    }

    pub fn cells(&self) -> &Vec<CellData> {
        &self.cells
    }

    pub fn get(&self, pos: BoardSize) -> Option<&CellData> {
        if self.contains(pos) {
            Some(&self.cells[usize::from(pos.slot())])
        } else {
            None
        }
    }

    pub fn contains(&self, pos: BoardSize) -> bool {
        usize::from(pos.slot()) < self.cells.len()
    }

    /// Идентификатор клетки по её координатам.
    pub fn position(&self, x: u8, y: u8) -> Option<BoardSize> {
        if usize::from(x) >= usize::from(self.size.w())
            || usize::from(y) >= usize::from(self.size.h())
        {
            return None;
        }
        let slot = u16::from(y) * u16::from(self.size.w()) + u16::from(x);
        Some(BoardSize::from_slot(slot as u8))
    }

    /// Координаты клетки.
    pub fn coordinates(&self, pos: BoardSize) -> Option<(u8, u8)> {
        self.contains(pos)
            .then(|| self.xy_from_slot(usize::from(pos.slot())))
    }

    #[inline]
    fn xy_from_slot(&self, slot: usize) -> (u8, u8) {
        let w = usize::from(self.size.w());
        ((slot % w) as u8, (slot / w) as u8)
    }

    #[inline]
    fn pos_from_slot(slot: usize) -> BoardSize {
        debug_assert!(slot <= usize::from(u8::MAX));
        BoardSize::from_slot(slot as u8)
    }

    fn distance_sq(&self, a: BoardSize, b: BoardSize) -> Score {
        let (ax, ay) = self.xy_from_slot(usize::from(a.slot()));
        let (bx, by) = self.xy_from_slot(usize::from(b.slot()));
        let dx = Score::from(ax) - Score::from(bx);
        let dy = Score::from(ay) - Score::from(by);
        dx * dx + dy * dy
    }

    pub fn ids(&self) -> impl Iterator<Item = BoardSize> + '_ {
        (0..self.cells.len()).map(|slot| BoardSize::from_slot(slot as u8))
    }

    pub fn iter(&self) -> impl Iterator<Item = (BoardSize, CellData)> + '_ {
        self.cells
            .iter()
            .enumerate()
            .map(|(slot, &cell)| (BoardSize::from_slot(slot as u8), cell))
    }

    pub fn owns_cells(&self, team: TeamId) -> bool {
        self.cells.iter().any(|c| c.owner == Some(team))
    }

    pub fn is_saturated(&self) -> bool {
        self.cells.iter().all(|c| c.can_explode())
    }

    pub fn mass_leader(&self) -> Option<TeamId> {
        let mut scores: [Score; 256] = [0; 256];
        let mut present = IdSet::default();

        for cell in &self.cells {
            if let Some(owner) = cell.owner {
                scores[usize::from(owner.get())] += Score::from(cell.value);
                present.insert(owner.get());
            }
        }

        present
            .iter()
            .max_by_key(|&team| (scores[usize::from(team.get())], Reverse(team)))
    }

    pub fn clear_cells(&mut self) {
        self.cells.fill(CellData::default());
    }

    fn apply_move(
        &mut self,
        pos: BoardSize,
        team: TeamId,
        delta: i8,
        blasts: &mut Vec<Blast>,
        mode: ExplosionMode,
    ) -> u32 {
        self.apply_move_until(pos, team, delta, blasts, mode, |_| false)
    }

    fn apply_move_until<F>(
        &mut self,
        pos: BoardSize,
        team: TeamId,
        delta: i8,
        blasts: &mut Vec<Blast>,
        mode: ExplosionMode,
        mut stop_after_wave: F,
    ) -> u32
    where
        F: FnMut(&Board) -> bool,
    {
        let cell = &mut self[pos];
        cell.value = cell.value.saturating_add(delta).max(0);
        cell.owner = Some(team);

        let mut waves = 0;
        while self.explode_wave(blasts, mode) {
            waves += 1;
            if stop_after_wave(self) {
                break;
            }
        }
        waves
    }

    fn explode_wave(&mut self, blasts: &mut Vec<Blast>, mode: ExplosionMode) -> bool {
        blasts.clear();

        for (slot, &cell) in self.cells.iter().enumerate() {
            if cell.can_explode() {
                blasts.push(Blast::new(self, Self::pos_from_slot(slot), cell, mode));
            }
        }

        if blasts.is_empty() {
            return false;
        }

        for b in blasts.iter() {
            let cell = &mut self[b.pos];
            cell.value = b.remainder;
            cell.ability = CellAbility::None;
        }

        for b in blasts.iter() {
            for &(direction, dmg) in b.targets {
                let Some(t) = self.neighbor(b.pos, direction) else {
                    continue;
                };
                let target = &mut self[t];
                if dmg == 0 {
                    target.value = target.value.saturating_add(b.share);
                    target.owner = b.owner;
                } else {
                    target.apply_ability_damage(dmg);
                }
            }
        }

        true
    }

    fn evaluate(&self, team: TeamId, cfg: &BotConfig) -> Score {
        let mut total = 0;

        for (slot, cell) in self.cells.iter().enumerate() {
            match cell.owner {
                Some(owner) if owner == team => {
                    let value = Score::from(cell.value);
                    let mut score = cfg.own_cell_bonus + value * cfg.own_value_weight;
                    if self.is_threatened(Self::pos_from_slot(slot), *cell, team) {
                        score -= cfg.threat_penalty;
                    }
                    total += score;
                }
                Some(_) => {
                    total -= Score::from(cell.value) * cfg.enemy_value_weight;
                }
                None => {}
            }
        }
        total
    }

    fn is_threatened(&self, pos: BoardSize, cell: CellData, team: TeamId) -> bool {
        cell.ability.targets().iter().any(|&(direction, _)| {
            self.neighbor(pos, direction).is_some_and(|n| {
                let other = self[n];
                other.owner.is_some_and(|o| o != team) && other.can_explode()
            })
        })
    }

    fn neighbor(&self, pos: BoardSize, direction: u8) -> Option<BoardSize> {
        let (x, y) = self.xy_from_slot(usize::from(pos.slot()));
        let (dx, dy) = direction_offset(direction);
        let nx = i16::from(x) + i16::from(dx);
        let ny = i16::from(y) + i16::from(dy);
        if nx < 0 || ny < 0 || nx >= i16::from(self.size.w()) || ny >= i16::from(self.size.h()) {
            None
        } else {
            self.position(nx as u8, ny as u8)
        }
    }
}

struct Blast {
    pos: BoardSize,
    owner: Option<TeamId>,
    targets: &'static [(u8, i8)],
    share: i8,
    remainder: i8,
}

impl Blast {
    fn new(board: &Board, pos: BoardSize, cell: CellData, mode: ExplosionMode) -> Self {
        let targets = cell.ability.targets();
        let output_slots = targets.iter().filter(|&&(_, damage)| damage == 0).count() as i8;

        let (share, remainder) = if output_slots > 0 {
            let share = cell.value / output_slots;
            let remainder = cell.value % output_slots;
            let remainder = if mode == ExplosionMode::Lossless {
                // Lossless explosions keep the shares that cannot be delivered
                // to a neighbour (for example, at the border) in the cell.
                let present = targets
                    .iter()
                    .filter(|&&(direction, damage)| {
                        damage == 0 && board.neighbor(pos, direction).is_some()
                    })
                    .count() as i8;
                remainder + (output_slots - present) * share
            } else {
                remainder
            };
            (share, remainder)
        } else {
            (0, cell.value)
        };

        Self {
            pos,
            owner: cell.owner,
            targets,
            share,
            remainder,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MemberId {
    Player(Uuid),
    Bot,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberStat {
    pub increase_moves: u16,
    pub decrease_moves: u16,
    pub skipped_moves: u16,
    pub wins: u8,
    pub losses: u8,
}

impl MemberStat {
    pub fn total_moves(&self) -> u16 {
        self.increase_moves
            .saturating_add(self.decrease_moves)
            .saturating_add(self.skipped_moves)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterData {
    pub controller: MemberId,
    pub team_id: TeamId,
}

impl CharacterData {
    pub fn new(controller: MemberId, team_id: TeamId) -> Self {
        Self {
            controller,
            team_id,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellData {
    pub value: i8,
    pub ability: CellAbility,
    pub owner: Option<TeamId>,
}

impl CellData {
    pub fn can_explode(&self) -> bool {
        match self.ability {
            CellAbility::None => self.value >= self.ability.max_value(),
            CellAbility::Bomb => true,
        }
    }

    fn apply_ability_damage(&mut self, dmg: i8) {
        self.value = self.value.saturating_add(dmg).max(self.ability.min_value());
        if self.value <= 0 {
            self.owner = None;
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellAbility {
    #[default]
    None,
    Bomb,
}

impl CellAbility {
    pub fn min_value(self) -> i8 {
        0
    }

    pub fn max_value(self) -> i8 {
        self.targets().len() as i8
    }

    pub fn targets(&self) -> &'static [(u8, i8)] {
        match self {
            CellAbility::None => &NORMAL_TARGETS,
            CellAbility::Bomb => &BOMB_TARGETS,
        }
    }

    pub fn char(self) -> char {
        match self {
            CellAbility::None => ' ',
            CellAbility::Bomb => '💣',
        }
    }
}

/// A regular cell explodes into its four cardinal neighbours only. The centre
/// is not a target: including it would both raise the explosion threshold and
/// feed the mass back into the exploding cell.
const NORMAL_TARGETS: [(u8, i8); 4] = [(3, 0), (7, 0), (5, 0), (1, 0)];

const BOMB_N: usize = disk_cell_count(2);
const BOMB_TARGETS: [(u8, i8); BOMB_N] = interpolate_by_distance(disk_cells::<BOMB_N>(2), -3, -1);
