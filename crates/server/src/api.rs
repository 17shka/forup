use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::{
    Router,
    body::Bytes,
    extract::Extension,
    http::{HeaderMap, StatusCode, header::AUTHORIZATION, header::CONTENT_TYPE},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use forup_core::api::auth::{
    ApiError, AuthError, AuthResponse, LoginRequest, RegisterRequest, UpdateProfileRequest, UserId,
    UserResponse, UserStore, generate_token,
};
use serde::{Serialize, de::DeserializeOwned};

const BINARY: &str = "application/octet-stream";

/// Shared state for the account endpoints: the persistent user store and the
/// in-memory sessions issued on login.
#[derive(Clone)]
pub struct ApiState {
    users: UserStore,
    sessions: Arc<Mutex<HashMap<String, u32>>>,
}

impl ApiState {
    pub fn new(users: UserStore) -> Self {
        Self {
            users,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn issue_token(&self, user_id: u32) -> String {
        let token = generate_token();
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sessions.insert(token.clone(), user_id);
        token
    }

    fn user_for_token(&self, token: &str) -> Option<UserId> {
        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sessions.get(token).copied().and_then(UserId::new)
    }

    fn revoke_token(&self, token: &str) {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sessions.remove(token);
    }
}

/// Account endpoints merged into the main app router.
pub fn router<S>(state: ApiState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/me", get(me))
        .route("/api/auth/profile", patch(update_profile))
        .layer(Extension(state))
}

async fn register(Extension(state): Extension<ApiState>, body: Bytes) -> Response {
    let request: RegisterRequest = match decode(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    match state
        .users
        .register(&request.name, &request.email, &request.password)
    {
        Ok(user) => {
            let token = state.issue_token(user.id);
            encode(StatusCode::CREATED, &AuthResponse { token, user })
        }
        Err(error) => auth_error_response(error),
    }
}

async fn login(Extension(state): Extension<ApiState>, body: Bytes) -> Response {
    let request: LoginRequest = match decode(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };
    match state.users.authenticate(&request.email, &request.password) {
        Ok(user) => {
            let token = state.issue_token(user.id);
            encode(StatusCode::OK, &AuthResponse { token, user })
        }
        Err(error) => auth_error_response(error),
    }
}

async fn logout(Extension(state): Extension<ApiState>, headers: HeaderMap) -> Response {
    if let Some(token) = bearer_token(&headers) {
        state.revoke_token(&token);
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn me(Extension(state): Extension<ApiState>, headers: HeaderMap) -> Response {
    let id = match authenticated(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match state.users.profile(id) {
        Ok(Some(user)) => encode(StatusCode::OK, &UserResponse { user }),
        Ok(None) => api_error(StatusCode::UNAUTHORIZED, "account not found"),
        Err(error) => auth_error_response(error),
    }
}

async fn update_profile(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let id = match authenticated(&state, &headers) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let request: UpdateProfileRequest = match decode(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };

    if let Some(name) = &request.name {
        if let Err(error) = state.users.rename(id, name) {
            return auth_error_response(error);
        }
    }
    if let Some(email) = &request.email {
        if let Err(error) = state.users.update_email(id, email) {
            return auth_error_response(error);
        }
    }
    if let Some(password) = &request.password {
        let Some(current_password) = &request.current_password else {
            return api_error(
                StatusCode::BAD_REQUEST,
                "current password is required to set a new one",
            );
        };
        if let Err(error) = state.users.change_password(id, current_password, password) {
            return auth_error_response(error);
        }
    }

    match state.users.profile(id) {
        Ok(Some(user)) => encode(StatusCode::OK, &UserResponse { user }),
        Ok(None) => api_error(StatusCode::UNAUTHORIZED, "account not found"),
        Err(error) => auth_error_response(error),
    }
}

fn authenticated(state: &ApiState, headers: &HeaderMap) -> Result<UserId, Response> {
    let token = bearer_token(headers)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "authentication required"))?;
    state
        .user_for_token(&token)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "session expired"))
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_owned)
}

fn decode<T: DeserializeOwned>(body: &Bytes) -> Result<T, Response> {
    postcard::from_bytes(body.as_ref()).map_err(|error| {
        api_error(
            StatusCode::BAD_REQUEST,
            format!("invalid request body: {error}"),
        )
    })
}

fn encode<T: Serialize>(status: StatusCode, value: &T) -> Response {
    match postcard::to_allocvec(value) {
        Ok(body) => (status, [(CONTENT_TYPE, BINARY)], body).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("response encoding error: {error}"),
        )
            .into_response(),
    }
}

fn api_error(status: StatusCode, message: impl Into<String>) -> Response {
    encode(
        status,
        &ApiError {
            error: message.into(),
        },
    )
}

fn auth_error_response(error: AuthError) -> Response {
    let status = match &error {
        AuthError::InvalidName | AuthError::InvalidEmail | AuthError::WeakPassword => {
            StatusCode::BAD_REQUEST
        }
        AuthError::EmailTaken => StatusCode::CONFLICT,
        AuthError::InvalidCredentials | AuthError::UserNotFound => StatusCode::UNAUTHORIZED,
        AuthError::Database(_) | AuthError::Serialization(_) | AuthError::IdSpaceExhausted => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    };
    if status == StatusCode::INTERNAL_SERVER_ERROR {
        eprintln!("[Server] auth storage error: {error}");
    }
    api_error(status, error.to_string())
}
