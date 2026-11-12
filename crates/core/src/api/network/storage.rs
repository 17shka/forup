//! Хранилище комнат: сериализация карты `BTreeMap<RoomId, Room>` в redb.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition, TableError};
use serde::{Serialize, de::DeserializeOwned};

use super::room::{Room, RoomId};

const ROOMS: TableDefinition<u32, &[u8]> = TableDefinition::new("rooms");

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageError {
    Database(String),
    Serialization(String),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(message) => write!(f, "database error: {message}"),
            Self::Serialization(message) => write!(f, "serialization error: {message}"),
        }
    }
}

impl std::error::Error for StorageError {}

#[derive(Clone)]
pub struct RoomStore {
    database: Arc<Database>,
}

impl std::fmt::Debug for RoomStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoomStore").finish_non_exhaustive()
    }
}

impl RoomStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let database =
            Database::create(path).map_err(|error| StorageError::Database(error.to_string()))?;
        Ok(Self {
            database: Arc::new(database),
        })
    }

    /// Полностью синхронизирует базу с текущей картой комнат: удалённые
    /// комнаты исчезают из хранилища, остальные перезаписываются.
    pub fn save_rooms(&self, rooms: &BTreeMap<RoomId, Room>) -> Result<(), StorageError> {
        let write = self
            .database
            .begin_write()
            .map_err(|error| StorageError::Database(error.to_string()))?;
        {
            let mut table = write
                .open_table(ROOMS)
                .map_err(|error| StorageError::Database(error.to_string()))?;
            // Удалять можно только после сбора ключей: менять таблицу прямо
            // во время итерации по ней нельзя.
            let mut existing = Vec::new();
            for entry in table
                .iter()
                .map_err(|error| StorageError::Database(error.to_string()))?
            {
                let (key, _) = entry.map_err(|error| StorageError::Database(error.to_string()))?;
                existing.push(key.value());
            }
            for key in existing {
                if !rooms.contains_key(&RoomId::new(key).expect("room ids are non-zero")) {
                    table
                        .remove(key)
                        .map_err(|error| StorageError::Database(error.to_string()))?;
                }
            }
            for (id, room) in rooms {
                let bytes = encode(room)?;
                table
                    .insert(id.get(), bytes.as_slice())
                    .map_err(|error| StorageError::Database(error.to_string()))?;
            }
        }
        write
            .commit()
            .map_err(|error| StorageError::Database(error.to_string()))?;
        Ok(())
    }

    /// Загружает все комнаты. Пустая база (таблицы ещё нет) — не ошибка.
    pub fn load_rooms(&self) -> Result<BTreeMap<RoomId, Room>, StorageError> {
        let read = self
            .database
            .begin_read()
            .map_err(|error| StorageError::Database(error.to_string()))?;
        let table = match read.open_table(ROOMS) {
            Ok(table) => table,
            Err(TableError::TableDoesNotExist(_)) => return Ok(BTreeMap::new()),
            Err(error) => return Err(StorageError::Database(error.to_string())),
        };
        let mut rooms = BTreeMap::new();
        for entry in table
            .iter()
            .map_err(|error| StorageError::Database(error.to_string()))?
        {
            let (key, value) = entry.map_err(|error| StorageError::Database(error.to_string()))?;
            let id = RoomId::new(key.value())
                .ok_or_else(|| StorageError::Database("a stored room id is zero".into()))?;
            rooms.insert(id, decode(value.value())?);
        }
        Ok(rooms)
    }
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, StorageError> {
    postcard::to_allocvec(value).map_err(|error| StorageError::Serialization(error.to_string()))
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, StorageError> {
    postcard::from_bytes(bytes).map_err(|error| StorageError::Serialization(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::game::board::TeamId;
    use crate::api::network::{RoomDirectory, RoomMode, Uuid, UuidExt};

    fn uid(id: u32) -> Uuid {
        Uuid::new(id).unwrap()
    }

    #[test]
    fn rooms_roundtrip() {
        let path = std::env::temp_dir().join(format!("forup-rooms-{}.redb", Uuid::generate()));
        let store = RoomStore::open(&path).unwrap();

        let owner = uid(10);
        let guest = uid(20);
        let mut directory = RoomDirectory::default();
        let online_id = directory.create(
            owner,
            RoomMode::Online {
                global: false,
                open: true,
            },
        );
        directory.join(online_id, guest, TeamId::MIN).unwrap();

        let offline_id = directory.create(uid(11), RoomMode::Offline);
        {
            let offline = directory.get_mut(offline_id).unwrap();
            offline.add_bot(uid(11), TeamId::new(2).unwrap()).unwrap();
            offline.set_ready(uid(11), true).unwrap();
            offline.start(uid(11), true).unwrap();
        }
        assert!(directory.get(offline_id).unwrap().game().is_started());
        assert_eq!(directory.get(online_id).unwrap().members().len(), 2);

        store.save_rooms(directory.rooms()).unwrap();
        let mut loaded = store.load_rooms().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.get(&online_id).map(Room::members).unwrap().len(), 2);
        assert!(loaded.get(&offline_id).unwrap().game().is_started());

        // Перезапись: удалённая комната исчезает из хранилища.
        directory.remove(online_id, owner).unwrap();
        store.save_rooms(directory.rooms()).unwrap();
        loaded = store.load_rooms().unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(!loaded.contains_key(&online_id));

        // Восстановление в новый справочник: ключи сохраняются.
        let mut restored = RoomDirectory::default();
        for (id, room) in loaded {
            restored.insert(id, room);
        }
        assert_eq!(restored.get(offline_id).unwrap().game().is_started(), true);
        assert!(restored.get(online_id).is_none());

        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn load_from_a_fresh_database_returns_no_rooms() {
        let path = std::env::temp_dir().join(format!("forup-rooms-{}.redb", Uuid::generate()));
        let store = RoomStore::open(&path).unwrap();
        assert!(store.load_rooms().unwrap().is_empty());
        drop(store);
        std::fs::remove_file(path).unwrap();
    }
}
