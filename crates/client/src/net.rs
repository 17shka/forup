//! HTTP-обёртка над серверным API. Все сетевые зависимости опциональны
//! (включаются фичами `hydrate`/`csr`), поэтому реальные запросы живут
//! за wasmgate, а нативные сборки получают заглушки.

use forup_core::api::network::{RoomError, RoomSummary};

#[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
use forup_core::api::auth::ApiError;
#[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
use forup_core::api::network::RoomsResponse;
#[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
use leptos::task::spawn_local;
#[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
use serde::{Serialize, de::DeserializeOwned};

/// Ошибка сетевого взаимодействия с комнатным API.
///
/// serde здесь не нужен: это чисто клиентская логика поверх `RoomError`,
/// который уже декодирован из ответа сервера.
#[derive(Clone, Debug, PartialEq, Eq, strum::IntoStaticStr)]
pub enum NetError {
    /// Сервер отклонил действие: бизнес-логика комнат.
    Room(RoomError),
    /// Сессия недействительна — игрок должен снова войти в аккаунт.
    #[strum(serialize = "error_connection")]
    Unauthorized,
    /// Игрок был исключён из комнаты: его место исчезло из снапшота.
    #[strum(serialize = "error_kicked")]
    Kicked,
    /// Транспорт: сеть, декодирование или неожиданный формат ответа.
    #[strum(serialize = "error_connection")]
    Transport(String),
}

/// Запрашивает список открытых комнат; результат приходит в `on_done`
/// из порождённой задачи. Токен передаётся по значению, чтобы future
/// не зависела от реактивного контекста.
pub fn fetch_rooms(
    token: String,
    on_done: impl FnOnce(Result<Vec<RoomSummary>, NetError>) + 'static,
) {
    #[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
    spawn_local(async move {
        let response = room_request::<RoomsResponse>("GET", "/api/rooms", &token, None).await;
        on_done(response.map(|response| response.rooms));
    });
    #[cfg(not(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr"))))]
    {
        let _ = (token, on_done);
    }
}

/// Низкий уровень: метод, путь, токен и тело запроса → статус и байты ответа.
/// Возвращает статус отдельно: не все вызывающие стороны хотят расшифрованное
/// тело ошибки одним и тем же способом.
#[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
async fn raw_request(
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Vec<u8>>,
) -> Result<(u16, Vec<u8>), String> {
    use gloo_net::http::Request;

    let builder = match method {
        "GET" => Request::get(path),
        "POST" => Request::post(path),
        "PATCH" => Request::patch(path),
        other => return Err(format!("unsupported method {other}")),
    };
    let builder = match token {
        Some(token) if !token.is_empty() => {
            builder.header("Authorization", &format!("Bearer {token}"))
        }
        _ => builder,
    };
    let request = match body {
        Some(body) => builder
            .header("Content-Type", "application/octet-stream")
            .body(body)
            .map_err(|error| error.to_string())?,
        None => builder.build().map_err(|error| error.to_string())?,
    };

    let response = request.send().await.map_err(|error| error.to_string())?;
    let status = response.status();
    let bytes = response.binary().await.map_err(|error| error.to_string())?;
    Ok((status, bytes))
}

/// Запрос к аккаунтным эндпоинтам: успешное тело декодируется в `T`,
/// неуспешное — в общем формате `ApiError` (текст ошибки).
#[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
pub(crate) async fn api_request<T: DeserializeOwned>(
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Vec<u8>>,
) -> Result<T, String> {
    let (status, bytes) = raw_request(method, path, token, body).await?;
    if (200..300).contains(&status) {
        postcard::from_bytes::<T>(&bytes).map_err(|error| error.to_string())
    } else {
        match postcard::from_bytes::<ApiError>(&bytes) {
            Ok(error) => Err(error.error),
            Err(_) => Err(format!("request failed ({status})")),
        }
    }
}

/// Запрос к комнатным эндпоинтам: 401 теряет токен сессии, ошибки комнаты
/// декодируются в `RoomError`, всё остальное — транспортная ошибка.
/// `ApiError` здесь не читается: комнаты отвечают `RoomError`-телами.
#[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
pub(crate) async fn room_request<T: DeserializeOwned>(
    method: &str,
    path: &str,
    token: &str,
    body: Option<Vec<u8>>,
) -> Result<T, NetError> {
    let (status, bytes) = raw_request(method, path, Some(token), body)
        .await
        .map_err(NetError::Transport)?;
    match status {
        401 => Err(NetError::Unauthorized),
        200..=299 => postcard::from_bytes::<T>(&bytes)
            .map_err(|error| NetError::Transport(error.to_string())),
        _ => match postcard::from_bytes::<RoomError>(&bytes) {
            Ok(error) => Err(NetError::Room(error)),
            Err(_) => Err(NetError::Transport(format!("request failed ({status})"))),
        },
    }
}

/// Кодирует тело запроса в postcard-байты.
#[cfg(all(target_arch = "wasm32", any(feature = "hydrate", feature = "csr")))]
pub(crate) fn encode_body<B: Serialize>(body: &B) -> Result<Vec<u8>, NetError> {
    postcard::to_allocvec(body).map_err(|error| NetError::Transport(error.to_string()))
}
