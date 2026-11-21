use std::cmp::Reverse;
use std::ops::{Index, IndexMut};

use serde::{Deserialize, Serialize};

use crate::api::game::circle::prelude::*;

use super::bot::BotConfig;
use super::cells::{CellAbility, CellData};
use super::code::BoardCode;
use super::ids::{IdSet, Score, TeamId};
use super::rules::ExplosionMode;

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

    pub(super) fn distance_sq(&self, a: BoardCode, b: BoardCode) -> Score {
        let (ax, ay) = self.xy_from_slot(usize::from(a.slot()));
        let (bx, by) = self.xy_from_slot(usize::from(b.slot()));
        let dx = Score::from(ax) - Score::from(bx);
        let dy = Score::from(ay) - Score::from(by);
        dx * dx + dy * dy
    }

    pub fn positions(&self) -> impl Iterator<Item = BoardCode> + '_ {
        (0..self.cells.len()).map(|slot| BoardCode::from_slot(slot as u8))
    }

    /// Только занятые клетки. Проверки расстояний (стартовые клетки, взрывы)
    /// перебирают этот набор вместо всей доски: на большой доске пустых клеток
    /// большинство, и их обход на порядок дороже самого сравнения.
    pub fn occupied_positions(&self) -> impl Iterator<Item = BoardCode> + '_ {
        self.cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| cell.owner.is_some())
            .map(|(slot, _)| BoardCode::from_slot(slot as u8))
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

    pub fn is_saturated(&self, mode: ExplosionMode) -> bool {
        self.cells
            .iter()
            .enumerate()
            .all(|(slot, c)| self.can_explode_at(Self::pos_from_slot(slot), mode, *c))
    }

    pub fn cell_threshold(&self, pos: BoardCode, mode: ExplosionMode) -> i8 {
        let cell = self[pos];
        match cell.ability {
            CellAbility::Bomb => 0,
            CellAbility::None => match mode {
                ExplosionMode::BorderLoss(_) | ExplosionMode::BorderKeep(_) => {
                    cell.ability.max_value()
                }
                ExplosionMode::CriticalMass(_) => {
                    let (x, y) = self.xy_from_slot(usize::from(pos.slot()));
                    let w = self.size.width().get();
                    let h = self.size.height().get();
                    let is_corner = (x == 0 || x == w - 1) && (y == 0 || y == h - 1);
                    let is_edge = x == 0 || x == w - 1 || y == 0 || y == h - 1;
                    if is_corner {
                        2
                    } else if is_edge {
                        3
                    } else {
                        4
                    }
                }
            },
        }
    }

    pub fn can_explode_at(&self, pos: BoardCode, mode: ExplosionMode, cell: CellData) -> bool {
        let threshold = self.cell_threshold(pos, mode);
        cell.can_explode_with(threshold)
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

    /// Применяет изменение клетки: значение, владелец и отметка взрыва.
    pub(super) fn apply_delta(&mut self, pos: BoardCode, team: TeamId, delta: i8) {
        let cell = &mut self[pos];
        cell.value = cell.value.saturating_add(delta).max(0);
        cell.owner = Some(team);
        cell.activated = true;
    }

    /// Доигрывает ход целиком (оценка ботов): изменение плюс все волны
    /// взрывов. Возвращает количество волн.
    pub(super) fn apply_move(
        &mut self,
        pos: BoardCode,
        team: TeamId,
        delta: i8,
        mode: ExplosionMode,
    ) -> u32 {
        self.apply_delta(pos, team, delta);
        let mut waves = 0;
        while self.explode_activated(mode) {
            waves += 1;
        }
        waves
    }

    /// Один проход цепочки взрывов. Взрывает клетки с отметкой `activated`,
    /// способные взорваться, и отмечает все задетые клетки для следующего
    /// прохода. Когда активных взрывов не осталось, отметки сбрасываются и
    /// цепочка считается завершённой. Ранний выход (например, победа одной
    /// команды) оставляет отметки — так состояние цепочки переживает
    /// сохранение и продолжится на следующем ходу.
    pub(super) fn explode_activated(&mut self, mode: ExplosionMode) -> bool {
        let mut blasts = Vec::new();
        let reset_owner_on_zero = mode.reset_owner_on_zero();

        for (slot, &cell) in self.cells.iter().enumerate() {
            let pos = Self::pos_from_slot(slot);
            if cell.activated && self.can_explode_at(pos, mode, cell) {
                blasts.push(Blast::new(self, pos, cell, mode));
            }
        }

        if blasts.is_empty() {
            for cell in &mut self.cells {
                cell.activated = false;
            }
            return false;
        }

        for b in blasts.iter() {
            let cell = &mut self[b.pos];
            cell.value = b.remainder;
            cell.ability = CellAbility::None;
            if reset_owner_on_zero && b.remainder <= 0 {
                cell.owner = None;
            }
        }

        for b in blasts.iter() {
            for &(direction, dmg) in b.targets {
                let Some(t) = self.neighbor(b.pos, direction) else {
                    continue;
                };
                let target = &mut self[t];
                target.activated = true;
                if dmg == 0 {
                    target.value = target.value.saturating_add(b.share);
                    target.owner = b.owner;
                } else {
                    target.apply_ability_damage(dmg);
                }
            }
        }

        // После раздачи урона пересчитываем отметку взорванной клетки: она
        // могла снова зарядиться от соседей и взорвётся в следующем проходе.
        for b in blasts.iter() {
            let pos = b.pos;
            let cell = self[b.pos];
            self[pos].activated = self.can_explode_at(pos, mode, cell);
        }

        true
    }

    pub(super) fn evaluate(&self, team: TeamId, cfg: &BotConfig, mode: ExplosionMode) -> Score {
        let mut total = 0;

        for (slot, cell) in self.cells.iter().enumerate() {
            match cell.owner {
                Some(owner) if owner == team => {
                    let value = Score::from(cell.value);
                    let mut score = cfg.own_cell_bonus + value * cfg.own_value_weight;
                    if self.is_threatened(Self::pos_from_slot(slot), *cell, team, mode) {
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

    fn is_threatened(
        &self,
        pos: BoardCode,
        cell: CellData,
        team: TeamId,
        mode: ExplosionMode,
    ) -> bool {
        cell.ability.targets().iter().any(|&(direction, _)| {
            self.neighbor(pos, direction).is_some_and(|n| {
                let other = self[n];
                other.owner.is_some_and(|o| o != team) && self.can_explode_at(n, mode, other)
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

        let present = targets
            .iter()
            .filter(|&&(direction, damage)| damage == 0 && board.neighbor(pos, direction).is_some())
            .count() as i8;

        let divisor = match mode {
            ExplosionMode::BorderLoss(_) | ExplosionMode::BorderKeep(_) => output_slots,
            ExplosionMode::CriticalMass(_) => present.max(1),
        };

        let (share, remainder) = if divisor > 0 {
            let share = cell.value / divisor;
            let remainder = cell.value % divisor;
            let remainder = if matches!(mode, ExplosionMode::BorderKeep(_)) {
                // BorderKeep explosions keep the shares that cannot be delivered
                // to a neighbour (for example, at the border) in the cell.
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
