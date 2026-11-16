//! HTTP-обработчики аккаунтов: регистрация, вход, выход, профиль.

use axum::{
    Router,
    body::Bytes,
    extract::Extension,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use forup_core::api::auth::{
    AuthError, AuthResponse, LoginRequest, RegisterRequest, UpdateProfileRequest, UserResponse,
};

use crate::http::{api_error, decode, encode};
use crate::state::{ApiState, authenticated, bearer_token};

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
