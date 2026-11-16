#[cfg(feature = "ssr")]
use std::path::Path;
#[cfg(feature = "ssr")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "ssr")]
mod auth;
#[cfg(feature = "ssr")]
mod http;
#[cfg(feature = "ssr")]
mod rooms;
#[cfg(feature = "ssr")]
mod state;

#[cfg(feature = "ssr")]
use axum::Router;
#[cfg(feature = "ssr")]
use axum::http::{StatusCode, header};
#[cfg(feature = "ssr")]
use axum::response::Response;
#[cfg(feature = "ssr")]
use axum::routing::get;
#[cfg(feature = "ssr")]
use client::app::{App, shell};
#[cfg(feature = "ssr")]
use forup_core::api::auth::UserStore;
#[cfg(feature = "ssr")]
use forup_core::api::network::{RoomDirectory, RoomStore};
#[cfg(feature = "ssr")]
use leptos::prelude::*;
#[cfg(feature = "ssr")]
use leptos_axum::{LeptosRoutes, generate_route_list};
#[cfg(feature = "ssr")]
use state::ApiState;

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let conf = get_configuration(None)?;
    let addr = conf.leptos_options.site_addr;
    let leptos_options = conf.leptos_options;

    // Комнаты хранятся целиком: при старте сервер загружает их из базы,
    // при изменениях — сохраняет карту обратно. Старый формат базы может
    // не декодироваться — тогда стартуем с пустой картой.
    let data_dir = Path::new("data");
    tokio::fs::create_dir_all(data_dir).await?;
    let rooms_path = data_dir.join("rooms.redb");
    let store = RoomStore::open(&rooms_path)?;
    let mut directory = RoomDirectory::default();
    match store.load_rooms() {
        Ok(rooms) => {
            for (id, room) in rooms {
                directory.insert(id, room);
            }
        }
        Err(error) => eprintln!("[Server] failed to load rooms: {error}"),
    }

    let users_path = data_dir.join("users.redb");
    let users = UserStore::open(&users_path)?;
    let rooms = Arc::new(Mutex::new(directory));
    let state = ApiState::new(users, rooms, store);
    let api = auth::router(state.clone()).merge(rooms::router(state));

    println!("[Server] Initialized room storage at {rooms_path:?}");
    println!("[Server] Initialized accounts at {users_path:?}");

    let routes = generate_route_list(App);

    let app = Router::new()
        .merge(api)
        .route("/favicon.ico", get(favicon))
        .leptos_routes(&leptos_options, routes, {
            let leptos_options = leptos_options.clone();
            move || shell(leptos_options.clone())
        })
        .fallback(leptos_axum::file_and_error_handler(shell))
        .with_state(leptos_options);

    println!("[Server] Listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}

#[cfg(feature = "ssr")]
async fn favicon() -> Result<Response, StatusCode> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let favicon_path = manifest_dir.join("../client/favicon.ico");
    let bytes = tokio::fs::read(&favicon_path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "image/x-icon")
        .body(bytes.into())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?)
}

#[cfg(not(feature = "ssr"))]
fn main() {
    eprintln!("The UI server is built with `--features ssr`. Run it via `cargo leptos watch`.");
}
