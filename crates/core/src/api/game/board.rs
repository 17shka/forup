use std::cmp::Reverse;
use std::num::NonZeroU8;
use std::ops::{Index, IndexMut};

use indexmap::IndexMap;

use serde::{Deserialize, Serialize};

use crate::api::circle::prelude::*;
use crate::api::network::Uuid; // NonZeroU32

pub mod prelude {
    pub use super::{
        Board, BoardCode, BotConfig, CellAbility, CellData, CharacterData, CharacterId,
        ExplosionMode, Game, GameAction, GameError, GameSnapshot, MemberId, MemberStat, Score,
        TeamId, Timeline,
    };
}

/// Ненулевые идентификаторы: `Option<CharacterId>` и `Option<TeamId>` занимают 1 байт.
pub type CharacterId = NonZeroU8;
pub type TeamId = NonZeroU8;
pub type Score = i32;

/// Однобайтовый код для всего, что относится к клеточному полю. Одно и то же
/// значение используется в трёх ролях:
///
/// * **размер доски** — код строится через [`BoardCode::from_size`]; стороны
///   нормализуются так, что `w <= h`, поэтому `4 × 8` и `8 × 4` — один размер;
/// * **клетка** — код строится через [`BoardCode::from_slot`] и равен слоту
///   клетки, нумерация с нуля;
/// * **направление** — слот в окрестности `3 × 3` вокруг клетки (см.
///   [`crate::api::circle`]).
///
/// В одном байте помещается 256 слотов клеток, поэтому максимальная доска —
/// `16 × 16`. Кодов канонических размеров (пар сторон `3..=16` с `w <= h`)
/// нужно всего 105, так что ограничивает именно количество клеток, а не
/// кодировка размера.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardCode(u8);

impl BoardCode {
    /// Наименьшая сторона допустимой доски.
    pub const MIN_SIDE: u8 = 3;
    /// Наибольшая сторона допустимой доски: `16 × 16` занимает все 256 слотов.
    pub const MAX_SIDE: u8 = 16;

    /// Минимальная доска `3 × 3`.
    pub const MIN_SIZE: Self = Self::size_code(Self::MIN_SIDE, Self::MIN_SIDE);
    /// Стандартная доска `8 × 8`.
    pub const STANDARD_SIZE: Self = Self::size_code(8, 8);
    /// Максимальная доска `16 × 16`, использует все 256 слотов клеток.
    pub const MAX_SIZE: Self = Self::size_code(Self::MAX_SIDE, Self::MAX_SIDE);
    /// Количество канонических размеров; коды размеров занимают `1..=SIZE_COUNT`.
    pub const SIZE_COUNT: u8 = Self::MAX_SIZE.0;

    /// Код размера доски по его сторонам. `None`, если сторона меньше
    /// [`MIN_SIDE`] или больше [`MAX_SIDE`].
    pub const fn from_size(w: NonZeroU8, h: NonZeroU8) -> Option<Self> {
        let (w, h) = (w.get(), h.get());
        if w < Self::MIN_SIDE || h < Self::MIN_SIDE || w > Self::MAX_SIDE || h > Self::MAX_SIDE {
            return None;
        }
        Some(Self::size_code(w, h))
    }

    /// Размер по его порядковому номеру в перечислении канонических размеров
    /// (`0..SIZE_COUNT`). Порядок перечисления — по возрастанию высоты, затем
    /// ширины: `3 × 3`, `3 × 4`, `4 × 4`, `3 × 5`, ...
    pub const fn from_ordinal(ordinal: u8) -> Option<Self> {
        if ordinal < Self::SIZE_COUNT {
            Some(Self(ordinal + 1))
        } else {
            None
        }
    }

    /// Код клетки по её слоту: первая клетка — слот `0`.
    pub const fn from_slot(slot: u8) -> Self {
        Self(slot)
    }

    /// Порядковый номер размера в перечислении канонических размеров,
    /// обратный к [`BoardCode::from_ordinal`]. Осмыслен только для кодов,
    /// полученных через `from_size`/`from_ordinal`: слотовый код даёт значение
    /// за пределами `0..SIZE_COUNT`.
    pub const fn ordinal(self) -> u8 {
        self.0.wrapping_sub(1)
    }

    /// Слот клетки, обратный к [`BoardCode::from_slot`].
    pub const fn slot(self) -> u8 {
        self.0
    }

    /// Стороны доски `(w, h)` с `w <= h`.
    pub const fn dimensions(self) -> (NonZeroU8, NonZeroU8) {
        let id = self.0 as u16;
        let min_side = Self::MIN_SIDE as u16;
        let mut h = min_side;
        loop {
            let n = h - min_side;
            let max_id = (n + 1) * (n + 2) / 2;
            if id <= max_id {
                let w = id - n * (n + 1) / 2 + min_side - 1;
                // Для любого байта w >= 2, поэтому `unwrap` не срабатывает.
                return (
                    NonZeroU8::new(w as u8).unwrap(),
                    NonZeroU8::new(h as u8).unwrap(),
                );
            }
            h += 1;
        }
    }

    /// Ширина доски, `w <= h`.
    pub const fn width(self) -> NonZeroU8 {
        self.dimensions().0
    }

    /// Высота доски, `h >= w`.
    pub const fn height(self) -> NonZeroU8 {
        self.dimensions().1
    }

    /// Количество клеток доски.
    pub const fn cell_count(self) -> usize {
        let (w, h) = self.dimensions();
        w.get() as usize * h.get() as usize
    }

    /// Код канонического размера со сторонами `MIN_SIDE..=MAX_SIDE`.
    const fn size_code(w: u8, h: u8) -> Self {
        let (w, h) = if w > h { (h, w) } else { (w, h) };
        let prev_h = (h - Self::MIN_SIDE) as u16;
        let id = prev_h * (prev_h + 1) / 2 + (w - Self::MIN_SIDE + 1) as u16;
        Self(id as u8)
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
mod board_code_tests {
    use super::{BoardCode, NonZeroU8};

    fn side(n: u8) -> NonZeroU8 {
        NonZeroU8::new(n).unwrap()
    }

    fn size(w: u8, h: u8) -> Option<BoardCode> {
        BoardCode::from_size(side(w), side(h))
    }

    #[test]
    fn out_of_range_sides_are_rejected() {
        assert_eq!(size(1, 3), None);
        assert_eq!(size(3, 2), None);
        assert_eq!(size(17, 17), None);
    }

    #[test]
    fn orientation_is_normalized() {
        assert_eq!(size(4, 8), size(8, 4));
    }

    #[test]
    fn cell_slots_are_zero_based() {
        assert_eq!(BoardCode::from_slot(0).slot(), 0);
        assert_eq!(BoardCode::from_slot(u8::MAX).slot(), u8::MAX);
    }

    #[test]
    fn max_board_uses_every_cell_slot() {
        let max = BoardCode::MAX_SIZE;
        assert_eq!((max.width(), max.height()), (side(16), side(16)));
        assert_eq!(max.cell_count(), usize::from(u8::MAX) + 1);
        assert_eq!(size(17, 17), None);
    }

    #[test]
    fn the_ordinal_enumerates_every_canonical_size() {
        assert_eq!(BoardCode::MIN_SIZE.dimensions(), (side(3), side(3)));
        assert_eq!(
            BoardCode::from_ordinal(BoardCode::SIZE_COUNT - 1)
                .unwrap()
                .dimensions(),
            (side(16), side(16))
        );
        assert_eq!(BoardCode::from_ordinal(BoardCode::SIZE_COUNT), None);

        let sizes: Vec<_> = (0..BoardCode::SIZE_COUNT)
            .filter_map(BoardCode::from_ordinal)
            .map(|size| size.dimensions())
            .collect();
        assert!(sizes.contains(&(side(3), side(6))));
        assert!(sizes.contains(&(side(4), side(9))));
        assert!(sizes.contains(&(side(5), side(16))));
        for (ordinal, (w, h)) in sizes.into_iter().enumerate() {
            assert!(w.get() >= 3 && w.get() <= h.get() && h.get() <= 16);
            assert_eq!(BoardCode::from_size(w, h).unwrap().ordinal(), ordinal as u8);
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
    fn compact_snapshot_reconstructs_initial_distribution_and_turn() {
        let mut game = Game::new(Board::new(size(6, 6)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.start(true).unwrap();

        let snapshot = game.snapshot(true);
        let restored = snapshot.restore();

        assert_eq!(restored.board, game.board);
        assert_eq!(restored.current_turn, game.current_turn);
    }

    #[test]
    fn snapshot_keeps_the_explosion_mode() {
        let mut game = Game::standard();
        game.explosion_mode = ExplosionMode::Lossless;

        assert_eq!(
            game.snapshot(false).restore().explosion_mode,
            ExplosionMode::Lossless
        );
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
    fn start_cells_keep_the_safe_radius() {
        let mut game = Game::new(Board::new(size(8, 8)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        let safe_radius = game.set_safe_radius(8).unwrap();
        assert_eq!(safe_radius, 8);
        game.start(false).unwrap();

        game.apply_action(cid(1), GameAction::Increase(pos(0)))
            .unwrap();
        let near = game.board.position(2, 2).unwrap();
        let far = game.board.position(3, 3).unwrap();
        assert!(!game.can_start_at(near));
        assert_eq!(
            game.apply_action(cid(2), GameAction::Increase(near)),
            Err(GameError::StartCellTooClose)
        );
        assert!(game.can_start_at(far));
        game.apply_action(cid(2), GameAction::Increase(far))
            .unwrap();
    }

    #[test]
    fn starting_decrease_claims_the_cell_without_a_value() {
        let mut game = Game::new(Board::new(size(6, 6)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.start(false).unwrap();

        // Уменьшение числится как ход, но клетка остаётся нулевой.
        game.apply_action(cid(1), GameAction::Decrease(pos(0)))
            .unwrap();
        let cell = game.board[pos(0)];
        assert_eq!(cell.owner, Some(tid(1)));
        assert_eq!(cell.value, 0);
        assert_eq!(game.characters[&cid(1)].stats.decrease_moves, 1);
        assert_eq!(game.current_turn, Some(1));

        // Команда уже владеет клеткой нулевого значения — обычное уменьшение
        // требует запас до минимального значения и отклоняется.
        game.apply_action(cid(2), GameAction::Increase(pos(35)))
            .unwrap();
        assert_eq!(
            game.apply_action(cid(1), GameAction::Decrease(pos(0))),
            Err(GameError::CellCannotBeDecreased)
        );
    }

    #[test]
    fn every_team_finds_a_start_cell_at_the_maximum_safe_radius() {
        for teams in 2..=crate::api::circle::MAX_CACHED_PLAYERS {
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

        game.apply_action(cid(1), GameAction::Surrender).unwrap();

        assert_eq!(game.characters[&cid(1)].stats.losses, 1);
        assert_eq!(game.current_turn, Some(1));
        assert_eq!(game.check_turn(cid(1)), Err(GameError::CharacterEliminated));
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

        let winner = game
            .apply_action(cid(1), GameAction::Increase(pos(0)))
            .unwrap();

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
        let mut board = Board::new(size(3, 3));
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
    /// Стартовая клетка ближе к занятой клетке, чем разрешает [`Game::safe_radius`].
    StartCellTooClose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Timeline {
    Action(GameAction),
    End,
}

impl Timeline {
    pub fn new(action: GameAction) -> Self {
        Self::Action(action)
    }

    fn game_ended() -> Self {
        Self::End
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameAction {
    Increase(BoardCode),
    Decrease(BoardCode),
    Skip,
    Surrender,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Game {
    pub characters: IndexMap<CharacterId, CharacterData>,
    pub current_turn: Option<usize>,
    pub timeline: Vec<Timeline>,
    pub board: Board,
    pub explosion_mode: ExplosionMode,
    /// Квадрат «безопасного радиуса»: минимальное расстояние от стартовой клетки
    /// до уже занятых клеток. Меняется только через [`Game::set_safe_radius`],
    /// которая не даёт превысить [`Game::max_safe_radius`].
    #[serde(default)]
    safe_radius: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameSnapshot {
    pub characters: Vec<(CharacterId, CharacterData)>,
    pub board_size: BoardCode,
    pub explosion_mode: ExplosionMode,
    #[serde(default)]
    pub safe_radius: u8,
    /// Началась ли партия с автоматической раздачей стартовых клеток.
    /// [`GameSnapshot::restore`] повторяет раздачу, чтобы поле совпало с исходным.
    #[serde(default)]
    pub automatic_distribution: bool,
}

impl GameSnapshot {
    pub fn restore(&self) -> Game {
        let mut game = Game::new(Board::new(self.board_size));
        game.characters = self.characters.iter().copied().collect();
        game.characters.sort_keys();
        game.current_turn = Some(0);
        game.explosion_mode = self.explosion_mode;
        game.safe_radius = self.safe_radius;
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
            ..Default::default()
        }
    }

    pub fn is_started(&self) -> bool {
        self.current_turn.is_some()
    }

    /// Компактный снимок для повтора партии. `automatic_distribution` решает,
    /// повторит ли [`GameSnapshot::restore`] раздачу стартовых клеток.
    pub fn snapshot(&self, automatic_distribution: bool) -> GameSnapshot {
        let mut characters: Vec<_> = self
            .characters
            .iter()
            .map(|(&id, &character)| (id, character))
            .collect();
        characters.sort_unstable_by_key(|(id, _)| *id);
        GameSnapshot {
            characters,
            board_size: self.board.size(),
            explosion_mode: self.explosion_mode,
            safe_radius: self.safe_radius(),
            automatic_distribution,
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
            && self.board.iter().all(|(other, cell)| {
                cell.owner.is_none() || self.board.distance_sq(pos, other) > safe_radius
            })
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
    fn start_cells(&self) -> Vec<BoardCode> {
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

    fn log(&mut self, action: GameAction) {
        self.timeline.push(Timeline::new(action));
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
        Ok(())
    }

    pub fn leave(&mut self, member_id: CharacterId) {
        if self.is_started() {
            let _ = self.apply_action(member_id, GameAction::Surrender);
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

        self.timeline.clear();
        for (_, character) in self.characters.iter_mut() {
            character.stats = MemberStat::default();
        }
        self.board.clear_cells();

        self.characters.sort_keys();
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

    pub fn end_game(&mut self) -> Result<(), GameError> {
        if !self.is_started() {
            return Err(GameError::GameNotStarted);
        }
        self.current_turn = None;
        self.timeline.push(Timeline::game_ended());
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

    pub fn skip(&mut self, member_id: CharacterId) -> Result<(), GameError> {
        self.check_turn(member_id)?;
        let character = self.characters.get_mut(&member_id).unwrap();
        character.stats.skipped_moves = character.stats.skipped_moves.saturating_add(1);
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
            member.stats.total_moves() == 0 && !self.board.owns_cells(member.team_id)
        })
    }

    pub fn make_move(
        &mut self,
        member_id: CharacterId,
        pos: BoardCode,
        increase: bool,
    ) -> Result<(), GameError> {
        self.make_move_with_result(member_id, pos, increase, &mut |_| {})
            .map(|_| ())
    }

    fn make_move_with_result(
        &mut self,
        member_id: CharacterId,
        pos: BoardCode,
        increase: bool,
        on_step: &mut dyn FnMut(&Board),
    ) -> Result<Option<TeamId>, GameError> {
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
        let mut blasts = Vec::new();
        self.board.apply_move_until(
            pos,
            team,
            delta,
            &mut blasts,
            self.explosion_mode,
            on_step,
            |board| {
                Self::sole_surviving_team(board, &self.characters).is_some() || board.is_saturated()
            },
        );

        if let Some(winner) = Self::sole_surviving_team(&self.board, &self.characters) {
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
        self.apply_action_with_steps(member_id, action, |_| {})
    }

    /// Same as [`Self::apply_action`], but reports the board after the initial
    /// change and after every explosion wave so the UI can replay it step by
    /// step.
    pub fn apply_action_with_steps(
        &mut self,
        member_id: CharacterId,
        action: GameAction,
        mut on_step: impl FnMut(&Board),
    ) -> Result<Option<TeamId>, GameError> {
        if !self.is_started() {
            return Err(GameError::GameNotStarted);
        }
        if self.is_starting_action(member_id)
            && !matches!(
                action,
                GameAction::Increase(_) | GameAction::Decrease(_) | GameAction::Surrender
            )
        {
            return Err(GameError::FirstMoveRequiresEmptyCell);
        }

        let move_winner = match action {
            GameAction::Increase(pos) => {
                self.make_move_with_result(member_id, pos, true, &mut on_step)?
            }
            GameAction::Decrease(pos) => {
                self.make_move_with_result(member_id, pos, false, &mut on_step)?
            }
            GameAction::Skip => {
                self.skip(member_id)?;
                None
            }
            GameAction::Surrender => {
                self.surrender(member_id)?;
                None
            }
        };

        self.log(action);

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
        let mut all = IdSet::default();
        let mut alive = IdSet::default();

        for (&_member_id, member) in characters {
            all.insert(member.team_id.get());
            let has_moved = member.stats.total_moves() > 0;
            let is_eliminated = member.stats.losses > 0;
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
        if let Some(character) = self.characters.get_mut(&member_id) {
            if character.stats.losses == 0 {
                character.stats.losses = 1;
            }
        }
        if self.is_turn(member_id) {
            self.advance_turn();
        }
    }

    pub fn move_weights(&self, member_id: CharacterId, cfg: &BotConfig) -> Vec<(BoardCode, Score)> {
        let Ok(team) = self.check_turn(member_id) else {
            return Vec::new();
        };

        let mut scratch = Board::default();
        scratch.clone_from(&self.board);
        let mut blasts = Vec::new();

        let has_cells = self.board.owns_cells(team);

        // When a team has no cells yet (e.g. automatic distribution is off),
        // collect enemy positions so starting cells can be placed as far away
        // from other captured cells as possible.
        let enemy_positions: Vec<BoardCode> = if !has_cells {
            self.board
                .iter()
                .filter(|(_, c)| c.owner.is_some_and(|o| o != team))
                .map(|(pos, _)| pos)
                .collect()
        } else {
            Vec::new()
        };

        self.legal_cells(team)
            .into_iter()
            .map(|pos| {
                scratch.clone_from(&self.board);
                let steps = scratch.apply_move(pos, team, 1, &mut blasts, self.explosion_mode);
                let chain = (steps.saturating_sub(1) as Score) * cfg.chain_bonus;
                let mut score = scratch.evaluate(team, cfg) + chain;

                // With no owned cells, every empty cell scores equally. Break the
                // tie by distance: prefer starting positions far from any enemy
                // cell so bots don't all clump in the same corner.
                if !has_cells && !enemy_positions.is_empty() {
                    let min_dist_sq = enemy_positions
                        .iter()
                        .map(|&enemy| self.board.distance_sq(pos, enemy))
                        .min()
                        .unwrap_or(Score::MAX);
                    score += min_dist_sq;
                }

                (pos, score)
            })
            .collect()
    }

    pub fn bot_action(&self, member_id: CharacterId, cfg: &BotConfig) -> GameAction {
        cfg.pick_best_action(&self.move_weights(member_id, cfg))
    }

    fn legal_cells(&self, team: TeamId) -> Vec<BoardCode> {
        if !self.board.owns_cells(team) {
            return self.start_cells();
        }
        self.board
            .iter()
            .filter(|(_, c)| c.owner == Some(team))
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

    pub fn pick_best_action(&self, weights: &[(BoardCode, Score)]) -> GameAction {
        weights
            .iter()
            .max_by_key(|&&(pos, score)| (score, Reverse(pos.slot())))
            .map_or(GameAction::Skip, |&(pos, _)| GameAction::Increase(pos))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Board {
    cells: Vec<CellData>,
    size: BoardCode,
}

impl Default for Board {
    fn default() -> Self {
        let size = BoardCode::MIN_SIZE;
        Self {
            cells: vec![CellData::default(); size.cell_count()],
            size,
        }
    }
}

impl Index<BoardCode> for Board {
    type Output = CellData;

    #[inline]
    fn index(&self, pos: BoardCode) -> &CellData {
        debug_assert!(self.contains(pos));
        &self.cells[usize::from(pos.slot())]
    }
}

impl IndexMut<BoardCode> for Board {
    #[inline]
    fn index_mut(&mut self, pos: BoardCode) -> &mut CellData {
        debug_assert!(self.contains(pos));
        &mut self.cells[usize::from(pos.slot())]
    }
}

impl Board {
    pub fn standard() -> Self {
        Self::new(BoardCode::STANDARD_SIZE)
    }

    pub fn new(size: BoardCode) -> Self {
        let mut board = Self::default();
        board.set_size(size);
        board
    }

    pub fn set_size(&mut self, size: BoardCode) {
        // Код из снимка может оказаться слотом клетки, а не размером доски:
        // прогоняем его через from_size, чтобы получить валидный размер.
        let (w, h) = size.dimensions();
        let size = BoardCode::from_size(w, h).unwrap_or(BoardCode::MIN_SIZE);
        self.size = size;
        self.cells.resize(size.cell_count(), CellData::default());
        self.clear_cells();
    }

    pub fn size(&self) -> BoardCode {
        self.size
    }

    pub fn cells(&self) -> &Vec<CellData> {
        &self.cells
    }

    pub fn get(&self, pos: BoardCode) -> Option<&CellData> {
        if self.contains(pos) {
            Some(&self.cells[usize::from(pos.slot())])
        } else {
            None
        }
    }

    pub fn contains(&self, pos: BoardCode) -> bool {
        usize::from(pos.slot()) < self.cells.len()
    }

    pub fn position(&self, x: u8, y: u8) -> Option<BoardCode> {
        if x >= self.size.width().get() || y >= self.size.height().get() {
            return None;
        }
        let slot = u16::from(y) * u16::from(self.size.width().get()) + u16::from(x);
        Some(BoardCode::from_slot(slot as u8))
    }

    pub fn coordinates(&self, pos: BoardCode) -> Option<(u8, u8)> {
        self.contains(pos)
            .then(|| self.xy_from_slot(usize::from(pos.slot())))
    }

    #[inline]
    fn xy_from_slot(&self, slot: usize) -> (u8, u8) {
        let w = usize::from(self.size.width().get());
        ((slot % w) as u8, (slot / w) as u8)
    }

    #[inline]
    fn pos_from_slot(slot: usize) -> BoardCode {
        debug_assert!(slot <= usize::from(u8::MAX));
        BoardCode::from_slot(slot as u8)
    }

    fn distance_sq(&self, a: BoardCode, b: BoardCode) -> Score {
        let (ax, ay) = self.xy_from_slot(usize::from(a.slot()));
        let (bx, by) = self.xy_from_slot(usize::from(b.slot()));
        let dx = Score::from(ax) - Score::from(bx);
        let dy = Score::from(ay) - Score::from(by);
        dx * dx + dy * dy
    }

    pub fn positions(&self) -> impl Iterator<Item = BoardCode> + '_ {
        (0..self.cells.len()).map(|slot| BoardCode::from_slot(slot as u8))
    }

    pub fn iter(&self) -> impl Iterator<Item = (BoardCode, CellData)> + '_ {
        self.cells
            .iter()
            .enumerate()
            .map(|(slot, &cell)| (BoardCode::from_slot(slot as u8), cell))
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
        pos: BoardCode,
        team: TeamId,
        delta: i8,
        blasts: &mut Vec<Blast>,
        mode: ExplosionMode,
    ) -> u32 {
        self.apply_move_until(pos, team, delta, blasts, mode, &mut |_| {}, |_| false)
    }

    fn apply_move_until<F>(
        &mut self,
        pos: BoardCode,
        team: TeamId,
        delta: i8,
        blasts: &mut Vec<Blast>,
        mode: ExplosionMode,
        on_step: &mut dyn FnMut(&Board),
        mut stop_after_wave: F,
    ) -> u32
    where
        F: FnMut(&Board) -> bool,
    {
        let cell = &mut self[pos];
        cell.value = cell.value.saturating_add(delta).max(0);
        cell.owner = Some(team);
        on_step(self);

        let mut waves = 0;
        while self.explode_wave(blasts, mode) {
            waves += 1;
            on_step(self);
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

    fn is_threatened(&self, pos: BoardCode, cell: CellData, team: TeamId) -> bool {
        cell.ability.targets().iter().any(|&(direction, _)| {
            self.neighbor(pos, direction).is_some_and(|n| {
                let other = self[n];
                other.owner.is_some_and(|o| o != team) && other.can_explode()
            })
        })
    }

    fn neighbor(&self, pos: BoardCode, direction: BoardCode) -> Option<BoardCode> {
        let (x, y) = self.xy_from_slot(usize::from(pos.slot()));
        let (dx, dy) = direction_offset(direction);
        let nx = i16::from(x) + i16::from(dx);
        let ny = i16::from(y) + i16::from(dy);
        if nx < 0
            || ny < 0
            || nx >= i16::from(self.size.width().get())
            || ny >= i16::from(self.size.height().get())
        {
            None
        } else {
            self.position(nx as u8, ny as u8)
        }
    }
}

struct Blast {
    pos: BoardCode,
    owner: Option<TeamId>,
    targets: &'static [(BoardCode, i8)],
    share: i8,
    remainder: i8,
}

impl Blast {
    fn new(board: &Board, pos: BoardCode, cell: CellData, mode: ExplosionMode) -> Self {
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
    pub stats: MemberStat,
}

impl CharacterData {
    pub fn new(controller: MemberId, team_id: TeamId) -> Self {
        Self {
            controller,
            team_id,
            stats: MemberStat::default(),
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

    pub fn targets(self) -> &'static [(BoardCode, i8)] {
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

/// Обычная клетка взрывается только в четыре боковых соседа. Центр не входит
/// в цели: иначе вырос бы порог взрыва и масса возвращалась бы во взорвавшуюся
/// клетку. Направления — коды из [`crate::api::circle`], пары `(код, урон)`:
/// `0` — обычная доля массы, отрицательное значение — урон способности.
const NORMAL_TARGETS: [(BoardCode, i8); 4] = [
    (BoardCode::from_slot(3), 0),
    (BoardCode::from_slot(7), 0),
    (BoardCode::from_slot(5), 0),
    (BoardCode::from_slot(1), 0),
];

const BOMB_CELL_COUNT: usize = disk_cell_count(2);
const BOMB_TARGETS: [(BoardCode, i8); BOMB_CELL_COUNT] =
    interpolate_by_distance(disk_cells::<BOMB_CELL_COUNT>(2), -3, -1);
