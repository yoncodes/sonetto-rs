use std::collections::HashSet;

use sonettobuf::{CardInfo, Fight, FightStep};

use crate::state::battle::event::Event;

#[derive(Default, Debug, Clone)]
pub struct RoundState {
    pub ai_use_cards: Vec<CardInfo>,
    pub ai_override_steps: Option<Vec<FightStep>>,
    pub used_cards: Vec<i32>,
    pub enemy_skill_actors: HashSet<i64>,
    pub move_num: i32,
    pub is_finish: bool,
    pub before_cards2: Vec<CardInfo>,
    pub team_a_cards2: Vec<CardInfo>,
    /// Replay mode: overrides `selected_cards` when set. `play_card` uses
    /// `replay_selected_cards[op_index]` instead of `selected_cards[op_index]`.
    pub replay_selected_cards: Option<Vec<CardInfo>>,
    /// Replay mode: per-op flag. When `replay_silent_ops[op_index]` is true,
    /// `play_card` returns `FightStep::default()` (no SKILL emission) — matching
    /// LIVE's behavior for ops that consume cards via mechanics that don't emit
    /// top-level SKILL.
    pub replay_silent_ops: Option<Vec<bool>>,
    pub replay_wave_snapshot_applied: bool,
    pub replay_wave_snapshot_target_wave: Option<i32>,
    /// Cards selected this round in play order, populated by round_open.
    pub selected_cards: Vec<CardInfo>,
    /// Player events emitted by round_open parser, consumed by player_actions.
    pub player_events: Vec<Event>,
}

impl RoundState {
    pub fn new(_fight: &Fight) -> Self {
        Self::default()
    }
}
