use forup_core::api::game::board::prelude::GameError;
use glam::U8Vec2;
use leptos::prelude::*;
use leptos_router::hooks::use_navigate;
use thaw::{
    Button, ButtonAppearance, ButtonSize, Toast, ToastIntent, ToastOptions, ToastTitle,
    ToasterInjection,
};

use crate::components::{GameSettingsPanel, TeamsPanel};
use crate::game::{Controller, GamePhase, GameSession, LocalGame, MoveMode};
use crate::i18n::*;

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

fn cell_class(state: &LocalGame, pos: U8Vec2) -> String {
    let mut class = String::from("board-cell");
    if let Some(cell) = state.game.board.get(pos) {
        if cell.owner.is_some() {
            class.push_str(" board-cell--owned");
            if cell.can_explode() {
                class.push_str(" board-cell--charged");
            }
        }
    }
    if state.can_move_to(pos) {
        class.push_str(" board-cell--legal");
    }
    let delta = state.change_at(pos);
    if delta != 0 {
        class.push(' ');
        class.push_str(cell_animation_class(state.animation_tick, delta > 0));
    }
    class
}

/// The cell colour is the team hue; only its brightness changes with the
/// value (5% per point, so `4` is 20% lighter and `-2` is 10% darker).
fn cell_style(state: &LocalGame, pos: U8Vec2) -> String {
    let Some(cell) = state.game.board.get(pos) else {
        return String::new();
    };
    let Some(owner) = cell.owner else {
        return String::new();
    };
    let hue = state.team_hue(owner);
    let brightness = (1.0 + 0.05 * f32::from(cell.value)).clamp(0.5, 1.8);
    format!("--team-hue: {hue}; --cell-brightness: {brightness:.2};")
}

fn cell_value(state: &LocalGame, pos: U8Vec2) -> String {
    state
        .game
        .board
        .get(pos)
        .filter(|cell| cell.value > 0)
        .map(|cell| cell.value.to_string())
        .unwrap_or_default()
}

#[component]
pub fn GamePage() -> impl IntoView {
    let i18n = use_i18n();
    let navigate = use_navigate();
    let session = GameSession::expect();

    // The board can be resized in the lobby, so keep the size in a memo: the
    // grid is rebuilt only when the size changes, while every cell stays bound
    // to the reactive game state.
    let board_size = Memo::new(move |_| session.state.with(|game| game.board_size()));

    let turn_text = move || {
        session.state.with(|game| match game.phase {
            GamePhase::Lobby => String::new(),
            GamePhase::Finished => match game.winner {
                Some(team) => t_string!(i18n, team_won, id = team.get()).to_string(),
                None => t_string!(i18n, game_over).to_string(),
            },
            GamePhase::Playing => match game
                .current_character()
                .and_then(|character| game.character_participant(character))
            {
                Some((_, Controller::Bot(number))) => {
                    t_string!(i18n, turn_bot, id = number).to_string()
                }
                Some((_, Controller::Human)) => t_string!(i18n, your_turn).to_string(),
                None => t_string!(i18n, game_over).to_string(),
            },
        })
    };

    let turn_dot_class = move || {
        session.state.with(|game| {
            if game.current_team().is_some() {
                "turn-banner__dot"
            } else {
                "turn-banner__dot turn-banner__dot--idle"
            }
        })
    };

    let turn_dot_style = move || {
        session.state.with(|game| {
            game.current_team()
                .map(|team| format!("--team-hue: {};", game.team_hue(team)))
                .unwrap_or_default()
        })
    };

    let banner_class = move || {
        let mut class = String::from("turn-banner");
        session.state.with(|game| match game.phase {
            GamePhase::Lobby => class.push_str(" turn-banner--lobby"),
            GamePhase::Playing if game.is_human_turn() => class.push_str(" turn-banner--you"),
            GamePhase::Playing => class.push_str(" turn-banner--other"),
            GamePhase::Finished => class.push_str(" turn-banner--over"),
        });
        class
    };

    let error_text = move |error: GameError| -> String {
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
        if let Some(error) = current {
            if previous.flatten() != Some(error) {
                let message = error_text(error);
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
        }
        current
    });

    let on_exit = move |_| {
        session.reset();
        navigate("/", Default::default());
    };
    let on_skip = move |_| session.dispatch(LocalGame::skip);
    let on_surrender = move |_| session.dispatch(LocalGame::surrender);
    let on_ready = move |_| session.dispatch(LocalGame::ready);

    let ready_label = move || {
        session.state.with(|game| {
            if game.phase == GamePhase::Finished {
                t_string!(i18n, rematch).to_string()
            } else {
                t_string!(i18n, ready).to_string()
            }
        })
    };

    let mode_class = move |target: MoveMode| {
        let mut class = match target {
            MoveMode::Increase => String::from("mode-action mode-action--increase"),
            MoveMode::Decrease => String::from("mode-action mode-action--decrease"),
        };
        if session.mode.get() == target {
            class.push_str(" mode-action--active");
        }
        class
    };

    view! {
        <div class="page game-page">
            <div class="board-wrapper">
                {move || {
                    let size = board_size.get();
                    let width = u16::from(size.x);
                    let height = u16::from(size.y);
                    let cell_count = usize::from(width) * usize::from(height);
                    let cells = (0..cell_count)
                        .map(|index| {
                            let index = u16::try_from(index).unwrap_or_default();
                            let pos = U8Vec2::new((index % width) as u8, (index / width) as u8);
                            let mode = session.mode;

                            let class = move || session.state.with(|game| cell_class(game, pos));
                            let style = move || session.state.with(|game| cell_style(game, pos));
                            let value = move || session.state.with(|game| cell_value(game, pos));
                            let disabled = move || {
                                let selected = mode.get();
                                session.state.with(|game| {
                                    let empty_decrease = selected == MoveMode::Decrease
                                        && game
                                            .game
                                            .board
                                            .get(pos)
                                            .is_some_and(|cell| cell.value <= 0);
                                    !game.can_move_to(pos)
                                        || empty_decrease
                                        || (selected == MoveMode::Decrease && !game.can_decrease())
                                })
                            };
                            let on_click = move |_| {
                                let selected = mode.get_untracked();
                                session.dispatch(move |game| game.make_move(pos, selected));
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

                    view! {
                        <div
                            class="board"
                            style=format!(
                                "--board-cols: {width}; --board-rows: {height}; aspect-ratio: {width} / {height};"
                            )
                        >
                            {cells}
                        </div>
                    }
                }}
            </div>

            <Show when=move || session.state.with(|game| game.phase != GamePhase::Lobby)>
                <div class="turn-controls">
                    <div class=banner_class>
                        <span class=turn_dot_class style=turn_dot_style></span>
                        <span class="turn-banner__text">{turn_text}</span>
                    </div>
                    <Show
                        when=move || session.state.with(|game| game.phase == GamePhase::Playing)
                    >
                        <div class="mode-toggle" role="group">
                            <Button
                                class=Signal::derive(move || mode_class(MoveMode::Decrease))
                                size=ButtonSize::Small
                                appearance=ButtonAppearance::Transparent
                                disabled=move || !session.state.with(LocalGame::can_decrease)
                                on_click=move |_| session.mode.set(MoveMode::Decrease)
                            >
                                <span class="mode-action__icon" aria-hidden="true">"−"</span>
                                <span>{t!(i18n, decrease)}</span>
                            </Button>
                            <Button
                                class=Signal::derive(move || mode_class(MoveMode::Increase))
                                size=ButtonSize::Small
                                appearance=ButtonAppearance::Transparent
                                on_click=move |_| session.mode.set(MoveMode::Increase)
                            >
                                <span class="mode-action__icon" aria-hidden="true">"＋"</span>
                                <span>{t!(i18n, increase)}</span>
                            </Button>
                        </div>
                    </Show>
                </div>
            </Show>

            <div class="game-actions">
                <Button
                    class="game-action"
                    appearance=ButtonAppearance::Subtle
                    on_click=on_exit
                >
                    <span class="game-action__icon" aria-hidden="true">"⎋"</span>
                    <span class="game-action__label">{t!(i18n, exit)}</span>
                </Button>
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
                <Show when=move || session.state.with(|game| game.phase == GamePhase::Playing)>
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
                <Show when=move || session.state.with(|game| game.phase != GamePhase::Playing)>
                    <Button
                        class="game-action game-action--ready"
                        appearance=ButtonAppearance::Primary
                        size=ButtonSize::Medium
                        disabled=move || !session.state.with(LocalGame::can_ready)
                        on_click=on_ready
                    >
                        <span class="game-action__icon" aria-hidden="true">"✓"</span>
                        <span class="game-action__label">{ready_label}</span>
                    </Button>
                </Show>
            </div>

            <GameSettingsPanel/>

            <TeamsPanel/>
        </div>
    }
}
