use forup_core::api::game::board::prelude::BoardCode;
use leptos::prelude::*;
use thaw::{Card, CardHeader, Slider, Switch};

use crate::game::{GamePhase, GameSession};
use crate::i18n::*;

/// Pre-match settings as flat rows: board size slider, the automatic cell
/// distribution switch, the safe radius slider and the Lossy/Lossless
/// explosion toggle.
///
/// Онлайн: панель видит и меняет только администратор комнаты, а
/// источником истины становится серверный снапшот.
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

    // Серверный снапшот тянет локальные сигналы за собой: чужие изменения
    // настроек и кламп значений проявляются без перезагрузки страницы.
    // Чтения локальных сигналов без подписки: иначе этот же эффект снапил
    // бы слайдер обратно прямо во время перетаскивания.
    Effect::new(move |_| {
        session.net_busy.get();
        let (ordinal, distribution, explosions, radius) = session.state.with(|game| {
            (
                game.board_size().ordinal(),
                game.automatic_distribution(),
                game.lossless_explosions(),
                game.safe_radius(),
            )
        });
        if size.get_untracked().round() as u8 != ordinal {
            size.set(f64::from(ordinal));
        }
        if automatic.get_untracked() != distribution {
            automatic.set(distribution);
        }
        if lossless.get_untracked() != explosions {
            lossless.set(explosions);
        }
        if spacing.get_untracked().round() as u8 != radius {
            spacing.set(f64::from(radius));
        }
    });

    Effect::new(move |previous: Option<f64>| {
        let current = size.get();
        if previous.is_some_and(|previous| (previous - current).abs() > f64::EPSILON) {
            let ordinal = current
                .round()
                .clamp(0.0, f64::from(BoardCode::SIZE_COUNT - 1)) as u8;
            if let Some(board_size) = BoardCode::from_ordinal(ordinal) {
                let changed = session
                    .state
                    .with_untracked(|game| game.board_size() != board_size);
                if changed {
                    if session.room.get_untracked().is_some() {
                        session.net_set_board_size(
                            board_size.width().get(),
                            board_size.height().get(),
                        );
                    } else {
                        session.dispatch(move |game| game.set_board_size(board_size));
                    }
                }
            }
        }
        current
    });

    Effect::new(move |previous: Option<bool>| {
        let current = automatic.get();
        if previous.is_some_and(|previous| previous != current) {
            let changed = session
                .state
                .with_untracked(|game| game.automatic_distribution() != current);
            if changed {
                if session.room.get_untracked().is_some() {
                    session.net_set_automatic_distribution(current);
                } else {
                    session.dispatch(move |game| {
                        game.set_automatic_distribution(current);
                        Ok(())
                    });
                }
            }
        }
        current
    });

    let spacing_max = move || f64::from(session.state.with(|game| game.max_safe_radius()));

    Effect::new(move |previous: Option<f64>| {
        let current = spacing.get();
        if previous.is_some_and(|previous| (previous - current).abs() > f64::EPSILON) {
            let max = spacing_max();
            let value = current.round().clamp(0.0, max) as u8;
            let changed = session
                .state
                .with_untracked(|game| game.safe_radius() != value);
            if changed {
                if session.room.get_untracked().is_some() {
                    // Сервер клампит значение по доске и командам; снапшот
                    // вернёт клампнутое — его покажет синхронизирующий эффект.
                    session.net_set_safe_radius(value);
                } else {
                    session.dispatch(move |game| game.set_safe_radius(value));
                }
            }
        }
        current
    });

    Effect::new(move |previous: Option<bool>| {
        let current = lossless.get();
        if previous.is_some_and(|previous| previous != current) {
            let changed = session
                .state
                .with_untracked(|game| game.lossless_explosions() != current);
            if changed {
                if session.room.get_untracked().is_some() {
                    session.net_set_lossless_explosions(current);
                } else {
                    session.dispatch(move |game| {
                        game.set_lossless_explosions(current);
                        Ok(())
                    });
                }
            }
        }
        current
    });

    let size_text = move || {
        session.state.with(|game| {
            let size = game.board_size();
            format!("{}/{}", size.width(), size.height())
        })
    };

    let automatic_label = Memo::new(move |_| t_string!(i18n, automatic_distribution).to_string());
    let spacing_label = Memo::new(move |_| t_string!(i18n, safe_radius).to_string());
    let spacing_text = move || {
        format!(
            "{}/{}",
            session.state.with(|game| game.safe_radius()),
            spacing_max()
        )
    };
    let lossy_label = Memo::new(move |_| t_string!(i18n, lossy).to_string());
    let lossless_label = Memo::new(move |_| t_string!(i18n, lossless).to_string());
    let group_label = Memo::new(move |_| t_string!(i18n, explosion_mode).to_string());

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

    view! {
        <Show when=move || {
            session.state.with(|game| game.phase != GamePhase::Playing)
                && (session.room.get().is_none()
                    || session.room.get().is_some_and(|room| room.is_admin))
        }>
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

                <div class="settings-row settings-row--switch">
                    <span class="settings-row__label">{automatic_label}</span>
                    <div class="settings-row__right">
                        <Switch checked=automatic />
                    </div>
                </div>

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

                <div class="settings-row">
                    <span class="settings-row__label">{t!(i18n, explosion_mode)}</span>
                    <div class="settings-row__right">
                        <div class="explosion-toggle" role="group" aria-label=group_label>
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
                    </div>
                </div>
            </Card>
        </Show>
    }
}
