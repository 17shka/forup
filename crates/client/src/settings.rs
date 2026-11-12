use codee::string::FromToStringCodec;
use leptos::prelude::*;
use leptos_use::storage::{StorageType, UseStorageOptions, use_storage_with_options};
use leptos_use::{
    ColorMode, UseColorModeOptions, UseColorModeReturn, use_color_mode_with_options, use_locales,
};
use thaw::{ConfigProvider, Theme, ToasterProvider};

use crate::i18n::{I18nLocaleTrait, Locale, use_i18n};

pub const THEME_AUTO: &str = "auto";
pub const THEME_LIGHT: &str = "light";
pub const THEME_DARK: &str = "dark";

/// Pseudo-language meaning "follow the browser locale". Every real language is
/// discovered from `locales/*.yml` by `build.rs`.
pub const LANGUAGE_SYSTEM: &str = "system";

const LANGUAGE_STORAGE_KEY: &str = "language";
const EXPLOSION_DELAY_STORAGE_KEY: &str = "explosion_delay";

/// Bounds of the pause between explosion waves, in seconds. `0` disables the
/// step-by-step replay.
pub const EXPLOSION_DELAY_MIN: f64 = 0.0;
pub const EXPLOSION_DELAY_MAX: f64 = 0.5;
pub const EXPLOSION_DELAY_STEP: f64 = 0.01;
const EXPLOSION_DELAY_DEFAULT: f64 = 0.15;

#[derive(Clone, Copy)]
pub struct Settings {
    /// Raw selected theme mode (`auto`, `light` or `dark`).
    pub theme_choice: RwSignal<String>,
    /// Selected language mode (`system` or a locale code).
    pub language: Signal<String>,
    pub set_language: WriteSignal<String>,
    /// Pause between explosion waves, in seconds.
    pub explosion_delay: Signal<f64>,
    pub set_explosion_delay: WriteSignal<f64>,
}

impl Settings {
    pub fn expect() -> Self {
        expect_context::<Self>()
    }
}

/// Provides persisted user settings (theme + language) and syncs them with
/// leptos-i18n and Thaw.
#[component]
pub fn SettingsScope(children: Children) -> impl IntoView {
    let UseColorModeReturn {
        mode,
        set_mode,
        state,
        ..
    } = use_color_mode_with_options(
        UseColorModeOptions::default()
            .emit_auto(true)
            .storage_key("theme".to_string()),
    );

    // Thaw needs a concrete theme: `auto` is resolved by leptos-use from the
    // system preference, so keep the Thaw theme in sync with the resolved state.
    let theme = RwSignal::new(Theme::light());
    Effect::new(move |_| {
        let next = if state.get() == ColorMode::Dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        theme.set(next);
    });

    // Radio group value used by the settings drawer.
    let theme_choice = RwSignal::new(mode.get_untracked().to_string());

    Effect::new(move |_| {
        let choice = ColorMode::from(theme_choice.get());
        if choice != mode.get_untracked() {
            set_mode.set(choice);
        }
    });
    Effect::new(move |_| {
        let current = mode.get().to_string();
        if theme_choice.get_untracked() != current {
            theme_choice.set(current);
        }
    });

    // Persisted with leptos-use. `system` is resolved through the browser locales.
    let (language, set_language, _) = use_storage_with_options::<String, FromToStringCodec>(
        StorageType::Local,
        LANGUAGE_STORAGE_KEY.to_string(),
        UseStorageOptions::default().initial_value(LANGUAGE_SYSTEM.to_string()),
    );

    let (explosion_delay, set_explosion_delay, _) =
        use_storage_with_options::<f64, FromToStringCodec>(
            StorageType::Local,
            EXPLOSION_DELAY_STORAGE_KEY.to_string(),
            UseStorageOptions::default().initial_value(EXPLOSION_DELAY_DEFAULT),
        );
    let explosion_delay = Signal::derive(move || {
        let delay = explosion_delay.get();
        if delay.is_finite() {
            delay.clamp(EXPLOSION_DELAY_MIN, EXPLOSION_DELAY_MAX)
        } else {
            EXPLOSION_DELAY_DEFAULT
        }
    });

    let system_locales = use_locales();
    let i18n = use_i18n();

    let resolved_locale = Memo::new(move |_| {
        let selected = language.get();
        if selected == LANGUAGE_SYSTEM {
            return system_locale(&system_locales.get());
        }
        locale_by_code(&selected).unwrap_or_else(|| system_locale(&system_locales.get()))
    });

    Effect::new(move |_| {
        i18n.set_locale(resolved_locale.get());
    });

    provide_context(Settings {
        theme_choice,
        language,
        set_language,
        explosion_delay,
        set_explosion_delay,
    });

    view! {
        <ConfigProvider theme=theme>
            <ToasterProvider>{children()}</ToasterProvider>
        </ConfigProvider>
    }
}

fn locale_by_code(code: &str) -> Option<Locale> {
    Locale::get_all()
        .iter()
        .copied()
        .find(|locale| locale.to_string() == code)
}

fn system_locale(locales: &[String]) -> Locale {
    let available = Locale::get_all();
    locales
        .iter()
        .find_map(|locale| {
            let locale = locale.to_ascii_lowercase();
            available
                .iter()
                .copied()
                .filter(|candidate| {
                    let code = candidate.to_string().to_ascii_lowercase();
                    locale == code
                        || locale
                            .strip_prefix(&code)
                            .is_some_and(|rest| rest.starts_with('-') || rest.starts_with('_'))
                })
                .max_by_key(|candidate| candidate.to_string().len())
        })
        .unwrap_or_default()
}
