use std::collections::BTreeMap;

use super::board::{BotConfig, CellData, CharacterId, MemberId, TeamId};
use crate::api::network::Room;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameMemberState {
    pub id: CharacterId,
    pub team_id: TeamId,
    pub is_bot: bool,
    pub is_current_turn: bool,
    pub is_eliminated: bool,
    pub increase_moves: u16,
    pub decrease_moves: u16,
    pub skipped_moves: u16,
    pub wins: u8,
    pub losses: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameTeamState {
    pub team_id: TeamId,
    pub cell_count: u16,
    pub mass: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameViewState {
    pub width: u8,
    pub height: u8,
    /// Ячейки доски построчно (index = y * width + x), как хранит `Board`.
    pub cells: Vec<CellData>,
    /// Индексы ячеек, в которые текущий игрок может сделать ход.
    pub legal_moves: Vec<u16>,
    pub members: Vec<GameMemberState>,
    pub teams: Vec<GameTeamState>,
    pub current_member_id: Option<CharacterId>,
    pub is_bot_turn: bool,
    pub can_decrease: bool,
    pub can_skip: bool,
    pub is_started: bool,
    pub is_game_over: bool,
    pub automatic_distribution: bool,
    pub winner_team: Option<TeamId>,
}

/// Проецирует состояние игры в комнате в плоскую структуру для UI
/// (или для отправки сетевым клиентам сервером).
impl Room {
    pub fn view_state(&self) -> GameViewState {
        let game = self.game();
        let current_member_id = game
            .current_turn
            .and_then(|turn| game.turn_order.get(turn).copied());
        let is_bot_turn = current_member_id
            .and_then(|id| game.characters.get(&id))
            .is_some_and(|member| member.controller == MemberId::Bot);
        let is_human_turn = game.is_started() && !is_bot_turn;
        let needs_first_move = current_member_id.is_some_and(|id| game.is_starting_action(id));
        let legal_moves = current_member_id
            .filter(|_| is_human_turn)
            .map(|id| {
                game.move_weights(id, &BotConfig::STANDARD)
                    .into_iter()
                    .map(|(position, _)| u16::from(position.slot()))
                    .collect()
            })
            .unwrap_or_default();

        let mut member_ids: Vec<_> = game.characters.keys().copied().collect();
        member_ids.sort_unstable();
        let mut members: Vec<_> = member_ids
            .into_iter()
            .filter_map(|id| {
                let member = game.characters.get(&id)?;
                let stats = game.stats.get(&id).copied().unwrap_or_default();
                Some(GameMemberState {
                    id,
                    team_id: member.team_id,
                    is_bot: member.controller == MemberId::Bot,
                    is_current_turn: current_member_id == Some(id),
                    is_eliminated: stats.losses > 0,
                    increase_moves: stats.increase_moves,
                    decrease_moves: stats.decrease_moves,
                    skipped_moves: stats.skipped_moves,
                    wins: stats.wins,
                    losses: stats.losses,
                })
            })
            .collect();
        members.sort_unstable_by_key(|member| (member.team_id, member.id));

        let mut team_scores = BTreeMap::<TeamId, (u16, i32)>::new();
        for member in game.characters.values() {
            team_scores.entry(member.team_id).or_default();
        }
        for cell in game.board.cells() {
            if let Some(team_id) = cell.owner {
                let score = team_scores.entry(team_id).or_default();
                score.0 = score.0.saturating_add(1);
                score.1 = score.1.saturating_add(i32::from(cell.value));
            }
        }
        let teams = team_scores
            .into_iter()
            .map(|(team_id, (cell_count, mass))| GameTeamState {
                team_id,
                cell_count,
                mass,
            })
            .collect();
        let winner_team = game.characters.iter().find_map(|(&id, member)| {
            game.stats
                .get(&id)
                .is_some_and(|stats| stats.wins > 0)
                .then_some(member.team_id)
        });

        GameViewState {
            width: game.board.size().w(),
            height: game.board.size().h(),
            cells: game.board.cells().clone(),
            legal_moves,
            members,
            teams,
            current_member_id,
            is_bot_turn,
            can_decrease: is_human_turn && !needs_first_move,
            can_skip: is_human_turn && !needs_first_move,
            is_started: game.is_started(),
            is_game_over: self.has_started() && !game.is_started(),
            automatic_distribution: game.automatic_distribution,
            winner_team,
        }
    }
}
