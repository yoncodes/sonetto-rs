use std::collections::HashSet;

use sonettobuf::{ActEffect, Fight, FightStep};

use super::EffectContext;
use super::action::{BuffActCtx, BuffStage, DispatchCtx, run_registered_handler};
use crate::state::battle::event::{events_to_act_effects};
use crate::state::battle::manager::{buff_mgr::BuffInstance, fight_data_mgr::Managers};
use crate::state::battle::context::hook_call;
use crate::state::battle::skill::get_entity;
use crate::state::battle::types::effects::EffectType;

/// Dispatch every active carrier buff feature whose `buff_act.effect_time`
/// matches `stage`. Unclaimed act_types are skipped so staged migrations can
/// land one mechanic family at a time.
pub fn dispatch_stage(stage: BuffStage, ctx: &mut DispatchCtx<'_, '_>) -> Vec<ActEffect> {
    tracing::debug!(target: "buff_act_dispatch", ?stage, "dispatch_stage start");

    let mut out = Vec::new();
    let target_effect_time = stage.effect_time();
    let cfg = config::configs::get();
    let carriers = iter_active_carriers(stage, ctx.effect_ctx);

    for (owner_uid, carrier) in carriers {
        let Some(buff_cfg) = cfg.skill_buff.get(carrier.buff_id) else {
            continue;
        };
        if buff_cfg.features.is_empty() {
            continue;
        }

        for entry in buff_cfg.features.split('|') {
            let parts: Vec<&str> = entry.split('#').collect();
            let Some(act_id) = parse_act_id(&parts) else {
                continue;
            };
            let Some(act) = cfg.buff_act.get(act_id) else {
                continue;
            };

            let eligible = match target_effect_time {
                Some(effect_time) => act.effect_time == effect_time,
                None => stage_matches_by_type(stage, act.r#type.as_str()),
            };
            if !eligible {
                continue;
            }

            if ctx.is_synthetic && stage_blocks_re_entry(stage) {
                tracing::trace!(
                    target: "buff_act_dispatch",
                    buff_id = carrier.buff_id,
                    act_id,
                    ?stage,
                    "skip synthetic re-entry"
                );
                continue;
            }

            tracing::trace!(
                target: "buff_act_dispatch",
                buff_id = carrier.buff_id,
                act_id,
                act_type = %act.r#type,
                effect_time = act.effect_time,
                ?stage,
                owner_uid,
            );

            let mut buff_ctx = BuffActCtx {
                effect_ctx: ctx.effect_ctx,
                executor: ctx.executor,
                buff_id: carrier.buff_id,
                owner_uid,
                carrier: Some(carrier.clone()),
                condition_id: ctx.condition_id,
                has_bloodpool: ctx.has_bloodpool,
                is_synthetic: ctx.is_synthetic,
            };
            let Some(result) =
                run_registered_handler(act.r#type.as_str(), stage, &parts, &mut buff_ctx)
            else {
                continue;
            };

            out.extend(result.effects);
            ctx.executor.side_effects.extend(result.side_effects);
            ctx.executor.pending_buff_dels.extend(result.buff_dels);
            ctx.executor
                .pending_monitor_triggers
                .extend(result.monitor_triggers);
        }
    }

    let fight = ctx.effect_ctx.fight().clone();
    append_stage_postprocess(stage, &fight, ctx.effect_ctx.managers, &mut out);

    tracing::debug!(
        target: "buff_act_dispatch",
        ?stage,
        emit_count = out.len(),
        "dispatch_stage end"
    );

    out
}

pub fn dedupe_dead_effects_against_prior_steps(step: &mut FightStep, prior_steps: &[FightStep]) {
    let mut prior_dead_targets = HashSet::new();
    for prior_step in prior_steps {
        collect_dead_targets(&prior_step.act_effect, &mut prior_dead_targets);
    }
    if prior_dead_targets.is_empty() {
        return;
    }

    step.act_effect.retain(|effect| {
        effect.effect_type != Some(EffectType::Dead as i32)
            || effect
                .target_id
                .map(|target_id| !prior_dead_targets.contains(&target_id))
                .unwrap_or(true)
    });
}

fn parse_act_id(parts: &[&str]) -> Option<i32> {
    parts.first()?.trim().parse().ok()
}

fn stage_matches_by_type(_stage: BuffStage, _act_type: &str) -> bool {
    false
}

fn stage_blocks_re_entry(stage: BuffStage) -> bool {
    matches!(
        stage,
        BuffStage::OnCast | BuffStage::PostSkill | BuffStage::BeAttackedReactive
    )
}

fn iter_active_carriers(
    stage: BuffStage,
    effect_ctx: &EffectContext<'_>,
) -> Vec<(i64, BuffInstance)> {
    let mut carriers = Vec::new();
    for owner_uid in iter_entity_uids_in_order(effect_ctx.fight(), stage_keeps_only_alive(stage)) {
        carriers.extend(
            effect_ctx
                .buff_mgr()
                .get(owner_uid)
                .iter()
                .cloned()
                .map(|carrier| (owner_uid, carrier)),
        );
    }
    carriers
}

fn stage_keeps_only_alive(stage: BuffStage) -> bool {
    !matches!(stage, BuffStage::OnDeath)
}

fn iter_entity_uids_in_order(fight: &Fight, alive_only: bool) -> Vec<i64> {
    let mut out = Vec::new();
    if let Some(attacker) = fight.attacker.as_ref() {
        for entity in attacker.entitys.iter().chain(attacker.sub_entitys.iter()) {
            let Some(uid) = entity.uid else {
                continue;
            };
            if alive_only && entity.current_hp.unwrap_or(0) <= 0 {
                continue;
            }
            out.push(uid);
        }
    }
    if let Some(defender) = fight.defender.as_ref() {
        for entity in defender.entitys.iter().chain(defender.sub_entitys.iter()) {
            let Some(uid) = entity.uid else {
                continue;
            };
            if alive_only && entity.current_hp.unwrap_or(0) <= 0 {
                continue;
            }
            out.push(uid);
        }
    }

    out
}

fn append_stage_postprocess(stage: BuffStage, fight: &Fight, managers: &mut Managers, effects: &mut Vec<ActEffect>) {
    if stage == BuffStage::RoundEndDot {
        append_round_end_dot_dead_effects(fight, managers, effects);
    }
}

fn append_round_end_dot_dead_effects(fight: &Fight, managers: &mut Managers, effects: &mut Vec<ActEffect>) {
    let mut hp_state = std::collections::HashMap::<i64, (i32, i32)>::new();
    let mut killed_in_order = Vec::new();

    for effect in effects.iter() {
        let Some(step) = effect.fight_step.as_ref() else {
            continue;
        };
        let victim_uid = step.to_id.unwrap_or(0);
        if victim_uid == 0 || killed_in_order.contains(&victim_uid) {
            continue;
        }

        let damage = step
            .act_effect
            .iter()
            .find_map(|inner| match inner.effect_type {
                Some(effect_type)
                    if effect_type == EffectType::OriginDamage as i32
                        || effect_type == EffectType::OriginCrit as i32
                        || effect_type == EffectType::DeadlyPoisonOriginCrit as i32 =>
                {
                    inner.effect_num
                }
                _ => None,
            })
            .unwrap_or(0);
        if damage <= 0 {
            continue;
        }

        let (hp, shield) = hp_state.entry(victim_uid).or_insert_with(|| {
            let entity = get_entity(fight, victim_uid);
            let hp = entity.and_then(|e| e.current_hp).unwrap_or(0);
            let shield = entity.and_then(|e| e.shield_value).unwrap_or(0);
            (hp, shield)
        });

        let shield_absorbed = damage.min(*shield);
        let hp_damage = damage.saturating_sub(shield_absorbed);
        *shield = shield.saturating_sub(shield_absorbed);
        *hp = hp.saturating_sub(hp_damage);
        if *hp <= 0 {
            killed_in_order.push(victim_uid);
        }
    }

    effects.extend(killed_in_order.into_iter().flat_map(|target_id| {
        events_to_act_effects(hook_call::on_dead(managers, fight, target_id))
    }));
}

fn collect_dead_targets(effects: &[ActEffect], out: &mut HashSet<i64>) {
    for effect in effects {
        if effect.effect_type == Some(EffectType::Dead as i32)
            && let Some(target_id) = effect.target_id
        {
            out.insert(target_id);
        }
        if let Some(step) = effect.fight_step.as_ref() {
            collect_dead_targets(&step.act_effect, out);
        }
    }
}
