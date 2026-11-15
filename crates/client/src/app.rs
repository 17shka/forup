use leptos::prelude::*;
use leptos_meta::{MetaTags, Stylesheet, Title, provide_meta_context};
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use leptos_use::use_media_query;
use thaw::DrawerPosition;

use crate::auth::AuthState;
use crate::components::{BottomNav, ProfileDrawer, SettingsDrawer};
use crate::game::GameSession;
use crate::i18n::I18nContextProvider;
use crate::pages::{GamePage, HomePage};
use crate::settings::SettingsScope;

/// Document shell used by the SSR server.
pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html>
            <head>
                <meta charset="utf-8"/>
                <meta
                    name="viewport"
                    content="width=device-width, initial-scale=1, viewport-fit=cover"
                />
                <link rel="icon" href="/favicon.ico"/>
                <AutoReload options=options.clone()/>
                <HydrationScripts options/>
                <MetaTags/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    // `cargo leptos` serves the stylesheet from `/pkg`; static builds bundled
    // by Trunk inject it through `index.html` instead.
    let stylesheet =
        (!cfg!(feature = "csr")).then(|| view! { <Stylesheet id="leptos" href="/pkg/forup.css"/> });

    view! {
        {stylesheet}
        <Title text="Forup"/>

        <I18nContextProvider enable_cookie=false>
            <SettingsScope>
                <Router base=router_base()>
                    <AppShell/>
                </Router>
            </SettingsScope>
        </I18nContextProvider>
    }
}

/// Client-side builds are deployed as a GitHub Pages project site under
/// `/forup/`, so the router has to strip that prefix from browser URLs.
fn router_base() -> &'static str {
    if cfg!(feature = "csr") { "/forup" } else { "" }
}

#[component]
fn AppShell() -> impl IntoView {
    let profile_open = RwSignal::new(false);
    let settings_open = RwSignal::new(false);

    // The local game lives above the settings/profile panel: navigating or
    // opening a drawer never resets the match.
    GameSession::provide();
    AuthState::provide();

    // In landscape the profile/settings drawers slide in from the right,
    // next to the nav rail, instead of from the bottom.
    let is_landscape = use_media_query("(orientation: landscape)");
    let drawer_position = Signal::derive(move || {
        if is_landscape.get() {
            DrawerPosition::Right
        } else {
            DrawerPosition::Bottom
        }
    });

    view! {
        <div class="app-shell">
            <main class="app-main">
                <Routes fallback=|| view! { <NotFound/> }>
                    <Route path=path!("/") view=HomePage/>
                    <Route path=path!("/game") view=GamePage/>
                </Routes>
            </main>

            <BottomNav profile_open settings_open/>
            <ProfileDrawer open=profile_open position=drawer_position/>
            <SettingsDrawer open=settings_open position=drawer_position/>
        </div>
    }
}

#[component]
fn NotFound() -> impl IntoView {
    view! {
        <div class="page not-found">
            <p class="muted">"404"</p>
        </div>
    }
}
