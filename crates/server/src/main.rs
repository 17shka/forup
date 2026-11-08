use std::net::SocketAddr;
use std::path::Path;

use forup_core::api::game::storage::GameStore;
use forup_core::api::network::{RoomDirectory, RoomMode, Uuid, UuidExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("[Server] Starting dedicated headless game server...");

    let data_dir = Path::new("data");
    tokio::fs::create_dir_all(data_dir).await?;
    let db_path = data_dir.join("game_history.redb");

    let store = GameStore::open(&db_path)?;
    let mut directory = RoomDirectory::with_history_store(store);

    // Create a default public room
    let owner_id = Uuid::generate();
    let default_room_id = directory.create(
        owner_id,
        RoomMode::Online {
            global: true,
            open: true,
        },
    );

    println!("[Server] Initialized storage at {:?}", db_path);
    println!(
        "[Server] Created default online room (ID: {default_room_id}) owned by user {owner_id}"
    );

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    println!("[Server] Headless server listening on http://{addr} (UI disabled)");

    // Keep server process running
    tokio::signal::ctrl_c().await?;
    println!("[Server] Shutting down gracefully...");

    Ok(())
}
