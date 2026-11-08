use forup_core::api::game::board::prelude::Board;
use glam::U8Vec2;
use leptos::prelude::*;
use thaw::{Card, CardHeader, Slider, Switch};

use crate::game::{GamePhase, GameSession};
use crate::i18n::*;

/// Pre-match settings: board width/height sliders and the automatic cell
/// distribution switch. The panel is only visible while no match is running.
#[component]
pub fn GameSettingsPanel() -> impl IntoView {
    let i18n = use_i18n();
    let session = GameSession::expect();

    let board_size = session.state.with_untracked(|game| game.board_size());
    let width = RwSignal::new(f64::from(board_size.x));
    let height = RwSignal::new(f64::from(board_size.y));
    let automatic = RwSignal::new(
        session
            .state
            .with_untracked(|game| game.automatic_distribution()),
    );

    Effect::new(move |previous: Option<f64>| {
        let current = width.get();
        if previous.is_some_and(|previous| (previous - current).abs() > f64::EPSILON) {
            let width = current.round() as u8;
            let height = height.get_untracked().round() as u8;
            session.dispatch(move |game| game.set_board_size(U8Vec2::new(width, height)));
        }
        current
    });

    Effect::new(move |previous: Option<f64>| {
        let current = height.get();
        if previous.is_some_and(|previous| (previous - current).abs() > f64::EPSILON) {
            let width = width.get_untracked().round() as u8;
            let height = current.round() as u8;
            session.dispatch(move |game| game.set_board_size(U8Vec2::new(width, height)));
        }
        current
    });

    Effect::new(move |previous: Option<bool>| {
        let current = automatic.get();
        if previous.is_some_and(|previous| previous != current) {
            session.dispatch(move |game| {
                game.set_automatic_distribution(current);
                Ok(())
            });
        }
        current
    });

    view! {
        <Show when=move || session.state.with(|game| game.phase != GamePhase::Playing)>
            <Card class="game-settings">
                <CardHeader>{t!(i18n, game_settings)}</CardHeader>

                <div class="settings-row">
                    <span class="settings-row__label">{t!(i18n, width)}</span>
                    <Slider
                        class="settings-slider"
                        value=width
                        min=f64::from(Board::MIN_SIZE.x)
                        max=f64::from(Board::MAX_SIZE.x)
                        step=1.0
                        show_stops=false
                    />
                    <span class="settings-row__value">
                        {move || width.get().round() as i32}
                    </span>
                </div>

                <div class="settings-row">
                    <span class="settings-row__label">{t!(i18n, height)}</span>
                    <Slider
                        class="settings-slider"
                        value=height
                        min=f64::from(Board::MIN_SIZE.y)
                        max=f64::from(Board::MAX_SIZE.y)
                        step=1.0
                        show_stops=false
                    />
                    <span class="settings-row__value">
                        {move || height.get().round() as i32}
                    </span>
                </div>

                <div class="settings-row settings-row--switch">
                    <Switch
                        checked=automatic
                        label=t_string!(i18n, automatic_distribution).to_string()
                    />
                </div>
            </Card>
        </Show>
    }
}
