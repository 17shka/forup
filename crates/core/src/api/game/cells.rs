use serde::{Deserialize, Serialize};

use crate::api::game::circle::prelude::*;

use super::code::BoardCode;
use super::ids::TeamId;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellData {
    pub value: i8,
    pub ability: CellAbility,
    pub owner: Option<TeamId>,
    /// Клетка задета текущей цепочкой взрывов: она проверяется в следующем
    /// проходе [`Board::explode_activated`]. Сбрасывается, когда цепочка
    /// завершается. Переживает сохранение, так что цепочку можно продолжить.
    #[serde(default)]
    pub activated: bool,
}

impl CellData {
    pub fn can_explode(&self) -> bool {
        match self.ability {
            CellAbility::None => self.value >= self.ability.max_value(),
            CellAbility::Bomb => true,
        }
    }

    pub(super) fn apply_ability_damage(&mut self, dmg: i8) {
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
/// клетку. Направления — коды из [`crate::api::game::circle`], пары `(код, урон)`:
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
