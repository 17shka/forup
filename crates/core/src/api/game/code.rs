use std::num::NonZeroU8;

use serde::{Deserialize, Serialize};

/// Однобайтовый код для всего, что относится к клеточному полю. Одно и то же
/// значение используется в трёх ролях:
///
/// * **размер доски** — код строится через [`BoardCode::from_size`]; стороны
///   нормализуются так, что `w <= h`, поэтому `4 × 8` и `8 × 4` — один размер;
/// * **клетка** — код строится через [`BoardCode::from_slot`] и равен слоту
///   клетки, нумерация с нуля;
/// * **направление** — слот в окрестности `3 × 3` вокруг клетки (см.
///   [`crate::api::game::circle`]).
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
