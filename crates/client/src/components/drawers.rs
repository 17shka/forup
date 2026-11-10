use forup_core::api::auth::{ValidationError, validate_email, validate_name, validate_password};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use thaw::{
    Button, ButtonAppearance, DrawerBody, DrawerHeader, DrawerHeaderTitle, DrawerHeaderTitleAction,
    DrawerPosition, DrawerSize, OverlayDrawer, Select, SelectSize,
};

use crate::auth::AuthState;
use crate::i18n::*;
use crate::settings::{LANGUAGE_SYSTEM, Settings, THEME_AUTO, THEME_DARK, THEME_LIGHT};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthMode {
    Login,
    Register,
}

#[component]
pub fn ProfileDrawer(open: RwSignal<bool>) -> impl IntoView {
    let i18n = use_i18n();
    let auth = AuthState::expect();

    view! {
        <OverlayDrawer
            class="app-drawer"
            open=open
            position=DrawerPosition::Bottom
            size=DrawerSize::Small
        >
            <DrawerHeader>
                <DrawerHeaderTitle>
                    {t!(i18n, profile)}
                    <DrawerHeaderTitleAction slot>
                        <Button
                            appearance=ButtonAppearance::Subtle
                            on_click=move |_| open.set(false)
                        >
                            "✕"
                        </Button>
                    </DrawerHeaderTitleAction>
                </DrawerHeaderTitle>
            </DrawerHeader>
            <DrawerBody>
                <Show
                    when=move || auth.bootstrapped.get()
                    fallback=move || view! { <p class="muted profile-loading">{t!(i18n, loading)}</p> }
                >
                    <Show
                        when=move || auth.user.get().is_some()
                        fallback=move || view! { <AuthPanel/> }
                    >
                        <ProfilePanel/>
                    </Show>
                </Show>
            </DrawerBody>
        </OverlayDrawer>
    }
}

#[component]
fn AuthPanel() -> impl IntoView {
    let i18n = use_i18n();
    let auth = AuthState::expect();

    let mode = RwSignal::new(AuthMode::Login);
    let name = RwSignal::new(String::new());
    let email = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let confirm = RwSignal::new(String::new());
    let submitted = RwSignal::new(false);

    let is_register = move || mode.get() == AuthMode::Register;

    let name_error = Memo::new(move |_| {
        let visible = is_register() && (submitted.get() || !name.get().trim().is_empty());
        if visible {
            validate_name(&name.get()).err()
        } else {
            None
        }
    });
    let email_error = Memo::new(move |_| {
        let visible = submitted.get() || !email.get().trim().is_empty();
        if visible {
            validate_email(&email.get()).err()
        } else {
            None
        }
    });
    let password_error = Memo::new(move |_| {
        let visible = is_register() && (submitted.get() || !password.get().is_empty());
        if visible {
            validate_password(&password.get()).err()
        } else {
            None
        }
    });
    let confirm_error = Memo::new(move |_| {
        let visible = is_register() && (submitted.get() || !confirm.get().is_empty());
        let mismatch = !password.get().is_empty() && confirm.get() != password.get();
        (visible && mismatch).then_some(ValidationError::PasswordMismatch)
    });

    let name_label = Memo::new(move |_| t_string!(i18n, name).to_string());
    let name_hint_label = Memo::new(move |_| t_string!(i18n, name_hint).to_string());
    let email_label = Memo::new(move |_| t_string!(i18n, email).to_string());
    let password_label = Memo::new(move |_| t_string!(i18n, password).to_string());

    let submit = move |event: SubmitEvent| {
        event.prevent_default();
        submitted.set(true);
        auth.error.set(None);

        let email_value = email.get_untracked().trim().to_owned();
        let password_value = password.get_untracked();
        let invalid = validate_email(&email_value).is_err()
            || (is_register()
                && (validate_name(&name.get_untracked()).is_err()
                    || validate_password(&password_value).is_err()
                    || password_value != confirm.get_untracked()));
        if invalid {
            return;
        }

        if is_register() {
            auth.register(
                name.get_untracked().trim().to_owned(),
                email_value,
                password_value,
            );
        } else {
            auth.login(email_value, password_value);
        }
        password.set(String::new());
        confirm.set(String::new());
    };

    let tab_class = move |target: AuthMode| {
        if mode.get() == target {
            "auth-tab auth-tab--active"
        } else {
            "auth-tab"
        }
    };

    view! {
        <div class="auth-panel">
            <div class="auth-tabs" role="tablist">
                <button
                    type="button"
                    class=move || tab_class(AuthMode::Login)
                    on:click=move |_| {
                        auth.error.set(None);
                        submitted.set(false);
                        mode.set(AuthMode::Login);
                    }
                >
                    {t!(i18n, login)}
                </button>
                <button
                    type="button"
                    class=move || tab_class(AuthMode::Register)
                    on:click=move |_| {
                        auth.error.set(None);
                        submitted.set(false);
                        mode.set(AuthMode::Register);
                    }
                >
                    {t!(i18n, register)}
                </button>
            </div>

            <form class="auth-form" on:submit=submit>
                <Show when=is_register>
                    <AuthField
                        label=name_label.get_untracked()
                        value=name
                        input_type="text"
                        autocomplete="nickname"
                        error=name_error
                        hint=name_hint_label.get_untracked()
                    />
                </Show>

                <AuthField
                    label=email_label.get_untracked()
                    value=email
                    input_type="email"
                    autocomplete="email"
                    error=email_error
                    hint=String::new()
                />

                <AuthField
                    label=password_label.get_untracked()
                    value=password
                    input_type="password"
                    autocomplete="current-password"
                    error=password_error
                    hint=String::new()
                />

                <Show when=is_register>
                    <AuthField
                        label=t_string!(i18n, confirm_password).to_string()
                        value=confirm
                        input_type="password"
                        autocomplete="new-password"
                        error=confirm_error
                        hint=String::new()
                    />
                </Show>

                <Show when=move || auth.error.get().is_some()>
                    <p class="auth-error">{move || auth.error.get().unwrap_or_default()}</p>
                </Show>

                <button class="auth-submit" type="submit" disabled=move || auth.loading.get()>
                    {move || {
                        if is_register() {
                            t_string!(i18n, create_account).to_string()
                        } else {
                            t_string!(i18n, sign_in).to_string()
                        }
                    }}
                </button>
            </form>
        </div>
    }
}

#[component]
fn AuthField(
    label: String,
    value: RwSignal<String>,
    input_type: &'static str,
    autocomplete: &'static str,
    error: Memo<Option<ValidationError>>,
    hint: String,
) -> impl IntoView {
    view! {
        <label class=move || {
            if error.get().is_some() {
                "auth-field auth-field--error"
            } else {
                "auth-field"
            }
        }>
            <span class="auth-field__label">{label}</span>
            <input
                class="auth-input"
                type=input_type
                autocomplete=autocomplete
                prop:value=move || value.get()
                on:input=move |event| value.set(event_target_value(&event))
            />
            <FieldError error=error/>
            <span class="auth-field__hint">{hint}</span>
        </label>
    }
}

/// Renders the localized message for one field validation error.
#[component]
fn FieldError(error: Memo<Option<ValidationError>>) -> impl IntoView {
    let i18n = use_i18n();
    view! {
        <Show when=move || error.get().is_some()>
            <span class="field-error">
                {move || {
                    error
                        .get()
                        .map(|error| match error {
                            ValidationError::NameTooShort => {
                                t_string!(i18n, auth_name_short).to_string()
                            }
                            ValidationError::NameTooLong => {
                                t_string!(i18n, auth_name_long).to_string()
                            }
                            ValidationError::InvalidEmail => {
                                t_string!(i18n, auth_invalid_email).to_string()
                            }
                            ValidationError::PasswordTooShort => {
                                t_string!(i18n, auth_password_length).to_string()
                            }
                            ValidationError::PasswordMismatch => {
                                t_string!(i18n, auth_password_mismatch).to_string()
                            }
                        })
                        .unwrap_or_default()
                }}
            </span>
        </Show>
    }
}

#[component]
fn ProfilePanel() -> impl IntoView {
    let i18n = use_i18n();
    let auth = AuthState::expect();

    let draft = RwSignal::new(
        auth.user
            .get_untracked()
            .map(|user| user.name)
            .unwrap_or_default(),
    );
    let name_error = Memo::new(move |_| validate_name(&draft.get()).err());

    let commit = move || {
        let value = draft.get_untracked();
        if validate_name(&value).is_err() {
            return;
        }
        let value = value.trim().to_owned();
        if auth.user.get_untracked().map(|user| user.name).as_deref() != Some(value.as_str()) {
            draft.set(value.clone());
            auth.rename(value);
        }
    };

    let email = move || auth.user.get().map(|user| user.email).unwrap_or_default();
    let registered = move || {
        auth.user
            .get()
            .map(|user| user.registered_on())
            .unwrap_or_default()
    };
    let initial = move || {
        auth.user
            .get()
            .and_then(|user| user.name.chars().next())
            .map(|letter| letter.to_uppercase().to_string())
            .unwrap_or_else(|| "?".to_owned())
    };

    view! {
        <div class="profile-panel">
            <div class="profile-card">
                <div class="profile-avatar" aria-hidden="true">{initial}</div>
                <div class="profile-card__info">
                    <input
                        class=move || {
                            if name_error.get().is_some() {
                                "auth-input profile-name-input auth-input--error"
                            } else {
                                "auth-input profile-name-input"
                            }
                        }
                        type="text"
                        autocomplete="nickname"
                        prop:value=move || draft.get()
                        on:input=move |event| draft.set(event_target_value(&event))
                        on:blur=move |_| commit()
                        on:keydown=move |event| {
                            if event.key() == "Enter" {
                                commit();
                            }
                        }
                    />
                    <FieldError error=name_error/>
                </div>
            </div>

            <dl class="profile-facts">
                <div class="profile-fact">
                    <dt>{t!(i18n, email)}</dt>
                    <dd>{email}</dd>
                </div>
                <div class="profile-fact">
                    <dt>{t!(i18n, registered_on)}</dt>
                    <dd>{registered}</dd>
                </div>
            </dl>

            <Show when=move || auth.error.get().is_some()>
                <p class="auth-error">{move || auth.error.get().unwrap_or_default()}</p>
            </Show>

            <button
                type="button"
                class="profile-action profile-action--danger"
                on:click=move |_| auth.logout()
            >
                {t!(i18n, logout)}
            </button>
        </div>
    }
}

#[component]
pub fn SettingsDrawer(open: RwSignal<bool>) -> impl IntoView {
    let i18n = use_i18n();
    let ctx = Settings::expect();
    let theme_value = ctx.theme_choice;
    let language_value = (ctx.language, ctx.set_language);

    view! {
        <OverlayDrawer
            class="app-drawer"
            open=open
            position=DrawerPosition::Bottom
            size=DrawerSize::Small
        >
            <DrawerHeader>
                <DrawerHeaderTitle>
                    {t!(i18n, settings)}
                    <DrawerHeaderTitleAction slot>
                        <Button
                            appearance=ButtonAppearance::Subtle
                            on_click=move |_| open.set(false)
                        >
                            "✕"
                        </Button>
                    </DrawerHeaderTitleAction>
                </DrawerHeaderTitle>
            </DrawerHeader>
            <DrawerBody>
                <section class="settings-section">
                    <div class="settings-field">
                        <span class="settings-field__label">{t!(i18n, theme_mode)}</span>
                        <Select
                            class="settings-select"
                            size=SelectSize::Large
                            value=theme_value
                        >
                            <option value=THEME_AUTO selected=move || theme_value.get() == THEME_AUTO>
                                {t!(i18n, theme_system)}
                            </option>
                            <option value=THEME_LIGHT selected=move || theme_value.get() == THEME_LIGHT>
                                {t!(i18n, theme_light)}
                            </option>
                            <option value=THEME_DARK selected=move || theme_value.get() == THEME_DARK>
                                {t!(i18n, theme_dark)}
                            </option>
                        </Select>
                    </div>

                    <div class="settings-field">
                        <span class="settings-field__label">{t!(i18n, language)}</span>
                        <Select
                            class="settings-select"
                            size=SelectSize::Large
                            value=language_value
                        >
                            <option
                                value=LANGUAGE_SYSTEM
                                selected=move || ctx.language.get() == LANGUAGE_SYSTEM
                            >
                                {t!(i18n, language_system)}
                            </option>
                            {available_languages(ctx.language)}
                        </Select>
                    </div>
                </section>
            </DrawerBody>
        </OverlayDrawer>
    }
}

/// One option per `locales/<code>.yml` file. Nothing here knows which
/// languages exist — the list is generated from the i18n bundle.
fn available_languages(language: Signal<String>) -> impl IntoView {
    Locale::get_all()
        .iter()
        .copied()
        .map(|locale| {
            let code = locale.to_string();
            let selected_code = code.clone();
            view! {
                <option value=code selected=move || language.get() == selected_code>
                    {td_string!(locale, locale_flag)} " " {td_string!(locale, language_name)}
                </option>
            }
        })
        .collect_view()
}
