use forup_core::api::{
    auth::PlayerId,
    game::prelude::GameError,
    network::{RoomId, RoomStateResponse},
};
use leptos::prelude::*;

use super::local::{LocalGame, MoveMode};
use crate::net::NetError;

/// Подключение к серверной комнате. Токен копируется сюда при входе,
/// чтобы порождённые сетевые задачи не зависели от реактивного контекста.
/// На native поля читаются только сетевым клиентом (wasm).
#[derive(Clone, Debug)]
#[cfg_attr(
    not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))),
    allow(dead_code)
)]
pub struct NetRoom {
    pub id: RoomId,
    pub me: PlayerId,
    pub token: String,
    pub is_admin: bool,
}

/// Reactive handle shared by the game page, the team panels and the lobby.
#[derive(Clone, Copy)]
pub struct GameSession {
    pub state: RwSignal<LocalGame>,
    pub error: RwSignal<Option<GameError>>,
    pub mode: RwSignal<MoveMode>,
    /// Visual quarter-turns of the board, clockwise.
    pub rotation: RwSignal<u8>,
    /// Текущая сетевая комната; `None` — партия идёт офлайн.
    pub room: RwSignal<Option<NetRoom>>,
    /// Последняя сетевая ошибка (показывается тостом на странице игры).
    pub net_error: RwSignal<Option<NetError>>,
    /// Сетевой запрос в полёте: блокирует дублирующиеся действия.
    pub net_busy: RwSignal<bool>,
    /// Поколение цикла опроса: меняется при смене комнаты, чтобы старые
    /// таймеры погасли и не плодили дублей цепочки.
    pub poll_generation: RwSignal<u64>,
    /// Снапшот, пришедший во время анимации взрывов: применяется, как
    /// только ход доиграется, чтобы волны не обрывались одиночным флешем.
    pub pending_snapshot: RwSignal<Option<Box<RoomStateResponse>>>,
}

impl GameSession {
    pub fn provide() -> Self {
        let game_session = Self {
            state: RwSignal::new(LocalGame::new()),
            error: RwSignal::new(None),
            mode: RwSignal::new(MoveMode::Increase),
            rotation: RwSignal::new(0),
            room: RwSignal::new(None),
            net_error: RwSignal::new(None),
            net_busy: RwSignal::new(false),
            poll_generation: RwSignal::new(0),
            pending_snapshot: RwSignal::new(None),
        };
        let session = game_session;
        provide_context(session);
        session
    }

    pub fn expect() -> Self {
        expect_context::<Self>()
    }

    /// Runs a local update, surfacing engine errors in the UI.
    pub fn dispatch(&self, apply: impl FnOnce(&mut LocalGame) -> Result<(), GameError>) {
        if let Some(outcome) = self.state.try_update(apply) {
            self.error.set(outcome.err());
        }
    }

    pub fn reset(&self) {
        self.state.update(LocalGame::reset);
        self.error.set(None);
        self.rotation.set(0);
    }

    /// Rotates the board view 90° clockwise without touching the game state.
    pub fn rotate_board(&self) {
        self.rotation
            .update(|turn| *turn = turn.wrapping_add(1) % 4);
    }
}
