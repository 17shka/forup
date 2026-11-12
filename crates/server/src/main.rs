#[cfg(feature = "ssr")]
use std::path::Path;

#[cfg(feature = "ssr")]
mod api;

#[cfg(feature = "ssr")]
use api::{ApiState, router as api_router};
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
use forup_core::api::network::{RoomDirectory, RoomMode, RoomStore, Uuid, UuidExt};
#[cfg(feature = "ssr")]
use leptos::prelude::*;
#[cfg(feature = "ssr")]
use leptos_axum::{LeptosRoutes, generate_route_list};

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let conf = get_configuration(None)?;
    let addr = conf.leptos_options.site_addr;
    let leptos_options = conf.leptos_options;

    // Комнаты хранятся целиком: при старте сервер загружает их из базы,
    // при изменениях — сохраняет карту обратно.
    let data_dir = Path::new("data");
    tokio::fs::create_dir_all(data_dir).await?;
    let rooms_path = data_dir.join("rooms.redb");
    let store = RoomStore::open(&rooms_path)?;
    let mut directory = RoomDirectory::default();
    for (id, room) in store.load_rooms()? {
        directory.insert(id, room);
    }
    if directory.is_empty() {
        let owner_id = Uuid::generate();
        let default_room_id = directory.create(
            owner_id,
            RoomMode::Online {
                global: true,
                open: true,
            },
        );
        store.save_rooms(directory.rooms())?;
        println!(
            "[Server] Created default online room (ID: {default_room_id}) owned by user {owner_id}"
        );
    }

    let users_path = data_dir.join("users.redb");
    let users = UserStore::open(&users_path)?;
    let api = api_router(ApiState::new(users));

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
