//! Round-open phase: card selection, deck setup, sync from `Fight`.
//!
//! This is the first phase the round-manager runs in `process_round`.
//! It seeds runtime managers (BuffMgr, ExPointMgr) from the `Fight`
//! snapshot, applies the cloth-power round-start delta, walks the
//! `BeginRoundOper` list to determine which cards the player played,
//! and produces the `RoundOpenPhaseData` payload that the rest of
//! the round phases consume.
//!
//! `FightRoundMgr` is a unit struct, so this used to be a `&self`
//! method on it — moving it to a free function and bundling the
//! payload struct alongside shrinks the round_mgr god-class without
//! threading state through the call.

use std::sync::Once;

use rand::{Rng, rngs::StdRng};
use sonettobuf::{BeginRoundOper, CardInfo, FightStep};

use sonettobuf::Fight;

use crate::state::battle::deck::DeckManager;
use crate::state::battle::{
    context::RoundContext,
    event_queue::reset_round_host_index,
    fight_step::FightStepBuilder,
    manager::{
        buff_mgr::{
            DEFENDER_BUFF_UID_START, attacker_buff_uid_checkpoint, defender_buff_uid_checkpoint,
            reset_buff_uid_to, sync_buff_uid_counters_from_mgr,
            sync_from_fight_preserve_runtime as sync_buffs_from_fight,
        },
        entity_mgr::sync_from_fight,
        round_mgr::seed_entry_max_hp_from_fight,
    },
    mechanics::injury_counter,
    operation::parser::parse_round_open_ops,
    passives::collector::{CollectedPassives, collect},
    round::{RoundState, steps::refresh::build_refresh_step},
    card::apply_card_upgrades,
};

fn ensure_battle_tracing() {
    static TRACE_INIT: Once = Once::new();

    if std::env::var_os("RUST_LOG").is_none() {
        return;
    }

    TRACE_INIT.call_once(|| {
        let _ = std::panic::catch_unwind(common::init_tracing);
    });
}

/// Output payload of the round-open phase. Carries the seeded
/// `RoundState`, the initial step list (currently a single
/// `build_refresh_step`), the collected passives for this round, and
/// the per-round handles the rest of the phases reuse
/// (selected cards, deck cap, defender uid checkpoint).
pub(crate) struct RoundOpenPhaseData {
    pub state: RoundState,
    pub steps: Vec<FightStep>,
    pub collected: CollectedPassives,
    pub selected_for_round_end: Vec<CardInfo>,
    pub defender_uid_checkpoint: i64,
}

/// Run the round-open phase:
/// - Sync `RoundContext`, `BuffMgr`, `ExPointMgr` from the `Fight`.
/// - Apply cloth-power round-start seed + recover delta.
/// - Walk `BeginRoundOper` to derive selected/temp/non-temp/remaining
///   card splits.
/// - Build the round's initial refresh step.
/// - Collect attacker/defender passives.
pub(crate) fn run(
    round_ctx: &mut RoundContext<'_, '_>,
    rng: &mut StdRng,
    deck_mgr: &mut DeckManager,
    ai_override_steps: Option<&[FightStep]>,
    operations: &[BeginRoundOper],
    replay_selected_cards: Option<&[CardInfo]>,
    replay_silent_ops: Option<&[bool]>,
) -> RoundOpenPhaseData {
    ensure_battle_tracing();
    round_ctx.sync();
    tracing::warn!("process_round round_index={}", round_ctx.round_index);
    let ctx = &mut *round_ctx.fight_ctx;
    ctx.clear_round_active_card_casts();
    ctx.mechanics.emission_timeline.reset(round_ctx.round_index);
    reset_round_host_index();
    let battle_id = ctx.fight.battle_id.unwrap_or(0);
    injury_counter::sync_round_injury_index(battle_id, 1, round_ctx.round_index);
    injury_counter::sync_round_injury_index(battle_id, 2, round_ctx.round_index);
    seed_entry_max_hp_from_fight(ctx.fight);
    sync_from_fight(ctx.fight, &mut ctx.managers.entity_mgr);
    sync_buffs_from_fight(ctx.fight, &mut ctx.managers.buff_mgr);
    sync_buff_uid_counters_from_mgr(&ctx.managers.buff_mgr);

    if let Some(a) = &ctx.fight.attacker {
        for e in &a.entitys {
            tracing::warn!(
                "process_round ctx.fight uid={} hp={}",
                e.uid.unwrap_or(0),
                e.current_hp.unwrap_or(0)
            );
        }
    }

    let mut state = RoundState::new(ctx.fight);
    let attacker_uid_checkpoint = attacker_buff_uid_checkpoint();
    let mut defender_uid_checkpoint = defender_buff_uid_checkpoint();
    if defender_uid_checkpoint < DEFENDER_BUFF_UID_START {
        defender_uid_checkpoint = DEFENDER_BUFF_UID_START;
    }
    reset_buff_uid_to(attacker_uid_checkpoint.max(0));

    state.ai_override_steps = ai_override_steps.map(|steps| steps.to_vec());
    if let Some(cards) = replay_selected_cards {
        if !cards.is_empty() {
            state.replay_selected_cards = Some(cards.to_vec());
        }
    }
    if let Some(ops) = replay_silent_ops {
        if !ops.is_empty() {
            state.replay_silent_ops = Some(ops.to_vec());
        }
    }

    deck_mgr.player_hand.retain(|c| c.uid.unwrap_or(0) > 0 || c.temp_card.unwrap_or(false));

    let parsed = parse_round_open_ops(operations, deck_mgr, ctx.fight);
    state.selected_cards = parsed.selected_cards.clone();
    state.player_events = parsed.player_events;

    let selected_cards = parsed.selected_cards;

    let steps = vec![build_refresh_step(selected_cards.clone(), deck_mgr.player_hand.clone(), deck_mgr.player_deck.len() as i32)];

    let collected = collect(ctx.fight, ctx.fight.battle_id.unwrap_or(0));

    RoundOpenPhaseData {
        state,
        steps,
        collected,
        selected_for_round_end: selected_cards.clone(),
        defender_uid_checkpoint,
    }
}
