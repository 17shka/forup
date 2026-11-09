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

    let rows = team
        .participants
        .into_iter()
        .map(|member| view! { <ParticipantRow member/> })
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
fn ParticipantRow(member: TeamParticipantView) -> impl IntoView {
    let i18n = use_i18n();
    let session = GameSession::expect();
    let participant_id = member.id;

    let editable = move || session.state.with(|game| game.phase != GamePhase::Playing);

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
            session.dispatch(move |game| game.set_participant_team(participant_id, team));
        }
    });

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
                    <Select
                        class="participant-row__select"
                        size=SelectSize::Small
                        value=(controller_read, controller_write)
                    >
                        {controller_options}
                    </Select>

                    <Select
                        class="participant-row__select participant-row__select--team"
                        size=SelectSize::Small
                        value=(team_read, team_write)
                    >
                        {team_options}
                    </Select>

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
            </Show>
        </div>
    }
}
