use std::cmp::Reverse;

use serde::{Deserialize, Serialize};

use super::code::BoardCode;
use super::ids::{CharacterId, Score};
use super::rules::Game;

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

    /// Лучший ход по весам: наибольший счёт, при равенстве — клетка с
    /// меньшим кодом. `None` — легальных ходов нет.
    pub fn best_position(weights: &[(BoardCode, Score)]) -> Option<BoardCode> {
        weights
            .iter()
            .max_by_key(|&&(pos, score)| (score, Reverse(pos.slot())))
            .map(|&(pos, _)| pos)
    }
}

impl Game {
    pub fn move_weights(&self, member_id: CharacterId, cfg: &BotConfig) -> Vec<(BoardCode, Score)> {
        let Ok(team) = self.check_turn(member_id) else {
            return Vec::new();
        };

        let has_cells = self.board.owns_cells(team);

        // When a team has no cells yet (e.g. automatic distribution is off),
        // collect enemy positions so starting cells can be placed as far away
        // from other captured cells as possible.
        let enemy_positions: Vec<BoardCode> = if !has_cells {
            self.board
                .occupied_positions()
                .filter(|&pos| self.board[pos].owner != Some(team))
                .collect()
        } else {
            Vec::new()
        };

        // Копия доски под перебор: каждая цель проверяется на своём состоянии,
        // поэтому буфер один и переиспользуется через `clone_from`.
        let mut scratch = self.board.clone();
        self.legal_targets(member_id)
            .into_iter()
            .map(|pos| {
                scratch.clone_from(&self.board);
                let steps = scratch.apply_move(pos, team, 1, self.explosion_mode);
                let chain = (steps.saturating_sub(1) as Score) * cfg.chain_bonus;
                let mut score = scratch.evaluate(team, cfg) + chain;

                // With no owned cells, every empty cell scores equally. Break the
                // tie by distance: prefer starting positions far from any enemy
                // cell so bots don't all clump in the same corner.
                if !has_cells && !enemy_positions.is_empty() {
                    let min_dist_sq = enemy_positions
                        .iter()
                        .map(|&enemy| self.board.distance_sq(pos, enemy))
                        .min()
                        .unwrap_or(Score::MAX);
                    score += min_dist_sq;
                }

                (pos, score)
            })
            .collect()
    }

    /// Ход бота: лучшая клетка по весам или `None` — легальных ходов нет
    /// и бот пропускает ход.
    pub fn bot_move(&self, member_id: CharacterId, cfg: &BotConfig) -> Option<BoardCode> {
        BotConfig::best_position(&self.move_weights(member_id, cfg))
    }
}
