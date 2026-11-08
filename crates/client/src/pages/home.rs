use leptos::prelude::*;
use leptos_router::hooks::use_navigate;
use thaw::{Button, ButtonAppearance, ButtonSize};

use crate::i18n::*;

#[component]
pub fn HomePage() -> impl IntoView {
    let i18n = use_i18n();
    let navigate = use_navigate();
    let on_play = move |_| navigate("/game", Default::default());

    view! {
        <div class="page home-page">
            <div class="home-page__content">
                <h1 class="home-page__logo">"Forup"</h1>
                <Button
                    class="play-button"
                    appearance=ButtonAppearance::Primary
                    size=ButtonSize::Large
                    on_click=on_play
                >
                    {t!(i18n, play)}
                </Button>
            </div>
        </div>
    }
}
