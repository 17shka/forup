//! HTTP-обработчики комнатного API: список открытых комнат, создание, вход и
//! выход, готовность, команды и ходы. Все маршруты требуют авторизацию; тела
//! запросов и ответов кодируются postcard'ом (см. `http::encode`/`http::decode`).
//!
//! Общий шаблон обработчика: захватить мьютекс директории, применить изменение,
//! синхронно сохранить карту в redb (при ошибке только логируем — память
//! остаётся источником истины), собрать снапшот `RoomStateResponse` под
//! блокировкой и только затем отпустить её. Обработчики не содержат `await`,
//! поэтому guard живёт до конца функции и больше нигде не захватывается.

use std::{
    collections::{BTreeSet, HashMap},
    sync::MutexGuard,
};

use axum::{
    Router,
    body::Bytes,
    extract::{Extension, Path},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use forup_core::api::{
    auth::PlayerId,
    network::{
        BoardSizeRequest, CreateRoomRequest, DistributionRequest, ExplosionRequest, MemberRequest,
        MoveRequest, OpenRequest, RadiusRequest, ReadyRequest, Room, RoomDirectory, RoomError,
        RoomId, RoomStateResponse, RoomSummary, RoomsResponse, SeatRequest, TeamRequest,
        TurnOrderRequest,
    },
};

use crate::http::{decode, encode};
use crate::state::{ApiState, authenticated};

/// Комнатные эндпоинты, встренные в основной роутер приложения.
pub fn router<S>(state: ApiState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/api/rooms", get(list_rooms).post(create_room))
        .route("/api/rooms/{id}", get(room_state))
        .route("/api/rooms/{id}/join", post(join_room))
        .route("/api/rooms/{id}/leave", post(leave_room))
        .route("/api/rooms/{id}/ready", post(set_ready))
        .route("/api/rooms/{id}/open", post(set_open))
        .route("/api/rooms/{id}/bot", post(add_bot))
        .route("/api/rooms/{id}/team", post(change_team))
        .route("/api/rooms/{id}/seat", post(set_seat))
        .route("/api/rooms/{id}/board", post(set_board_size))
        .route("/api/rooms/{id}/distribution", post(set_distribution))
        .route("/api/rooms/{id}/explosions", post(set_explosions))
        .route("/api/rooms/{id}/radius", post(set_radius))
        .route("/api/rooms/{id}/kick", post(kick_member))
        .route("/api/rooms/{id}/move", post(make_move))
        .route("/api/rooms/{id}/skip", post(skip_turn))
        .route("/api/rooms/{id}/surrender", post(surrender))
        .route("/api/rooms/{id}/end", post(end_game))
        .route("/api/rooms/{id}/order", post(swap_order))
        .route("/api/rooms/{id}/start", post(start_game))
        .route("/api/rooms/{id}/rematch", post(rematch_game))
        .layer(Extension(state))
}

/// `GET /api/rooms`: все открытые онлайн-комнаты для списка в главном меню.
async fn list_rooms(Extension(state): Extension<ApiState>, headers: HeaderMap) -> Response {
    if let Err(response) = authenticated(&state, &headers) {
        return response;
    }
    let directory = lock_rooms(&state);
    let rooms = directory
        .rooms()
        .iter()
        .filter(|(_, room)| room.is_open())
        .map(|(id, room)| RoomSummary {
            id: *id,
            members: saturating_u8(room.members().len()),
            bots: saturating_u8(room.bots().len()),
            teams: count_teams(room),
            started: room.game().is_started(),
        })
        .collect::<Vec<_>>();
    encode(StatusCode::OK, &RoomsResponse { rooms })
}

/// `POST /api/rooms`: создать комнату от имени авторизованного игрока.
async fn create_room(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let request: CreateRoomRequest = match decode(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    let id = directory.create(me, request.open);
    // Воссоздаём лобби игрока, чтобы переключение Оффлайн/Онлайн ничего
    // не перестраивало. При ошибке комната откатывается целиком.
    let applied = directory
        .get_mut(id)
        .expect("a freshly created room is in the directory")
        .apply_lobby_setup(me, &request.setup);
    if let Err(error) = applied {
        let _ = directory.remove(id, me);
        return room_error(error);
    }
    persist(&state, &directory);
    build_state(&state, id, me, &directory)
}

/// `GET /api/rooms/{id}`: полный снапшот комнаты (клиент опрашивает его).
async fn room_state(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let directory = lock_rooms(&state);
    build_state(&state, id, me, &directory)
}

/// `POST /api/rooms/{id}/join`: войти в комнату, заняв свободную команду.
async fn join_room(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    let Some(room) = directory.get_mut(id) else {
        return room_error(RoomError::NotFound);
    };
    let team = room.next_team_id();
    if let Err(error) = room.join(me, team) {
        return room_error(error);
    }
    persist(&state, &directory);
    build_state(&state, id, me, &directory)
}

/// `POST /api/rooms/{id}/leave`: покинуть комнату; пустая комната удаляется.
/// Ответ 204 без тела — клиент после выхода сбрасывает локальное состояние.
async fn leave_room(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    if let Err(error) = directory.leave(id, me) {
        return room_error(error);
    }
    // Пустая комната удаляется целиком; save_rooms синхронизирует весь map,
    // поэтому удалённая комната исчезает и с диска.
    directory.remove_if_empty(id);
    persist(&state, &directory);
    StatusCode::NO_CONTENT.into_response()
}

/// `POST /api/rooms/{id}/ready`: отметить (или снять) готовность.
async fn set_ready(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
    body: Bytes,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let request: ReadyRequest = match decode(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    let Some(room) = directory.get_mut(id) else {
        return room_error(RoomError::NotFound);
    };
    if let Err(error) = room.set_ready(me, request.ready) {
        return room_error(error);
    }
    persist(&state, &directory);
    build_state(&state, id, me, &directory)
}

/// `POST /api/rooms/{id}/open`: открыть или закрыть комнату (только владелец).
async fn set_open(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
    body: Bytes,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let request: OpenRequest = match decode(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    let Some(room) = directory.get_mut(id) else {
        return room_error(RoomError::NotFound);
    };
    if let Err(error) = room.set_open(me, request.open) {
        return room_error(error);
    }
    persist(&state, &directory);
    build_state(&state, id, me, &directory)
}

/// `POST /api/rooms/{id}/bot`: добавить бота в свободную команду.
async fn add_bot(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    let Some(room) = directory.get_mut(id) else {
        return room_error(RoomError::NotFound);
    };
    let team = room.next_team_id();
    if let Err(error) = room.add_bot(me, team) {
        return room_error(error);
    }
    persist(&state, &directory);
    build_state(&state, id, me, &directory)
}

/// `POST /api/rooms/{id}/team`: переместить участника в другую команду.
async fn change_team(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
    body: Bytes,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let request: TeamRequest = match decode(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    let Some(room) = directory.get_mut(id) else {
        return room_error(RoomError::NotFound);
    };
    if let Err(error) = room.change_team(me, request.member, request.team) {
        return room_error(error);
    }
    persist(&state, &directory);
    build_state(&state, id, me, &directory)
}

/// `POST /api/rooms/{id}/seat`: назначить контроллера места (только администратор).
async fn set_seat(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
    body: Bytes,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let request: SeatRequest = match decode(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    let Some(room) = directory.get_mut(id) else {
        return room_error(RoomError::NotFound);
    };
    if let Err(error) = room.set_member_controller(me, request.member, request.controller) {
        return room_error(error);
    }
    persist(&state, &directory);
    build_state(&state, id, me, &directory)
}

/// `POST /api/rooms/{id}/board`: размер доски до старта партии (администратор).
async fn set_board_size(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
    body: Bytes,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let request: BoardSizeRequest = match decode(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    let Some(room) = directory.get_mut(id) else {
        return room_error(RoomError::NotFound);
    };
    if let Err(error) = room.set_board_size(me, request.width, request.height) {
        return room_error(error);
    }
    persist(&state, &directory);
    build_state(&state, id, me, &directory)
}

/// `POST /api/rooms/{id}/distribution`: автоматическая расстановка стартовых
/// клеток (администратор).
async fn set_distribution(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
    body: Bytes,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let request: DistributionRequest = match decode(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    let Some(room) = directory.get_mut(id) else {
        return room_error(RoomError::NotFound);
    };
    if let Err(error) = room.set_automatic_distribution(me, request.automatic) {
        return room_error(error);
    }
    persist(&state, &directory);
    build_state(&state, id, me, &directory)
}

/// `POST /api/rooms/{id}/explosions`: режим взрывов (администратор).
async fn set_explosions(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
    body: Bytes,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let request: ExplosionRequest = match decode(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    let Some(room) = directory.get_mut(id) else {
        return room_error(RoomError::NotFound);
    };
    if let Err(error) = room.set_explosion_mode(me, request.mode) {
        return room_error(error);
    }
    persist(&state, &directory);
    build_state(&state, id, me, &directory)
}

/// `POST /api/rooms/{id}/radius`: квадрат безопасного радиуса стартовых
/// клеток (администратор); сервер клампит значение по доске и командам.
async fn set_radius(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
    body: Bytes,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let request: RadiusRequest = match decode(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    let Some(room) = directory.get_mut(id) else {
        return room_error(RoomError::NotFound);
    };
    if let Err(error) = room.set_safe_radius(me, request.radius_sq) {
        return room_error(error);
    }
    persist(&state, &directory);
    build_state(&state, id, me, &directory)
}

/// `POST /api/rooms/{id}/kick`: исключить участника (только администратор).
async fn kick_member(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
    body: Bytes,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let request: MemberRequest = match decode(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    let Some(room) = directory.get_mut(id) else {
        return room_error(RoomError::NotFound);
    };
    if let Err(error) = room.kick_member(me, request.member) {
        return room_error(error);
    }
    // Место администратора тоже можно удалить — он остаётся оператором без
    // места. Если после исключения комната опустела, она удаляется целиком.
    directory.remove_if_empty(id);
    persist(&state, &directory);
    build_state(&state, id, me, &directory)
}

/// `POST /api/rooms/{id}/move`: ход игрока — усилить или ослабить клетку.
async fn make_move(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
    body: Bytes,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let request: MoveRequest = match decode(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    let Some(room) = directory.get_mut(id) else {
        return room_error(RoomError::NotFound);
    };
    let result = if request.increase {
        room.increase(me, request.member, request.pos)
    } else {
        room.decrease(me, request.member, request.pos)
    };
    if let Err(error) = result {
        return room_error(error);
    }
    persist(&state, &directory);
    build_state(&state, id, me, &directory)
}

/// `POST /api/rooms/{id}/skip`: пропустить ход.
async fn skip_turn(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
    body: Bytes,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let request: MemberRequest = match decode(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    let Some(room) = directory.get_mut(id) else {
        return room_error(RoomError::NotFound);
    };
    if let Err(error) = room.skip(me, request.member) {
        return room_error(error);
    }
    persist(&state, &directory);
    build_state(&state, id, me, &directory)
}

/// `POST /api/rooms/{id}/surrender`: сдаться.
async fn surrender(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
    body: Bytes,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let request: MemberRequest = match decode(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    let Some(room) = directory.get_mut(id) else {
        return room_error(RoomError::NotFound);
    };
    if let Err(error) = room.surrender(me, request.member) {
        return room_error(error);
    }
    persist(&state, &directory);
    build_state(&state, id, me, &directory)
}

/// `POST /api/rooms/{id}/end`: принудительно завершить партию.
/// Доступно только администратору комнаты.
async fn end_game(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    let Some(room) = directory.get_mut(id) else {
        return room_error(RoomError::NotFound);
    };
    if let Err(error) = room.end_game(me) {
        return room_error(error);
    }
    persist(&state, &directory);
    build_state(&state, id, me, &directory)
}

/// `POST /api/rooms/{id}/order`: поменять местами двух участников
/// в последовательности ходов лобби.
async fn swap_order(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
    body: Bytes,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let request: TurnOrderRequest = match decode(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    let Some(room) = directory.get_mut(id) else {
        return room_error(RoomError::NotFound);
    };
    if let Err(error) = room.swap_turn_order(me, request.a, request.b) {
        return room_error(error);
    }
    persist(&state, &directory);
    build_state(&state, id, me, &directory)
}

/// `POST /api/rooms/{id}/start`: старт игры. Готовность запрашивающего
/// подразумевается: отмечаем его готовым — если после этого все готовы,
/// партия стартует автоматически (двойной старт невозможен).
async fn start_game(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    let Some(room) = directory.get_mut(id) else {
        return room_error(RoomError::NotFound);
    };
    if let Err(error) = room.set_ready(me, true) {
        return room_error(error);
    }
    // Автостарт уже запустил партию внутри set_ready — тогда
    // повторный запуск пропускаем.
    if !room.game().is_started() {
        let automatic = room.automatic_distribution();
        if let Err(error) = room.start(me, automatic) {
            return room_error(error);
        }
    }
    persist(&state, &directory);
    build_state(&state, id, me, &directory)
}

/// `POST /api/rooms/{id}/rematch`: сбросить завершённую партию в лобби.
/// Доступен любому участнику комнаты, состав мест и настройки не меняются.
async fn rematch_game(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(id): Path<u32>,
) -> Response {
    let me = match authenticated(&state, &headers) {
        Ok(me) => me,
        Err(response) => return response,
    };
    let id = match parse_room_id(id) {
        Ok(id) => id,
        Err(response) => return response,
    };

    let mut directory = lock_rooms(&state);
    let Some(room) = directory.get_mut(id) else {
        return room_error(RoomError::NotFound);
    };
    if let Err(error) = room.rematch(me) {
        return room_error(error);
    }
    persist(&state, &directory);
    build_state(&state, id, me, &directory)
}

fn parse_room_id(raw: u32) -> Result<RoomId, Response> {
    RoomId::new(raw).ok_or_else(|| room_error(RoomError::NotFound))
}

fn lock_rooms(state: &ApiState) -> MutexGuard<'_, RoomDirectory> {
    state
        .rooms
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Пишет текущую карту комнат на диск. redb синхронный, хранилище — только
/// страховка на случай рестарта сервера: при ошибке логируем и продолжаем.
fn persist(state: &ApiState, directory: &RoomDirectory) {
    if let Err(error) = state.room_store.save_rooms(directory.rooms()) {
        eprintln!("[Server] failed to persist rooms: {error}");
    }
}

/// Собирает ответ со снапшотом комнаты. Вызывается под блокировкой директории:
/// комната клонируется, guard отпускается уже после сборки ответа.
fn build_state(state: &ApiState, id: RoomId, me: PlayerId, directory: &RoomDirectory) -> Response {
    let Some(room) = directory.get(id) else {
        return room_error(RoomError::NotFound);
    };
    let room = room.clone();
    let names = member_names(state, &room);
    encode(
        StatusCode::OK,
        &RoomStateResponse {
            id,
            me,
            room,
            names,
        },
    )
}

/// Отображаемые имена участников по их идентификаторам (профили живут только на сервере).
fn member_names(state: &ApiState, room: &Room) -> HashMap<PlayerId, String> {
    room.members()
        .keys()
        .filter_map(|id| {
            let profile = state.users.profile(*id).ok().flatten()?;
            Some((*id, profile.name))
        })
        .collect()
}

fn count_teams(room: &Room) -> u8 {
    let teams: BTreeSet<_> = room
        .game()
        .characters()
        .values()
        .map(|c| c.team_id)
        .collect();
    saturating_u8(teams.len())
}

fn saturating_u8(value: usize) -> u8 {
    value.min(u8::MAX as usize) as u8
}

fn room_error(error: RoomError) -> Response {
    let status = match &error {
        RoomError::NotFound => StatusCode::NOT_FOUND,
        RoomError::NotOwner | RoomError::PermissionDenied => StatusCode::FORBIDDEN,
        RoomError::MembersNotReady
        | RoomError::Closed
        | RoomError::MemberNotFound
        | RoomError::Game(_) => StatusCode::CONFLICT,
        RoomError::InvalidBoardSize | RoomError::InvalidLobby => StatusCode::BAD_REQUEST,
    };
    encode(status, &error)
}
