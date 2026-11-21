//! Типы протокола комнатного HTTP API: тела запросов и ответов в postcard.
//! Клиент и сервер делят эти DTO, поэтому маппинг полей одинаков на обеих
//! сторонах.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::api::{
    auth::PlayerId,
    game::{BoardCode, CharacterId, ExplosionMode, MemberId, TeamId},
    network::RoomId,
};

use super::room::Room;

/// Комната в списке открытых комнат главного меню.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomSummary {
    pub id: RoomId,
    pub members: u8,
    pub bots: u8,
    pub teams: u8,
    pub started: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoomsResponse {
    pub rooms: Vec<RoomSummary>,
}

/// Один шаг игрового действия сервера. Клиент применяет шаги к своей
/// копии движка, чтобы чужие ходы анимировались волна за волной, как
/// локальные. Порядок и содержимое повторяют вызовы движка на сервере.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplayStep {
    /// Старт партии (`Game::start`).
    Start { automatic: bool },
    /// Ход клетки: изменение плюс вся цепочка взрывов (`Game::increase` или
    /// `Game::decrease`).
    Move {
        member: CharacterId,
        pos: BoardCode,
        increase: bool,
    },
    /// Пропуск хода (`Game::skip`).
    Skip { member: CharacterId },
    /// Сдача (`Game::surrender`).
    Surrender { member: CharacterId },
}

/// Место лобби при переносе партии в онлайн: бот или локальный
/// игрок на своей команде.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbySeat {
    pub bot: bool,
    pub team: u8,
}

/// Снимок лобби, по которому сервер восстанавливает комнату игрока при
/// переключении Оффлайн/Онлайн: доска, правила и места участников.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbySetup {
    pub width: u8,
    pub height: u8,
    pub automatic: bool,
    pub explosion_mode: ExplosionMode,
    pub radius_sq: u8,
    /// Команда собственного места создателя; `None` — он сидит без места
    /// и остаётся оператором комнаты.
    pub my_team: Option<u8>,
    pub seats: Vec<LobbySeat>,
}

/// Тело `POST /api/rooms`: создать комнату, воссоздав лобби игрока,
/// чтобы переключение Оффлайн/Онлайн ничего не перестраивало.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateRoomRequest {
    pub open: bool,
    pub setup: LobbySetup,
}

/// Тело `POST /{id}/open`: открыть или закрыть комнату.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct OpenRequest {
    pub open: bool,
}

/// Тело `POST /{id}/ready`: готовность участника.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ReadyRequest {
    pub ready: bool,
}

/// Тело `POST /{id}/team`: переместить участника в команду.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TeamRequest {
    pub member: CharacterId,
    pub team: TeamId,
}

/// Тело запросов, адресованных одному участнику (kick/skip/surrender).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct MemberRequest {
    pub member: CharacterId,
}

/// Тело `POST /{id}/order`: поменять местами двух участников
/// в последовательности ходов.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TurnOrderRequest {
    pub a: CharacterId,
    pub b: CharacterId,
}

/// Тело `POST /{id}/move`: ход игрока.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct MoveRequest {
    pub member: CharacterId,
    pub pos: BoardCode,
    pub increase: bool,
}

/// Тело `POST /{id}/seat`: назначить контроллера места.
/// `MemberId::Player` ссылается на участника комнаты.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SeatRequest {
    pub member: CharacterId,
    pub controller: MemberId,
}

/// Тело `POST /{id}/board`: размер доски до старта партии.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct BoardSizeRequest {
    pub width: u8,
    pub height: u8,
}

/// Тело `POST /{id}/distribution`: автоматическая расстановка стартовых клеток.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct DistributionRequest {
    pub automatic: bool,
}

/// Тело `POST /{id}/explosions`: режим взрывов.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ExplosionRequest {
    pub mode: ExplosionMode,
}

/// Тело `POST /{id}/radius`: квадрат безопасного радиуса стартовых клеток.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct RadiusRequest {
    pub radius_sq: u8,
}

/// Полный снапшот комнаты: состояние, идентичность вызвавшего и отображаемые
/// имена участников (профили хранятся только на сервере).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoomStateResponse {
    pub id: RoomId,
    pub me: PlayerId,
    pub room: Room,
    pub names: HashMap<PlayerId, String>,
}
