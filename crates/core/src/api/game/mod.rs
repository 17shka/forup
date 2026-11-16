pub mod bot;
pub mod cells;
pub mod circle;
pub mod code;
pub mod field;
pub mod ids;
pub mod rules;

pub use self::{
    bot::BotConfig,
    cells::{CellAbility, CellData},
    code::BoardCode,
    field::Board,
    ids::{CharacterData, CharacterId, MemberId, MemberStat, Score, TeamId},
    rules::{ExplosionMode, Game, GameError},
};

pub mod prelude {
    pub use super::{
        Board, BoardCode, BotConfig, CellAbility, CellData, CharacterData, CharacterId,
        ExplosionMode, Game, GameError, MemberId, MemberStat, Score, TeamId,
    };
}
