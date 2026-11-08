//! Геометрия взрывов на клеточном поле: диски клеток, затухание значения от центра
//! и расчёт максимального радиуса взрыва.
//!
//! Везде `radius_sq` — квадрат радиуса (радиус = `sqrt(radius_sq)`),
//! поэтому расстояния сравниваются в целых числах без извлечения корня.

use glam::I8Vec2;

pub mod prelude {
    pub use super::{
        disk_cell_count, disk_cells, interpolate_by_distance, max_radius_sq_for_players,
    };
}

/// Целая часть квадратного корня из `n`, то есть `floor(sqrt(n))`.
/// Для `n < 1` возвращает 0.
pub const fn floor_sqrt(n: i32) -> i32 {
    let mut root = 0;
    while (root + 1) * (root + 1) <= n {
        root += 1;
    }
    root
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

/// Квадрат расстояния от точки до начала координат: `x² + y²`.
const fn dist_sq(point: I8Vec2) -> i32 {
    let x = point.x as i32;
    let y = point.y as i32;
    x * x + y * y
}

/// Количество целых точек `(x, y)`, для которых `x² + y² <= radius_sq`,
/// то есть размер диска клеток с центром в начале координат.
pub const fn disk_cell_count(radius_sq: u8) -> usize {
    let radius_sq = radius_sq as i32;
    let max_coord = floor_sqrt(radius_sq);
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

/// Все точки диска из [`disk_cell_count`]. Порядок: строки по `y` от меньшего к большему,
/// внутри строки по `x` от меньшего к большему.
///
/// `N` должно равняться `disk_cell_count(radius_sq)`. Если `N` меньше,
/// возникнет ошибка выхода за границы массива при вычислении константы.
/// Если `N` больше, хвост массива останется нулевым.
pub const fn disk_cells<const N: usize>(radius_sq: u8) -> [I8Vec2; N] {
    let radius_sq = radius_sq as i32;
    let max_coord = floor_sqrt(radius_sq);
    let mut cells = [I8Vec2::ZERO; N];
    let mut index = 0;
    let mut y = -max_coord;
    while y <= max_coord {
        let mut x = -max_coord;
        while x <= max_coord {
            if x * x + y * y <= radius_sq {
                cells[index] = I8Vec2::new(x as i8, y as i8);
                index += 1;
            }
            x += 1;
        }
        y += 1;
    }
    cells
}

/// Присваивает каждой точке значение, линейно зависящее от её расстояния до начала координат.
///
/// Точка в начале координат получает `min_value`, самая удалённая точка получает `max_value`.
/// Промежуточные значения округляются до ближайшего целого, при равной дробной части
/// округление идёт от нуля. Если все точки совпадают с началом, все получают `min_value`.
pub const fn interpolate_by_distance<const N: usize>(
    points: [I8Vec2; N],
    min_value: i8,
    max_value: i8,
) -> [(I8Vec2, i8); N] {
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

    let mut result = [(I8Vec2::ZERO, 0); N];
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

/// Количество клеток поля `WIDTH × HEIGHT`, которые попадают во взрыв с квадратом радиуса
/// `radius_sq` и центром в клетке `(center_x, center_y)`. Клетки за границами поля не учитываются.
const fn blast_cell_count<const WIDTH: usize, const HEIGHT: usize>(
    center_x: i32,
    center_y: i32,
    radius_sq: i32,
) -> usize {
    let max_offset = floor_sqrt(radius_sq);
    let mut count = 0;
    let mut dy = -max_offset;
    while dy <= max_offset {
        let mut dx = -max_offset;
        while dx <= max_offset {
            let x = center_x + dx;
            let y = center_y + dy;
            let inside_field = x >= 0 && y >= 0 && x < WIDTH as i32 && y < HEIGHT as i32;
            if dx * dx + dy * dy <= radius_sq && inside_field {
                count += 1;
            }
            dx += 1;
        }
        dy += 1;
    }
    count
}

/// Наибольшее число клеток, которое может накрыть один взрыв с квадратом радиуса `radius_sq`,
/// если перебрать все клетки поля как центр.
const fn max_blast_cell_count<const WIDTH: usize, const HEIGHT: usize>(radius_sq: i32) -> usize {
    let mut max_count = 0;
    let mut y = 0;
    while y < HEIGHT {
        let mut x = 0;
        while x < WIDTH {
            let count = blast_cell_count::<WIDTH, HEIGHT>(x as i32, y as i32, radius_sq);
            if count > max_count {
                max_count = count;
            }
            x += 1;
        }
        y += 1;
    }
    max_count
}

/// Наибольший квадрат радиуса взрыва (радиус = `sqrt(result)`), при котором поле
/// `WIDTH × HEIGHT` всё ещё требует не менее `players` взрывов для покрытия.
///
/// Оценка снизу: один взрыв накрывает не более `max_blast_cell_count` клеток, поэтому
/// для покрытия всех `total_cells` клеток нужно не меньше `ceil(total_cells / max_cells)` взрывов.
/// Возвращает 0, если поле пустое или игроков больше, чем клеток.
pub const fn max_radius_sq_for_players<const WIDTH: usize, const HEIGHT: usize>(
    players: usize,
) -> u8 {
    let total_cells = WIDTH * HEIGHT;
    if total_cells == 0 || players > total_cells {
        return 0;
    }
    let mut best_radius_sq = 0;
    let mut radius_sq: u8 = 0;
    loop {
        let max_cells = max_blast_cell_count::<WIDTH, HEIGHT>(radius_sq as i32);
        let min_blasts = (total_cells + max_cells - 1) / max_cells;
        if min_blasts < players {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Диск с квадратом радиуса 2: центр, 4 соседние по прямой и 4 диагональные клетки.
    const BOMB_CELL_COUNT: usize = disk_cell_count(2);

    /// Затухание для диска радиуса² = 2: от -3 в центре до -1 на диагонали.
    const BOMB_FALLOFF: [(I8Vec2, i8); BOMB_CELL_COUNT] =
        interpolate_by_distance(disk_cells::<BOMB_CELL_COUNT>(2), -3, -1);

    const _: () = assert!(BOMB_CELL_COUNT == 9);

    #[test]
    fn falloff_center_has_min_value() {
        assert_eq!(BOMB_FALLOFF[4], (I8Vec2::ZERO, -3));
    }

    #[test]
    fn falloff_corner_has_max_value() {
        assert_eq!(BOMB_FALLOFF[0], (I8Vec2::new(-1, -1), -1));
    }

    #[test]
    fn max_radius_sq_for_4_players_on_10x10() {
        // Радиус² 9: максимум 29 клеток, ceil(100 / 29) = 4 взрыва.
        // Радиус² 10: 37 клеток, хватит 3 взрывов.
        assert_eq!(max_radius_sq_for_players::<10, 10>(4), 9);
    }
}
