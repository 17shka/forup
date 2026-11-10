use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const NAME_MIN_CHARS: usize = 1;
pub const NAME_MAX_CHARS: usize = 16;
pub const PASSWORD_MIN_CHARS: usize = 6;

/// Field rules shared by the browser form and the server so they can never
/// drift apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationError {
    NameTooShort,
    NameTooLong,
    InvalidEmail,
    PasswordTooShort,
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
    pub id: u32,
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

/// Persistent account storage. It is server-only: the browser bundle only needs
/// the DTOs above and never talks to redb or hashes passwords.
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::num::NonZeroU32;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
    use serde::{Serialize, de::DeserializeOwned};
    use sha2::{Digest, Sha256};

    use super::{
        NAME_MAX_CHARS, NAME_MIN_CHARS, PASSWORD_MIN_CHARS, UserProfile, ValidationError,
        normalize_email, validate_name, validate_password,
    };

    const USERS: TableDefinition<u32, &[u8]> = TableDefinition::new("auth_users");
    const EMAILS: TableDefinition<&str, u32> = TableDefinition::new("auth_user_emails");
    const META: TableDefinition<&str, u64> = TableDefinition::new("auth_meta");
    const NEXT_ID_KEY: &str = "next_user_id";
    const SALT_LEN: usize = 16;

    /// A user id is assigned once, strictly increasing from 1, and never changes.
    pub type UserId = NonZeroU32;

    impl From<ValidationError> for AuthError {
        fn from(error: ValidationError) -> Self {
            match error {
                ValidationError::NameTooShort | ValidationError::NameTooLong => {
                    AuthError::InvalidName
                }
                ValidationError::InvalidEmail => AuthError::InvalidEmail,
                ValidationError::PasswordTooShort => AuthError::WeakPassword,
                ValidationError::PasswordMismatch => AuthError::InvalidCredentials,
            }
        }
    }

    #[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
    struct StoredUser {
        id: u32,
        name: String,
        email: String,
        salt: [u8; SALT_LEN],
        password_hash: String,
        created_at: u64,
    }

    impl StoredUser {
        fn profile(&self) -> UserProfile {
            UserProfile {
                id: self.id,
                name: self.name.clone(),
                email: self.email.clone(),
                created_at: self.created_at,
            }
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum AuthError {
        Database(String),
        Serialization(String),
        InvalidName,
        InvalidEmail,
        WeakPassword,
        EmailTaken,
        InvalidCredentials,
        UserNotFound,
        IdSpaceExhausted,
    }

    impl std::fmt::Display for AuthError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Database(message) => write!(f, "database error: {message}"),
                Self::Serialization(message) => write!(f, "serialization error: {message}"),
                Self::InvalidName => write!(
                    f,
                    "name must be between {NAME_MIN_CHARS} and {NAME_MAX_CHARS} characters"
                ),
                Self::InvalidEmail => write!(f, "that email address is not valid"),
                Self::WeakPassword => {
                    write!(
                        f,
                        "password must be at least {PASSWORD_MIN_CHARS} characters"
                    )
                }
                Self::EmailTaken => write!(f, "an account with that email already exists"),
                Self::InvalidCredentials => write!(f, "wrong email or password"),
                Self::UserNotFound => write!(f, "account not found"),
                Self::IdSpaceExhausted => write!(f, "no user ids left"),
            }
        }
    }

    impl std::error::Error for AuthError {}

    #[derive(Clone)]
    pub struct UserStore {
        database: Arc<Database>,
    }

    impl std::fmt::Debug for UserStore {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("UserStore").finish_non_exhaustive()
        }
    }

    impl UserStore {
        pub fn open(path: impl AsRef<Path>) -> Result<Self, AuthError> {
            let database = Database::create(path).map_err(database_error)?;
            Ok(Self {
                database: Arc::new(database),
            })
        }

        /// Creates an account and assigns the next sequential id.
        pub fn register(
            &self,
            name: &str,
            email: &str,
            password: &str,
        ) -> Result<UserProfile, AuthError> {
            validate_name(name)?;
            let name = name.trim().to_owned();
            let email = normalize_email(email).ok_or(AuthError::InvalidEmail)?;
            validate_password(password)?;

            let salt = random_bytes::<SALT_LEN>();
            let password_hash = hash_password(password, &salt);
            let created_at = unix_now()?;

            let write = self.database.begin_write().map_err(database_error)?;

            let id = {
                let mut meta = write.open_table(META).map_err(database_error)?;
                let next = meta
                    .get(NEXT_ID_KEY)
                    .map_err(database_error)?
                    .map_or(1, |value| value.value().max(1));
                let id = u32::try_from(next)
                    .ok()
                    .and_then(UserId::new)
                    .ok_or(AuthError::IdSpaceExhausted)?;
                meta.insert(NEXT_ID_KEY, next + 1).map_err(database_error)?;
                id
            };

            {
                let mut emails = write.open_table(EMAILS).map_err(database_error)?;
                if emails
                    .get(email.as_str())
                    .map_err(database_error)?
                    .is_some()
                {
                    return Err(AuthError::EmailTaken);
                }
                emails
                    .insert(email.as_str(), id.get())
                    .map_err(database_error)?;
            }

            let stored = StoredUser {
                id: id.get(),
                name,
                email,
                salt,
                password_hash,
                created_at,
            };
            {
                let mut users = write.open_table(USERS).map_err(database_error)?;
                let serialized = encode(&stored)?;
                users
                    .insert(id.get(), serialized.as_slice())
                    .map_err(database_error)?;
            }

            write.commit().map_err(database_error)?;
            Ok(stored.profile())
        }

        pub fn authenticate(&self, email: &str, password: &str) -> Result<UserProfile, AuthError> {
            let email = normalize_email(email).ok_or(AuthError::InvalidCredentials)?;
            let read = self.database.begin_read().map_err(database_error)?;

            let id = {
                let emails = read.open_table(EMAILS).map_err(database_error)?;
                emails
                    .get(email.as_str())
                    .map_err(database_error)?
                    .map(|value| value.value())
            };
            let Some(id) = id else {
                return Err(AuthError::InvalidCredentials);
            };

            let stored = {
                let users = read.open_table(USERS).map_err(database_error)?;
                match users.get(id).map_err(database_error)? {
                    Some(bytes) => decode::<StoredUser>(bytes.value())?,
                    None => return Err(AuthError::InvalidCredentials),
                }
            };

            if hash_password(password, &stored.salt) != stored.password_hash {
                return Err(AuthError::InvalidCredentials);
            }
            Ok(stored.profile())
        }

        pub fn profile(&self, id: UserId) -> Result<Option<UserProfile>, AuthError> {
            Ok(self.load(id)?.map(|user| user.profile()))
        }

        pub fn rename(&self, id: UserId, name: &str) -> Result<UserProfile, AuthError> {
            validate_name(name)?;
            let mut stored = self.load(id)?.ok_or(AuthError::UserNotFound)?;
            stored.name = name.trim().to_owned();
            self.save(&stored)?;
            Ok(stored.profile())
        }

        pub fn update_email(&self, id: UserId, email: &str) -> Result<UserProfile, AuthError> {
            let email = normalize_email(email).ok_or(AuthError::InvalidEmail)?;
            let mut stored = self.load(id)?.ok_or(AuthError::UserNotFound)?;
            if stored.email == email {
                return Ok(stored.profile());
            }

            let write = self.database.begin_write().map_err(database_error)?;
            {
                let emails = write.open_table(EMAILS).map_err(database_error)?;
                if let Some(existing) = emails.get(email.as_str()).map_err(database_error)? {
                    if existing.value() != id.get() {
                        return Err(AuthError::EmailTaken);
                    }
                }
            }
            {
                let mut emails = write.open_table(EMAILS).map_err(database_error)?;
                emails
                    .remove(stored.email.as_str())
                    .map_err(database_error)?;
                emails
                    .insert(email.as_str(), id.get())
                    .map_err(database_error)?;
            }

            stored.email = email;
            {
                let mut users = write.open_table(USERS).map_err(database_error)?;
                let serialized = encode(&stored)?;
                users
                    .insert(id.get(), serialized.as_slice())
                    .map_err(database_error)?;
            }
            write.commit().map_err(database_error)?;
            Ok(stored.profile())
        }

        pub fn change_password(
            &self,
            id: UserId,
            current_password: &str,
            new_password: &str,
        ) -> Result<(), AuthError> {
            let mut stored = self.load(id)?.ok_or(AuthError::UserNotFound)?;
            if hash_password(current_password, &stored.salt) != stored.password_hash {
                return Err(AuthError::InvalidCredentials);
            }
            validate_password(new_password)?;
            stored.salt = random_bytes();
            stored.password_hash = hash_password(new_password, &stored.salt);
            self.save(&stored)
        }

        fn load(&self, id: UserId) -> Result<Option<StoredUser>, AuthError> {
            let read = self.database.begin_read().map_err(database_error)?;
            let users = read.open_table(USERS).map_err(database_error)?;
            match users.get(id.get()).map_err(database_error)? {
                Some(bytes) => Ok(Some(decode::<StoredUser>(bytes.value())?)),
                None => Ok(None),
            }
        }

        fn save(&self, user: &StoredUser) -> Result<(), AuthError> {
            let serialized = encode(user)?;
            let write = self.database.begin_write().map_err(database_error)?;
            {
                let mut users = write.open_table(USERS).map_err(database_error)?;
                users
                    .insert(user.id, serialized.as_slice())
                    .map_err(database_error)?;
            }
            write.commit().map_err(database_error)?;
            Ok(())
        }
    }

    fn random_bytes<const N: usize>() -> [u8; N] {
        let mut bytes = [0u8; N];
        getrandom::fill(&mut bytes).expect("OS randomness is unavailable");
        bytes
    }

    /// Random opaque token used by the HTTP layer for sessions.
    pub fn generate_token() -> String {
        hex::encode(random_bytes::<32>())
    }

    fn hash_password(password: &str, salt: &[u8; SALT_LEN]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(salt);
        hasher.update(password.as_bytes());
        hex::encode(hasher.finalize())
    }

    fn unix_now() -> Result<u64, AuthError> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .map_err(database_error)
    }

    fn database_error(error: impl std::fmt::Display) -> AuthError {
        AuthError::Database(error.to_string())
    }

    fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, AuthError> {
        postcard::to_allocvec(value).map_err(|error| AuthError::Serialization(error.to_string()))
    }

    fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, AuthError> {
        postcard::from_bytes(bytes).map_err(|error| AuthError::Serialization(error.to_string()))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::{AuthError, UserId, UserStore, generate_token};

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn store() -> (UserStore, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("forup-users-{}.redb", generate_token()));
        let store = UserStore::open(&path).unwrap();
        (store, path)
    }

    #[test]
    fn registration_assigns_sequential_ids_and_persists_profiles() {
        let (store, path) = store();

        let first = store
            .register("Alice", "Alice@Example.com", "secret1")
            .unwrap();
        let second = store.register("Bob", "bob@example.com", "secret2").unwrap();
        let third = store
            .register("Cara", "cara@example.com", "secret3")
            .unwrap();

        assert_eq!(first.id, 1);
        assert_eq!(second.id, 2);
        assert_eq!(third.id, 3);
        assert_eq!(first.email, "alice@example.com");
        assert_eq!(first.name, "Alice");

        drop(store);
        let reloaded = UserStore::open(&path).unwrap();
        assert_eq!(
            reloaded.profile(UserId::new(2).unwrap()).unwrap(),
            Some(second)
        );
        drop(reloaded);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn duplicate_emails_are_rejected() {
        let (store, path) = store();
        store
            .register("Alice", "alice@example.com", "secret1")
            .unwrap();
        assert_eq!(
            store.register("Other", "ALICE@example.com", "secret2"),
            Err(AuthError::EmailTaken)
        );
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn names_are_between_one_and_sixteen_characters() {
        let (store, path) = store();
        assert_eq!(
            store.register("", "a@example.com", "secret1"),
            Err(AuthError::InvalidName)
        );
        assert_eq!(
            store.register("0123456789abcdefg", "b@example.com", "secret1"),
            Err(AuthError::InvalidName)
        );
        assert!(
            store
                .register("0123456789abcdef", "c@example.com", "secret1")
                .is_ok()
        );
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn authenticate_and_change_credentials() {
        let (store, path) = store();
        let user = store
            .register("Alice", "alice@example.com", "secret1")
            .unwrap();

        assert_eq!(
            store.authenticate("alice@example.com", "wrong"),
            Err(AuthError::InvalidCredentials)
        );
        assert_eq!(
            store.authenticate("alice@example.com", "secret1").unwrap(),
            user
        );

        let renamed = store
            .rename(UserId::new(user.id).unwrap(), "Alicia")
            .unwrap();
        assert_eq!(renamed.name, "Alicia");

        let updated = store
            .update_email(UserId::new(user.id).unwrap(), "new@example.com")
            .unwrap();
        assert_eq!(updated.email, "new@example.com");
        assert_eq!(
            store.authenticate("alice@example.com", "secret1"),
            Err(AuthError::InvalidCredentials)
        );
        assert!(store.authenticate("new@example.com", "secret1").is_ok());

        store
            .change_password(UserId::new(user.id).unwrap(), "secret1", "secret2")
            .unwrap();
        assert_eq!(
            store.authenticate("new@example.com", "secret1"),
            Err(AuthError::InvalidCredentials)
        );
        assert!(store.authenticate("new@example.com", "secret2").is_ok());

        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn formats_registration_dates() {
        assert_eq!(format_unix_date(0), "1970-01-01");
        assert_eq!(format_unix_date(86_400), "1970-01-02");
    }
}
