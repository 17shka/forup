use forup_core::api::game::board::prelude::{MemberId, TeamId};
use forup_core::api::network::Uuid;
use leptos::prelude::*;
use thaw::{Button, ButtonAppearance, ButtonSize, Card, CardHeader, Select, SelectSize};

use crate::game::{Controller, GamePhase, GameSession, TeamParticipantView, TeamView};
use crate::i18n::*;

/// Single bottom panel for both the lobby and the match: team cards with their
/// participants and controller/team editing while the game has not started.
#[component]
pub fn TeamsPanel() -> impl IntoView {
    let i18n = use_i18n();
    let session = GameSession::expect();

    view! {
        <section class="teams-panel">
            <div class="teams-panel__head">
                <h2 class="section-title">{t!(i18n, teams)}</h2>
                <Show when=move || {
                    session.state.with(|game| game.phase != GamePhase::Playing)
                        && session
                            .room
                            .get()
                            .map(|room| room.is_admin)
                            .unwrap_or(true)
                }>
                    <Button
                        class="teams-panel__add"
                        size=ButtonSize::Small
                        appearance=ButtonAppearance::Primary
                        on_click=move |_| {
                            if session.room.get_untracked().is_some() {
                                session.net_add_bot();
                            } else {
                                session.dispatch(|game| game.add_bot().map(|_| ()));
                            }
                        }
                    >
                        <span aria-hidden="true">"＋"</span>
                        <span>{t!(i18n, add_member)}</span>
                    </Button>
                </Show>
            </div>

            <div class="team-cards">
                {move || {
                    session
                        .state
                        .with(|game| game.team_views())
                        .into_iter()
                        .map(|team| view! { <TeamCard team/> })
                        .collect_view()
                }}
            </div>
        </section>
    }
}

#[component]
fn TeamCard(team: TeamView) -> impl IntoView {
    let i18n = use_i18n();
    let hue = team.hue;
    let team_id = team.team.get();
    let cells = team.cells;
    let mass = team.mass;
    // Команда, в которой все участники проиграли, показывается тусклее.
    let wrap_class = if team.lost {
        "team-card-wrap team-card-wrap--lost"
    } else {
        "team-card-wrap"
    };

    let rows = team
        .participants
        .into_iter()
        .map(|member| view! { <ParticipantRow member/> })
        .collect_view();

    view! {
        <div class=wrap_class style=format!("--team-hue: {hue};")>
            <Card class="team-card">
                <CardHeader>
                    <span class="team-id-chip">"#" {team_id}</span>
                    <span class="team-card__summary">
                        {move || format!(
                            "{}: {cells} · {}: {mass}",
                            t_string!(i18n, cells),
                            t_string!(i18n, mass),
                        )}
                    </span>
                </CardHeader>
                <div class="team-card__rows">{rows}</div>
            </Card>
        </div>
    }
}

#[component]
fn ParticipantRow(member: TeamParticipantView) -> impl IntoView {
    let i18n = use_i18n();
    let session = GameSession::expect();
    let participant_id = member.id;
    let lost = member.lost;
    // Отличия онлайн-строки: имя и готовность приходят с сервера, место
    // редактируется только командой (или кикается администратором).
    let controller = member.controller;
    let name = member.name;
    let ready = member.ready;
    let first_character = member.characters.first().copied();
    // Метка имени — Memo (Copy-хэндл): строку нельзя переносить в children-
    // замыкания, а копию хэндла — можно.
    let name_label = Memo::new(move |_| match controller {
        Controller::Human => name
            .clone()
            .unwrap_or_else(|| t_string!(i18n, local_player).to_string()),
        Controller::Bot(number) => t_string!(i18n, bot_member, id = number).to_string(),
        Controller::Remote(id) => name.clone().unwrap_or_else(|| format!("#{id}")),
    });

    let editable = move || session.state.with(|game| game.phase != GamePhase::Playing);
    let remove_title = Memo::new(move |_| t_string!(i18n, remove).to_string());

    // The selects are controlled by the game state: the read side always
    // reflects the participant, and the write side ignores the first
    // programmatic value an element writes back on mount. This prevents a row
    // from moving another participant to a different team while the list is
    // rebuilt.
    let controller_read = Signal::derive(move || {
        session.state.with(|game| {
            game.participant_controller(participant_id)
                .unwrap_or(Controller::Human)
                .key()
        })
    });
    let controller_ready = RwSignal::new(false);
    let controller_write = SignalSetter::map(move |value: String| {
        if !controller_ready.get_untracked() {
            controller_ready.set(true);
            return;
        }
        let Some(controller) = Controller::from_key(&value) else {
            return;
        };
        let current = session
            .state
            .with_untracked(|game| game.participant_controller(participant_id));
        if current != Some(controller) {
            session
                .dispatch(move |game| game.set_participant_controller(participant_id, controller));
        }
    });

    let team_read = Signal::derive(move || {
        session.state.with(|game| {
            game.participant_team(participant_id)
                .map(|team| team.get().to_string())
                .unwrap_or_default()
        })
    });
    let team_ready = RwSignal::new(false);
    let team_write = SignalSetter::map(move |value: String| {
        if !team_ready.get_untracked() {
            team_ready.set(true);
            return;
        }
        let Ok(raw) = value.parse::<u8>() else {
            return;
        };
        let Some(team) = TeamId::new(raw) else {
            return;
        };
        let current = session
            .state
            .with_untracked(|game| game.participant_team(participant_id));
        if current != Some(team) {
            if session.room.get_untracked().is_some() {
                if let Some(character) = first_character {
                    session.net_change_team(character, team);
                }
            } else {
                session.dispatch(move |game| game.set_participant_team(participant_id, team));
            }
        }
    });

    // В онлайн-комнате убрать место может только администратор — любое,
    // включая своё: владелец остаётся оператором комнаты без места.
    let can_remove = move || match session.room.get().map(|room| room.is_admin) {
        Some(true) => true,
        Some(false) => false,
        None => true,
    };

    // Порядок ходов меняет только администратор (в офлайне — сам стол).
    let can_edit_order = can_remove;

    // Селектор контроллера онлайн-места: администратор решает, кто играет
    // за место — конкретный участник комнаты или бот с номером («Бот 1»,
    // «Бот 2», …), как в офлайне. Ключи — `player{N}` для аккаунтов и
    // `bot{N}` для ботов.
    let seat_read = Signal::derive(move || {
        session
            .state
            .with(|game| game.seat_controller_key(participant_id))
    });
    let seat_ready = RwSignal::new(false);
    let seat_write = SignalSetter::map(move |value: String| {
        if !seat_ready.get_untracked() {
            seat_ready.set(true);
            return;
        }
        let current = session
            .state
            .with_untracked(|game| game.seat_controller_key(participant_id));
        if current == value {
            return;
        }
        let controller = if value
            .strip_prefix("bot")
            .is_some_and(|number| number.parse::<u8>().is_ok())
        {
            MemberId::Bot
        } else {
            match value
                .strip_prefix("player")
                .and_then(|number| number.parse::<u32>().ok())
                .and_then(Uuid::new)
            {
                Some(player) => MemberId::Player(player),
                None => return,
            }
        };
        if let Some(character) = first_character {
            session.net_set_controller(character, controller);
        }
    });

    let seat_options = move || {
        let mut options: Vec<(String, String)> = session
            .state
            .with(|game| game.players.clone())
            .into_iter()
            .map(|player| {
                let label = player
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("#{}", player.id));
                (format!("player{}", player.id), label)
            })
            .collect();
        for number in session.state.with(|game| game.seat_bot_options()) {
            options.push((
                format!("bot{number}"),
                t_string!(i18n, bot_member, id = number).to_string(),
            ));
        }
        options
            .into_iter()
            .map(|(key, label)| {
                let selected_key = key.clone();
                view! {
                    <option value=key selected=move || seat_read.get() == selected_key>
                        {label}
                    </option>
                }
            })
            .collect_view()
    };

    let controller_options = move || {
        session
            .state
            .with(|game| game.controller_options())
            .into_iter()
            .map(|controller| {
                let key = controller.key();
                let selected_key = key.clone();
                let label = match controller {
                    Controller::Human => t_string!(i18n, controller_player).to_string(),
                    Controller::Bot(number) => t_string!(i18n, bot_member, id = number).to_string(),
                    // Remote-контроллеры сюда не попадают (в офлайне их нет),
                    // поэтому имя не нужно — замыкание остаётся Copy.
                    Controller::Remote(id) => format!("#{id}"),
                };
                view! {
                    <option value=key selected=move || controller_read.get() == selected_key>
                        {label}
                    </option>
                }
            })
            .collect_view()
    };

    let team_options = move || {
        session
            .state
            .with(|game| game.team_options())
            .into_iter()
            .map(|option| {
                let code = option.get().to_string();
                let selected_code = code.clone();
                let id = option.get();
                view! {
                    <option value=code selected=move || team_read.get() == selected_code>
                        {id}
                    </option>
                }
            })
            .collect_view()
    };

    // Селектор позиции в последовательности ходов: значения 1..N, смена
    // номера меняет участника местами с тем, кто стоит на выбранной позиции.
    let order_read = Signal::derive(move || {
        session.state.with(|game| {
            game.turn_position(participant_id)
                .map(|position| (position + 1).to_string())
                .unwrap_or_default()
        })
    });
    let order_ready = RwSignal::new(false);
    let order_write = SignalSetter::map(move |value: String| {
        if !order_ready.get_untracked() {
            order_ready.set(true);
            return;
        }
        let Some(position) = value.parse::<usize>().ok().filter(|p| *p > 0) else {
            return;
        };
        let target = position - 1;
        let current = session
            .state
            .with_untracked(|game| game.turn_position(participant_id));
        if current == Some(target) {
            return;
        }
        if session.room.get_untracked().is_some() {
            let other = session
                .state
                .with_untracked(|game| game.character_at_turn_position(target));
            if let (Some(a), Some(b)) = (first_character, other) {
                session.net_swap_turn_order(a, b);
            }
        } else {
            session.dispatch(move |game| game.swap_turn_position(participant_id, target));
        }
    });

    let order_options = move || {
        let count = session.state.with(|game| game.participants.len());
        (0..count)
            .map(|position| {
                let value = (position + 1).to_string();
                let selected_value = value.clone();
                view! {
                    <option value=value selected=move || order_read.get() == selected_value>
                        {position + 1}
                    </option>
                }
            })
            .collect_view()
    };

    let row_class = move || {
        let mut class = String::from("participant-row");
        // Проигравший участник показывается тусклее, чтобы сразу видно,
        // кто выбыл.
        if lost {
            class.push_str(" participant-row--lost");
        }
        let (is_current, is_next) = session.state.with(|game| {
            if game.phase != GamePhase::Playing {
                return (false, false);
            }
            (
                game.current_participant_id() == Some(participant_id),
                game.next_participant_id() == Some(participant_id),
            )
        });
        if is_current {
            class.push_str(" participant-row--current");
        } else if is_next {
            class.push_str(" participant-row--next");
        }
        class
    };

    let stats = Memo::new(move |_| {
        session
            .state
            .with(|game| game.participant_stats(participant_id))
    });

    view! {
        <div class=row_class>
            <Show
                when=editable
                fallback=move || {
                    view! {
                        <div class="participant-row__stats">
                            <span>{t!(i18n, wins)} " " {move || stats.get().0}</span>
                            <span>{t!(i18n, losses)} " " {move || stats.get().1}</span>
                            <span>{t!(i18n, moves)} " " {move || stats.get().2}</span>
                        </div>
                    }
                }
            >
                <div class="participant-row__controls">
                    <Show when=can_edit_order>
                        <Select
                            class="participant-row__select participant-row__select--order"
                            size=SelectSize::Small
                            value=(order_read, order_write)
                        >
                            {order_options}
                        </Select>
                    </Show>

                    {move || {
                        // Сетевое место: администратор переназначает контроллера
                        // селектом, остальные видят имя игрока и чип готовности.
                        match session.room.get() {
                            None => view! {
                                <Select
                                    class="participant-row__select"
                                    size=SelectSize::Small
                                    value=(controller_read, controller_write)
                                >
                                    {controller_options}
                                </Select>
                            }
                            .into_any(),
                            Some(room) if room.is_admin => view! {
                                <Select
                                    class="participant-row__select participant-row__select--seat"
                                    size=SelectSize::Small
                                    value=(seat_read, seat_write)
                                >
                                    {seat_options}
                                </Select>
                            }
                            .into_any(),
                            Some(_) => view! {
                                <span class="participant-row__name">
                                    {move || name_label.get()}
                                </span>
                                <Show when=move || ready == Some(true)>
                                    <span class="participant-row__ready" aria-hidden="true">
                                        "✓"
                                    </span>
                                </Show>
                            }
                            .into_any(),
                        }
                    }}

                    <Select
                        class="participant-row__select participant-row__select--team"
                        size=SelectSize::Small
                        value=(team_read, team_write)
                    >
                        {team_options}
                    </Select>

                    <Show when=can_remove>
                        <Button
                            class="participant-row__icon participant-row__icon--danger"
                            size=ButtonSize::Small
                            appearance=ButtonAppearance::Subtle
                            on_click=move |_| {
                                if session.room.get_untracked().is_some() {
                                    if let Some(character) = first_character {
                                        session.net_kick(character);
                                    }
                                } else {
                                    session
                                        .dispatch(move |game| game.remove_participant(participant_id));
                                }
                            }
                        >
                            <span title=remove_title>"✕"</span>
                        </Button>
                    </Show>
                </div>
            </Show>
        </div>
    }
}

/// Панель игроков онлайн-комнаты над списком команд: имена участников
/// (ники из аккаунтов), готовность и отметка администратора.
#[component]
pub fn RoomPlayersPanel() -> impl IntoView {
    let i18n = use_i18n();
    let session = GameSession::expect();

    view! {
        <section class="room-players">
            <h2 class="section-title">{t!(i18n, room_players)}</h2>
            <div class="room-players__list">
                {move || {
                    session
                        .state
                        .with(|game| game.players.clone())
                        .into_iter()
                        .map(|player| {
                            // Memo-хэндлы — Copy: дети с Show требуют Fn-замыкание,
                            // поэтому строки не переносятся напрямую.
                            let name = Memo::new(move |_| {
                                player.name.clone().unwrap_or_else(|| format!("#{}", player.id))
                            });
                            let admin_title =
                                Memo::new(move |_| t_string!(i18n, admin_title).to_string());
                            let ready_title =
                                Memo::new(move |_| t_string!(i18n, ready).to_string());
                            let ready = player.ready;
                            let chip_class = if player.admin {
                                "room-players__chip room-players__chip--admin"
                            } else {
                                "room-players__chip"
                            };
                            view! {
                                <span class=chip_class>
                                    {move || name.get()}
                                    <Show when=move || ready>
                                        <span
                                            class="room-players__ready"
                                            title=ready_title
                                            aria-hidden="true"
                                        >
                                            "✓"
                                        </span>
                                    </Show>
                                    <Show when=move || player.admin>
                                        <span
                                            class="room-players__admin"
                                            title=admin_title
                                            aria-hidden="true"
                                        >
                                            "★"
                                        </span>
                                    </Show>
                                </span>
                            }
                        })
                        .collect_view()
                }}
            </div>
        </section>
    }
}
