use forup_core::api::game::board::prelude::BoardCode;
use leptos::prelude::*;
use thaw::{Card, CardHeader, Slider, Switch};

use crate::components::ModeCheckbox;
use crate::game::{GamePhase, GameSession};
use crate::i18n::*;

/// Pre-match settings: a single board-size slider, the automatic cell
/// distribution switch and the Lossy/Lossless explosion checkbox.
#[component]
pub fn GameSettingsPanel() -> impl IntoView {
    let i18n = use_i18n();
    let session = GameSession::expect();

    let board_size = session.state.with_untracked(|game| game.board_size());
    let size = RwSignal::new(f64::from(board_size.ordinal()));
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
                .clamp(0.0, f64::from(BoardCode::SIZE_COUNT - 1)) as u8;
            if let Some(board_size) = BoardCode::from_ordinal(ordinal) {
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
            format!("{}/{}", size.width(), size.height())
        })
    };

    // Чекбокс режима взрывов: включён — Lossless (зелёный "+"),
    // выключен — Lossy (красный "−").
    let explosion_checked = Signal::derive(move || lossless.get());
    let set_explosion = SignalSetter::map(move |checked: bool| lossless.set(checked));
    let explosion_label = Signal::derive(move || {
        let state = if lossless.get() {
            t_string!(i18n, lossless)
        } else {
            t_string!(i18n, lossy)
        };
        format!("{}: {state}", t_string!(i18n, explosion_mode))
    });

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
                        max=f64::from(BoardCode::SIZE_COUNT - 1)
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
                        <ModeCheckbox
                            checked=explosion_checked
                            set_checked=set_explosion
                            label=explosion_label
                        />
                    </div>
                </div>
            </Card>
        </Show>
    }
}
