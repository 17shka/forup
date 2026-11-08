use leptos::prelude::*;
use thaw::{Button, ButtonAppearance};

use crate::i18n::*;

#[component]
pub fn BottomNav(profile_open: RwSignal<bool>, settings_open: RwSignal<bool>) -> impl IntoView {
    let i18n = use_i18n();

    view! {
        <nav class="bottom-nav">
            <Button
                class="nav-item"
                appearance=ButtonAppearance::Subtle
                on_click=move |_| {
                    settings_open.set(false);
                    profile_open.set(true);
                }
            >
                <span class="nav-item__icon" aria-hidden="true">"👤"</span>
                <span class="nav-item__label">{t!(i18n, profile)}</span>
            </Button>

            <Button
                class="nav-item"
                appearance=ButtonAppearance::Subtle
                on_click=move |_| {
                    profile_open.set(false);
                    settings_open.set(true);
                }
            >
                <span class="nav-item__icon" aria-hidden="true">"⚙"</span>
                <span class="nav-item__label">{t!(i18n, settings)}</span>
            </Button>
        </nav>
    }
}
