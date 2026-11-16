use std::num::NonZeroU8;

use serde::{Deserialize, Serialize};

use crate::api::auth::PlayerId;

/// Ненулевые идентификаторы: `Option<CharacterId>` и `Option<TeamId>` занимают 1 байт.
pub type CharacterId = NonZeroU8;
pub type TeamId = NonZeroU8;
pub type Score = i32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct IdSet([u64; 4]);

impl IdSet {
    pub(super) fn insert(&mut self, id: u8) {
        self.0[usize::from(id >> 6)] |= 1u64 << (id & 63);
    }

    pub(super) fn contains(&self, id: u8) -> bool {
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

    pub(super) fn len(&self) -> usize {
        self.0.iter().map(|word| word.count_ones() as usize).sum()
    }

    pub(super) fn iter(self) -> impl Iterator<Item = CharacterId> {
        (1..=u8::MAX).filter_map(move |id| CharacterId::new(id).filter(|_| self.contains(id)))
    }
}

pub(super) fn first_available_id(
    ids: impl IntoIterator<Item = CharacterId>,
) -> Option<CharacterId> {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MemberId {
    Player(PlayerId),
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
