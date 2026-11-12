//! Геометрия взрывов на клеточном поле: диски клеток, затухание значения от центра
//! и расчёт максимального радиуса взрыва.
//!
//! Везде `radius_sq` — квадрат радиуса (радиус = `sqrt(radius_sq)`),
//! поэтому расстояния сравниваются в целых числах без извлечения корня.
//!
//! Клетки в диске адресуются компактным кодом направления: `0` — центр,
//! `1..=8` — соседи по часовой стрелке начиная с верхнего:
//!
//! ```text
//! 8 1 2
//! 7 0 3
//! 6 5 4
//! ```

use crate::api::game::board::BoardSize;

pub mod prelude {
    pub use super::{
        MAX_CACHED_PLAYERS, direction_offset, disk_cell_count, disk_cells, interpolate_by_distance,
        max_radius_sq_for_players,
    };
}

/// Наибольший квадрат радиуса, который ещё помещается в коды направлений:
/// полный диск `3 × 3` вокруг центра.
pub const MAX_DIRECTION_RADIUS_SQ: u8 = 2;

/// Смещение `(dx, dy)` для кода направления.
pub const fn direction_offset(direction: u8) -> (i8, i8) {
    match direction {
        1 => (0, -1),
        2 => (1, -1),
        3 => (1, 0),
        4 => (1, 1),
        5 => (0, 1),
        6 => (-1, 1),
        7 => (-1, 0),
        8 => (-1, -1),
        _ => (0, 0),
    }
}

/// Код направления для смещения внутри диска `3 × 3`.
/// Для смещений за пределами диска возвращает `0`.
const fn direction_from_offset(dx: i8, dy: i8) -> u8 {
    match (dx, dy) {
        (0, -1) => 1,
        (1, -1) => 2,
        (1, 0) => 3,
        (1, 1) => 4,
        (0, 1) => 5,
        (-1, 1) => 6,
        (-1, 0) => 7,
        (-1, -1) => 8,
        _ => 0,
    }
}

/// Целая часть квадратного корня из `n`, то есть `floor(sqrt(n))`.
pub const fn floor_sqrt(n: u8) -> u8 {
    let n = n as u16;
    let mut root: u16 = 0;
    while (root + 1) * (root + 1) <= n {
        root += 1;
    }
    root as u8
}

/// Квадратный корень из `f32`, пригодный для `const` контекста.
///
/// `f32::sqrt` не является `const fn`, поэтому здесь используется метод Ньютона
/// с фиксированным числом итераций (64). Для `x <= 0` возвращает 0.
pub const fn const_sqrt_f32(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut guess = x;
    let mut iteration = 0;
    while iteration < 64 {
        guess = 0.5 * (guess + x / guess);
        iteration += 1;
    }
    guess
}

/// Квадрат расстояния от клетки до центра: `x² + y²`.
const fn dist_sq(direction: u8) -> u8 {
    let (x, y) = direction_offset(direction);
    (x * x + y * y) as u8
}

/// Количество целых точек `(x, y)`, для которых `x² + y² <= radius_sq`,
/// то есть размер диска клеток с центром в начале координат.
pub const fn disk_cell_count(radius_sq: u8) -> usize {
    let radius_sq = radius_sq as i16;
    let max_coord = floor_sqrt(radius_sq as u8) as i16;
    let mut count = 0;
    let mut y = -max_coord;
    while y <= max_coord {
        let mut x = -max_coord;
        while x <= max_coord {
            if x * x + y * y <= radius_sq {
                count += 1;
            }
            x += 1;
        }
        y += 1;
    }
    count
}

/// Все точки диска из [`disk_cell_count`], закодированные как направления.
/// Порядок: строки по `y` от меньшего к большему, внутри строки по `x` от меньшего к большему.
///
/// Коды направлений описывают только диск `3 × 3`, поэтому `radius_sq`
/// не должен превышать [`MAX_DIRECTION_RADIUS_SQ`]. Нарушение приводит к ошибке
/// на этапе вычисления константы.
///
/// `N` должно равняться `disk_cell_count(radius_sq)`. Если `N` меньше,
/// возникнет ошибка выхода за границы массива при вычислении константы.
/// Если `N` больше, хвост массива останется нулевым.
pub const fn disk_cells<const N: usize>(radius_sq: u8) -> [u8; N] {
    assert!(
        radius_sq <= MAX_DIRECTION_RADIUS_SQ,
        "direction codes only describe the 3x3 neighbourhood"
    );
    let max_coord = floor_sqrt(radius_sq) as i8;
    let radius_sq = radius_sq as i8;
    let mut cells = [0; N];
    let mut index = 0;
    let mut y = -max_coord;
    while y <= max_coord {
        let mut x = -max_coord;
        while x <= max_coord {
            if x * x + y * y <= radius_sq {
                cells[index] = direction_from_offset(x, y);
                index += 1;
            }
            x += 1;
        }
        y += 1;
    }
    cells
}

/// Присваивает каждой клетке значение, линейно зависящее от её расстояния до центра.
///
/// Центр получает `min_value`, самая удалённая клетка получает `max_value`.
/// Промежуточные значения округляются до ближайшего целого, при равной дробной части
/// округление идёт от нуля. Если все клетки совпадают с центром, все получают `min_value`.
pub const fn interpolate_by_distance<const N: usize>(
    points: [u8; N],
    min_value: i8,
    max_value: i8,
) -> [(u8, i8); N] {
    let mut max_dist_sq = 0;
    let mut i = 0;
    while i < N {
        let d = dist_sq(points[i]);
        if d > max_dist_sq {
            max_dist_sq = d;
        }
        i += 1;
    }
    let max_dist = const_sqrt_f32(max_dist_sq as f32);

    let min_f = min_value as f32;
    let max_f = max_value as f32;

    let mut result = [(0, 0); N];
    let mut i = 0;
    while i < N {
        let point = points[i];
        let dist = const_sqrt_f32(dist_sq(point) as f32);
        let ratio = if max_dist == 0.0 {
            0.0
        } else {
            dist / max_dist
        };
        let raw_value = min_f + (max_f - min_f) * ratio;
        let rounded = if raw_value >= 0.0 {
            (raw_value + 0.5) as i8
        } else {
            (raw_value - 0.5) as i8
        };
        result[i] = (point, rounded);
        i += 1;
    }
    result
}

/// Наибольшее число клеток поля `width × height`, которое может накрыть один взрыв
/// с квадратом радиуса `radius_sq`.
///
/// Пересечение строки диска с доской максимально, когда центр ближе всего к середине,
/// и каждая строка диска в отдельности максимальна при центральной клетке, поэтому
/// достаточно посчитать взрыв в центральной клетке, а не перебирать все клетки поля.
/// Результат — `u16`, так как доска `16 × 16` содержит 256 клеток.
const fn max_blast_cell_count(width: u8, height: u8, radius_sq: u8) -> u16 {
    let (w, h) = (width as i16, height as i16);
    let (cx, cy) = (w / 2, h / 2);
    let radius_sq = radius_sq as i16;
    let max_offset = floor_sqrt(radius_sq as u8) as i16;
    let mut count: u16 = 0;
    let mut dy = -max_offset;
    while dy <= max_offset {
        let y = cy + dy;
        if y >= 0 && y < h {
            let half = floor_sqrt((radius_sq - dy * dy) as u8) as i16;
            let left = if cx - half < 0 { 0 } else { cx - half };
            let right = if cx + half >= w { w - 1 } else { cx + half };
            count += (right - left + 1) as u16;
        }
        dy += 1;
    }
    count
}

/// Наименьшее число взрывов с квадратом радиуса `radius_sq`, необходимое для
/// покрытия поля `width × height`: `ceil(total_cells / max_blast_cell_count)`.
const fn min_blasts(width: u8, height: u8, radius_sq: u8) -> u16 {
    let total_cells = width as u16 * height as u16;
    total_cells.div_ceil(max_blast_cell_count(width, height, radius_sq))
}

/// Максимальное число игроков, для которого результат
/// [`max_radius_sq_for_players`] берётся из таблицы [`MAX_RADIUS_SQ_TABLE`].
pub const MAX_CACHED_PLAYERS: u8 = 16;

/// Предвычисленная таблица `[ordinal размера доски][players] -> max_radius_sq`.
/// Индекс по игрокам — само число игроков (`0..=MAX_CACHED_PLAYERS`).
const MAX_RADIUS_SQ_TABLE: [[u8; MAX_CACHED_PLAYERS as usize + 1]; BoardSize::SIZE_COUNT as usize] =
    build_max_radius_sq_table();

const fn build_max_radius_sq_table()
-> [[u8; MAX_CACHED_PLAYERS as usize + 1]; BoardSize::SIZE_COUNT as usize] {
    let mut table = [[0; MAX_CACHED_PLAYERS as usize + 1]; BoardSize::SIZE_COUNT as usize];
    let mut ordinal = 0;
    while ordinal < BoardSize::SIZE_COUNT {
        let (w, h) = match BoardSize::from_ordinal(ordinal) {
            Some(size) => size.dimensions(),
            None => unreachable!(),
        };
        let row = &mut table[ordinal as usize];
        // `min_blasts` не возрастает с ростом радиуса, поэтому один проход по радиусам
        // заполняет ответы сразу для всех количеств игроков.
        let mut radius_sq: u8 = 0;
        loop {
            let blasts = min_blasts(w, h, radius_sq);
            let mut players = 0;
            while players <= MAX_CACHED_PLAYERS {
                if blasts >= players as u16 {
                    row[players as usize] = radius_sq;
                }
                players += 1;
            }
            // Один взрыв накрывает всё поле: дальше значения уже не изменятся
            // ни для кого, кроме `players <= 1`, которым подходит любой радиус.
            if blasts <= 1 || radius_sq == u8::MAX {
                break;
            }
            radius_sq += 1;
        }
        // Для 0 и 1 игрока подходит любой радиус.
        row[0] = u8::MAX;
        row[1] = u8::MAX;
        ordinal += 1;
    }
    table
}

/// Вычисление без таблицы, для количества игроков больше [`MAX_CACHED_PLAYERS`].
const fn compute_max_radius_sq(width: u8, height: u8, players: u8) -> u8 {
    let mut best_radius_sq = 0;
    let mut radius_sq: u8 = 0;
    loop {
        if min_blasts(width, height, radius_sq) < players as u16 {
            break;
        }
        best_radius_sq = radius_sq;
        if radius_sq == u8::MAX {
            break;
        }
        radius_sq += 1;
    }
    best_radius_sq
}

/// Наибольший квадрат радиуса взрыва (радиус = `sqrt(result)`), при котором поле
/// `size` всё ещё требует не менее `players` взрывов для покрытия.
///
/// Оценка снизу: один взрыв накрывает не более `max_blast_cell_count` клеток, поэтому
/// для покрытия всех клеток нужно не меньше `ceil(total_cells / max_cells)` взрывов.
///
/// Для `players <= MAX_CACHED_PLAYERS` значение берётся из предвычисленной таблицы.
/// Возвращает 0, если `size` не является допустимым размером доски или игроков
/// больше, чем клеток. Для 0 и 1 игрока подходит любой радиус — возвращается `u8::MAX`.
pub const fn max_radius_sq_for_players(size: BoardSize, players: u8) -> u8 {
    let Some(ordinal) = size.ordinal() else {
        return 0;
    };
    if players <= MAX_CACHED_PLAYERS {
        return MAX_RADIUS_SQ_TABLE[ordinal as usize][players as usize];
    }
    let (w, h) = size.dimensions();
    if players as u16 > w as u16 * h as u16 {
        return 0;
    }
    compute_max_radius_sq(w, h, players)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Диск с квадратом радиуса 2: центр, 4 соседние по прямой и 4 диагональные клетки.
    const BOMB_CELL_COUNT: usize = disk_cell_count(2);

    /// Затухание для диска радиуса² = 2: от -3 в центре до -1 на диагонали.
    const BOMB_FALLOFF: [(u8, i8); BOMB_CELL_COUNT] =
        interpolate_by_distance(disk_cells::<BOMB_CELL_COUNT>(2), -3, -1);

    const _: () = assert!(BOMB_CELL_COUNT == 9);

    #[test]
    fn falloff_center_has_min_value() {
        assert_eq!(BOMB_FALLOFF[4], (0, -3));
    }

    #[test]
    fn falloff_corner_has_max_value() {
        assert_eq!(BOMB_FALLOFF[0], (8, -1));
    }

    #[test]
    fn max_radius_sq_for_4_players_on_10x10() {
        // Радиус² 9: максимум 29 клеток, ceil(100 / 29) = 4 взрыва.
        // Радиус² 10: 37 клеток, хватит 3 взрывов.
        let size = BoardSize::new(10, 10).unwrap();
        assert_eq!(max_radius_sq_for_players(size, 4), 9);
    }

    /// Полный перебор центров взрыва — эталон для [`max_blast_cell_count`].
    fn brute_force_max_blast(w: u8, h: u8, radius_sq: u8) -> u16 {
        let (w, h, r) = (i32::from(w), i32::from(h), i32::from(radius_sq));
        let mut best = 0;
        for cy in 0..h {
            for cx in 0..w {
                let mut count = 0;
                for y in 0..h {
                    for x in 0..w {
                        let (dx, dy) = (x - cx, y - cy);
                        if dx * dx + dy * dy <= r {
                            count += 1;
                        }
                    }
                }
                best = best.max(count);
            }
        }
        best
    }

    #[test]
    fn central_blast_is_the_largest() {
        for (w, h) in [(3, 3), (3, 4), (5, 8), (8, 8), (7, 16), (16, 16)] {
            for radius_sq in 0..=60 {
                assert_eq!(
                    max_blast_cell_count(w, h, radius_sq),
                    brute_force_max_blast(w, h, radius_sq),
                    "{w}x{h}, radius_sq {radius_sq}"
                );
            }
        }
    }

    #[test]
    fn table_matches_direct_computation() {
        for ordinal in 0..BoardSize::SIZE_COUNT {
            let size = BoardSize::from_ordinal(ordinal).unwrap();
            let (w, h) = size.dimensions();
            for players in 2..=MAX_CACHED_PLAYERS {
                assert_eq!(
                    max_radius_sq_for_players(size, players),
                    compute_max_radius_sq(w, h, players),
                    "{w}x{h}, {players} players"
                );
            }
        }
    }
}
