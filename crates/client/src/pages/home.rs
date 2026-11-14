use forup_core::api::network::RoomSummary;
use leptos::prelude::*;
use leptos_router::hooks::use_navigate;
use thaw::{Button, ButtonAppearance, ButtonSize, Card, CardHeader};

use crate::auth::AuthState;
use crate::game::GameSession;
use crate::i18n::*;

#[component]
pub fn HomePage() -> impl IntoView {
    let i18n = use_i18n();
    let navigate = use_navigate();
    let auth = AuthState::expect();
    let session = GameSession::expect();

    let rooms = RwSignal::new(Vec::<RoomSummary>::new());
    let loading = RwSignal::new(false);
    let failed = RwSignal::new(false);

    let on_play = {
        let navigate = navigate.clone();
        move |_| navigate("/game", Default::default())
    };

    // Сигнал-команда «перейти на страницу игры». Панель комнат вложена в
    // children компонентов (Show/Card), где замыкания обязаны быть Fn, поэтому
    // обработчики не могут владеть navigate (он не Copy). Навигацию выполняет
    // эффект на верхнем уровне, а success-замыкания комнат только взводят
    // сигнал.
    let go_to_game = RwSignal::new(false);
    Effect::new(move |_| {
        if go_to_game.get() {
            go_to_game.set(false);
            navigate("/game", Default::default());
        }
    });

    let refresh = move || {
        let token = auth.token.get_untracked();
        if token.is_empty() {
            return;
        }
        loading.set(true);
        failed.set(false);
        crate::net::fetch_rooms(token, move |result| {
            match result {
                Ok(list) => rooms.set(list),
                Err(_) => failed.set(true),
            }
            loading.set(false);
        });
    };

    // При входе (в том числе после восстановления сессии) список комнат
    // подгружается, при выходе — очищается.
    Effect::new(move |_| {
        if auth.user.get().is_some() {
            refresh();
        } else {
            rooms.set(Vec::new());
            loading.set(false);
            failed.set(false);
        }
    });

    let on_create = move |_| {
        let token = auth.token.get_untracked();
        if token.is_empty() {
            return;
        }
        session.connect_new_room(&token, true, move || go_to_game.set(true));
    };

    let room_rows = move || {
        rooms
            .get()
            .into_iter()
            .map(|room| {
                let id = room.id;
                let members = room.members;
                let bots = room.bots;
                let started = room.started;
                let status_text = if started {
                    t_string!(i18n, in_game)
                } else {
                    t_string!(i18n, lobby)
                };
                let status_class = if started {
                    "rooms-list__status rooms-list__status--started"
                } else {
                    "rooms-list__status"
                };
                view! {
                    <div class="rooms-list__row">
                        <span class="rooms-list__id">"#" {id.get()}</span>
                        <span class="rooms-list__meta">
                            {t_string!(i18n, players_count, count = members)}
                            {" · "}
                            {t_string!(i18n, bots_count, count = bots)}
                        </span>
                        <span class=status_class>{status_text}</span>
                        <Button
                            class="rooms-list__join"
                            appearance=ButtonAppearance::Secondary
                            size=ButtonSize::Small
                            on_click=move |_| {
                                let token = auth.token.get_untracked();
                                if token.is_empty() {
                                    return;
                                }
                                session.join_room(&token, id, move || {
                                    go_to_game.set(true)
                                });
                            }
                        >
                            {t!(i18n, join_room)}
                        </Button>
                    </div>
                }
            })
            .collect_view()
    };

    view! {
        <div class="page home-page">
            <div class="home-page__content">
                <h1 class="home-page__logo">"Forup"</h1>
                <Button
                    class="play-button"
                    appearance=ButtonAppearance::Primary
                    size=ButtonSize::Large
                    on_click=on_play
                >
                    {t!(i18n, play)}
                </Button>

                <Show when=move || auth.user.get().is_some()>
                    <Card class="rooms-panel">
                        <CardHeader>
                            <span class="rooms-panel__title">{t!(i18n, rooms_title)}</span>
                            <div class="rooms-panel__actions">
                                <Button
                                    appearance=ButtonAppearance::Secondary
                                    size=ButtonSize::Small
                                    disabled=move || loading.get()
                                    on_click=move |_| refresh()
                                >
                                    {t!(i18n, refresh_rooms)}
                                </Button>
                                <Button
                                    appearance=ButtonAppearance::Primary
                                    size=ButtonSize::Small
                                    on_click=on_create
                                >
                                    {t!(i18n, create_room)}
                                </Button>
                            </div>
                        </CardHeader>

                        <Show
                            when=move || !failed.get()
                            fallback=move || {
                                view! {
                                    <p class="rooms-panel__error">{t!(i18n, error_connection)}</p>
                                }
                            }
                        >
                            <Show
                                when=move || !rooms.get().is_empty()
                                fallback=move || {
                                    view! {
                                        <p class="rooms-list__empty">
                                            {move || {
                                                if loading.get() {
                                                    t_string!(i18n, loading)
                                                } else {
                                                    t_string!(i18n, rooms_empty)
                                                }
                                            }}
                                        </p>
                                    }
                                }
                            >
                                <div class="rooms-list">{room_rows}</div>
                            </Show>
                        </Show>
                    </Card>
                </Show>
            </div>
        </div>
    }
}
