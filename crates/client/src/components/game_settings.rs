use forup_core::api::game::prelude::{BoardCode, ExplosionMode};
use leptos::prelude::*;
use thaw::{Card, CardHeader, Select, SelectSize, Slider, Switch};

use crate::game::{GamePhase, GameSession};
use crate::i18n::*;

/// Pre-match settings as flat rows: board size slider, the automatic cell
/// distribution switch, the safe radius slider and the explosion mode
/// dropdown selector.
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
    let explosion_mode = RwSignal::new(session.state.with_untracked(|game| game.explosion_mode()));
    let spacing = RwSignal::new(f64::from(
        session.state.with_untracked(|game| game.safe_radius()),
    ));

    // Серверный снапшот тянет локальные сигналы за собой: чужие изменения
    // настроек и кламп значений проявляются без перезагрузки страницы.
    // Чтения локальных сигналов без подписки: иначе этот же эффект снапил
    // бы слайдер обратно прямо во время перетаскивания.
    Effect::new(move |_| {
        session.net_busy.get();
        let (ordinal, distribution, mode, radius) = session.state.with(|game| {
            (
                game.board_size().ordinal(),
                game.automatic_distribution(),
                game.explosion_mode(),
                game.safe_radius(),
            )
        });
        if size.get_untracked().round() as u8 != ordinal {
            size.set(f64::from(ordinal));
        }
        if automatic.get_untracked() != distribution {
            automatic.set(distribution);
        }
        if explosion_mode.get_untracked() != mode {
            explosion_mode.set(mode);
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

    Effect::new(move |previous: Option<ExplosionMode>| {
        let current = explosion_mode.get();
        if previous.is_some_and(|previous| previous != current) {
            let changed = session
                .state
                .with_untracked(|game| game.explosion_mode() != current);
            if changed {
                if session.room.get_untracked().is_some() {
                    session.net_set_explosion_mode(current);
                } else {
                    session.dispatch(move |game| {
                        game.set_explosion_mode(current);
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
    let reset_label = Memo::new(move |_| t_string!(i18n, reset_owner_on_zero).to_string());

    let reset_owner = RwSignal::new(explosion_mode.get_untracked().reset_owner_on_zero());

    // explosion_mode drives reset_owner for external changes (server snapshot)
    Effect::new(move |_| {
        let mode = explosion_mode.get();
        let reset = mode.reset_owner_on_zero();
        if reset_owner.get_untracked() != reset {
            reset_owner.set(reset);
        }
    });

    // reset_owner toggle updates the bool inside the current variant
    Effect::new(move |previous: Option<bool>| {
        let current = reset_owner.get();
        if previous.is_some_and(|previous| previous != current) {
            let mode = explosion_mode.get_untracked();
            let new_mode = mode.with_reset_owner(current);
            if mode != new_mode {
                explosion_mode.set(new_mode);
            }
        }
        current
    });

    let explosion_mode_str = RwSignal::new(explosion_mode.get_untracked().to_string());

    Effect::new(move |_| {
        let mode = explosion_mode.get();
        let key = mode.to_string();
        if explosion_mode_str.get_untracked() != key {
            explosion_mode_str.set(key);
        }
    });

    Effect::new(move |prev: Option<String>| {
        let key = explosion_mode_str.get();
        if prev.is_some_and(|p| p != key) {
            let reset = explosion_mode.get_untracked().reset_owner_on_zero();
            if let Some(mode) = ExplosionMode::from_key(&key, reset) {
                if explosion_mode.get_untracked() != mode {
                    explosion_mode.set(mode);
                }
            }
        }
        key
    });

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
                        <Select
                            class="settings-select"
                            size=SelectSize::Large
                            value=explosion_mode_str
                        >
                            <option
                                value="border_loss"
                                selected=move || explosion_mode_str.get() == "border_loss"
                            >
                                {t!(i18n, border_loss)}
                            </option>
                            <option
                                value="border_keep"
                                selected=move || explosion_mode_str.get() == "border_keep"
                            >
                                {t!(i18n, border_keep)}
                            </option>
                            <option
                                value="critical_mass"
                                selected=move || explosion_mode_str.get() == "critical_mass"
                            >
                                {t!(i18n, critical_mass)}
                            </option>
                        </Select>
                    </div>
                </div>

                <div class="settings-row settings-row--switch">
                    <span class="settings-row__label">{reset_label}</span>
                    <div class="settings-row__right">
                        <Switch checked=reset_owner />
                    </div>
                </div>
            </Card>
        </Show>
    }
}
