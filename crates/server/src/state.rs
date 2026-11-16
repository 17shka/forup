//! Общее состояние API: хранилище аккаунтов, сессии, директория комнат
//! и её постоянное хранилище. Здесь же — авторизация по токену из
//! заголовка `Authorization`.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    response::Response,
};
use forup_core::api::{
    auth::{PlayerId, UserStore, generate_token},
    network::{RoomDirectory, RoomStore},
};

use crate::http::api_error;

/// Shared state for the account endpoints: the persistent user store and the
/// in-memory sessions issued on login.
#[derive(Clone)]
pub struct ApiState {
    pub(crate) users: UserStore,
    sessions: Arc<Mutex<HashMap<String, PlayerId>>>,
    pub(crate) rooms: Arc<Mutex<RoomDirectory>>,
    pub(crate) room_store: RoomStore,
}

impl ApiState {
    pub fn new(users: UserStore, rooms: Arc<Mutex<RoomDirectory>>, room_store: RoomStore) -> Self {
        Self {
            users,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            rooms,
            room_store,
        }
    }

    pub(crate) fn issue_token(&self, user_id: PlayerId) -> String {
        let token = generate_token();
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sessions.insert(token.clone(), user_id);
        token
    }

    pub(crate) fn user_for_token(&self, token: &str) -> Option<PlayerId> {
        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sessions.get(token).copied()
    }

    pub(crate) fn revoke_token(&self, token: &str) {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sessions.remove(token);
    }
}

pub(crate) fn authenticated(state: &ApiState, headers: &HeaderMap) -> Result<PlayerId, Response> {
    let token = bearer_token(headers)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "authentication required"))?;
    state
        .user_for_token(&token)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "session expired"))
}

pub(crate) fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_owned)
}
