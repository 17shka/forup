//! Сетевая часть игровой сессии: вход в серверные комнаты, отправка действий
//! и цикл опроса снапшотов. Локальные сигналы — источник истины для UI,
//! сервер — для сетевых комнат: каждое действие возвращает полный снапшот,
//! который применяется через [`LocalGame::absorb`].
//!
//! Все сетевые зависимости опциональны (wasmgate), поэтому методы существуют
//! на нативных сборках как заглушки.

use forup_core::api::game::board::prelude::{BoardCode, CharacterId, MemberId, TeamId};
use forup_core::api::network::RoomId;
use leptos::prelude::*;

use super::session::{GameSession, LocalGame, MoveMode};

#[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
use super::session::NetRoom;
#[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
use crate::net::{self, NetError};
#[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
use forup_core::api::game::board::prelude::GameError;
#[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
use forup_core::api::network::{
    BoardSizeRequest, CreateRoomRequest, DistributionRequest, ExplosionRequest, MemberRequest,
    MoveRequest, RadiusRequest, ReadyRequest, RoomError, RoomStateResponse, SeatRequest,
    TeamRequest, TurnOrderRequest,
};
#[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
use leptos::task::spawn_local;
#[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
use serde::Serialize;

impl GameSession {
    /// Интервал опроса комнаты.
    #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
    const POLL_INTERVAL_MS: u64 = 700;

    /// Сбрасывает поколение цикла опроса: ожидающиеся таймеры увидят
    /// несовпадение и погаснут, не породив дублей.
    pub fn bump_poll(&self) {
        self.poll_generation
            .update(|generation| *generation = generation.wrapping_add(1));
    }

    /// Покидает комнату: локальное состояние сбрасывается сразу (без absorb —
    /// дельты старой доски не должны мигнуть на новой), запрос уходит в фоне.
    pub fn leave_room(&self) {
        // Чтение комнаты живёт внутри сетевого гейта: на native весь блок
        // вырезается и не оставляет неиспользуемых переменных.
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        if let Some(room) = self.room.get_untracked() {
            let path = format!("/api/rooms/{}/leave", room.id.get());
            spawn_local(async move {
                let _ = net::room_request::<()>("POST", &path, &room.token, None).await;
            });
        }
        self.room.set(None);
        self.state.set(LocalGame::new());
        self.net_error.set(None);
        self.net_busy.set(false);
    }

    /// Переводит партию в офлайн, сохраняя стол: комната покидается на
    /// сервере, а локальные участники и настройки остаются как есть —
    /// переключение Оффлайн/Онлайн ничего не перестраивает.
    pub fn go_offline(&self) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        if let Some(room) = self.room.get_untracked() {
            let path = format!("/api/rooms/{}/leave", room.id.get());
            let token = room.token.clone();
            spawn_local(async move {
                let _ = net::room_request::<()>("POST", &path, &token, None).await;
            });
        }
        self.room.set(None);
        self.pending_snapshot.set(None);
        self.state.update(LocalGame::detach);
        self.net_error.set(None);
        self.net_busy.set(false);
    }

    /// Создаёт новую комнату от имени текущего игрока; предыдущая покидается
    /// молча. Комната воссоздаётся по снимку текущего лобби, чтобы стол не
    /// перестраивался. `on_success` вызывается после успешного входа.
    pub fn connect_new_room(&self, token: &str, open: bool, on_success: impl FnOnce() + 'static) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        {
            self.net_error.set(None);
            self.net_busy.set(true);
            let session = *self;
            let token = token.to_string();
            let setup = self.state.with_untracked(LocalGame::lobby_setup);
            spawn_local(async move {
                leave_old_room(&session, &token).await;
                let response =
                    post_room("/api/rooms", &token, &CreateRoomRequest { open, setup }).await;
                match response {
                    Ok(response) => {
                        session.adopt(&token, response);
                        on_success();
                    }
                    Err(error) => session.handle_action_error(error),
                }
                session.net_busy.set(false);
            });
        }
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        {
            let _ = (token, open, on_success);
        }
    }

    /// Входит в существующую комнату по идентификатору.
    pub fn join_room(&self, token: &str, id: RoomId, on_success: impl FnOnce() + 'static) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        {
            self.net_error.set(None);
            self.net_busy.set(true);
            let session = *self;
            let token = token.to_string();
            spawn_local(async move {
                leave_old_room(&session, &token).await;
                let path = format!("/api/rooms/{}/join", id.get());
                let response = post_room(&path, &token, &()).await;
                match response {
                    Ok(response) => {
                        session.adopt(&token, response);
                        on_success();
                    }
                    Err(error) => session.handle_action_error(error),
                }
                session.net_busy.set(false);
            });
        }
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        {
            let _ = (token, id, on_success);
        }
    }

    /// Разовый обмен снапшотом комнаты. Ошибки не показываются тостами:
    /// опрос повторяется по расписанию, разрывы соединения лечатся
    /// следующей итерацией. Комната исчезла или сессия истекла — отключаемся.
    #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
    fn sync_room(&self) {
        let Some(room) = self.room.get_untracked() else {
            return;
        };
        let session = *self;
        let path = format!("/api/rooms/{}", room.id.get());
        let token = room.token.clone();
        spawn_local(async move {
            match net::room_request::<RoomStateResponse>("GET", &path, &token, None).await {
                // Цикл не зависит от уведомлений сигналов: равный снапшот не
                // должен перерисовывать панели (и закрывать открытые меню).
                Ok(response) => {
                    session.apply_snapshot(response);
                    session.schedule_room_poll();
                }
                Err(NetError::Room(RoomError::NotFound)) | Err(NetError::Unauthorized) => {
                    session.disconnect();
                }
                // Транспортная проблема: пробуем дальше по расписанию.
                Err(_) => session.schedule_room_poll(),
            }
        });
    }

    /// Планирует следующий опрос. Таймер сверяется с поколением: смена
    /// комнаты или размонтирование страницы отменяют запланированный опрос.
    pub fn schedule_room_poll(&self) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        {
            let session = *self;
            let generation = self.poll_generation.get_untracked();
            set_timeout(
                move || {
                    if session.poll_generation.get_untracked() == generation
                        && session.room.get_untracked().is_some()
                    {
                        session.sync_room();
                    }
                },
                std::time::Duration::from_millis(Self::POLL_INTERVAL_MS),
            );
        }
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        {}
    }

    // --- действия в комнате ------------------------------------------------

    pub fn net_set_ready(&self, ready: bool) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        self.room_action("ready", ReadyRequest { ready });
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        let _ = ready;
    }

    /// Заново: локальное предсказание (возврат в чистое лобби) + подтверждение
    /// сервером. Предсказание сработает тихо: сервер вернёт равный снапшот.
    pub fn net_rematch(&self) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        self.send_if_ok(self.state.try_update(|game| game.rematch()), "rematch", ());
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        {}
    }

    pub fn net_add_bot(&self) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        self.room_action("bot", ());
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        {}
    }

    pub fn net_kick(&self, member: CharacterId) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        self.room_action("kick", MemberRequest { member });
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        let _ = member;
    }

    pub fn net_change_team(&self, member: CharacterId, team: TeamId) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        self.room_action("team", TeamRequest { member, team });
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        let _ = (member, team);
    }

    /// Назначить контроллера места: игрок из комнаты или бот.
    pub fn net_set_controller(&self, member: CharacterId, controller: MemberId) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        self.room_action("seat", SeatRequest { member, controller });
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        let _ = (member, controller);
    }

    /// Размер доски; меняет только администратор до старта партии.
    pub fn net_set_board_size(&self, width: u8, height: u8) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        self.room_action("board", BoardSizeRequest { width, height });
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        let _ = (width, height);
    }

    /// Автоматическая расстановка стартовых клеток.
    pub fn net_set_automatic_distribution(&self, automatic: bool) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        self.room_action("distribution", DistributionRequest { automatic });
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        let _ = automatic;
    }

    /// Взрывы без потерь.
    pub fn net_set_lossless_explosions(&self, lossless: bool) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        self.room_action("explosions", ExplosionRequest { lossless });
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        let _ = lossless;
    }

    /// Квадрат безопасного радиуса стартовых клеток.
    pub fn net_set_safe_radius(&self, radius_sq: u8) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        self.room_action("radius", RadiusRequest { radius_sq });
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        let _ = radius_sq;
    }

    /// Ход с предсказанием: применяется к своей локальной игре и только
    /// после успеха отправляется серверу. Отказ сервера (рассинхрон)
    /// лечится ресинком через свежий снапшот комнаты.
    pub fn net_move(&self, pos: BoardCode, mode: MoveMode) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        {
            if !self.ready_to_predict() {
                return;
            }
            let Some(member) = self.current_net_member() else {
                return;
            };
            self.send_if_ok(
                self.state.try_update(|game| game.make_move(pos, mode)),
                "move",
                MoveRequest {
                    member,
                    pos,
                    increase: mode == MoveMode::Increase,
                },
            );
        }
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        let _ = (pos, mode);
    }

    /// Пропуск хода с предсказанием (см. [`Self::net_move`]).
    pub fn net_skip(&self) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        {
            if !self.ready_to_predict() {
                return;
            }
            let Some(member) = self.current_net_member() else {
                return;
            };
            self.send_if_ok(
                self.state.try_update(|game| game.skip()),
                "skip",
                MemberRequest { member },
            );
        }
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        {}
    }

    /// Сдача с предсказанием (см. [`Self::net_move`]).
    pub fn net_surrender(&self) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        {
            if !self.ready_to_predict() {
                return;
            }
            // Сдаться можно в любой момент: своё место, даже если сейчас чужой ход.
            let Some(member) = self.state.with_untracked(LocalGame::surrender_character) else {
                return;
            };
            self.send_if_ok(
                self.state.try_update(|game| game.surrender()),
                "surrender",
                MemberRequest { member },
            );
        }
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        {}
    }

    /// Принудительное завершение партии администратором: локальное
    /// предсказание (движок закончил партию тем же способом) + подтверждение
    /// сервером. Предсказание сработает тихо: сервер вернёт равный снапшот.
    pub fn net_end_game(&self) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        {
            if !self.ready_to_predict() {
                return;
            }
            self.send_if_ok(self.state.try_update(|game| game.end_game()), "end", ());
        }
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        {}
    }

    /// Перестановка местами двух участников в последовательности ходов
    /// лобби. Предсказания нет: снапшот сервера применится сам.
    pub fn net_swap_turn_order(&self, a: CharacterId, b: CharacterId) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        self.room_action("order", TurnOrderRequest { a, b });
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        let _ = (a, b);
    }

    // --- приватные помощники -----------------------------------------------

    /// Рвёт связь с комнатой, оставляя локальную партию как офлайн-стол:
    /// сетевые места становятся локальными игроками, стол не
    /// перестраивается. `net_error` сознательно не чистится: тост должен
    /// пережить отключение.
    #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
    fn disconnect(&self) {
        self.room.set(None);
        self.pending_snapshot.set(None);
        self.state.update(LocalGame::detach);
        self.net_busy.set(false);
    }

    /// Обрабатывает ошибку действия: фатальные (комната исчезла, сессия
    /// истекла) отключают, остальные показываются тостом и лечатся
    /// ресинком — сервер по какой-то причине отклонил предсказанный
    /// ход, значит локальная копия разошлась с сервером.
    #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
    fn handle_action_error(&self, error: NetError) {
        let fatal = matches!(error, NetError::Room(RoomError::NotFound))
            || matches!(error, NetError::Unauthorized);
        if fatal {
            self.disconnect();
        } else {
            self.resync();
        }
        self.net_error.set(Some(error));
    }

    /// Первое подключение: снапшот применяется как есть, без анимации дельт.
    #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
    fn adopt(&self, token: &str, response: RoomStateResponse) {
        self.pending_snapshot.set(None);
        self.state.set(LocalGame::from_room(
            &response.room,
            response.me,
            &response.names,
        ));
        self.room.set(Some(NetRoom {
            id: response.id,
            me: response.me,
            token: token.to_string(),
            is_admin: response.room.is_admin(response.me),
        }));
        self.net_error.set(None);
        self.net_busy.set(false);
    }

    /// Применяет очередной снапшот. Пока идёт анимация взрывов, снапшот
    /// откладывается: волны не должны обрываться одиночным флешем.
    #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
    fn apply_snapshot(&self, response: RoomStateResponse) {
        if self.state.with_untracked(LocalGame::is_animating) {
            self.pending_snapshot.set(Some(Box::new(response)));
            return;
        }
        self.apply_snapshot_now(response, false);
    }

    /// Применяет снапшот немедленно (`force` игнорирует водяной знак —
    /// ресинк всегда актуален). Порядок проверок важен: стейл-снапшот
    /// старой комнаты не должен кикать из новой, поэтому сверка id идёт
    /// до проверки участников.
    #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
    fn apply_snapshot_now(&self, response: RoomStateResponse, force: bool) {
        let Some(current) = self.room.get_untracked() else {
            return;
        };
        if response.id != current.id {
            return;
        }
        if response.room.members().get(&current.me).is_none() {
            self.disconnect();
            self.net_error.set(Some(NetError::Kicked));
            return;
        }
        let seq = response.room.seq();
        let steps = response.room.last_steps().to_vec();
        let next = LocalGame::from_room(&response.room, current.me, &response.names);

        // Устаревший снапшот: мы уже видели это состояние (или новее).
        if !force && self.state.with_untracked(|state| seq <= state.sync_seq()) {
            return;
        }

        if self.state.with_untracked(|state| state.game == next.game) {
            // Предсказание подтвердилось: тихо обновляем метаданные,
            // не трогая контекст анимации.
            if self
                .state
                .with_untracked(|state| !state.same_snapshot_meta(&next))
            {
                self.state.update(|state| state.confirm(next));
            }
        } else if !steps.is_empty()
            && self
                .state
                .try_update(|state| state.start_replay(&steps, &next.game, seq))
                == Some(true)
        {
            // Чужие ходы доигрываются реплеем — волна за волной.
        } else if self.state.with_untracked(|state| *state != next) {
            self.state.update(|state| state.absorb(next));
        }

        let is_admin = response.room.is_admin(current.me);
        if current.is_admin != is_admin {
            self.room.set(Some(NetRoom {
                id: current.id,
                me: current.me,
                token: current.token.clone(),
                is_admin,
            }));
        }
    }

    /// Подтверждённый снапшот берётся заново: сервер отклонил предсказанный
    /// ход, локальная копия разошлась. Ошибки молчат: цикл опроса
    /// продолжит исправлять состояние по расписанию.
    #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
    fn resync(&self) {
        let Some(room) = self.room.get_untracked() else {
            return;
        };
        self.pending_snapshot.set(None);
        let session = *self;
        let path = format!("/api/rooms/{}", room.id.get());
        let token = room.token.clone();
        spawn_local(async move {
            match net::room_request::<RoomStateResponse>("GET", &path, &token, None).await {
                Ok(response) => session.apply_snapshot_now(response, true),
                Err(NetError::Room(RoomError::NotFound)) | Err(NetError::Unauthorized) => {
                    session.disconnect();
                }
                Err(_) => {}
            }
        });
    }

    /// Применяет отложенный снапшот, как только анимация закончилась.
    /// Вызывается насосом анимации на странице игры.
    pub fn flush_pending_snapshot(&self) {
        #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
        if let Some(response) = self.pending_snapshot.get_untracked() {
            self.pending_snapshot.set(None);
            self.apply_snapshot_now(*response, false);
        }
        #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
        {}
    }

    /// Общий отправитель действий: POST `{id}/{suffix}` с телом; ответ
    /// применяется как снапшот. Пока летит один запрос, остальные блокируются.
    #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
    fn room_action(&self, suffix: &str, body: impl Serialize + 'static) {
        let Some(room) = self.room.get_untracked() else {
            return;
        };
        if self.net_busy.get_untracked() {
            return;
        }
        self.net_error.set(None);
        self.net_busy.set(true);
        let session = *self;
        let path = format!("/api/rooms/{}/{}", room.id.get(), suffix);
        let token = room.token.clone();
        spawn_local(async move {
            match post_room(&path, &token, &body).await {
                Ok(response) => session.apply_snapshot(response),
                Err(error) => session.handle_action_error(error),
            }
            session.net_busy.set(false);
        });
    }

    /// Наше место, если сейчас наш ход.
    #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
    fn current_net_member(&self) -> Option<CharacterId> {
        self.state.with_untracked(|game| {
            if game.is_human_turn() {
                game.current_character()
            } else {
                None
            }
        })
    }

    /// Готов ли стол к предсказанию хода: отложенный снапшот применён,
    /// сеть свободна и анимация взрывов не идёт.
    #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
    fn ready_to_predict(&self) -> bool {
        self.flush_pending_snapshot();
        !self.net_busy.get_untracked() && !self.state.with_untracked(LocalGame::is_animating)
    }

    /// Применяет результат локального предсказания: ошибка показывается
    /// тостом, успех отправляется серверу; `None` (залоченный сигнал)
    /// тихо игнорируется.
    #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
    fn send_if_ok(
        &self,
        result: Option<Result<(), GameError>>,
        suffix: &str,
        body: impl Serialize + 'static,
    ) {
        match result {
            Some(Err(error)) => self.error.set(Some(error)),
            Some(Ok(())) => self.room_action(suffix, body),
            None => {}
        }
    }
}

/// POST в комнатный API с postcard-телом; ответ — снапшот комнаты.
#[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
async fn post_room<B: Serialize>(
    path: &str,
    token: &str,
    body: &B,
) -> Result<RoomStateResponse, NetError> {
    let body = net::encode_body(body)?;
    net::room_request("POST", path, token, Some(body)).await
}

/// Молча покидает предыдущую комнату перед входом в новую.
#[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
async fn leave_old_room(session: &GameSession, token: &str) {
    if let Some(room) = session.room.get_untracked() {
        let path = format!("/api/rooms/{}/leave", room.id.get());
        let _ = net::room_request::<()>("POST", &path, token, None).await;
    }
}
