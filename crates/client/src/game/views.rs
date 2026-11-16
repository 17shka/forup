use forup_core::api::game::prelude::{CharacterId, TeamId};

use super::local::{Controller, ParticipantId};

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
    /// Сетевое имя участника; локально — `None`.
    pub name: Option<String>,
    /// Готовность к старту сетевой партии; локально — `None`.
    pub ready: Option<bool>,
    pub wins: u16,
    pub losses: u16,
    pub moves: u16,
    /// Все персонажи участника в этой команде выбыли.
    pub lost: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeamView {
    pub team: TeamId,
    pub hue: u16,
    pub cells: u16,
    pub mass: i32,
    pub participants: Vec<TeamParticipantView>,
    /// Все участники команды проиграли.
    pub lost: bool,
}
