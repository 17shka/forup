use std::cmp::Reverse;
use std::collections::HashMap;
use std::num::NonZeroU8;
use std::ops::{Index, IndexMut};

use glam::{I8Vec2, U8Vec2};
use serde::{Deserialize, Serialize};

use crate::api::circle::prelude::*;
use crate::api::network::Uuid; // NonZeroU32

pub mod prelude {
    pub use super::{
        Board, BotConfig, CellAbility, CellData, CharacterData, CharacterId, Game, GameAction,
        GameError, GameSnapshot, MemberId, MemberStat, Score, TeamId, Timeline,
    };
}

/// Ненулевые идентификаторы: `Option<CharacterId>` и `Option<TeamId>` занимают 1 байт.
pub type CharacterId = NonZeroU8;
pub type TeamId = NonZeroU8;
pub type Score = i32;

fn dist_sq(a: U8Vec2, b: U8Vec2) -> Score {
    let dx = Score::from(a.x) - Score::from(b.x);
    let dy = Score::from(a.y) - Score::from(b.y);
    dx * dx + dy * dy
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct IdSet([u64; 4]);

impl IdSet {
    fn insert(&mut self, id: u8) {
        self.0[usize::from(id >> 6)] |= 1u64 << (id & 63);
    }

    fn contains(&self, id: u8) -> bool {
        (self.0[usize::from(id >> 6)] & (1u64 << (id & 63))) != 0
    }

    fn first_missing(&self) -> Option<CharacterId> {
        for (index, word) in self.0.iter().enumerate() {
            // Бит 0 считается занятым: id 0 не существует у NonZero-типов.
            let reserved = (index == 0) as u64;
            let available = !*word & !reserved;
            if available != 0 {
                let slot = index * u64::BITS as usize + available.trailing_zeros() as usize;
                if let Some(id) = CharacterId::new(slot as u8) {
                    return Some(id);
                }
            }
        }
        None
    }

    fn len(&self) -> usize {
        self.0.iter().map(|word| word.count_ones() as usize).sum()
    }

    fn iter(self) -> impl Iterator<Item = CharacterId> {
        (1..=u8::MAX).filter_map(move |id| CharacterId::new(id).filter(|_| self.contains(id)))
    }
}

fn first_available_id(ids: impl IntoIterator<Item = CharacterId>) -> Option<CharacterId> {
    let mut used = IdSet::default();
    for id in ids {
        used.insert(id.get());
    }
    used.first_missing()
}

#[cfg(test)]
mod id_set_tests {
    use super::first_available_id;

    #[test]
    fn returns_the_lowest_unused_id() {
        let ids = [1, 2, 4].map(|id| super::CharacterId::new(id).unwrap());
        assert_eq!(first_available_id(ids), super::CharacterId::new(3));
    }

    #[test]
    fn returns_none_when_all_ids_are_used() {
        let ids = (1..=u8::MAX).map(|id| super::CharacterId::new(id).unwrap());
        assert_eq!(first_available_id(ids), None);
    }
}

#[cfg(test)]
mod game_rule_tests {
    use super::*;

    fn cid(id: u8) -> CharacterId {
        CharacterId::new(id).unwrap()
    }

    fn tid(id: u8) -> TeamId {
        TeamId::new(id).unwrap()
    }

    fn uid(id: u32) -> Uuid {
        Uuid::new(id).unwrap()
    }

    #[test]
    fn game_requires_two_members_on_different_teams() {
        let mut game = Game::standard();
        game.add_local_member(uid(1), tid(1)).unwrap();
        assert_eq!(game.start(false), Err(GameError::NotEnoughMembers));

        game.add_local_member(uid(2), tid(1)).unwrap();
        assert_eq!(game.start(false), Err(GameError::NotEnoughTeams));
    }

    #[test]
    fn compact_snapshot_reconstructs_initial_distribution_and_turn() {
        let mut game = Game::new(Board::new_rect(U8Vec2::splat(6)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.start(true).unwrap();

        let snapshot = game.snapshot();
        let restored = snapshot.restore();

        assert_eq!(restored.board, game.board);
        assert_eq!(restored.turn_order, game.turn_order);
        assert_eq!(restored.current_turn, game.current_turn);
    }

    #[test]
    fn elimination_status_is_kept_in_member_stats() {
        let mut game = Game::new(Board::new_rect(U8Vec2::splat(3)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.add_local_member(uid(3), tid(3)).unwrap();
        game.start(false).unwrap();

        game.apply_action(cid(1), GameAction::Surrender).unwrap();

        assert_eq!(game.stats[&cid(1)].losses, 1);
        assert_eq!(game.current_turn, Some(1));
        assert_eq!(game.check_turn(cid(1)), Err(GameError::CharacterEliminated));
    }

    #[test]
    fn explosions_stop_as_soon_as_one_team_is_left() {
        let mut game = Game::new(Board::new_rect(U8Vec2::splat(3)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.start(false).unwrap();
        game.stats.get_mut(&cid(1)).unwrap().increase_moves = 1;
        game.stats.get_mut(&cid(2)).unwrap().increase_moves = 1;
        game.board[U8Vec2::new(0, 0)] = CellData {
            value: 3,
            owner: Some(tid(1)),
            ..CellData::default()
        };
        game.board[U8Vec2::new(1, 0)] = CellData {
            value: 1,
            owner: Some(tid(2)),
            ..CellData::default()
        };
        game.board[U8Vec2::new(0, 1)] = CellData {
            value: 3,
            owner: Some(tid(1)),
            ..CellData::default()
        };

        let winner = game
            .apply_action(cid(1), GameAction::Increase(U8Vec2::new(0, 0)))
            .unwrap();

        assert_eq!(winner, Some(tid(1)));
        assert_eq!(game.board[U8Vec2::new(0, 1)].value, 4);
        assert!(!game.is_started());
    }

    #[test]
    fn border_absorbs_explosion_mass_and_the_chain_stabilizes() {
        let mut game = Game::new(Board::new_rect(U8Vec2::splat(3)));
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.start(false).unwrap();
        for cell in &mut game.board.cells {
            *cell = CellData {
                value: 3,
                owner: Some(tid(1)),
                ..CellData::default()
            };
        }

        let winner = game
            .apply_action(cid(1), GameAction::Increase(U8Vec2::ZERO))
            .unwrap();

        assert_eq!(winner, None);
        assert!(game.is_started());
        assert!(game.board.cells.iter().all(|cell| !cell.can_explode()));
        let remaining_mass: i32 = game
            .board
            .cells
            .iter()
            .map(|cell| i32::from(cell.value))
            .sum();
        assert!(remaining_mass < 28);
    }

    #[test]
    fn decreasing_a_zero_cell_does_not_consume_the_turn() {
        let mut game = Game::standard();
        game.add_local_member(uid(1), tid(1)).unwrap();
        game.add_local_member(uid(2), tid(2)).unwrap();
        game.start(true).unwrap();

        let pos = game
            .board
            .iter()
            .find(|(_, cell)| cell.owner == Some(tid(1)))
            .map(|(pos, _)| pos)
            .expect("team 1 starts with a cell");

        // The first decrease takes the cell from 1 to 0 and advances the turn.
        game.apply_action(cid(1), GameAction::Decrease(pos))
            .unwrap();
        assert_eq!(game.board[pos].value, 0);

        // Put the same member back on turn and try to decrease the empty cell
        // again: it must be rejected without advancing the turn.
        game.current_turn = Some(0);
        let before = game.current_turn;

        assert_eq!(
            game.apply_action(cid(1), GameAction::Decrease(pos)),
            Err(GameError::CellCannotBeDecreased)
        );
        assert_eq!(game.current_turn, before);
        assert_eq!(game.board[pos].value, 0);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameError {
    GameNotStarted,
    CharacterNotFound,
    CharacterEliminated,
    NotYourTurn,
    PositionOutOfBounds,
    FirstMoveRequiresEmptyCell,
    CellNotOwnedByTeam,
    CellCannotBeDecreased,
    NoMembers,
    NotEnoughMembers,
    NotEnoughTeams,
    GameAlreadyStarted,
    CharacterIdSpaceExhausted,
    BotIdSpaceExhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Timeline {
    Action {
        member_id: CharacterId,
        action: GameAction,
    },
    End,
}

impl Timeline {
    pub fn new(member_id: CharacterId, action: GameAction) -> Self {
        Self::Action { member_id, action }
    }

    fn game_ended() -> Self {
        Self::End
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameAction {
    Increase(U8Vec2),
    Decrease(U8Vec2),
    Skip,
    Surrender,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Game {
    pub characters: HashMap<CharacterId, CharacterData>,
    pub stats: HashMap<CharacterId, MemberStat>,
    pub turn_order: Vec<CharacterId>,
    pub current_turn: Option<usize>,
    pub timeline: Vec<Timeline>,
    pub board: Board,
    pub allow_team_change: bool,
    pub automatic_distribution: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameSnapshot {
    pub characters: Vec<(CharacterId, CharacterData)>,
    pub board_size: U8Vec2,
    pub automatic_distribution: bool,
}

impl GameSnapshot {
    pub fn restore(&self) -> Game {
        let mut game = Game::new(Board::new_rect(self.board_size));
        game.characters = self.characters.iter().copied().collect();
        game.stats = game
            .characters
            .keys()
            .map(|&id| (id, MemberStat::default()))
            .collect();
        game.turn_order = game.characters.keys().copied().collect();
        game.turn_order.sort_unstable();
        game.current_turn = Some(0);
        game.automatic_distribution = self.automatic_distribution;
        if self.automatic_distribution {
            game.distribute_start_cells();
        }
        game
    }
}

impl Game {
    pub fn standard() -> Self {
        Self::new(Board::standard())
    }

    fn next_character_id(&self) -> Option<CharacterId> {
        first_available_id(self.characters.keys().copied())
    }

    pub fn new(board: Board) -> Self {
        Self {
            board,
            allow_team_change: true,
            automatic_distribution: true,
            ..Default::default()
        }
    }

    pub fn is_started(&self) -> bool {
        self.current_turn.is_some()
    }

    pub fn snapshot(&self) -> GameSnapshot {
        let mut characters: Vec<_> = self
            .characters
            .iter()
            .map(|(&id, &character)| (id, character))
            .collect();
        characters.sort_unstable_by_key(|(id, _)| *id);
        GameSnapshot {
            characters,
            board_size: self.board.size(),
            automatic_distribution: self.automatic_distribution,
        }
    }

    pub fn set_allow_team_change(&mut self, allow: bool) {
        self.allow_team_change = allow;
    }

    pub fn change_team(
        &mut self,
        target_member: CharacterId,
        new_team: TeamId,
    ) -> Result<(), GameError> {
        if self.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }

        match self.characters.get_mut(&target_member) {
            Some(character) => {
                character.team_id = new_team;
                Ok(())
            }
            None => Err(GameError::CharacterNotFound),
        }
    }

    fn log(&mut self, member_id: CharacterId, action: GameAction) {
        self.timeline.push(Timeline::new(member_id, action));
    }

    /// Adds another seat controlled by the same local player/device.
    pub fn add_local_member(
        &mut self,
        controller: Uuid,
        team_id: TeamId,
    ) -> Result<CharacterId, GameError> {
        if self.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        let member_id = self
            .next_character_id()
            .ok_or(GameError::CharacterIdSpaceExhausted)?;

        let member = CharacterData::new(MemberId::Player(controller), team_id);
        self.characters.insert(member_id, member);
        self.stats.insert(member_id, MemberStat::default());
        Ok(member_id)
    }

    pub fn join(&mut self, uuid: Uuid, team_id: TeamId) -> Result<(), GameError> {
        if self.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        let controller = MemberId::Player(uuid);
        if self
            .characters
            .values()
            .any(|member| member.controller == controller)
        {
            return Ok(());
        }
        let character_id = self
            .next_character_id()
            .ok_or(GameError::CharacterIdSpaceExhausted)?;
        self.characters
            .insert(character_id, CharacterData::new(controller, team_id));
        self.stats.insert(character_id, MemberStat::default());
        Ok(())
    }

    pub fn leave(&mut self, member_id: CharacterId) {
        if self.is_started() {
            let _ = self.apply_action(member_id, GameAction::Surrender);
            return;
        }

        self.characters.remove(&member_id);
        self.stats.remove(&member_id);
    }

    pub fn add_bot(&mut self, team_id: TeamId) -> Result<CharacterId, GameError> {
        if self.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        let member_id = self
            .next_character_id()
            .ok_or(GameError::CharacterIdSpaceExhausted)?;
        self.characters
            .insert(member_id, CharacterData::new(MemberId::Bot, team_id));
        self.stats.insert(member_id, MemberStat::default());
        Ok(member_id)
    }

    pub fn kick_member(&mut self, member_id: CharacterId) -> Result<(), GameError> {
        if self.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        if !self.characters.contains_key(&member_id) {
            return Err(GameError::CharacterNotFound);
        }
        self.characters.remove(&member_id);
        self.stats.remove(&member_id);

        if self.is_turn(member_id) {
            self.advance_turn();
        }
        Ok(())
    }

    pub fn start(&mut self, automatic_distribution: bool) -> Result<(), GameError> {
        if self.is_started() {
            return Err(GameError::GameAlreadyStarted);
        }
        if self.characters.is_empty() {
            return Err(GameError::NoMembers);
        }
        if self.characters.len() < 2 {
            return Err(GameError::NotEnoughMembers);
        }
        let mut teams = IdSet::default();
        for member in self.characters.values() {
            teams.insert(member.team_id.get());
        }
        if teams.len() < 2 {
            return Err(GameError::NotEnoughTeams);
        }

        self.timeline.clear();
        for stat in self.stats.values_mut() {
            *stat = MemberStat::default();
        }
        self.board.clear_cells();
        self.automatic_distribution = automatic_distribution;

        self.turn_order.clear();
        self.turn_order.extend(self.characters.keys().copied());
        self.turn_order.sort_unstable();
        self.current_turn = Some(0);

        if automatic_distribution {
            self.distribute_start_cells();
        }
        Ok(())
    }

    fn distribute_start_cells(&mut self) {
        let mut teams = IdSet::default();
        for m in self.characters.values() {
            teams.insert(m.team_id.get());
        }

        let mut placed: Vec<U8Vec2> = Vec::with_capacity(teams.len());
        for team in teams.iter() {
            let mut best: Option<(Score, U8Vec2)> = None;

            for pos in self.board.ids() {
                if placed.contains(&pos) {
                    continue;
                }
                let dist = placed
                    .iter()
                    .map(|&p| dist_sq(p, pos))
                    .min()
                    .unwrap_or(Score::MAX);

                if best.is_none_or(|(d, _)| dist > d) {
                    best = Some((dist, pos));
                }
            }

            if let Some((_, pos)) = best {
                placed.push(pos);
                let cell = &mut self.board[pos];
                cell.owner = Some(team);
                cell.value = 1;
            }
        }
    }

    pub fn stop(&mut self) {
        self.current_turn = None;
        self.turn_order.clear();
        self.board.clear_cells();
    }

    pub fn end_game(&mut self) -> Result<(), GameError> {
        if !self.is_started() {
            return Err(GameError::GameNotStarted);
        }
        self.current_turn = None;
        self.timeline.push(Timeline::game_ended());
        Ok(())
    }

    fn is_turn(&self, member_id: CharacterId) -> bool {
        self.current_turn.and_then(|idx| self.turn_order.get(idx)) == Some(&member_id)
    }

    fn is_alive(&self, member_id: CharacterId) -> bool {
        self.characters.contains_key(&member_id) && !self.is_eliminated(member_id)
    }

    fn is_eliminated(&self, member_id: CharacterId) -> bool {
        self.stats
            .get(&member_id)
            .is_some_and(|stat| stat.losses > 0)
    }

    fn advance_turn(&mut self) {
        let Some(idx) = self.current_turn else {
            return;
        };
        let len = self.turn_order.len();
        self.current_turn = (1..=len)
            .map(|step| (idx + step) % len)
            .find(|&next| self.is_alive(self.turn_order[next]));
    }

    fn active_team(&self, member_id: CharacterId) -> Result<TeamId, GameError> {
        if !self.is_started() {
            return Err(GameError::GameNotStarted);
        }
        let member = self
            .characters
            .get(&member_id)
            .ok_or(GameError::CharacterNotFound)?;

        if self.is_eliminated(member_id) {
            return Err(GameError::CharacterEliminated);
        }
        Ok(member.team_id)
    }

    pub fn check_turn(&self, member_id: CharacterId) -> Result<TeamId, GameError> {
        let team_id = self.active_team(member_id)?;
        if !self.is_turn(member_id) {
            return Err(GameError::NotYourTurn);
        }
        Ok(team_id)
    }

    pub fn skip(&mut self, member_id: CharacterId) -> Result<(), GameError> {
        self.check_turn(member_id)?;
        let stat = self.stats.entry(member_id).or_default();
        stat.skipped_moves = stat.skipped_moves.saturating_add(1);
        self.advance_turn();
        Ok(())
    }

    pub fn surrender(&mut self, member_id: CharacterId) -> Result<(), GameError> {
        self.active_team(member_id)?;
        self.eliminate(member_id);
        Ok(())
    }

    pub fn is_starting_action(&self, member_id: CharacterId) -> bool {
        self.characters.get(&member_id).is_some_and(|member| {
            self.stats
                .get(&member_id)
                .is_none_or(|stat| stat.total_moves() == 0)
                && !self.board.owns_cells(member.team_id)
        })
    }

    pub fn make_move(
        &mut self,
        member_id: CharacterId,
        pos: U8Vec2,
        increase: bool,
    ) -> Result<(), GameError> {
        self.make_move_with_result(member_id, pos, increase)
            .map(|_| ())
    }

    fn make_move_with_result(
        &mut self,
        member_id: CharacterId,
        pos: U8Vec2,
        increase: bool,
    ) -> Result<Option<TeamId>, GameError> {
        let team = self.check_turn(member_id)?;
        if !self.board.contains(pos) {
            return Err(GameError::PositionOutOfBounds);
        }

        let owner = self.board[pos].owner;
        if self.board.owns_cells(team) {
            if owner != Some(team) {
                return Err(GameError::CellNotOwnedByTeam);
            }
        } else if owner.is_some() {
            return Err(GameError::FirstMoveRequiresEmptyCell);
        }

        if !increase && self.board[pos].value <= self.board[pos].ability.min_value() {
            return Err(GameError::CellCannotBeDecreased);
        }

        let stat = self.stats.entry(member_id).or_default();
        let counter = if increase {
            &mut stat.increase_moves
        } else {
            &mut stat.decrease_moves
        };
        *counter = counter.saturating_add(1);

        let delta = if increase { 1 } else { -1 };
        let mut blasts = Vec::new();
        let (characters, stats) = (&self.characters, &self.stats);
        self.board
            .apply_move_until(pos, team, delta, &mut blasts, |board| {
                Self::sole_surviving_team(board, characters, stats).is_some()
                    || board.is_saturated()
            });

        if let Some(winner) = Self::sole_surviving_team(&self.board, &self.characters, &self.stats)
        {
            self.eliminate_teams_without_cells();
            self.current_turn = None;
            self.award_win(winner);
            return Ok(Some(winner));
        }
        if self.board.is_saturated() {
            return Ok(None);
        }

        self.advance_turn();
        Ok(None)
    }

    pub fn apply_action(
        &mut self,
        member_id: CharacterId,
        action: GameAction,
    ) -> Result<Option<TeamId>, GameError> {
        if !self.is_started() {
            return Err(GameError::GameNotStarted);
        }
        if self.is_starting_action(member_id)
            && !matches!(action, GameAction::Increase(_) | GameAction::Surrender)
        {
            return Err(GameError::FirstMoveRequiresEmptyCell);
        }

        let move_winner = match action {
            GameAction::Increase(pos) => self.make_move_with_result(member_id, pos, true)?,
            GameAction::Decrease(pos) => self.make_move_with_result(member_id, pos, false)?,
            GameAction::Skip => {
                self.skip(member_id)?;
                None
            }
            GameAction::Surrender => {
                self.surrender(member_id)?;
                None
            }
        };

        self.log(member_id, action);

        if move_winner.is_some() {
            return Ok(move_winner);
        }
        if !self.is_started() {
            return Ok(None);
        }
        self.check_game_status()
    }

    pub fn check_game_status(&mut self) -> Result<Option<TeamId>, GameError> {
        if !self.is_started() {
            return Err(GameError::GameNotStarted);
        }

        if let Some(winner) = Self::sole_surviving_team(&self.board, &self.characters, &self.stats)
        {
            self.eliminate_teams_without_cells();
            self.current_turn = None;
            self.award_win(winner);
            return Ok(Some(winner));
        }

        if self.board.is_saturated() {
            return Ok(self.finish_by_mass());
        }

        self.eliminate_teams_without_cells();
        Ok(self.finish_by_last_team())
    }

    fn sole_surviving_team(
        board: &Board,
        characters: &HashMap<CharacterId, CharacterData>,
        stats: &HashMap<CharacterId, MemberStat>,
    ) -> Option<TeamId> {
        let mut all = IdSet::default();
        let mut alive = IdSet::default();

        for (&member_id, member) in characters {
            all.insert(member.team_id.get());
            let stat = stats.get(&member_id);
            let has_moved = stat.is_some_and(|stat| stat.total_moves() > 0);
            let is_eliminated = stat.is_some_and(|stat| stat.losses > 0);
            if !is_eliminated && (!has_moved || board.owns_cells(member.team_id)) {
                alive.insert(member.team_id.get());
            }
        }

        if all.len() < 2 || alive.len() != 1 {
            return None;
        }
        alive.iter().next()
    }

    fn finish_by_mass(&mut self) -> Option<TeamId> {
        self.current_turn = None;
        let winner = self.board.mass_leader()?;

        self.award_win(winner);

        let losers: Vec<CharacterId> = self
            .characters
            .iter()
            .filter(|(id, m)| m.team_id != winner && !self.is_eliminated(**id))
            .map(|(id, _)| *id)
            .collect();

        for id in losers {
            self.eliminate(id);
        }
        Some(winner)
    }

    fn eliminate_teams_without_cells(&mut self) {
        let mut alive = IdSet::default();
        for (_, cell) in self.board.iter() {
            if let Some(owner) = cell.owner {
                alive.insert(owner.get());
            }
        }

        let dead: Vec<CharacterId> = self
            .characters
            .iter()
            .filter(|(id, m)| {
                !self.is_eliminated(**id)
                    && self
                        .stats
                        .get(*id)
                        .is_some_and(|stat| stat.total_moves() > 0)
                    && !alive.contains(m.team_id.get())
            })
            .map(|(id, _)| *id)
            .collect();

        for id in dead {
            self.eliminate(id);
        }
    }

    fn finish_by_last_team(&mut self) -> Option<TeamId> {
        let mut all = IdSet::default();
        let mut alive = IdSet::default();

        for (&member_id, member) in &self.characters {
            all.insert(member.team_id.get());
            if !self.is_eliminated(member_id) {
                alive.insert(member.team_id.get());
            }
        }

        if all.len() < 2 || alive.len() != 1 {
            return None;
        }

        let winner = alive.iter().next()?;
        self.current_turn = None;
        self.award_win(winner);
        Some(winner)
    }

    fn award_win(&mut self, team: TeamId) {
        for (&member_id, member) in &self.characters {
            if member.team_id == team {
                let stat = self.stats.entry(member_id).or_default();
                stat.wins = stat.wins.saturating_add(1);
            }
        }
    }

    fn eliminate(&mut self, member_id: CharacterId) {
        if self.characters.contains_key(&member_id) {
            let stat = self.stats.entry(member_id).or_default();
            if stat.losses == 0 {
                stat.losses = 1;
            }
        }
        if self.is_turn(member_id) {
            self.advance_turn();
        }
    }

    pub fn move_weights(&self, member_id: CharacterId, cfg: &BotConfig) -> Vec<(U8Vec2, Score)> {
        let Ok(team) = self.check_turn(member_id) else {
            return Vec::new();
        };

        let mut scratch = Board::default();
        scratch.clone_from(&self.board);
        let mut blasts = Vec::new();

        self.legal_cells(team)
            .into_iter()
            .map(|pos| {
                scratch.clone_from(&self.board);
                let steps = scratch.apply_move(pos, team, 1, &mut blasts);
                let chain = (steps.saturating_sub(1) as Score) * cfg.chain_bonus;
                (pos, scratch.evaluate(team, cfg) + chain)
            })
            .collect()
    }

    pub fn bot_action(&self, member_id: CharacterId, cfg: &BotConfig) -> GameAction {
        cfg.pick_best_action(&self.move_weights(member_id, cfg))
    }

    fn legal_cells(&self, team: TeamId) -> Vec<U8Vec2> {
        let has_cells = self.board.owns_cells(team);

        self.board
            .iter()
            .filter(|(_, c)| {
                if has_cells {
                    c.owner == Some(team)
                } else {
                    c.owner.is_none()
                }
            })
            .map(|(pos, _)| pos)
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BotConfig {
    pub own_cell_bonus: Score,
    pub own_value_weight: Score,
    pub enemy_value_weight: Score,
    pub threat_penalty: Score,
    pub chain_bonus: Score,
}

impl BotConfig {
    pub const STANDARD: Self = Self {
        own_cell_bonus: 1,
        own_value_weight: 1,
        enemy_value_weight: 1,
        threat_penalty: 3,
        chain_bonus: 2,
    };

    pub fn pick_best_action(&self, weights: &[(U8Vec2, Score)]) -> GameAction {
        weights
            .iter()
            .max_by_key(|&&(pos, score)| (score, Reverse((pos.y, pos.x))))
            .map_or(GameAction::Skip, |&(pos, _)| GameAction::Increase(pos))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Board {
    cells: Vec<CellData>,
    size: U8Vec2,
}

impl Default for Board {
    fn default() -> Self {
        Self {
            cells: Vec::new(),
            size: U8Vec2::ZERO,
        }
    }
}

impl Index<U8Vec2> for Board {
    type Output = CellData;

    #[inline]
    fn index(&self, pos: U8Vec2) -> &CellData {
        debug_assert!(self.contains(pos));
        let idx = self.slot(pos);
        &self.cells[idx]
    }
}

impl IndexMut<U8Vec2> for Board {
    #[inline]
    fn index_mut(&mut self, pos: U8Vec2) -> &mut CellData {
        debug_assert!(self.contains(pos));
        let idx = self.slot(pos);
        &mut self.cells[idx]
    }
}

impl Board {
    // условные ограничения
    pub const MIN_SIZE: U8Vec2 = U8Vec2::splat(3);
    pub const MAX_SIZE: U8Vec2 = U8Vec2::splat(16);
    pub const STANDARD_SIZE: U8Vec2 = U8Vec2::splat(8);

    pub fn standard() -> Self {
        Self::new_rect(Self::STANDARD_SIZE)
    }

    pub fn new_rect(size: U8Vec2) -> Self {
        let mut board = Self::default();
        board.set_size(size);
        board
    }

    pub fn set_size(&mut self, size: U8Vec2) {
        self.size = size.clamp(Self::MIN_SIZE, Self::MAX_SIZE);
        let len = usize::from(self.size.x) * usize::from(self.size.y);
        self.cells.resize(len, CellData::default());
        self.clear_cells();
    }

    pub fn size(&self) -> U8Vec2 {
        self.size
    }

    pub fn cells(&self) -> &Vec<CellData> {
        &self.cells
    }

    pub fn get(&self, pos: U8Vec2) -> Option<&CellData> {
        if self.contains(pos) {
            let idx = self.slot(pos);
            Some(&self.cells[idx])
        } else {
            None
        }
    }

    pub fn contains(&self, pos: U8Vec2) -> bool {
        pos.x < self.size.x && pos.y < self.size.y
    }

    #[inline]
    fn slot(&self, pos: U8Vec2) -> usize {
        usize::from(pos.y) * usize::from(self.size.x) + usize::from(pos.x)
    }

    #[inline]
    fn pos_from_slot(&self, slot: usize) -> U8Vec2 {
        let w = usize::from(self.size.x);
        U8Vec2::new((slot % w) as u8, (slot / w) as u8)
    }

    pub fn ids(&self) -> impl Iterator<Item = U8Vec2> {
        let w = usize::from(self.size.x);
        (0..self.cells.len()).map(move |i| U8Vec2::new((i % w) as u8, (i / w) as u8))
    }

    pub fn iter(&self) -> impl Iterator<Item = (U8Vec2, CellData)> + '_ {
        let w = usize::from(self.size.x);
        self.cells
            .iter()
            .enumerate()
            .map(move |(i, &cell)| (U8Vec2::new((i % w) as u8, (i / w) as u8), cell))
    }

    pub fn owns_cells(&self, team: TeamId) -> bool {
        self.cells.iter().any(|c| c.owner == Some(team))
    }

    pub fn is_saturated(&self) -> bool {
        self.cells.iter().all(|c| c.can_explode())
    }

    pub fn mass_leader(&self) -> Option<TeamId> {
        let mut scores: [Score; 256] = [0; 256];
        let mut present = IdSet::default();

        for cell in &self.cells {
            if let Some(owner) = cell.owner {
                scores[usize::from(owner.get())] += Score::from(cell.value);
                present.insert(owner.get());
            }
        }

        present
            .iter()
            .max_by_key(|&team| (scores[usize::from(team.get())], Reverse(team)))
    }

    pub fn clear_cells(&mut self) {
        self.cells.fill(CellData::default());
    }

    fn apply_move(&mut self, pos: U8Vec2, team: TeamId, delta: i8, blasts: &mut Vec<Blast>) -> u32 {
        self.apply_move_until(pos, team, delta, blasts, |_| false)
    }

    fn apply_move_until<F>(
        &mut self,
        pos: U8Vec2,
        team: TeamId,
        delta: i8,
        blasts: &mut Vec<Blast>,
        mut stop_after_wave: F,
    ) -> u32
    where
        F: FnMut(&Board) -> bool,
    {
        let cell = &mut self[pos];
        cell.value = cell.value.saturating_add(delta).max(0);
        cell.owner = Some(team);

        let mut waves = 0;
        while self.explode_wave(blasts) {
            waves += 1;
            if stop_after_wave(self) {
                break;
            }
        }
        waves
    }

    fn explode_wave(&mut self, blasts: &mut Vec<Blast>) -> bool {
        blasts.clear();

        for (i, &cell) in self.cells.iter().enumerate() {
            if cell.can_explode() {
                let pos = self.pos_from_slot(i);
                blasts.push(Blast::new(pos, cell));
            }
        }

        if blasts.is_empty() {
            return false;
        }

        for b in blasts.iter() {
            let cell = &mut self[b.pos];
            cell.value = b.remainder;
            cell.ability = CellAbility::None;
        }

        for b in blasts.iter() {
            for &(off, dmg) in b.targets {
                let Some(t) = self.neighbor(b.pos, off) else {
                    continue;
                };
                let target = &mut self[t];
                if dmg == 0 {
                    target.value = target.value.saturating_add(b.share);
                    target.owner = b.owner;
                } else {
                    target.apply_ability_damage(dmg);
                }
            }
        }

        true
    }

    fn evaluate(&self, team: TeamId, cfg: &BotConfig) -> Score {
        let mut total = 0;
        let w = usize::from(self.size.x);

        for (i, cell) in self.cells.iter().enumerate() {
            match cell.owner {
                Some(owner) if owner == team => {
                    let value = Score::from(cell.value);
                    let mut score = cfg.own_cell_bonus + value * cfg.own_value_weight;
                    let pos = U8Vec2::new((i % w) as u8, (i / w) as u8);
                    if self.is_threatened(pos, *cell, team) {
                        score -= cfg.threat_penalty;
                    }
                    total += score;
                }
                Some(_) => {
                    total -= Score::from(cell.value) * cfg.enemy_value_weight;
                }
                None => {}
            }
        }
        total
    }

    fn is_threatened(&self, pos: U8Vec2, cell: CellData, team: TeamId) -> bool {
        cell.ability.targets().iter().any(|&(off, _)| {
            self.neighbor(pos, off).is_some_and(|n| {
                let other = self[n];
                other.owner.is_some_and(|o| o != team) && other.can_explode()
            })
        })
    }

    fn neighbor(&self, pos: U8Vec2, off: I8Vec2) -> Option<U8Vec2> {
        let nx = pos.x as i16 + off.x as i16;
        let ny = pos.y as i16 + off.y as i16;
        if nx >= 0 && nx < self.size.x as i16 && ny >= 0 && ny < self.size.y as i16 {
            Some(U8Vec2::new(nx as u8, ny as u8))
        } else {
            None
        }
    }
}

struct Blast {
    pos: U8Vec2,
    owner: Option<TeamId>,
    targets: &'static [(I8Vec2, i8)],
    share: i8,
    remainder: i8,
}

impl Blast {
    fn new(pos: U8Vec2, cell: CellData) -> Self {
        let targets = cell.ability.targets();
        // Missing cardinal outputs are sinks, so border explosions dissipate mass.
        let output_slots = targets.iter().filter(|&&(_, damage)| damage == 0).count() as i8;

        let (share, remainder) = if output_slots > 0 {
            (cell.value / output_slots, cell.value % output_slots)
        } else {
            (0, cell.value)
        };

        Self {
            pos,
            owner: cell.owner,
            targets,
            share,
            remainder,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MemberId {
    Player(Uuid),
    Bot,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberStat {
    pub increase_moves: u16,
    pub decrease_moves: u16,
    pub skipped_moves: u16,
    pub wins: u8,
    pub losses: u8,
}

impl MemberStat {
    pub fn total_moves(&self) -> u16 {
        self.increase_moves
            .saturating_add(self.decrease_moves)
            .saturating_add(self.skipped_moves)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterData {
    pub controller: MemberId,
    pub team_id: TeamId,
}

impl CharacterData {
    pub fn new(controller: MemberId, team_id: TeamId) -> Self {
        Self {
            controller,
            team_id,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellData {
    pub value: i8,
    pub ability: CellAbility,
    pub owner: Option<TeamId>,
}

impl CellData {
    pub fn can_explode(&self) -> bool {
        match self.ability {
            CellAbility::None => self.value >= self.ability.max_value(),
            CellAbility::Bomb => true,
        }
    }

    fn apply_ability_damage(&mut self, dmg: i8) {
        self.value = self.value.saturating_add(dmg).max(self.ability.min_value());
        if self.value <= 0 {
            self.owner = None;
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellAbility {
    #[default]
    None,
    Bomb,
}

impl CellAbility {
    pub fn min_value(self) -> i8 {
        0
    }

    pub fn max_value(self) -> i8 {
        self.targets().len() as i8
    }

    pub fn targets(&self) -> &'static [(I8Vec2, i8)] {
        match self {
            CellAbility::None => &NORMAL_TARGETS,
            CellAbility::Bomb => &BOMB_TARGETS,
        }
    }

    pub fn char(self) -> char {
        match self {
            CellAbility::None => ' ',
            CellAbility::Bomb => '💣',
        }
    }
}

/// A regular cell explodes into its four cardinal neighbours only. The centre
/// is not a target: including it would both raise the explosion threshold and
/// feed the mass back into the exploding cell.
const NORMAL_TARGETS: [(I8Vec2, i8); 4] = [
    (I8Vec2::X, 0),
    (I8Vec2::NEG_X, 0),
    (I8Vec2::Y, 0),
    (I8Vec2::NEG_Y, 0),
];

const BOMB_N: usize = disk_cell_count(2);
const BOMB_TARGETS: [(I8Vec2, i8); BOMB_N] =
    interpolate_by_distance(disk_cells::<BOMB_N>(2), -3, -1);

// const BOMB_TARGETS: [(I8Vec2, i8); 9] = [
//     (I8Vec2::new(-1, 1), -1),
//     (I8Vec2::new(1, -1), -1),
//     (I8Vec2::ONE, -1),
//     (I8Vec2::NEG_ONE, -1),
//     (I8Vec2::X, -2),
//     (I8Vec2::NEG_X, -2),
//     (I8Vec2::Y, -2),
//     (I8Vec2::NEG_Y, -2),
//     (I8Vec2::ZERO, -3),
// ];
