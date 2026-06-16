pub mod behavior;
pub mod condition;
pub mod condition_eval;
pub mod parser;
pub mod target;

use crate::state::battle::event::Event;
use crate::state::battle::{
    manager::fight_data_mgr::Managers, mechanics::bloodtithe::BloodtitheState,
};
use condition::ConditionOp;
use condition_eval::ConditionEval;
use sonettobuf::Fight;

#[derive(Debug, Clone)]
pub struct EffectSlot {
    pub op: ConditionOp,
    pub conditions: Vec<condition::Condition>,
    pub behaviours: Vec<behavior::Behaviour>,
    pub limit: i32,
    pub round_limit: i32,
    pub use_count: i32,
    pub round_use_count: i32,
}

impl EffectSlot {
    fn matches_hook(&self, hook: condition::Hook) -> bool {
        self.conditions.first().is_some_and(|c| c.hooks.contains(&hook))
    }

    fn check_conditions(&self, owner_uid: i64, eval: ConditionEval<'_>) -> Option<i32> {
        match self.op {
            ConditionOp::And => {
                let mut min_count = i32::MAX;
                for c in &self.conditions {
                    if let Some(count) = c.check(owner_uid, eval) {
                        min_count = min_count.min(count);
                    } else {
                        return None;
                    }
                }
                if min_count == i32::MAX { Some(1) } else { Some(min_count) }
            }
            ConditionOp::Or => {
                let mut max_count = 0;
                let mut matched = false;
                for c in &self.conditions {
                    if let Some(count) = c.check(owner_uid, eval) {
                        max_count = max_count.max(count);
                        matched = true;
                    }
                }
                if matched { Some(max_count) } else { None }
            }
        }
    }

    fn within_limits(&self) -> bool {
        (self.limit == 0 || self.use_count < self.limit)
            && (self.round_limit == 0 || self.round_use_count < self.round_limit)
    }
}

#[derive(Clone)]
pub struct SkillEffect {
    pub owner_uid: i64,
    pub(crate) slots: Vec<EffectSlot>,
}

impl std::fmt::Debug for SkillEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SkillEffect(owner={}, slots={})", self.owner_uid, self.slots.len())
    }
}

impl SkillEffect {
    pub fn empty(owner_uid: i64) -> Self {
        Self { owner_uid, slots: Vec::new() }
    }

    pub fn reset_round_counts(&mut self) {
        for slot in &mut self.slots {
            slot.round_use_count = 0;
        }
    }

    pub fn fire_hook(
        &mut self,
        hook: condition::Hook,
        fight: &Fight,
        managers: &mut Managers,
        target_uid: i64,
    ) -> Vec<Event> {
        let bloodtithe = BloodtitheState::default();
        let buff_mgr = managers.buff_mgr.clone();
        let entity_mgr = managers.entity_mgr.clone();
        let eval = ConditionEval {
            fight,
            buff_mgr: &buff_mgr,
            entity_mgr: &entity_mgr,
            bloodtithe: &bloodtithe,
            caster_uid: self.owner_uid,
            target_uid,
            condition_target: 0,
            has_trigger_state: false,
            active_card_cast_uids: None,
            lost_buff_id: None,
            emit_context: None,
        };
        let owner_uid = self.owner_uid;
        let mut matching = Vec::new();
        for (slot_idx, slot) in self.slots.iter_mut().enumerate() {
            if !slot.matches_hook(hook) { continue; }
            let within_limits = slot.within_limits();
            let cond_count = slot.check_conditions(owner_uid, eval);
            let cond_eval_pass = cond_count.is_some();
            tracing::debug!(
                "effect hooked: hook={:?} owner_uid={} target_uid={} slot_idx={} cond_eval_pass={} within_limits={} behaviours={} use_count={} round_use_count={}",
                hook, owner_uid, target_uid, slot_idx, cond_eval_pass, within_limits,
                slot.behaviours.len(), slot.use_count, slot.round_use_count
            );
            tracing::debug!("effect slot detail: slot_idx={} slot={:?}", slot_idx, slot);
            if !within_limits || !cond_eval_pass { continue; }
            let count = cond_count.unwrap();
            slot.use_count += 1;
            slot.round_use_count += 1;
            matching.extend(slot.behaviours.iter().map(|b| (b.raw.clone(), b.target, count)));
        }
        matching
            .into_iter()
            .filter(|(raw, _, _)| !behavior::is_attr_fix(raw))
            .flat_map(|(raw, target, count)| {
                let mut _stub_mechanics = crate::state::battle::mechanics::Mechanics::default();
                let mut _stub_executor = crate::state::battle::skill::SkillExecutor::default();
                let mut _stub_rng = <rand::rngs::StdRng as rand::SeedableRng>::seed_from_u64(0);
                behavior::execute(
                    fight,
                    managers,
                    &mut _stub_mechanics,
                    &mut _stub_executor,
                    &mut _stub_rng,
                    owner_uid,
                    0,
                    &raw,
                    target,
                    count,
                )
            })
            .collect()
    }

    pub fn fire_hook_attr_fix(
        &mut self,
        hook: condition::Hook,
        fight: &Fight,
        managers: &Managers,
        target_uid: i64,
    ) -> std::collections::HashMap<(i64, i32), Vec<i32>> {
        let bloodtithe = BloodtitheState::default();
        let buff_mgr = managers.buff_mgr.clone();
        let entity_mgr = managers.entity_mgr.clone();
        let eval = ConditionEval {
            fight,
            buff_mgr: &buff_mgr,
            entity_mgr: &entity_mgr,
            bloodtithe: &bloodtithe,
            caster_uid: self.owner_uid,
            target_uid,
            condition_target: 0,
            has_trigger_state: false,
            active_card_cast_uids: None,
            lost_buff_id: None,
            emit_context: None,
        };
        let owner_uid = self.owner_uid;
        let mut acc: std::collections::HashMap<(i64, i32), Vec<i32>> = std::collections::HashMap::new();
        for (slot_idx, slot) in self.slots.iter_mut().enumerate() {
            if !slot.matches_hook(hook) { continue; }
            let within_limits = slot.within_limits();
            let cond_count = slot.check_conditions(owner_uid, eval.clone());
            let cond_eval_pass = cond_count.is_some();
            if !within_limits || !cond_eval_pass { continue; }
            let count = cond_count.unwrap();
            // NB: this pass does NOT increment `use_count` / `round_use_count`.
            // The non-attr-fix companion `fire_hook` (which the caller invokes
            // immediately after) is the bookkeeping authority. Slot limits are
            // shared between the two passes; the same slot fires once.
            for b in &slot.behaviours {
                if !behavior::is_attr_fix(&b.raw) { continue; }
                let map = behavior::calculate_bonus(fight, managers, owner_uid, &b.raw, b.target, count);
                for (k, v) in map {
                    if v == 0 { continue; }
                    acc.entry(k).or_default().push(v);
                }
            }
        }
        acc
    }
    
    pub fn on_eval_active_skill(&mut self, fight: &Fight, managers: &mut Managers, skill_target_uid: i64) -> Vec<Event> {
        self.fire_hook(condition::Hook::EvalActiveSkill, fight, managers, skill_target_uid)
    }
}
