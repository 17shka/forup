pub mod prelude {
    pub use super::{MemberData, Room, RoomDirectory, RoomError, RoomId, Status};
}

mod room;
pub use room::{MemberData, Room, RoomDirectory, RoomError, RoomId, Status};

mod protocol;
pub use protocol::{
    BoardSizeRequest, CreateRoomRequest, DistributionRequest, ExplosionRequest, LobbySeat,
    LobbySetup, MemberRequest, MoveRequest, OpenRequest, RadiusRequest, ReadyRequest, ReplayStep,
    RoomStateResponse, RoomSummary, RoomsResponse, SeatRequest, TeamRequest, TurnOrderRequest,
};

mod storage;
pub use storage::{RoomStore, StorageError};
