use anyhow::Result;
use rand::{SeedableRng, rngs::StdRng};
use sonettobuf::{ActEffect, Fight, FightStep, fight_step};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use super::execution_guards::{ActiveEffectGuard, DepthGuard, ReentryGuard, SkillContextGuard};
use super::super::{
    context::{FightContext, behavior_context::BehaviorContext, hook_call},
    effect::{condition::Hook, parser as effect_parser},
    event::events_to_act_effects,
    fight::defender::Defender,
    fight_step::ActEffectBuilder,
    manager::{
        buff_mgr::{BuffMgr, observe_explicit_buff_uid_for_target},
        entity_mgr::sync_from_fight,
        fight_data_mgr::Managers,
        round_mgr::seed_entry_max_hp_from_fight,
        wave_mgr::WaveMgr,
    },
    mechanics::{Mechanics, empathy::has_empathy_buff},
    types::{behavior::BehaviorType, condition::ConditionType, effects::EffectType},
};

use super::{
    cache::{SKILL_CACHE, resolve_skill_effect_id},
    condition::{self, ConditionEval, eval::{BehaviorConditionCtx, eval_behavior_condition}},
    damage::{calculate_damage, should_crit_hit},
    euphoria,
    phase::{PhaseFilter, TriggerState},
    post_process::{
        consume_attr_only_damage_buffs, inject_sotheby_consume, is_bonus_damage_config_effect,
        is_damage_effect_type, normalize_nested_steps,
    },
    targets::{
        TargetResolver, alive_enemies, alive_enemies_by_position, get_ally_uids, get_entity,
    },
};

#[derive(Default, Debug, Clone)]
pub struct SkillExecutor {
    pub side_effects: Vec<ActEffect>,
    /// (caster_uid, trigger_skill_id) pairs queued by RaspberryBigSkill/CreateMaxHpAdditionalDamageAndRemove
    pub pending_monitor_triggers: Vec<(i64, i32)>,
    /// (target_uid, buff_id) pairs for buffs that self-delete after use (e.g. AttrFromEntity)
    pub pending_buff_dels: Vec<(i64, i32)>,
    /// Per-target damage-rate bonus accumulated by SkillRateUp-like behaviors.
    pub pending_target_rate_bonus: HashMap<i64, i32>,
    /// Global damage-rate bonus applied to all fallback damage targets.
    pub pending_global_rate_bonus: i32,
    /// Current executing skill context `(skill_id, selected_target_uid)`.
    /// Nested direct-skill chains temporarily overwrite this and restore it
    /// on unwind so buff-action fanout can resolve the active hostile targets.
    current_skill_context: Option<(i32, i64)>,
    /// Per-entity temporary attribute bonuses for this skill execution.
    /// Key: (entity_uid, attr_id)
    /// TODO: remove with skill/behavior/damage.rs migration. After Task 14
    /// nothing writes to this map; surviving readers (skill/behavior/damage.rs)
    /// pass it through to lost_life::apply, which now ignores it.
    pub pending_attr_bonus: HashMap<(i64, i32), i32>,
    /// Per-team preview of bloodtithe `(value, accumulator)` for this skill execution.
    /// This lets combat damage emit live-like positive 335 packets without mutating
    /// authoritative bloodtithe state before play_step_data replays the step.
    pub pending_bloodtithe_preview: HashMap<i32, (i32, i32)>,
    /// Deferred silent summons applied by the outer caller once a mutable `Fight` is available.
    pub(crate) pending_summons: Vec<PendingSummon>,
    /// Deferred monster-form transformations from `MonsterChange`
    /// behavior — drained by `apply_pending_monster_changes` once the
    /// outer caller can take a mutable `Fight`.
    pub(crate) pending_monster_changes: Vec<PendingMonsterChange>,
    override_damage_targets: Option<Vec<i64>>,
    call_depth: usize,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PendingSummon {
    pub caster_uid: i64,
    pub monster_id: i32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PendingMonsterChange {
    pub target_uid: i64,
    pub new_monster_id: i32,
}

impl SkillExecutor {
    pub fn new() -> Self {
        Self {
            side_effects: Vec::new(),
            pending_monitor_triggers: Vec::new(),
            pending_buff_dels: Vec::new(),
            pending_target_rate_bonus: HashMap::new(),
            pending_global_rate_bonus: 0,
            current_skill_context: None,
            pending_attr_bonus: HashMap::new(),
            pending_bloodtithe_preview: HashMap::new(),
            pending_summons: Vec::new(),
            pending_monster_changes: Vec::new(),
            override_damage_targets: None,
            call_depth: 0,
        }
    }

    pub fn apply_pending_summons(
        &mut self,
        fight: &mut Fight,
        managers: &mut Managers,
    ) -> Result<()> {
        let pending = self.take_pending_summons();
        if pending.is_empty() {
            return Ok(());
        }

        Self::apply_summon_batch(fight, managers, &pending)
    }

    /// Drain queued `MonsterChange` requests and apply them to the
    /// fight via `mechanics::phase_change::transform_entity`. Called
    /// from sites that hold a mutable `Fight` after behavior dispatch.
    pub fn apply_pending_monster_changes(&mut self, fight: &mut Fight) -> Result<()> {
        let pending: Vec<PendingMonsterChange> = self.pending_monster_changes.drain(..).collect();
        for change in pending {
            crate::state::battle::mechanics::phase_change::transform_entity(
                fight,
                change.target_uid,
                change.new_monster_id,
            )?;
        }
        Ok(())
    }

    pub fn set_override_damage_targets(&mut self, targets: Vec<i64>) {
        self.override_damage_targets = Some(targets);
    }

    pub fn take_override_damage_targets(&mut self) -> Option<Vec<i64>> {
        self.override_damage_targets.take()
    }

    pub(crate) fn take_pending_summons(&mut self) -> Vec<PendingSummon> {
        self.pending_summons.drain(..).collect()
    }

    pub(crate) fn apply_summon_batch(
        fight: &mut Fight,
        managers: &mut Managers,
        summons: &[PendingSummon],
    ) -> Result<()> {
        if summons.is_empty() {
            return Ok(());
        }

        for summon in summons.iter().copied() {
            apply_pending_summon(fight, managers, summon)?;
        }

        sync_from_fight(fight, &mut managers.entity_mgr);
        seed_entry_max_hp_from_fight(fight);
        managers.entity_mgr.rebuild_cache(fight);
        managers.calculate_mgr.update_cache(fight);
        Ok(())
    }

    #[allow(clippy::too_many_arguments, clippy::extend_with_drain)]
    pub fn execute_skill(
        &mut self,
        rng: &mut StdRng,
        fight: &Fight,
        managers: &mut Managers,
        mechanics: &mut Mechanics,
        caster_uid: i64,
        target_uid: i64,
        skill_id: i32,
        phase: &PhaseFilter,
    ) -> Result<Vec<ActEffect>> {
        let exec_start = Instant::now();
        let skill_id = // euphoria::resolve_with_euphoria(fight, caster_uid, skill_id);
            skill_id;
        // Catch-all timeline record: every emission funnels through here.
        // Higher-level call sites (CardCast, TriggerCombatPassive, etc.)
        // record their own entries too — appearing twice in the timeline
        // is expected. Skills that appear ONLY with ExecutorLowLevel
        // identify call sites that lack a dedicated phase tag yet.
        let exec_record_idx = mechanics.emission_timeline.record(
            crate::state::battle::emission_timeline::EmissionPhase::ExecutorLowLevel,
            caster_uid,
            skill_id,
            0,
            None,
            None,
        );
        let Some(_reentry_guard) = ReentryGuard::enter(caster_uid, target_uid, skill_id) else {
            tracing::warn!(
                "[execute_skill] reentry loop blocked at skill={} caster={} target={}",
                skill_id,
                caster_uid,
                target_uid
            );
            return Ok(vec![]);
        };

        self.call_depth += 1;
        let _depth_guard = DepthGuard {
            depth: &mut self.call_depth as *mut usize,
        };
        let previous_skill_context = self.current_skill_context.replace((skill_id, target_uid));
        let _skill_context_guard = SkillContextGuard {
            current: &mut self.current_skill_context as *mut Option<(i32, i64)>,
            previous: previous_skill_context,
        };
        if self.call_depth > 64 {
            tracing::warn!(
                "[execute_skill] depth limit reached at skill={} caster={} target={}, skipping",
                skill_id,
                caster_uid,
                target_uid
            );
            return Ok(vec![]);
        }

        let skill_effect_id = resolve_skill_effect_id(skill_id);
        let cfg = config::configs::get();
        let skill_cfg = cfg.skill_effect.get(skill_effect_id);
        let override_damage_targets = self.take_override_damage_targets();
        self.pending_target_rate_bonus.clear();
        self.pending_global_rate_bonus = 0;
        self.pending_attr_bonus.clear();
        managers.entity_mgr.clear_attr_bonus();
        let behaviors = SKILL_CACHE
            .get(&skill_effect_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        tracing::info!(
            "[execute_skill] skill={} caster={} target={} phase={:?} behaviors={}",
            skill_id,
            caster_uid,
            target_uid,
            phase,
            behaviors.len()
        );
        let mut all_effects: Vec<ActEffect> = Vec::new();

        let parsed_effect = effect_parser::parse(skill_effect_id, caster_uid)
            .unwrap_or_else(|| crate::state::battle::effect::SkillEffect::empty(caster_uid));
        let mgr_ptr: *mut crate::state::battle::manager::active_effect_mgr::ActiveEffectMgr =
            &mut managers.active_effect_mgr;
        let prev_active_idx: Option<usize> = managers.active_effect_mgr.active_idx;
        let active_idx = managers.active_effect_mgr.push(vec![parsed_effect]);
        managers.active_effect_mgr.active_idx = Some(active_idx);
        let _active_guard = ActiveEffectGuard {
            mgr: mgr_ptr,
            idx: active_idx,
            prev_active_idx,
        };

        tracing::info!(caster_uid, skill_id, "[execute_skill] hook: eval_active_skill");
        all_effects.extend(events_to_act_effects(
            hook_call::on_eval_active_skill(managers, fight, caster_uid),
        ));

        if is_ex_skill(skill_id) {
            tracing::info!(caster_uid, skill_id, "[execute_skill] hook: use_ex_skill");
            all_effects.extend(events_to_act_effects(
                hook_call::on_use_ex_skill(managers, fight, caster_uid),
            ));
        }
        let force_effect_step = false;
        let setup_done: Instant = Instant::now();
        // let mut sim_fight = fight.clone();
        // let mut sim_buff_mgr = managers.buff_mgr.clone();
        let clones_done = Instant::now();

        // fallback damage_rate
        if let Some(skill) = skill_cfg
            && skill.damage_rate > 0
            && !all_effects.iter().any(|e| e.effect_type.map(is_damage_effect_type).unwrap_or(false))
        {
            for dmg_target in fallback_damage_targets(
                fight,
                caster_uid,
                target_uid,
                skill.logic_target.trim().parse::<i32>().unwrap_or(0),
                override_damage_targets.as_deref(),
            ) {
                let is_crit = should_crit_hit(fight, &managers.buff_mgr, &managers.entity_mgr, caster_uid, dmg_target, skill_id);
                all_effects.extend(calculate_damage(fight, &managers.buff_mgr, &managers.entity_mgr, caster_uid, dmg_target, skill.damage_rate, skill_id, is_crit));
            }
        }

        let behaviors_done = Instant::now();

        let defender_uids = damage_targets_in(&all_effects);
        if !defender_uids.is_empty() {
            tracing::info!(caster_uid, ?defender_uids, "[execute_skill] hook: eval_being_attacked");
        }
        for uid in defender_uids {
            all_effects.extend(events_to_act_effects(
                hook_call::on_eval_being_attacked(managers, fight, uid),
            ));
        }

        let dead_effects = collect_dead_effects_after_damage(fight, managers, &all_effects);
        if !dead_effects.is_empty() {
            all_effects.extend(dead_effects);
        }

        if all_effects.is_empty() && self.side_effects.is_empty() {
            tracing::info!("[execute_skill] skill={} no effects fired", skill_id);
            return Ok(vec![]);
        }

        let logic_to_id = skill_cfg
            .and_then(|s| {
                let lt = s.logic_target.trim();
                if lt.is_empty() { None } else { lt.parse::<i32>().ok() }
            })
            .and_then(|target_type| {
                TargetResolver::new(fight, caster_uid, target_uid)
                    .behavior(target_type)
                    .resolve()
                    .into_iter()
                    .next()
            })
            .unwrap_or(target_uid);

        let mut step_act_type = fight_step::ActType::Skill.into();
        if force_effect_step
            && let PhaseFilter::Combat(event) = phase
            && !event.active_use_skill
        {
            step_act_type = fight_step::ActType::Effect.into();
        }
        let total_inner_effects = all_effects.len();

        let skill_step = FightStep {
            act_type: Some(step_act_type),
            from_id: Some(caster_uid),
            to_id: Some(logic_to_id),
            act_id: Some(skill_id),
            act_effect: all_effects,
            card_index: Some(0),
            support_hero_id: Some(0),
            fake_timeline: Some(false),
            real_skill_type: Some(0),
            real_skin_id: Some(0),
        };

        let mut skill_act_effect = ActEffectBuilder::skill_wrapper(skill_step);

        let mut result = Vec::new();

        // // Fire pending monitor triggers — commented out
        // let triggers: Vec<(i64, i32)> = self.pending_monitor_triggers.drain(..).collect();
        // for (trigger_uid, trigger_skill_id) in triggers { ... }

        // // Fire pending buff deletes — commented out
        // for (del_target, del_buff_id) in self.pending_buff_dels.drain(..) { ... }

        if self.call_depth > 1 {
            if let Some(step) = skill_act_effect.fight_step.as_mut() {
                step.act_effect.extend(self.side_effects.drain(..));
            }
            result.push(skill_act_effect);
        } else {
            result.push(skill_act_effect);
            result.extend(self.side_effects.drain(..));
        }

        tracing::info!(caster_uid, skill_id, "[execute_skill] hook: after_action");
        result.extend(events_to_act_effects(
            hook_call::on_after_action(managers, fight, caster_uid),
        ));

        let result_done = Instant::now();
        let setup_ms = setup_done.duration_since(exec_start).as_millis();
        let clone_ms = clones_done.duration_since(setup_done).as_millis();
        let body_ms = behaviors_done.duration_since(clones_done).as_millis();
        let tail_ms = result_done.duration_since(behaviors_done).as_millis();
        let total_ms = result_done.duration_since(exec_start).as_millis();
        if total_ms >= 20 {
            tracing::warn!(
                "[execute_skill][timing] skill={} setup={}ms clone={}ms body={}ms tail={}ms total={}ms effects={} out={}",
                skill_id,
                setup_ms,
                clone_ms,
                body_ms,
                tail_ms,
                total_ms,
                total_inner_effects,
                result.len()
            );
        }

        tracing::info!(
            "[execute_skill] skill={} total 162s: {}",
            skill_id,
            result.len()
        );

        if !result.is_empty() {
            mechanics.emission_timeline.mark_produced(exec_record_idx);
        }
        Ok(result)
    }

    /// Execute a trigger skill and wrap as a 162 inline step.
    /// Used for CreateMaxHpAdditionalDamageAndRemove / RaspberryBigSkill procs.
    fn execute_trigger_skill(
        rng: &mut StdRng,
        fight: &Fight,
        managers: &mut Managers,
        mechanics: &mut Mechanics,
        caster_uid: i64,
        skill_id: i32,
        parent_phase: &PhaseFilter,
    ) -> Result<(ActEffect, Vec<(i64, i32)>)> {
        let mut inner_executor = SkillExecutor::new();
        let active_card_cast_uids = if let PhaseFilter::Combat(event) = parent_phase {
            event.active_card_cast_uids.clone()
        } else {
            HashSet::new()
        };
        let phase = PhaseFilter::combat_with(TriggerState {
            active_use_skill: false,
            skill_id: 0,
            action_order_index: 0,
            used_ex_skill: false,
            teammate_use_ex_skill: false,
            trigger_bullet: false,
            event_driven_only: false,
            be_attacked: false,
            hurt_magic: false,
            lost_ex_point: false,
            hurt_not_restraint: false,
            hurt_restraint: false,
            teammate_injury_count: 0,
            teammate_injury_count_not_reset: 0,
            team_injury_count_round: false,
            deleted_buff_ids: managers.buff_mgr.step_deleted_buff_ids().to_vec(),
            active_card_cast_uids,
            bloodpool_max_attacker: Some(mechanics.bloodtithe.get_max(1)),
            bloodpool_value_attacker: Some(mechanics.bloodtithe.get_value(1)),
        });

        let mut results = inner_executor.execute_skill(
            rng, fight, managers, mechanics, caster_uid, -1, skill_id, &phase,
        )?;

        let buff_dels = inner_executor.pending_buff_dels.drain(..).collect();

        let mut act_effect = if results.is_empty() {
            ActEffectBuilder::skill_wrapper_without_num(FightStep {
                act_type: Some(fight_step::ActType::Skill.into()),
                from_id: Some(caster_uid),
                to_id: Some(caster_uid),
                act_id: Some(skill_id),
                act_effect: vec![],
                card_index: Some(0),
                support_hero_id: Some(0),
                fake_timeline: Some(false),
                real_skill_type: Some(0),
                real_skin_id: Some(0),
            })
        } else {
            let has_step_damage = |step: &FightStep| {
                step.act_effect
                    .iter()
                    .any(|effect| effect.effect_type.is_some_and(is_damage_effect_type))
            };
            let selected_idx = results
                .iter()
                .position(|effect| {
                    effect
                        .fight_step
                        .as_ref()
                        .is_some_and(|step| step.act_id == Some(skill_id) && has_step_damage(step))
                })
                .or_else(|| {
                    results.iter().position(|effect| {
                        effect
                            .fight_step
                            .as_ref()
                            .is_some_and(|step| step.act_id == Some(skill_id))
                    })
                })
                .unwrap_or(0);
            results.remove(selected_idx)
        };

        // Some trigger paths return a self-wrapper SkillStep (act_id = X) that only
        // nests another SkillStep with the same act_id. Prefer the nested payload step.
        if let Some(step) = act_effect.fight_step.as_ref() {
            let parent_act_id = step.act_id;
            let parent_has_damage = step
                .act_effect
                .iter()
                .any(|effect| effect.effect_type.is_some_and(is_damage_effect_type));
            if !parent_has_damage {
                let nested_idx =
                    step.act_effect
                        .iter()
                        .position(|effect| {
                            effect.effect_type == Some(EffectType::FightStep as i32)
                                && effect.fight_step.as_ref().is_some_and(|nested| {
                                    nested.act_id == parent_act_id
                                        && nested.act_effect.iter().any(|e| {
                                            e.effect_type.is_some_and(is_damage_effect_type)
                                        })
                                })
                        })
                        .or_else(|| {
                            step.act_effect.iter().position(|effect| {
                                effect.effect_type == Some(EffectType::FightStep as i32)
                                    && effect
                                        .fight_step
                                        .as_ref()
                                        .is_some_and(|nested| nested.act_id == parent_act_id)
                            })
                        });
                if let Some(index) = nested_idx {
                    if let Some(nested) = step.act_effect.get(index) {
                        act_effect = nested.clone();
                    }
                }
            }
        }

        Ok((act_effect, buff_dels))
    }

    pub fn get_ally_uids(&self, fight: &Fight, caster_uid: i64) -> Vec<i64> {
        get_ally_uids(fight, caster_uid)
    }

    pub fn current_skill_context(&self) -> Option<(i32, i64)> {
        self.current_skill_context
    }

    pub fn add_skill_rate_bonus(&mut self, caster_uid: i64, target_uid: i64, amount: i32) {
        if amount == 0 {
            return;
        }
        // target=self behaves like a global modifier for this skill instance.
        if target_uid == caster_uid {
            self.pending_global_rate_bonus = self.pending_global_rate_bonus.saturating_add(amount);
            return;
        }
        let entry = self
            .pending_target_rate_bonus
            .entry(target_uid)
            .or_insert(0);
        *entry = entry.saturating_add(amount);
    }


}

fn behavior_execution_order(behaviors: &[super::cache::ResolvedBehavior]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..behaviors.len()).collect();

    for add_idx in 0..behaviors.len() {
        let BehaviorType::AddBuff { buff_id, .. } = behaviors[add_idx].behavior else {
            continue;
        };
        if buff_id <= 0 {
            continue;
        }

        let Some(dep_idx) = (0..add_idx).find(|idx| {
            let dep = &behaviors[*idx];
            dep.condition_target == behaviors[add_idx].behavior_target
                && condition_depends_on_buff(&dep.condition, buff_id)
        }) else {
            continue;
        };

        let Some(from_pos) = order.iter().position(|&v| v == add_idx) else {
            continue;
        };
        let Some(to_pos) = order.iter().position(|&v| v == dep_idx) else {
            continue;
        };
        if from_pos > to_pos {
            let moved = order.remove(from_pos);
            order.insert(to_pos, moved);
        }
    }

    order
}

fn condition_depends_on_buff(condition: &ConditionType, buff_id: i32) -> bool {
    condition::fold(condition, &mut |cond| match cond {
        ConditionType::HasBuffId { buff_ids }
        | ConditionType::NoBuffId { buff_ids }
        | ConditionType::PerBuffIdCount { buff_ids } => buff_ids.contains(&buff_id),
        _ => false,
    })
}

fn condition_has_combat_event(condition: &ConditionType) -> bool {
    // Keep this list in sync with `classification::is_combat_event_condition`
    // (skill/classification.rs:86). They MUST agree on which conditions count
    // as "combat events" — otherwise `has_combat_reactive_condition`-gated
    // sweep entry can let a skill in whose behaviors the executor then skips
    // (because their condition isn't recognized as combat-event), producing
    // missing emissions. Battle1 r1 step[30] (Pickles' `30630141` end-of-round
    // "Clarified Topic" with `NoActRound`) regressed when this list omitted
    // `NoActRound`/`TriggerBullet` — fixed by adding them here.
    condition::is_combat_event_condition(
        condition,
        condition::CombatEventConditionOptions::default(),
    )
}

fn apply_preview_effects_to_sim_fight(fight: &mut Fight, effects: &[ActEffect]) {
    fn update_hp(fight: &mut Fight, uid: i64, delta: i32) {
        let apply = |entitys: &mut Vec<sonettobuf::FightEntityInfo>| {
            if let Some(entity) = entitys.iter_mut().find(|e| e.uid == Some(uid)) {
                let cur = entity.current_hp.unwrap_or(0);
                entity.current_hp = Some((cur + delta).max(0));
                return true;
            }
            false
        };
        if let Some(attacker) = fight.attacker.as_mut()
            && (apply(&mut attacker.entitys) || apply(&mut attacker.sub_entitys))
        {
            return;
        }
        if let Some(defender) = fight.defender.as_mut() {
            let _ = apply(&mut defender.entitys) || apply(&mut defender.sub_entitys);
        }
    }
    fn set_hp_zero(fight: &mut Fight, uid: i64) {
        let apply = |entitys: &mut Vec<sonettobuf::FightEntityInfo>| {
            if let Some(entity) = entitys.iter_mut().find(|e| e.uid == Some(uid)) {
                entity.current_hp = Some(0);
                return true;
            }
            false
        };
        if let Some(attacker) = fight.attacker.as_mut()
            && (apply(&mut attacker.entitys) || apply(&mut attacker.sub_entitys))
        {
            return;
        }
        if let Some(defender) = fight.defender.as_mut() {
            let _ = apply(&mut defender.entitys) || apply(&mut defender.sub_entitys);
        }
    }

    for effect in effects {
        let et = effect.effect_type.unwrap_or(0);
        if let Some(step) = &effect.fight_step {
            apply_preview_effects_to_sim_fight(fight, &step.act_effect);
            continue;
        }

        let target = effect.target_id.unwrap_or(0);
        if target == 0 {
            continue;
        }

        match et {
            x if is_damage_effect_type(x) => {
                update_hp(fight, target, -effect.effect_num.unwrap_or(0));
            }
            x if x == EffectType::Heal as i32
                || x == EffectType::HealCrit as i32
                || x == EffectType::Cure2 as i32
                || x == EffectType::InjuryBankHeal as i32 =>
            {
                update_hp(fight, target, effect.effect_num.unwrap_or(0));
            }
            x if x == EffectType::Kill as i32 => set_hp_zero(fight, target),
            _ => {}
        }
    }
}

fn inject_empathy_storage_injuries(
    mechanics: &mut Mechanics,
    preview_buff_mgr: &mut BuffMgr,
    live_buff_mgr: &mut BuffMgr,
    fight: &Fight,
    source_uid: i64,
    effects: Vec<ActEffect>,
) -> Vec<ActEffect> {
    let effects = crate::state::battle::heroes::kakania::inject_damage_redirect(
        &mut mechanics.empathy,
        preview_buff_mgr,
        live_buff_mgr,
        fight,
        source_uid,
        effects,
    );

    let mut effect_targets = Vec::new();
    for effect in &effects {
        let Some(target_uid) = effect.target_id else {
            continue;
        };
        let Some(effect_type) = effect.effect_type else {
            continue;
        };
        if effect_type == EffectType::DamageFromAbsorb as i32
            || !is_damage_effect_type(effect_type)
            || target_uid == source_uid
            || effect_targets.contains(&target_uid)
            || !has_empathy_buff(preview_buff_mgr, target_uid)
        {
            continue;
        }
        effect_targets.push(target_uid);
    }

    let mut out = effects;
    for target_uid in effect_targets {
        let Some(target) = get_entity(fight, target_uid) else {
            continue;
        };
        let target_max_hp = target.attr.as_ref().and_then(|attr| attr.hp).unwrap_or(0);
        if target_max_hp <= 0 {
            continue;
        }

        out = crate::state::battle::heroes::kakania::inject_storage_injury_for_damage_emissions(
            &mut mechanics.empathy,
            preview_buff_mgr,
            fight,
            source_uid,
            target_uid,
            target_max_hp,
            out,
        );
        mechanics.empathy.sync_buff_state(
            live_buff_mgr,
            target_uid,
            mechanics.empathy.current(target_uid),
            target_max_hp,
        );
    }

    crate::state::battle::heroes::kakania::inject_insight_iii_bounces_for_heal_emissions(
        &mechanics.empathy,
        preview_buff_mgr,
        fight,
        out,
    )
}

fn collect_dead_effects_after_damage(fight: &Fight, managers: &mut Managers, effects: &[ActEffect]) -> Vec<ActEffect> {
    let mut states: HashMap<i64, (i32, i32)> = HashMap::new();
    let mut dead_targets: HashSet<i64> = effects
        .iter()
        .filter(|effect| effect.effect_type == Some(EffectType::Dead as i32))
        .filter_map(|effect| effect.target_id)
        .collect();
    let mut killed_in_order = Vec::new();

    for effect in effects {
        let Some(effect_type) = effect.effect_type else {
            continue;
        };
        if !is_damage_effect_type(effect_type) {
            continue;
        }

        let Some(target_id) = effect.target_id else {
            continue;
        };
        if target_id == 0 || dead_targets.contains(&target_id) {
            continue;
        }

        let Some(entity) = get_entity(fight, target_id) else {
            continue;
        };
        let current_hp = entity.current_hp.unwrap_or(0);
        if current_hp <= 0 {
            dead_targets.insert(target_id);
            continue;
        }

        let (hp, shield) = states
            .entry(target_id)
            .or_insert((current_hp, entity.shield_value.unwrap_or(0)));
        let damage = effect.effect_num.unwrap_or(0).max(0);
        let shield_absorbed = damage.min(*shield);
        let hp_damage = damage.saturating_sub(shield_absorbed);

        *shield = shield.saturating_sub(shield_absorbed);
        *hp = hp.saturating_sub(hp_damage);

        if *hp <= 0 {
            dead_targets.insert(target_id);
            killed_in_order.push(target_id);
        }
    }

    killed_in_order
        .into_iter()
        .flat_map(|target_id| events_to_act_effects(hook_call::on_dead(managers, fight, target_id)))
        .collect()
}

fn apply_preview_effects_to_sim_buffs(
    buff_mgr: &mut BuffMgr,
    effects: &[ActEffect],
    from_skill_id: i32,
) {
    for effect in effects {
        match effect.effect_type.unwrap_or(0) {
            x if x == EffectType::BuffAdd as i32 => {
                let target_uid = effect.target_id.unwrap_or(0);
                let buff_id = effect.effect_num.unwrap_or(0);
                let Some(buff) = effect.buff.as_ref() else {
                    continue;
                };
                if target_uid != 0 && buff_id != 0 {
                    observe_explicit_buff_uid_for_target(target_uid, buff.uid.unwrap_or(0));
                    buff_mgr.add_with_uid(
                        target_uid,
                        buff_id,
                        buff.from_uid.unwrap_or(0),
                        from_skill_id,
                        buff.count.unwrap_or(0),
                        buff.layer.unwrap_or(0),
                        buff.uid.unwrap_or(0),
                    );
                    let _ = buff_mgr.set_instance_act_common_params(
                        target_uid,
                        buff.uid.unwrap_or(0),
                        buff.act_common_params.as_deref().unwrap_or_default(),
                    );
                }
            }
            x if x == EffectType::BuffDel as i32 => {
                // Keep skill-slot condition checks anchored to pre-delete state.
                // Live lanes like 305122 evaluate downstream HasBuffId slots before
                // in-step BuffDel effects become visible.
                let _ = x;
            }
            x if x == EffectType::BuffUpdate as i32 => {
                let target_uid = effect.target_id.unwrap_or(0);
                let Some(buff) = effect.buff.as_ref() else {
                    continue;
                };
                let buff_id = buff.buff_id.unwrap_or(0);
                if target_uid != 0 && buff_id != 0 {
                    observe_explicit_buff_uid_for_target(target_uid, buff.uid.unwrap_or(0));
                    buff_mgr.add_with_uid(
                        target_uid,
                        buff_id,
                        buff.from_uid.unwrap_or(0),
                        from_skill_id,
                        buff.count.unwrap_or(0),
                        buff.layer.unwrap_or(0),
                        buff.uid.unwrap_or(0),
                    );
                    let _ = buff_mgr.set_instance_act_common_params(
                        target_uid,
                        buff.uid.unwrap_or(0),
                        buff.act_common_params.as_deref().unwrap_or_default(),
                    );
                }
            }
            x if x == EffectType::FightStep as i32 => {
                if let Some(step) = &effect.fight_step {
                    let nested_from_skill_id =
                        if step.act_type == Some(fight_step::ActType::Skill as i32) {
                            step.act_id.map(resolve_skill_effect_id).unwrap_or(from_skill_id)
                        } else {
                            from_skill_id
                        };
                    apply_preview_effects_to_sim_buffs(
                        buff_mgr,
                        &step.act_effect,
                        nested_from_skill_id,
                    );
                }
            }
            _ => {}
        }
    }
}

fn fallback_damage_targets(
    fight: &Fight,
    caster_uid: i64,
    selected_target_uid: i64,
    logic_target: i32,
    override_damage_targets: Option<&[i64]>,
) -> Vec<i64> {
    if let Some(override_damage_targets) = override_damage_targets
        && !override_damage_targets.is_empty()
    {
        return override_damage_targets.to_vec();
    }

    // Live-like fallback targeting:
    // - default(single): selected target
    // - logicTarget=201: selected target + one more enemy
    // - logicTarget=202/301/302: all enemies
    if matches!(logic_target, 202 | 301 | 302) {
        return alive_enemies_by_position(fight, caster_uid);
    }

    if logic_target == 201 {
        let enemies = alive_enemies_by_position(fight, caster_uid);
        if enemies.is_empty() {
            return vec![selected_target_uid];
        }
        if enemies.contains(&selected_target_uid) {
            let idx = enemies
                .iter()
                .position(|uid| *uid == selected_target_uid)
                .unwrap_or(0);
            let mut out = vec![selected_target_uid];
            if enemies.len() > 1 {
                let extra = enemies[(idx + 1) % enemies.len()];
                if extra != selected_target_uid {
                    out.push(extra);
                }
            }
            return out;
        }
        let mut enemies = alive_enemies(fight, caster_uid);
        enemies.sort_by(|a, b| {
            let hp = |uid: i64| {
                get_entity(fight, uid)
                    .and_then(|e| e.current_hp)
                    .unwrap_or(0)
            };
            hp(*b).cmp(&hp(*a)).then_with(|| {
                let pos = |uid: i64| {
                    get_entity(fight, uid)
                        .and_then(|e| e.position)
                        .unwrap_or(99)
                };
                pos(*a).cmp(&pos(*b))
            })
        });
        return enemies.into_iter().take(2).collect();
    }

    let desired = 1usize;
    if desired <= 1 {
        return vec![selected_target_uid];
    }

    let enemies = alive_enemies_by_position(fight, caster_uid);

    let mut out = Vec::new();
    let selected_is_enemy = enemies.contains(&selected_target_uid)
        && get_entity(fight, selected_target_uid)
            .map(|e| e.current_hp.unwrap_or(0) > 0)
            .unwrap_or(false);
    if selected_is_enemy {
        out.push(selected_target_uid);
    }

    for uid in enemies {
        if out.len() >= desired {
            break;
        }
        if out.contains(&uid) {
            continue;
        }
        out.push(uid);
    }

    if out.is_empty() {
        vec![selected_target_uid]
    } else {
        out
    }
}

pub fn build_skill_act_effect(
    ctx: &mut FightContext<'_>,
    caster_uid: i64,
    target_uid: i64,
    skill_id: i32,
    phase: &PhaseFilter,
) -> Result<Vec<ActEffect>> {
    let mut executor = SkillExecutor::new();
    let seed = ctx.fight.cur_round.unwrap_or(0) as u64;
    let mut fallback_rng = StdRng::seed_from_u64(seed);
    let rng = match ctx.rng_ptr() {
        Some(mut rng) => {
            // SAFETY: FightContext only stores pointers captured from live mutable RNG refs.
            unsafe { rng.as_mut() }
        }
        None => &mut fallback_rng,
    };
    let effects = executor.execute_skill(
        rng,
        &*ctx.fight,
        &mut *ctx.managers,
        &mut *ctx.mechanics,
        caster_uid,
        target_uid,
        skill_id,
        phase,
    )?;
    executor.apply_pending_summons(ctx.fight, ctx.managers)?;
    Ok(effects)
}

fn apply_pending_summon(
    fight: &mut Fight,
    managers: &mut Managers,
    summon: PendingSummon,
) -> Result<()> {
    let new_uid = spawn_summoned_entity(fight, summon)?;
    managers.buff_mgr.clear(new_uid);

    if let Some(entity) = fight
        .defender
        .as_ref()
        .and_then(|d| d.sub_entitys.iter().find(|e| e.uid == Some(new_uid)))
        .cloned()
    {
        managers.passive_mgr.seed_entity(&entity);
        managers.rule_mgr.seed_entity_uid(new_uid, fight);
    }

    tracing::info!(
        "applied summon caster={} monster={} uid={}",
        summon.caster_uid,
        summon.monster_id,
        new_uid
    );
    Ok(())
}

fn next_summon_uid(fight: &Fight) -> i64 {
    let min_existing_uid = fight
        .defender
        .as_ref()
        .into_iter()
        .flat_map(|defender| defender.entitys.iter().chain(defender.sub_entitys.iter()))
        .filter_map(|entity| entity.uid)
        .min()
        .unwrap_or(0);
    let min_wave_reserved_uid = -(2 * WaveMgr::max_wave_for_fight(fight) as i64);
    min_existing_uid.min(min_wave_reserved_uid) - 1
}

fn next_summon_position(fight: &Fight) -> i32 {
    fight
        .defender
        .as_ref()
        .into_iter()
        .flat_map(|defender| defender.sub_entitys.iter())
        .filter_map(|entity| entity.position)
        .filter(|position| *position < 0)
        .min()
        .map(|position| position - 1)
        .unwrap_or(-1)
}

fn preview_pending_summon(fight: &mut Fight, summon: PendingSummon) -> Result<()> {
    let new_uid = spawn_summoned_entity(fight, summon)?;
    tracing::debug!(
        "previewed summon caster={} monster={} uid={}",
        summon.caster_uid,
        summon.monster_id,
        new_uid
    );
    Ok(())
}

fn spawn_summoned_entity(fight: &mut Fight, summon: PendingSummon) -> Result<i64> {
    let uid = next_summon_uid(fight);
    let position = next_summon_position(fight);
    let entity = Defender::build_enemy_with_uid(summon.monster_id, uid, position, 2)?;
    let new_uid = entity.uid.unwrap_or(uid);

    let defender = fight
        .defender
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("Fight missing defender team"))?;
    defender.sub_entitys.push(entity);
    Ok(new_uid)
}

fn is_ex_skill(skill_id: i32) -> bool {
    matches!(
        super::source_kind::classify(skill_id),
        super::source_kind::SkillSource::ExIncantation { .. }
    )
}

fn damage_targets_in(effects: &[ActEffect]) -> Vec<i64> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for e in effects {
        let Some(t) = e.effect_type else { continue };
        if !is_damage_effect_type(t) { continue }
        let Some(uid) = e.target_id else { continue };
        if uid != 0 && seen.insert(uid) {
            out.push(uid);
        }
    }
    out
}