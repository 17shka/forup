use forup_core::api::game::prelude::{BoardCode, GameError, TeamId};
use forup_core::api::network::RoomError;
use leptos::prelude::*;
use leptos_router::hooks::use_navigate;
use thaw::{
    Button, ButtonAppearance, ButtonSize, Toast, ToastIntent, ToastOptions, ToastTitle,
    ToasterInjection,
};

use crate::auth::AuthState;
use crate::components::{GameSettingsPanel, RoomPlayersPanel, TeamsPanel};
use crate::game::{BoardView, Controller, GamePhase, GameSession, LocalGame, MoveMode, SlotMask};
use crate::i18n::*;
use crate::net::NetError;
use crate::settings::Settings;

/// Maps a display grid coordinate back to the board slot after `turn`
/// clockwise quarter-turns. The game state is untouched: only the rendering
/// rotates.
fn rotated_slot(turn: u8, x: u8, y: u8, width: u8, height: u8) -> u8 {
    match turn % 4 {
        1 => (height - 1 - x) * width + y,
        2 => (height - 1 - y) * width + (width - 1 - x),
        3 => x * width + (width - 1 - y),
        _ => y * width + x,
    }
}

/// Animation variant used when a cell grows or shrinks. Two equivalent names
/// are alternated through [`LocalGame::animation_tick`] so repeated changes
/// restart the CSS animation even when the class list would otherwise match.
fn cell_animation_class(tick: u64, growing: bool) -> &'static str {
    match (growing, tick % 2) {
        (true, 0) => "board-cell--grow-a",
        (true, _) => "board-cell--grow-b",
        (false, 0) => "board-cell--shrink-a",
        (false, _) => "board-cell--shrink-b",
    }
}

fn cell_class(view: &BoardView, pos: BoardCode, legal: SlotMask) -> String {
    let mut class = String::from("board-cell");
    if let Some(cell) = view.board.get(pos)
        && cell.owner.is_some()
    {
        class.push_str(" board-cell--owned");
        if view.phase == GamePhase::Playing && cell.owner != view.current_team {
            // Ход другой команды: её клетки показываются тусклее.
            class.push_str(" board-cell--dim");
        }
        if view.board.can_explode_at(pos, view.explosion_mode, *cell) {
            class.push_str(" board-cell--charged");
        }
    }
    if legal.contains(pos.slot()) {
        class.push_str(" board-cell--legal");
    }
    let delta = view.change_at(pos);
    if delta != 0 {
        class.push(' ');
        class.push_str(cell_animation_class(view.animation_tick, delta > 0));
    }
    class
}

/// The cell colour is the team hue; only its brightness changes with the
/// value (5% per point, so `4` is 20% lighter and `-2` is 10% darker).
fn cell_style(view: &BoardView, pos: BoardCode, hues: &[(TeamId, u16)]) -> String {
    let Some(cell) = view.board.get(pos) else {
        return String::new();
    };

    let threshold = view.board.cell_threshold(pos, view.explosion_mode);
    let value = cell.value;
    const MAX_DISTANCE: f32 = 4.0;

    let (border_start, border_end, border_opacity) = if value == 0 {
        (
            String::from("transparent"),
            String::from("transparent"),
            "0",
        )
    } else {
        let t = if value > 0 {
            let remaining = threshold as f32 - value as f32;
            1.0 - (remaining / MAX_DISTANCE).clamp(0.0, 1.0)
        } else {
            (value.abs() as f32 / MAX_DISTANCE).clamp(0.0, 1.0)
        };
        let color = if value > 0 {
            lerp_color(COLOR_GREEN, COLOR_RED, t)
        } else {
            lerp_color(COLOR_GREEN, COLOR_BLUE, t)
        };
        let opacity = if cell.owner.is_none() { "0.5" } else { "1" };
        (color.clone(), color, opacity)
    };

    let Some(owner) = cell.owner else {
        return format!(
            "--cell-border-start: {border_start}; --cell-border-end: {border_end}; \
             --cell-border-opacity: {border_opacity};"
        );
    };
    let hue = hues
        .iter()
        .find(|&&(team, _)| team == owner)
        .map(|&(_, hue)| hue)
        .unwrap_or(0);
    let brightness = (1.0 + 0.05 * f32::from(cell.value)).clamp(0.5, 1.8);
    format!(
        "--team-hue: {hue}; --cell-brightness: {brightness:.2}; \
         --cell-border-start: {border_start}; --cell-border-end: {border_end}; \
         --cell-border-opacity: {border_opacity};"
    )
}

fn cell_value(view: &BoardView, pos: BoardCode) -> String {
    view.board
        .get(pos)
        .filter(|cell| cell.value > 0)
        .map(|cell| cell.value.to_string())
        .unwrap_or_default()
}

const COLOR_GREEN: [u8; 3] = [0x22, 0xc5, 0x5e];
const COLOR_RED: [u8; 3] = [0xef, 0x44, 0x44];
const COLOR_BLUE: [u8; 3] = [0x3b, 0x82, 0xf6];

fn lerp_channel(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round() as u8
}

fn lerp_color(from: [u8; 3], to: [u8; 3], t: f32) -> String {
    let t = t.clamp(0.0, 1.0);
    let r = lerp_channel(from[0], to[0], t);
    let g = lerp_channel(from[1], to[1], t);
    let b = lerp_channel(from[2], to[2], t);
    format!("#{r:02x}{g:02x}{b:02x}")
}

#[component]
pub fn GamePage() -> impl IntoView {
    let i18n = use_i18n();
    let navigate = use_navigate();
    let session = GameSession::expect();
    let settings = Settings::expect();
    let auth = AuthState::expect();

    // The board can be resized in the lobby, so keep the size in a memo: the
    // grid is rebuilt only when the size changes, while every cell stays bound
    // to the reactive game state.
    let board_size = Memo::new(move |_| session.state.with(|game| game.board_size()));

    // Реактивные производные: считаются один раз на изменение состояния, а не
    // для каждой клетки доски. Иначе перерисовка большой доски была бы
    // квадратичной (легальные клетки, оттенки команд, доступность уменьшения).
    //
    // `board_view` намеренно не включает настройки: смена безопасного радиуса
    // не меняет снимок, поэтому значение мемо остаётся равным и ни одна из
    // сотен клеток не перерисовывается на тик ползунка.
    let board_view = Memo::new(move |_| session.state.with(LocalGame::board_view));
    let legal_targets = Memo::new(move |_| session.state.with(LocalGame::legal_targets));
    let team_hues = Memo::new(move |_| session.state.with(LocalGame::team_hues));
    let can_decrease = Memo::new(move |_| session.state.with(LocalGame::can_decrease));
    let choosing_start = Memo::new(move |_| session.state.with(LocalGame::is_choosing_start_cell));
    let animating = Memo::new(move |_| session.state.with(LocalGame::is_animating));

    let turn_text = move || {
        session.state.with(|game| match game.phase {
            GamePhase::Lobby => String::new(),
            GamePhase::Finished => match game.winner {
                Some(team) => t_string!(i18n, team_won, id = team.get()).to_string(),
                None => t_string!(i18n, game_over).to_string(),
            },
            GamePhase::Playing => match game.current_participant() {
                Some(participant) => match participant.controller {
                    Controller::Bot(number) => t_string!(i18n, turn_bot, id = number).to_string(),
                    Controller::Human => t_string!(i18n, your_turn).to_string(),
                    Controller::Remote(id) => {
                        let label = participant.name.clone().unwrap_or_else(|| format!("#{id}"));
                        t_string!(i18n, turn_player, id = label).to_string()
                    }
                },
                None => t_string!(i18n, game_over).to_string(),
            },
        })
    };

    // Текст хода сам несёт цвет команды текущего участника — отдельная
    // точка-индикатор не нужна.
    let turn_text_class = move || {
        session.state.with(|game| {
            if game.current_team().is_some() {
                "turn-banner__text turn-banner__text--team"
            } else {
                "turn-banner__text"
            }
        })
    };

    let turn_text_style = move || {
        session.state.with(|game| {
            game.current_team()
                .map(|team| format!("--team-hue: {};", game.team_hue(team)))
                .unwrap_or_default()
        })
    };

    let banner_class = move || {
        let mut class = String::from("turn-banner");
        session.state.with(|game| {
            let suffix: &'static str = match game.phase {
                GamePhase::Playing if game.is_human_turn() => "you",
                GamePhase::Playing => "other",
                _ => game.phase.into(),
            };
            class.push_str(&format!(" turn-banner--{}", suffix));
        });
        class
    };

    let error_text = move |error: &GameError| -> String {
        match error {
            GameError::GameNotStarted => t_string!(i18n, error_game_not_started).to_string(),
            GameError::CharacterNotFound => t_string!(i18n, error_character_not_found).to_string(),
            GameError::CharacterEliminated => {
                t_string!(i18n, error_character_eliminated).to_string()
            }
            GameError::NotYourTurn => t_string!(i18n, error_not_your_turn).to_string(),
            GameError::PositionOutOfBounds => t_string!(i18n, error_out_of_bounds).to_string(),
            GameError::FirstMoveRequiresEmptyCell => {
                t_string!(i18n, error_first_move_empty).to_string()
            }
            GameError::StartCellTooClose => t_string!(i18n, error_start_too_close).to_string(),
            GameError::CellNotOwnedByTeam => t_string!(i18n, error_cell_not_owned).to_string(),
            GameError::CellCannotBeDecreased => {
                t_string!(i18n, error_cell_cannot_decrease).to_string()
            }
            _ => t_string!(i18n, error_generic).to_string(),
        }
    };

    // Surface rejected moves as toasts instead of a persistent inline message.
    let toaster = ToasterInjection::expect_context();
    Effect::new(move |previous: Option<Option<GameError>>| {
        let current = session.error.get();
        if let Some(error) = current
            && previous.flatten() != Some(error)
        {
            let message = error_text(&error);
            toaster.dispatch_toast(
                move || {
                    view! {
                        <Toast>
                            <ToastTitle>{message}</ToastTitle>
                        </Toast>
                    }
                },
                ToastOptions::default().with_intent(ToastIntent::Error),
            );
            session.error.set(None);
        }
        current
    });

    // Сетевые ошибки комнаты — тем же тостом. Тюнинг сообщения зависит от
    // варианта: ошибки движка переиспользуют локальные тексты.
    let net_error_text = move |error: &NetError| -> String {
        match error {
            NetError::Room(RoomError::Game(e)) => error_text(e),
            NetError::Room(RoomError::MembersNotReady) => {
                t_string!(i18n, error_members_not_ready).to_string()
            }
            NetError::Room(RoomError::NotFound) => {
                t_string!(i18n, error_room_not_found).to_string()
            }
            NetError::Room(RoomError::Closed) => t_string!(i18n, error_room_closed).to_string(),
            NetError::Room(_) => t_string!(i18n, error_generic).to_string(),
            NetError::Kicked => t_string!(i18n, error_kicked).to_string(),
            NetError::Unauthorized | NetError::Transport(_) => {
                t_string!(i18n, error_connection).to_string()
            }
        }
    };
    Effect::new(move |previous: Option<Option<NetError>>| {
        let current = session.net_error.get();
        if previous.flatten() != current
            && let Some(error) = current.clone()
        {
            let message = net_error_text(&error);
            toaster.dispatch_toast(
                move || {
                    view! {
                        <Toast>
                            <ToastTitle>{message}</ToastTitle>
                        </Toast>
                    }
                },
                ToastOptions::default().with_intent(ToastIntent::Error),
            );
            session.net_error.set(None);
        }
        current
    });

    // Цикл опроса комнаты: каждый снапшот перезаписывает `room` (set всегда
    // уведомляет), эффект перезапускает цепочку таймеров; смена/потеря
    // комнаты или размонтирование страницы гасят её через поколение.
    Effect::new(move |_| {
        let room = session.room.get();
        session.bump_poll();
        if room.is_some() {
            session.schedule_room_poll();
        }
    });
    on_cleanup(move || session.bump_poll());

    // Кнопки, работающие в обоих режимах: онлайн — серверной командой,
    // офлайн — прямым вызовом движка.
    fn act(
        session: &GameSession,
        net: fn(&GameSession),
        local: fn(&mut LocalGame) -> Result<(), GameError>,
    ) {
        if session.room.get_untracked().is_some() {
            net(session);
        } else {
            session.dispatch(local);
        }
    }

    let on_exit = move |_| {
        if session.room.get_untracked().is_some() {
            session.leave_room();
        } else {
            session.reset();
        }
        navigate("/", Default::default());
    };
    let on_skip = move |_| act(&session, GameSession::net_skip, LocalGame::skip);
    let on_surrender = move |_| act(&session, GameSession::net_surrender, LocalGame::surrender);
    // Офлайн-игрок — администратор собственного стола.
    let is_admin = move || session.room.get().is_none_or(|room| room.is_admin);
    let on_finish = move |_| act(&session, GameSession::net_end_game, LocalGame::end_game);
    let on_ready = move |_| {
        let finished = session
            .state
            .with_untracked(|game| game.phase == GamePhase::Finished);
        if session.room.get_untracked().is_some() {
            // Онлайн: кнопка работает как переключатель готовности для всех —
            // и админа, и участника. Повторное нажатие снимает готовность.
            // «Заново» после конца партии — тоже эта кнопка: локальное
            // предсказание подтверждается сервером.
            if finished {
                session.net_rematch();
            } else {
                let ready = session.state.with_untracked(LocalGame::own_ready) != Some(true);
                session.net_set_ready(ready);
            }
        } else if finished {
            session.dispatch(LocalGame::rematch);
        } else {
            session.dispatch(LocalGame::ready);
        }
    };

    let ready_label = move || {
        session.state.with(|game| {
            if game.phase == GamePhase::Finished {
                t_string!(i18n, rematch).to_string()
            } else if session.room.get().is_some() && game.own_ready() == Some(true) {
                t_string!(i18n, waiting_ready).to_string()
            } else {
                t_string!(i18n, ready).to_string()
            }
        })
    };

    // Зелёная пока не готов / красная у готового (повторное нажатие снимает).
    let ready_on = move || {
        session.room.get().is_none()
            || session.state.with(|game| game.phase == GamePhase::Finished)
            || session.state.with(LocalGame::own_ready) == Some(true)
    };

    // Переключатель режима хода — сегментированный, как режим взрывов в
    // настройках партии: активная половина подсвечена градиентом своего цвета.
    let mode_label = move |mode: MoveMode| match mode {
        MoveMode::Increase => t_string!(i18n, increase).to_string(),
        MoveMode::Decrease => t_string!(i18n, decrease).to_string(),
    };
    let mode_class = move |mode: MoveMode| {
        let suffix: &'static str = mode.into();
        let mut class = format!("mode-toggle__option mode-toggle__option--{}", suffix);
        if session.mode.get() == mode {
            class.push_str(" mode-toggle__option--active");
        }
        class
    };
    let decrease_label = move || mode_label(MoveMode::Decrease);
    let increase_label = move || mode_label(MoveMode::Increase);
    let decrease_class = move || mode_class(MoveMode::Decrease);
    let increase_class = move || mode_class(MoveMode::Increase);
    // Уменьшение недоступно (нет подходящих клеток): половина «Уменьшить»
    // блокируется реактивно, как раньше блокировался чекбокс.
    let decrease_disabled = move || !session.state.with(LocalGame::can_decrease);
    // Проверка в обработчике — страховка от смены состояния can_decrease
    // между отрисовкой кнопки и кликом.
    let on_decrease = move |_| {
        if session.state.with_untracked(LocalGame::can_decrease) {
            session.mode.set(MoveMode::Decrease);
        }
    };
    let on_increase = move |_| session.mode.set(MoveMode::Increase);

    // A start cell is claimed with Increase by default; while one is being
    // picked the mode checkbox stays hidden, so snap the mode back here.
    Effect::new(move |previous: Option<bool>| {
        let choosing = session.state.with(|game| game.is_choosing_start_cell());
        if choosing && !previous.unwrap_or(false) {
            session.mode.set(MoveMode::Increase);
        }
        choosing
    });

    // Насос анимации взрывов: пока ход доигрывается, планирует шаги
    // `animation_step` с задержкой из настроек. Ползунок в drawer настроек
    // отслеживается, поэтому смена настройки применяется сразу: ожидающийся
    // шаг отменяется и переносится на новую задержку. Задержка 0 — мгновенный
    // режим: ход доигрывается целиком без пауз. Движок ничего не ждёт.
    let pump_generation = RwSignal::new(0u64);
    Effect::new(move || {
        let delay = settings.explosion_delay.get();
        // Каждый перезапуск отменяет запланированный шаг: он сработает впустую.
        let generation = pump_generation.get_untracked() + 1;
        pump_generation.set(generation);

        if !session.state.with(LocalGame::is_animating) {
            // Анимация закончилась — применяем снапшот, отложенный на её
            // время, чтобы чужие ходы не обрывались одиночным флешем.
            session.flush_pending_snapshot();
            return;
        }
        if delay == 0.0 {
            session.dispatch(|game| {
                while game.animation_step()? {}
                // Ход доигран без показа кадров — финальную волну
                // анимировать не нужно.
                game.reset_animation();
                Ok(())
            });
            return;
        }
        set_timeout(
            move || {
                if pump_generation
                    .try_get_untracked()
                    .map_or(false, |g| g == generation)
                {
                    session.dispatch(|game| game.animation_step().map(|_| ()));
                }
            },
            std::time::Duration::from_secs_f64(delay),
        );
    });

    view! {
        <div class="page game-page">
            // Две колонки: игровое поле и панель управления столом.
            // Баннер хода — в панели, над командами.
            // В портрете колонки «расплющиваются» (display: contents), а
            // прежний порядок восстанавливается CSS-свойством order.
            <div class="game-column game-column--board">
                <div class="board-wrapper">
                    {move || {
                        let size = board_size.get();
                        let rotation = session.rotation.get() % 4;
                        let board_width = size.width().get();
                        let board_height = size.height().get();
                        let (cols, rows) = if rotation.is_multiple_of(2) {
                            (board_width, board_height)
                        } else {
                            (board_height, board_width)
                        };
                        let cell_count = usize::from(cols) * usize::from(rows);
                        let cells = (0..cell_count)
                            .map(|index| {
                                let x = (index % usize::from(cols)) as u8;
                                let y = (index / usize::from(cols)) as u8;
                                let slot =
                                    rotated_slot(rotation, x, y, board_width, board_height);
                                let pos = BoardCode::from_slot(slot);
                                let mode = session.mode;

                                // Легальные клетки и оттенки команд берутся из
                                // мемо: один расчёт на состояние вместо расчёта
                                // в каждой клетке доски.
                                let class = move || {
                                    let legal = legal_targets.get();
                                    board_view.with(|view| cell_class(view, pos, legal))
                                };
                                let style = move || {
                                    let hues = team_hues.get();
                                    board_view.with(|view| cell_style(view, pos, &hues))
                                };
                                let value = move || board_view.with(|view| cell_value(view, pos));
                                let disabled = move || {
                                    let selected = mode.get();
                                    let legal = legal_targets.get();
                                    // Клетки заблокированы, пока идёт анимация взрывов.
                                    if animating.get() {
                                        return true;
                                    }
                                    // A starting Decrease just claims an empty
                                    // cell, so the usual checks do not apply.
                                    if selected == MoveMode::Decrease && choosing_start.get() {
                                        return !legal.contains(pos.slot());
                                    }
                                    let empty_decrease = selected == MoveMode::Decrease
                                        && board_view
                                            .with(|view| view.cell(pos).is_some_and(|cell| cell.value <= 0));
                                    !legal.contains(pos.slot())
                                        || empty_decrease
                                        || (selected == MoveMode::Decrease && !can_decrease.get())
                                };
                                let on_click = move |_| {
                                    let selected = mode.get_untracked();
                                    if session.room.get_untracked().is_some() {
                                        session.net_move(pos, selected);
                                    } else {
                                        session.dispatch(move |game| {
                                            game.make_move(pos, selected)
                                        });
                                    }
                                };

                                view! {
                                    <button
                                        type="button"
                                        class=class
                                        style=style
                                        disabled=disabled
                                        on:click=on_click
                                    >
                                        <span class="board-cell__value">{value}</span>
                                    </button>
                                }
                            })
                            .collect_view();

                        let board_class = move || {
                            if settings.show_cell_borders.get() {
                                "board board--borders"
                            } else {
                                "board board--no-borders"
                            }
                        };
                        view! {
                            <div
                                class=board_class
                                style=format!(
                                    "--board-cols: {cols}; --board-rows: {rows}; aspect-ratio: {cols} / {rows};"
                                )
                            >
                                {cells}
                            </div>
                        }
                    }}
                </div>
            </div>

            <div class="game-column game-column--side">
                // Прокручиваемая верхняя часть панели: игроки, ход, команды
                // и настройки. Кнопки действий закреплены снизу.
                <div class="game-column__scroll">
                    <Show when=move || session.room.get().is_some()>
                        <RoomPlayersPanel/>
                    </Show>

                    <Show when=move || session.state.with(|game| game.phase != GamePhase::Lobby)>
                        <div class=banner_class>
                            <span class=turn_text_class style=turn_text_style>{turn_text}</span>
                            <div class="turn-banner__side"></div>
                        </div>
                    </Show>

                    <TeamsPanel/>

                    // Выбор режима и настройки — только когда стол локальный или
                    // у игрока есть права администратора комнаты.
                    <Show when=move || {
                        session.state.with(|game| game.phase == GamePhase::Lobby)
                            && (session.room.get().is_none()
                                || session.room.get().is_some_and(|room| room.is_admin))
                    }>
                        <div class="room-mode" role="group">
                            {move || {
                                let offline_active = session.room.get().is_none();
                                view! {
                                    <button
                                        type="button"
                                        class=if offline_active {
                                            "room-mode__button room-mode__button--active"
                                        } else {
                                            "room-mode__button"
                                        }
                                        on:click=move |_| {
                                            if session.room.get_untracked().is_some() {
                                                // Переключение в офлайн сохраняет стол:
                                                // участники и настройки не меняются.
                                                session.go_offline();
                                            }
                                        }
                                    >
                                        {t!(i18n, offline)}
                                    </button>
                                    <button
                                        type="button"
                                        class=if offline_active {
                                            "room-mode__button"
                                        } else {
                                            "room-mode__button room-mode__button--active"
                                        }
                                        disabled=move || auth.user.get().is_none()
                                        title=move || {
                                            if auth.user.get().is_some() {
                                                String::new()
                                            } else {
                                                t_string!(i18n, multiplayer_requires_account).to_string()
                                            }
                                        }
                                        on:click=move |_| {
                                            if session.room.get_untracked().is_none() {
                                                let token = auth.token.get_untracked();
                                                session.connect_new_room(&token, true, || {});
                                            }
                                        }
                                    >
                                        {t!(i18n, online)}
                                    </button>
                                }
                            }}
                        </div>
                    </Show>

                    <GameSettingsPanel/>
                </div>

                <Show
                    when=move || {
                        session.state.with(|game| {
                            game.phase == GamePhase::Playing
                                && !game.is_choosing_start_cell()
                                && game.is_human_turn()
                        })
                    }
                >
                    <div class="mode-toggle mode-toggle--actions" role="group">
                        <button
                            type="button"
                            class=decrease_class
                            aria-pressed=move || {
                                (session.mode.get() == MoveMode::Decrease)
                                    .to_string()
                            }
                            aria-label=decrease_label
                            title=decrease_label
                            disabled=decrease_disabled
                            on:click=on_decrease
                        >
                            <span
                                class="mode-toggle__dot mode-toggle__dot--decrease"
                                aria-hidden="true"
                            ></span>
                            <span class="mode-toggle__text">{decrease_label}</span>
                        </button>
                        <button
                            type="button"
                            class=increase_class
                            aria-pressed=move || {
                                (session.mode.get() == MoveMode::Increase)
                                    .to_string()
                            }
                            aria-label=increase_label
                            title=increase_label
                            on:click=on_increase
                        >
                            <span
                                class="mode-toggle__dot mode-toggle__dot--increase"
                                aria-hidden="true"
                            ></span>
                            <span class="mode-toggle__text">{increase_label}</span>
                        </button>
                    </div>
                </Show>

                <div class="game-actions">
                    <Button
                        class="game-action game-action--danger"
                        appearance=ButtonAppearance::Secondary
                        on_click=on_exit
                    >
                        <span class="game-action__icon" aria-hidden="true">"⎋"</span>
                        <span class="game-action__label">{t!(i18n, exit)}</span>
                    </Button>
                    <Show when=move || session.state.with(LocalGame::has_playable_seat)>
                        <Button
                            class="game-action game-action--danger"
                            appearance=ButtonAppearance::Secondary
                            disabled=move || !session.state.with(LocalGame::can_surrender)
                            on_click=on_surrender
                        >
                            <span class="game-action__icon" aria-hidden="true">"🏳"</span>
                            <span class="game-action__label">{t!(i18n, surrender)}</span>
                        </Button>
                    </Show>
                    <Show when=move || {
                        session.state.with(|game| {
                            game.phase == GamePhase::Playing
                                && !game.has_playable_seat()
                                && is_admin()
                        })
                    }>
                        <Button
                            class="game-action game-action--danger"
                            appearance=ButtonAppearance::Secondary
                            on_click=on_finish
                        >
                            <span class="game-action__icon" aria-hidden="true">"🏁"</span>
                            <span class="game-action__label">{t!(i18n, finish_game)}</span>
                        </Button>
                    </Show>
                    <Show when=move || session.state.with(LocalGame::is_human_turn)>
                        <Button
                            class="game-action"
                            appearance=ButtonAppearance::Secondary
                            disabled=move || !session.state.with(LocalGame::can_skip)
                            on_click=on_skip
                        >
                            <span class="game-action__icon" aria-hidden="true">"⏭"</span>
                            <span class="game-action__label">{t!(i18n, skip)}</span>
                        </Button>
                    </Show>
                    <Show when=move || session.state.with(|game| game.phase != GamePhase::Playing)>
                        <Button
                            class=Signal::derive(move || {
                                if ready_on() {
                                    "game-action game-action--ready game-action--ready-on"
                                } else {
                                    "game-action game-action--ready game-action--ready-off"
                                }
                                .to_string()
                            })
                            appearance=ButtonAppearance::Primary
                            size=ButtonSize::Medium
                            on_click=on_ready
                        >
                            <span class="game-action__icon" aria-hidden="true">"✓"</span>
                            <span class="game-action__label">{ready_label}</span>
                        </Button>
                    </Show>
                    <Button
                        class="game-action"
                        appearance=ButtonAppearance::Secondary
                        on_click=move |_| session.rotate_board()
                    >
                        <span class="game-action__icon" aria-hidden="true">"⟳"</span>
                        <span class="game-action__label">{t!(i18n, rotate_board)}</span>
                    </Button>
                </div>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::rotated_slot;

    /// Original 3×2 board:
    /// ```text
    /// 0 1 2
    /// 3 4 5
    /// ```
    #[test]
    fn clockwise_rotation_maps_the_top_left_cell() {
        // 90°: the display's top-left is the original bottom-left cell.
        assert_eq!(rotated_slot(1, 0, 0, 3, 2), 3);
        assert_eq!(rotated_slot(1, 1, 0, 3, 2), 0);
        // 180°: the original bottom-right cell.
        assert_eq!(rotated_slot(2, 0, 0, 3, 2), 5);
        // 270°: the original top-right cell.
        assert_eq!(rotated_slot(3, 0, 0, 3, 2), 2);
    }

    #[test]
    fn four_quarter_turns_return_to_the_original_layout() {
        for y in 0..2 {
            for x in 0..3 {
                assert_eq!(rotated_slot(4, x, y, 3, 2), rotated_slot(0, x, y, 3, 2));
            }
        }
    }
}
