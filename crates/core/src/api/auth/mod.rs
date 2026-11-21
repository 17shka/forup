//! Учётные записи: общие для клиента и сервера правила валидации полей и
//! DTO протокола. Постоянное хранилище аккаунтов — серверная часть,
//! живёт в `store`.
use std::num::NonZeroU32;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Идентификатор игрока. Выдаётся один раз при регистрации аккаунта
/// ([`crate::api::auth::UserStore::register`]) и больше не меняется:
/// аккаунт, сессия и участник комнаты опознаются по одному и тому же
/// идентификатору, поэтому генерировать его на месте не нужно.
pub type PlayerId = NonZeroU32;

pub const NAME_MIN_CHARS: usize = 1;
pub const NAME_MAX_CHARS: usize = 16;
pub const PASSWORD_MIN_CHARS: usize = 6;

/// Field rules shared by the browser form and the server so they can never
/// drift apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::IntoStaticStr)]
pub enum ValidationError {
    #[strum(serialize = "auth_name_short")]
    NameTooShort,
    #[strum(serialize = "auth_name_long")]
    NameTooLong,
    #[strum(serialize = "auth_invalid_email")]
    InvalidEmail,
    #[strum(serialize = "auth_password_length")]
    PasswordTooShort,
    #[strum(serialize = "auth_password_mismatch")]
    PasswordMismatch,
}

pub fn validate_name(name: &str) -> Result<(), ValidationError> {
    let length = name.trim().chars().count();
    if length < NAME_MIN_CHARS {
        Err(ValidationError::NameTooShort)
    } else if length > NAME_MAX_CHARS {
        Err(ValidationError::NameTooLong)
    } else {
        Ok(())
    }
}

pub fn normalize_email(email: &str) -> Option<String> {
    let email = email.trim().to_ascii_lowercase();
    if email.is_empty() || email.len() > 254 {
        return None;
    }
    let (local, domain) = email.split_once('@')?;
    if local.is_empty() || domain.is_empty() || !domain.contains('.') || domain.starts_with('.') {
        return None;
    }
    Some(email)
}

pub fn validate_email(email: &str) -> Result<(), ValidationError> {
    normalize_email(email)
        .map(|_| ())
        .ok_or(ValidationError::InvalidEmail)
}

pub fn validate_password(password: &str) -> Result<(), ValidationError> {
    if password.chars().count() < PASSWORD_MIN_CHARS {
        Err(ValidationError::PasswordTooShort)
    } else {
        Ok(())
    }
}

/// Public account data safe to send to the client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserProfile {
    pub id: PlayerId,
    pub name: String,
    pub email: String,
    /// Unix timestamp (seconds) of the registration moment.
    pub created_at: u64,
}

impl UserProfile {
    /// `YYYY-MM-DD` registration date.
    pub fn registered_on(&self) -> String {
        format_unix_date(self.created_at)
    }
}

/// Converts a Unix timestamp to a civil `YYYY-MM-DD` date.
pub fn format_unix_date(timestamp: u64) -> String {
    DateTime::<Utc>::from_timestamp(timestamp as i64, 0)
        .map(|datetime| datetime.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// Request/response contract shared by the server and the browser client.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub name: String,
    pub email: String,
    pub password: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateProfileRequest {
    pub name: Option<String>,
    pub email: Option<String>,
    pub password: Option<String>,
    pub current_password: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthResponse {
    pub token: String,
    pub user: UserProfile,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserResponse {
    pub user: UserProfile,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
}

/// Persistent account storage. It is server-only: the browser bundle only
/// needs the DTOs above and never talks to redb or hashes passwords.
#[cfg(not(target_arch = "wasm32"))]
mod store;

#[cfg(not(target_arch = "wasm32"))]
pub use store::{AuthError, UserStore, generate_token};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_registration_dates() {
        assert_eq!(format_unix_date(0), "1970-01-01");
        assert_eq!(format_unix_date(86_400), "1970-01-02");
    }
}
