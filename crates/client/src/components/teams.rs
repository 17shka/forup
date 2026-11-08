use forup_core::api::game::board::prelude::TeamId;
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
                <Show when=move || session.state.with(|game| game.phase != GamePhase::Playing)>
                    <Button
                        class="teams-panel__add"
                        size=ButtonSize::Small
                        appearance=ButtonAppearance::Primary
                        on_click=move |_| {
                            session.dispatch(|game| game.add_bot().map(|_| ()));
                        }
                    >
                        <span aria-hidden="true">"＋"</span>
                        <span>{t!(i18n, add_member)}</span>
                    </Button>
                </Show>
            </div>

            <div class="team-cards">
                {move || {
                    let (teams, teams_options, controller_options, editable) = session
                        .state
                        .with(|game| {
                            (
                                game.team_views(),
                                game.team_options(),
                                game.controller_options(),
                                game.phase != GamePhase::Playing,
                            )
                        });
                    teams
                        .into_iter()
                        .map(|team| {
                            view! {
                                <TeamCard
                                    team
                                    options=teams_options.clone()
                                    controllers=controller_options.clone()
                                    editable
                                />
                            }
                        })
                        .collect_view()
                }}
            </div>
        </section>
    }
}

#[component]
fn TeamCard(
    team: TeamView,
    options: Vec<TeamId>,
    controllers: Vec<Controller>,
    editable: bool,
) -> impl IntoView {
    let i18n = use_i18n();
    let hue = team.hue;
    let team_id = team.team.get();
    let cells = team.cells;
    let mass = team.mass;

    let rows = team
        .participants
        .into_iter()
        .map(|member| {
            view! {
                <ParticipantRow
                    member
                    team=team.team
                    hue=hue
                    options=options.clone()
                    controllers=controllers.clone()
                    editable
                />
            }
        })
        .collect_view();

    view! {
        <div class="team-card-wrap" style=format!("--team-hue: {hue};")>
            <Card class="team-card">
                <CardHeader>
                    <span class="team-id-chip">"#" {team_id}</span>
                    <span class="team-card__summary">
                        {format!(
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
fn ParticipantRow(
    member: TeamParticipantView,
    team: TeamId,
    hue: u16,
    options: Vec<TeamId>,
    controllers: Vec<Controller>,
    editable: bool,
) -> impl IntoView {
    let i18n = use_i18n();
    let session = GameSession::expect();
    let participant_id = member.id;
    let controller = member.controller;
    let wins = member.wins;
    let losses = member.losses;
    let moves = member.moves;

    let name = match controller {
        Controller::Human => t_string!(i18n, controller_player).to_string(),
        Controller::Bot(number) => t_string!(i18n, bot_member, id = number).to_string(),
    };

    let can_remove_character = member.characters.len() > 1;
    let chips = member
        .characters
        .into_iter()
        .map(|character| {
            view! {
                <span class="character-chip">
                    <span>"#" {character.get()}</span>
                    <Show when=move || editable && can_remove_character>
                        <Button
                            class="character-chip__remove"
                            size=ButtonSize::Small
                            appearance=ButtonAppearance::Transparent
                            on_click=move |_| {
                                session.dispatch(move |game| {
                                    game.remove_character(participant_id, character)
                                });
                            }
                        >
                            "×"
                        </Button>
                    </Show>
                </span>
            }
        })
        .collect_view();

    let controller_value = RwSignal::new(controller.key());
    let team_value = RwSignal::new(team.get().to_string());

    // Apply controller changes only after the user picks another value.
    Effect::new(move |previous: Option<String>| {
        let current = controller_value.get();
        if previous
            .as_deref()
            .is_some_and(|previous| previous != current.as_str())
        {
            if let Some(controller) = Controller::from_key(&current) {
                session.dispatch(move |game| {
                    game.set_participant_controller(participant_id, controller)
                });
            }
        }
        current
    });

    // Same for the team: moving a seat moves all of its characters.
    Effect::new(move |previous: Option<String>| {
        let current = team_value.get();
        if previous
            .as_deref()
            .is_some_and(|previous| previous != current.as_str())
        {
            if let Ok(raw) = current.parse::<u8>() {
                if let Some(team) = TeamId::new(raw) {
                    session.dispatch(move |game| game.set_participant_team(participant_id, team));
                }
            }
        }
        current
    });

    let team_options = options
        .into_iter()
        .map(|option| {
            let code = option.get().to_string();
            let selected_code = code.clone();
            let id = option.get();
            view! {
                <option value=code selected=move || team_value.get() == selected_code>
                    {id}
                </option>
            }
        })
        .collect_view();

    let controller_options = controllers
        .into_iter()
        .map(|controller| {
            let key = controller.key();
            let selected_key = key.clone();
            let label = match controller {
                Controller::Human => t_string!(i18n, controller_player).to_string(),
                Controller::Bot(number) => t_string!(i18n, bot_member, id = number).to_string(),
            };
            view! {
                <option value=key selected=move || controller_value.get() == selected_key>
                    {label}
                </option>
            }
        })
        .collect_view();

    let row_class = move || {
        let mut class = String::from("participant-row");
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

    let controls = if editable {
        view! {
            <div class="participant-row__controls">
                <Select
                    class="participant-row__select"
                    size=SelectSize::Small
                    value=controller_value
                >
                    {controller_options}
                </Select>

                <span class="team-dot" style=format!("--team-hue: {hue};")></span>

                <Select
                    class="participant-row__select participant-row__select--team"
                    size=SelectSize::Small
                    value=team_value
                >
                    {team_options}
                </Select>

                <Button
                    class="participant-row__icon"
                    size=ButtonSize::Small
                    appearance=ButtonAppearance::Subtle
                    on_click=move |_| {
                        session.dispatch(move |game| game.add_character(participant_id).map(|_| ()));
                    }
                >
                    <span title=t_string!(i18n, add_character).to_string()>"＋"</span>
                </Button>
                <Button
                    class="participant-row__icon participant-row__icon--danger"
                    size=ButtonSize::Small
                    appearance=ButtonAppearance::Subtle
                    on_click=move |_| {
                        session.dispatch(move |game| game.remove_participant(participant_id));
                    }
                >
                    <span title=t_string!(i18n, remove).to_string()>"✕"</span>
                </Button>
            </div>
        }
        .into_any()
    } else {
        view! {
            <div class="participant-row__stats">
                <span>{t!(i18n, wins)} " " {wins}</span>
                <span>{t!(i18n, losses)} " " {losses}</span>
                <span>{t!(i18n, moves)} " " {moves}</span>
            </div>
        }
        .into_any()
    };

    view! {
        <div class=row_class>
            <div class="participant-row__identity">
                <span class="participant-row__name">{name}</span>
                <span class="participant-row__characters">{chips}</span>
            </div>

            {controls}
        </div>
    }
}
