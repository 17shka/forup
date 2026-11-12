use forup_core::api::game::board::prelude::BoardSize;
use leptos::prelude::*;
use thaw::{Button, ButtonAppearance, ButtonSize, Card, CardHeader, Slider, Switch};

use crate::game::{GamePhase, GameSession};
use crate::i18n::*;

/// Pre-match settings: a single board-size slider, the automatic cell
/// distribution switch, the Lossy/Lossless explosion toggle and the
/// board-rotation control.
#[component]
pub fn GameSettingsPanel() -> impl IntoView {
    let i18n = use_i18n();
    let session = GameSession::expect();

    let board_size = session.state.with_untracked(|game| game.board_size());
    let size = RwSignal::new(f64::from(board_size.ordinal().unwrap_or(0)));
    let automatic = RwSignal::new(
        session
            .state
            .with_untracked(|game| game.automatic_distribution()),
    );
    let lossless = RwSignal::new(
        session
            .state
            .with_untracked(|game| game.lossless_explosions()),
    );
    let spacing = RwSignal::new(f64::from(
        session.state.with_untracked(|game| game.safe_radius()),
    ));

    Effect::new(move |previous: Option<f64>| {
        let current = size.get();
        if previous.is_some_and(|previous| (previous - current).abs() > f64::EPSILON) {
            let ordinal = current
                .round()
                .clamp(0.0, f64::from(BoardSize::SIZE_COUNT - 1)) as u8;
            if let Some(board_size) = BoardSize::from_ordinal(ordinal) {
                session.dispatch(move |game| game.set_board_size(board_size));
            }
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

    let spacing_max = move || f64::from(session.state.with(|game| game.max_safe_radius()));

    Effect::new(move |previous: Option<f64>| {
        let current = spacing.get();
        if previous.is_some_and(|previous| (previous - current).abs() > f64::EPSILON) {
            let max = spacing_max();
            let value = current.round().clamp(0.0, max) as u8;
            session.dispatch(move |game| game.set_safe_radius(value));
            // The engine clamps to the board/teams limit, follow it.
            spacing.set(f64::from(
                session.state.with_untracked(|game| game.safe_radius()),
            ));
        }
        current
    });

    Effect::new(move |previous: Option<bool>| {
        let current = lossless.get();
        if previous.is_some_and(|previous| previous != current) {
            session.dispatch(move |game| {
                game.set_lossless_explosions(current);
                Ok(())
            });
        }
        current
    });

    let size_text = move || {
        session.state.with(|game| {
            let size = game.board_size();
            format!("{}/{}", size.w(), size.h())
        })
    };

    let lossy_class = move || {
        let mut class = String::from("explosion-toggle__option explosion-toggle__option--lossy");
        if !lossless.get() {
            class.push_str(" explosion-toggle__option--active");
        }
        class
    };
    let lossless_class = move || {
        let mut class = String::from("explosion-toggle__option explosion-toggle__option--lossless");
        if lossless.get() {
            class.push_str(" explosion-toggle__option--active");
        }
        class
    };

    let lossy_label = "Lossy";
    let lossless_label = "Lossless";
    let group_label = Memo::new(move |_| t_string!(i18n, explosion_mode).to_string());
    let automatic_label = Memo::new(move |_| t_string!(i18n, automatic_distribution).to_string());
    let spacing_label = Memo::new(move |_| t_string!(i18n, safe_radius).to_string());
    let spacing_text = move || {
        format!(
            "{}/{}",
            session.state.with(|game| game.safe_radius()),
            spacing_max()
        )
    };

    view! {
        <Show when=move || session.state.with(|game| game.phase != GamePhase::Playing)>
            <Card class="game-settings">
                <CardHeader>{t!(i18n, game_settings)}</CardHeader>

                <div class="settings-row">
                    <span class="settings-row__label">{t!(i18n, size)}</span>
                    <Slider
                        class="settings-slider"
                        value=size
                        min=0.0
                        max=f64::from(BoardSize::SIZE_COUNT - 1)
                        step=1.0
                        show_stops=false
                    />
                    <span class="settings-row__value settings-row__size">{size_text}</span>
                </div>

                <Show when=move || !automatic.get()>
                    <div class="settings-row">
                        <span class="settings-row__label">{spacing_label}</span>
                        <Slider
                            class="settings-slider"
                            value=spacing
                            min=0.0
                            max=spacing_max
                            step=1.0
                            show_stops=false
                        />
                        <span class="settings-row__value">{spacing_text}</span>
                    </div>
                </Show>

                <div class="settings-row settings-row--switch">
                    <Switch
                        checked=automatic
                        label=automatic_label
                    />

                    <div class="settings-row__right">
                        <div
                            class="explosion-toggle"
                            role="group"
                            aria-label=group_label
                        >
                            <button
                                type="button"
                                class=lossy_class
                                aria-pressed=move || (!lossless.get()).to_string()
                                aria-label=lossy_label
                                title=lossy_label
                                on:click=move |_| lossless.set(false)
                            >
                                <span
                                    class="explosion-toggle__dot explosion-toggle__dot--lossy"
                                    aria-hidden="true"
                                ></span>
                                <span class="explosion-toggle__text">{lossy_label}</span>
                            </button>
                            <button
                                type="button"
                                class=lossless_class
                                aria-pressed=move || lossless.get().to_string()
                                aria-label=lossless_label
                                title=lossless_label
                                on:click=move |_| lossless.set(true)
                            >
                                <span
                                    class="explosion-toggle__dot explosion-toggle__dot--lossless"
                                    aria-hidden="true"
                                ></span>
                                <span class="explosion-toggle__text">{lossless_label}</span>
                            </button>
                        </div>
                        <RotateBoardButton/>
                    </div>
                </div>
            </Card>
        </Show>
    }
}

/// Rotates the board view 90° clockwise. Purely visual: the game state keeps
/// its own coordinates.
#[component]
pub fn RotateBoardButton() -> impl IntoView {
    let i18n = use_i18n();
    let session = GameSession::expect();
    let label = move || t_string!(i18n, rotate_board).to_string();

    view! {
        <Button
            class="rotate-board"
            size=ButtonSize::Small
            appearance=ButtonAppearance::Subtle
            on_click=move |_| session.rotate_board()
        >
            <span class="rotate-board__icon" aria-hidden="true">"⟳"</span>
            <span class="rotate-board__label">{label}</span>
        </Button>
    }
}
