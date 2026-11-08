use leptos::prelude::*;
use thaw::{
    Button, ButtonAppearance, DrawerBody, DrawerHeader, DrawerHeaderTitle, DrawerHeaderTitleAction,
    DrawerPosition, DrawerSize, OverlayDrawer, Select, SelectSize,
};

use crate::i18n::*;
use crate::settings::{LANGUAGE_SYSTEM, Settings, THEME_AUTO, THEME_DARK, THEME_LIGHT};

#[component]
pub fn ProfileDrawer(open: RwSignal<bool>) -> impl IntoView {
    let i18n = use_i18n();

    view! {
        <OverlayDrawer open=open position=DrawerPosition::Bottom size=DrawerSize::Small>
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
                <div class="profile-empty">
                    <div class="profile-empty__avatar" aria-hidden="true">"👤"</div>
                    <p class="profile-empty__text">{t!(i18n, profile_empty)}</p>
                </div>
            </DrawerBody>
        </OverlayDrawer>
    }
}

#[component]
pub fn SettingsDrawer(open: RwSignal<bool>) -> impl IntoView {
    let i18n = use_i18n();
    let ctx = Settings::expect();
    let theme_value = ctx.theme_choice;
    let language_value = (ctx.language, ctx.set_language);

    view! {
        <OverlayDrawer open=open position=DrawerPosition::Bottom size=DrawerSize::Small>
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
                    <h3 class="settings-section__title">{t!(i18n, theme_mode)}</h3>
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
                </section>

                <section class="settings-section">
                    <h3 class="settings-section__title">{t!(i18n, language)}</h3>
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
