use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub mod prelude {
    pub use super::{
        GameRights, MemberData, MemberRights, PlayerRights, Rights, Room, RoomDirectory, RoomError,
        RoomId, RoomMode, Status, Uuid, UuidExt,
    };
}

mod room;
pub use room::{
    GameRights, MemberData, MemberRights, PlayerRights, Rights, Room, RoomDirectory, RoomError,
    RoomId, RoomMode, Status,
};

mod storage;
pub use storage::{RoomStore, StorageError};

/// Ненулевой идентификатор: `Option<Uuid>` занимает столько же места, сколько `u32`.
pub type Uuid = NonZeroU32;

pub trait UuidExt {
    fn generate() -> Self;
}

impl UuidExt for Uuid {
    fn generate() -> Self {
        static NEXT_ID: AtomicU32 = AtomicU32::new(1);
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed) ^ time;
        NonZeroU32::new(id)
            .or_else(|| NonZeroU32::new(NEXT_ID.fetch_add(1, Ordering::Relaxed)))
            .expect("uuid space exhausted")
    }
}
