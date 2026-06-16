//! Round-phase orchestration split out of the `FightRoundMgr`
//! god-class. Each module owns one phase that `process_round` calls
//! in sequence: `round_open` → `player_actions` → `non_terminal_round`
//! (which internally invokes `enemy_actions`).
//!
//! `FightRoundMgr` is a unit struct, so the phase functions are free
//! functions — they take `mgr: &FightRoundMgr` only where they need
//! to call back into FightRoundMgr's helper methods (which are also
//! state-free).

pub(crate) mod enemy_actions;
pub(crate) mod build_round_output;
pub(crate) mod non_terminal_round;
pub(crate) mod player_actions;
pub(crate) mod round_open;
