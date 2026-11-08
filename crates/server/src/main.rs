#[cfg(feature = "ssr")]
use std::path::Path;

#[cfg(feature = "ssr")]
use axum::Router;
#[cfg(feature = "ssr")]
use forup_core::api::game::storage::GameStore;
#[cfg(feature = "ssr")]
use forup_core::api::network::{RoomDirectory, RoomMode, Uuid, UuidExt};
#[cfg(feature = "ssr")]
use client::app::{App, shell};
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

    // Keep the dedicated game storage bootstrap alive alongside the UI server.
    let data_dir = Path::new("data");
    tokio::fs::create_dir_all(data_dir).await?;
    let db_path = data_dir.join("game_history.redb");
    let store = GameStore::open(&db_path)?;
    let mut directory = RoomDirectory::with_history_store(store);
    let owner_id = Uuid::generate();
    let default_room_id = directory.create(
        owner_id,
        RoomMode::Online {
            global: true,
            open: true,
        },
    );

    println!("[Server] Initialized storage at {db_path:?}");
    println!(
        "[Server] Created default online room (ID: {default_room_id}) owned by user {owner_id}"
    );

    let routes = generate_route_list(App);

    let app = Router::new()
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

#[cfg(not(feature = "ssr"))]
fn main() {
    eprintln!("The UI server is built with `--features ssr`. Run it via `cargo leptos watch`.");
}
