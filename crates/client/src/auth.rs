use codee::string::FromToStringCodec;
use forup_core::api::auth::{AuthResponse, UpdateProfileRequest, UserProfile};
use leptos::prelude::*;
use leptos_use::storage::{StorageType, UseStorageOptions, use_storage_with_options};

#[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
use crate::net;

#[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
use forup_core::api::auth::{LoginRequest, RegisterRequest, UserResponse};
#[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
use leptos::task::spawn_local;

const TOKEN_STORAGE_KEY: &str = "token";

/// Shared account state: the profile currently logged in and the opaque session
/// token used to talk to the server.
#[derive(Clone, Copy)]
pub struct AuthState {
    pub user: RwSignal<Option<UserProfile>>,
    pub token: Signal<String>,
    pub set_token: WriteSignal<String>,
    pub loading: RwSignal<bool>,
    /// True once the stored token has been checked, so the drawer can tell
    /// "restoring session" apart from "submitting a form".
    pub bootstrapped: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
}

impl AuthState {
    /// Provides the account context and restores the session from local storage.
    /// Must be called from a component.
    pub fn provide() -> Self {
        let (token, set_token, _) = use_storage_with_options::<String, FromToStringCodec>(
            StorageType::Local,
            TOKEN_STORAGE_KEY.to_string(),
            UseStorageOptions::default().initial_value(String::new()),
        );
        let state = Self {
            user: RwSignal::new(None),
            token,
            set_token,
            loading: RwSignal::new(true),
            bootstrapped: RwSignal::new(false),
            error: RwSignal::new(None),
        };
        provide_context(state);
        state.bootstrap();
        state
    }

    pub fn expect() -> Self {
        expect_context::<Self>()
    }

    #[allow(dead_code)]
    fn apply_session(self, response: AuthResponse) {
        self.set_token.set(response.token);
        self.user.set(Some(response.user));
        self.error.set(None);
    }

    fn bootstrap(self) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        {
            let token = self.token.get_untracked();
            if token.is_empty() {
                self.loading.set(false);
                self.bootstrapped.set(true);
                return;
            }
            spawn_local(async move {
                match net::api_request::<UserResponse>("GET", "/api/auth/me", Some(&token), None)
                    .await
                {
                    Ok(response) => self.user.set(Some(response.user)),
                    Err(_) => {
                        // The stored token is no longer valid.
                        self.set_token.set(String::new());
                        self.user.set(None);
                    }
                }
                self.loading.set(false);
                self.bootstrapped.set(true);
            });
        }
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        {
            self.loading.set(false);
            self.bootstrapped.set(true);
        }
    }

    pub fn register(self, name: String, email: String, password: String) {
        self.error.set(None);
        self.loading.set(true);
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        spawn_local(async move {
            let body = postcard::to_allocvec(&RegisterRequest {
                name,
                email,
                password,
            });
            match body {
                Ok(body) => {
                    match net::api_request::<AuthResponse>(
                        "POST",
                        "/api/auth/register",
                        None,
                        Some(body),
                    )
                    .await
                    {
                        Ok(response) => self.apply_session(response),
                        Err(error) => self.error.set(Some(error)),
                    }
                }
                Err(error) => self.error.set(Some(error.to_string())),
            }
            self.loading.set(false);
        });
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        {
            let _ = (name, email, password);
            self.loading.set(false);
        }
    }

    pub fn login(self, email: String, password: String) {
        self.error.set(None);
        self.loading.set(true);
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        spawn_local(async move {
            let body = postcard::to_allocvec(&LoginRequest { email, password });
            match body {
                Ok(body) => {
                    match net::api_request::<AuthResponse>(
                        "POST",
                        "/api/auth/login",
                        None,
                        Some(body),
                    )
                    .await
                    {
                        Ok(response) => self.apply_session(response),
                        Err(error) => self.error.set(Some(error)),
                    }
                }
                Err(error) => self.error.set(Some(error.to_string())),
            }
            self.loading.set(false);
        });
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        {
            let _ = (email, password);
            self.loading.set(false);
        }
    }

    pub fn logout(self) {
        let token = self.token.get_untracked();
        self.set_token.set(String::new());
        self.user.set(None);
        self.error.set(None);

        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        if !token.is_empty() {
            spawn_local(async move {
                let _ =
                    net::api_request::<()>("POST", "/api/auth/logout", Some(&token), None).await;
            });
        }
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        let _ = token;
    }

    pub fn rename(self, name: String) {
        self.update_profile(UpdateProfileRequest {
            name: Some(name),
            email: None,
            password: None,
            current_password: None,
        });
    }

    pub fn change_email(self, email: String) {
        self.update_profile(UpdateProfileRequest {
            name: None,
            email: Some(email),
            password: None,
            current_password: None,
        });
    }

    pub fn change_password(self, current_password: String, password: String) {
        self.update_profile(UpdateProfileRequest {
            name: None,
            email: None,
            password: Some(password),
            current_password: Some(current_password),
        });
    }

    fn update_profile(self, request: UpdateProfileRequest) {
        self.error.set(None);
        let token = self.token.get_untracked();
        if token.is_empty() {
            self.error.set(Some("authentication required".into()));
            return;
        }
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        {
            self.loading.set(true);
            spawn_local(async move {
                let body = postcard::to_allocvec(&request);
                match body {
                    Ok(body) => {
                        match net::api_request::<UserResponse>(
                            "PATCH",
                            "/api/auth/profile",
                            Some(&token),
                            Some(body),
                        )
                        .await
                        {
                            Ok(response) => self.user.set(Some(response.user)),
                            Err(error) => self.error.set(Some(error)),
                        }
                    }
                    Err(error) => self.error.set(Some(error.to_string())),
                }
                self.loading.set(false);
            });
        }
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        let _ = request;
    }
}
