use super::traits::Manager;
use crate::state::battle::buff::{utils, Buff, RefreshPolicy};
use crate::state::battle::event::Event;
use crate::state::battle::manager::fight_data_mgr::Managers;
use crate::state::battle::types::buff::{ExcludeRule, IncludeType, TakeActBase, TakeStage};
use sonettobuf::Fight;
#[cfg(test)]
use std::cell::Cell;
#[cfg(test)]
use std::sync::Mutex;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, AtomicI64, Ordering},
};

#[allow(dead_code)]
#[derive(Default, Debug, Clone)]
pub struct BuffInstance {
    pub uid: i64,
    pub buff_id: i32,
    pub type_id: i32,
    pub from_uid: i64,
    pub from_skill_id: i32,
    pub duration: i32, // 0 = permanent
    pub stacks: i32,   // maps to buff.count in packets
    pub layer: i32,    // maps to buff.layer in packets
    pub act_common_params: String,
    pub refresh_policy: RefreshPolicy,
}

impl BuffInstance {
    pub fn build(buff_id: i32, from_uid: i64, from_skill_id: i32) -> Self {
        let configs = config::configs::get();
        let cfg = configs.skill_buff.iter().find(|b| b.id == buff_id);
        let buff_type = cfg.and_then(|b| configs.skill_bufftype.iter().find(|t| t.id == b.type_id));
        Self {
            uid: next_buff_uid(),
            buff_id,
            type_id: cfg.map(|b| b.type_id).unwrap_or(0),
            from_uid,
            from_skill_id,
            duration: cfg.map(|b| b.during_time).unwrap_or(0),
            stacks: cfg.map(|b| b.effect_count).unwrap_or(0),
            layer: 0,
            act_common_params: String::new(),
            refresh_policy: utils::derive_refresh_policy(buff_type),
        }
    }
}

#[derive(Default, Debug)]
pub struct BuffMgr {
    active: HashMap<i64, Vec<BuffInstance>>,
    pub active_buff: HashMap<i64, Vec<Buff>>,
    /// Buff ids (and their type ids) deleted during the currently-executing
    /// step. Reset by the round manager at each step boundary. Read by
    /// `BuffIdDel` trigger conditions firing from mid-step passive chains that
    /// don't receive an explicit event list (see `TriggerState::with_buff_mgr`).
    step_deleted_buff_ids: Vec<i32>,
    /// Cumulative teammate-injury packets observed for each holder.
    /// This tracker intentionally does not reset on round end.
    teammate_injury_not_reset: HashMap<i64, i32>,
    /// Per-round behavior slot usage tracker keyed by
    /// `(caster_uid, skill_effect_id, slot_index)`.
    skill_slot_round_usage: HashMap<(i64, i32, u8), i32>,
}

impl Clone for BuffMgr {
    fn clone(&self) -> Self {
        Self {
            active: self.active.clone(),
            active_buff: self.active_buff.clone(),
            step_deleted_buff_ids: self.step_deleted_buff_ids.clone(),
            teammate_injury_not_reset: self.teammate_injury_not_reset.clone(),
            skill_slot_round_usage: self.skill_slot_round_usage.clone(),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum StackConsumeResult {
    Updated(BuffInstance),
    Removed(BuffInstance),
}

#[derive(Debug, Clone)]
pub struct BuffLifecycleEvent {
    pub instance: BuffInstance,
    pub target_uid: i64,
    pub kind: LifecycleEventKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleEventKind {
    Expiring,
    Ticking,
}

fn apply_exclude_rules(buffs: &mut Vec<Buff>, new_idx: usize) -> bool {
    let (exclude_rules, bt_type, new_shield, new_duration) = {
        let b = &buffs[new_idx];
        (
            b.exclude_rules.clone(),
            b.buff_type.as_ref().map(|t| t.r#type).unwrap_or(0),
            b.shield_value,
            b.duration,
        )
    };
    let mut remove_new = false;
    let mut to_remove: Vec<usize> = Vec::new();
    for rule in &exclude_rules {
        match rule {
            ExcludeRule::ByBuffId(ids) => {
                for (i, b) in buffs.iter().enumerate() {
                    if i != new_idx && ids.contains(&b.buff_id) {
                        to_remove.push(i);
                    }
                }
            }
            ExcludeRule::ByCategory(ids) => {
                for (i, b) in buffs.iter().enumerate() {
                    if i == new_idx { continue; }
                    let bt = b.buff_type.as_ref().map(|t| t.r#type).unwrap_or(0);
                    if !ids.contains(&bt) { continue; }
                    if bt_type == 7 && bt == 7 {
                        if b.shield_value > new_shield
                            || (b.shield_value == new_shield && b.duration >= new_duration)
                        {
                            remove_new = true;
                        } else {
                            to_remove.push(i);
                        }
                    } else {
                        to_remove.push(i);
                    }
                }
            }
        }
    }
    to_remove.sort_unstable();
    to_remove.dedup();
    for i in to_remove.into_iter().rev() {
        buffs.remove(i);
    }
    remove_new
}

fn apply_stacking(buffs: &mut Vec<Buff>, buff_id: i32) {
    if utils::is_stacked_include_type(buff_id) { return; }
    let new_idx = buffs.len() - 1;
    if let Some(existing_idx) = buffs[..new_idx].iter().position(|b| b.buff_id == buff_id) {
        let new_stacks = buffs[new_idx].stacks;
        let (keep, drop) = if buffs[existing_idx].duration >= buffs[new_idx].duration {
            (existing_idx, new_idx)
        } else {
            (new_idx, existing_idx)
        };
        buffs[keep].stacks = new_stacks;
        buffs.remove(drop);
    }
}

impl BuffMgr {
    fn merge_from_skill_id(existing: &mut BuffInstance, from_skill_id: i32) {
        if from_skill_id != 0 || existing.from_skill_id == 0 {
            existing.from_skill_id = from_skill_id;
        }
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_buff(&mut self, target_uid: i64, buff_id: i32) -> Vec<Event> {
        use crate::state::battle::buff::buff_act::BuffAction;
        use sonettobuf::BuffInfo;
        let cfg = config::configs::get();
        let buff_cfg = cfg.skill_buff.iter().find(|b| b.id == buff_id);
        let duration = buff_cfg.map(|b| b.during_time).unwrap_or(0);
        let stacks = buff_cfg.map(|b| b.effect_count).unwrap_or(0).max(1);
        let type_id = buff_cfg.map(|b| b.type_id).unwrap_or(0);
        let buff_type = cfg.skill_bufftype.iter().find(|t| t.id == type_id).cloned();
        let refresh_policy: RefreshPolicy = utils::derive_refresh_policy(buff_type.as_ref());

        let actions: Vec<BuffAction> = buff_cfg.map(|b| {
            b.features.split('|').filter_map(|entry| {
                let act_id: i32 = entry.split('#').next()?.trim().parse().ok()?;
                let mut action = BuffAction::new(act_id)?;
                action.params = entry.trim().to_string();
                Some(action)
            }).collect()
        }).unwrap_or_default();

        let proto_buff = BuffInfo {
            buff_id: Some(buff_id),
            duration: Some(duration),
            count: Some(1),
            r#type: Some(type_id),
            ..Default::default()
        };

        let include_type = buff_type.as_ref().and_then(|bt| {
            bt.include_types.split('#').next()
                .and_then(|s| s.trim().parse::<i32>().ok())
                .and_then(IncludeType::from)
        });
        let include_max_stacks = buff_type.as_ref().and_then(|bt| {
            let mut parts = bt.include_types.split('#');
            parts.next();
            parts.next().and_then(|s| s.trim().parse::<i32>().ok())
        });
        let exclude_rules = buff_type.as_ref()
            .map(|bt| ExcludeRule::parse(&bt.exclude_types))
            .unwrap_or_default();
        let take_stage = buff_type.as_ref().and_then(|bt| TakeStage::from(bt.take_stage));
        let take_act = buff_type.as_ref().and_then(|bt| {
            bt.take_act.split('#').next()
                .and_then(|s| s.trim().parse::<i32>().ok())
                .and_then(|v| TakeActBase::from(v))
        });

        let new_buff = Buff { buff_id, duration, stacks, actions, proto_buff, buff_type, layer: 0, refresh_policy, attr_bonus_refs: Vec::new(), include_type, include_max_stacks, exclude_rules, take_stage, take_act, shield_value: 0 };
        let buffs = self.active_buff.entry(target_uid).or_default();
        buffs.push(new_buff);
        vec![]
    }

    pub fn finalize_buff_add(&mut self, target_uid: i64, buff_id: i32) {
        let Some(buffs) = self.active_buff.get_mut(&target_uid) else { return; };
        let new_idx = match buffs.iter().rposition(|b| b.buff_id == buff_id) {
            Some(i) => i,
            None => return,
        };
        if apply_exclude_rules(buffs, new_idx) {
            buffs.remove(new_idx);
            return;
        }
        apply_stacking(buffs, buff_id);
    }

    pub fn set_shield_value(&mut self, target_uid: i64, buff_id: i32, value: i32) {
        if let Some(buffs) = self.active_buff.get_mut(&target_uid) {
            if let Some(b) = buffs.iter_mut().find(|b| b.buff_id == buff_id) {
                b.shield_value = value;
            }
        }
    }

    pub fn reduce_shield_value(&mut self, target_uid: i64, amount: i32) {
        if let Some(buffs) = self.active_buff.get_mut(&target_uid) {
            if let Some(b) = buffs.iter_mut().find(|b| {
                b.buff_type.as_ref().map(|t| t.r#type) == Some(7)
            }) {
                b.shield_value = (b.shield_value - amount).max(0);
            }
        }
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    pub fn add(
        &mut self,
        target_uid: i64,
        buff_id: i32,
        from_uid: i64,
        from_skill_id: i32,
        count: i32,
        layer: i32,
    ) {
        let entry = self.active.entry(target_uid).or_default();

        let mut instance = BuffInstance::build(buff_id, from_uid, from_skill_id);

        instance.stacks = if count > 0 { count } else { instance.stacks };
        instance.layer = layer;

        if utils::uses_distinct_dot_carrier_instances(buff_id) {
            entry.push(instance);
        } else if utils::uses_single_uid_layer_refresh(buff_id) {
            if let Some(existing) = entry.iter_mut().find(|b| b.buff_id == buff_id) {
                existing.duration = existing.duration.max(instance.duration);
                existing.stacks = instance.stacks;
                existing.layer = instance.layer;
                existing.refresh_policy = instance.refresh_policy;
                Self::merge_from_skill_id(existing, from_skill_id);
            } else {
                entry.push(instance);
            }
        } else if utils::is_stacked_include_type(buff_id) {
            entry.push(instance);
        } else if utils::is_poison_family(buff_id) {
            if let Some(existing) = entry.iter_mut().find(|b| b.buff_id == buff_id) {
                existing.duration = existing.duration.max(instance.duration);
                existing.layer = existing.layer.max(1).saturating_add(instance.layer.max(1));
                Self::merge_from_skill_id(existing, from_skill_id);
            } else {
                entry.push(instance);
            }
        } else if let Some(existing) = entry.iter_mut().find(|b| b.buff_id == buff_id) {
            existing.duration = existing.duration.max(instance.duration);
            existing.stacks = instance.stacks;
            existing.layer = instance.layer;
            Self::merge_from_skill_id(existing, from_skill_id);
        } else {
            entry.push(instance);
        }
    }

    pub fn get(&self, uid: i64) -> &[BuffInstance] {
        self.active.get(&uid).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn find_instance_by_buff_id(&self, target_uid: i64, buff_id: i32) -> Option<&BuffInstance> {
        self.active
            .get(&target_uid)
            .and_then(|buffs| buffs.iter().find(|buff| buff.buff_id == buff_id))
    }

    pub fn preview_round_end_lifecycle_takestage_103(&self) -> Vec<BuffLifecycleEvent> {
        let cfg = config::configs::get();
        let mut events = Vec::new();
        for (target_uid, instances) in &self.active {
            for inst in instances {
                let take_stage = cfg
                    .skill_bufftype
                    .iter()
                    .find(|buff_type| buff_type.id == inst.type_id)
                    .map(|buff_type| buff_type.take_stage)
                    .unwrap_or(0);
                if take_stage != 103 || inst.duration <= 0 {
                    continue;
                }

                let kind = if inst.duration == 1 {
                    LifecycleEventKind::Expiring
                } else {
                    LifecycleEventKind::Ticking
                };
                events.push(BuffLifecycleEvent {
                    instance: inst.clone(),
                    target_uid: *target_uid,
                    kind,
                });
            }
        }
        events
    }

    /// Find a buff instance on `target_uid` by its `type_id` (the `bufftype.id`
    /// it belongs to, NOT the buff_id). Used when a mechanic family has
    /// rank/portrait variants with different buff_ids that all share one
    /// canonical typeId — e.g. Kakania's Empathy buffs 30800141 / 30800142 /
    /// 30800143 all carry `typeId = 30800141`, so portrait upgrades that
    /// swap the active variant still match.
    pub fn find_instance_by_type_id(&self, target_uid: i64, type_id: i32) -> Option<&BuffInstance> {
        self.active
            .get(&target_uid)
            .and_then(|buffs| buffs.iter().find(|buff| buff.type_id == type_id))
    }

    #[allow(dead_code)]
    pub fn all_instances(&self) -> Vec<(i64, BuffInstance)> {
        let mut out = Vec::new();
        for (uid, buffs) in &self.active {
            for buff in buffs {
                out.push((*uid, buff.clone()));
            }
        }
        out
    }

    #[allow(dead_code)]
    pub fn set_instance_duration(&mut self, target_uid: i64, buff_uid: i64, duration: i32) -> bool {
        if let Some(buffs) = self.active.get_mut(&target_uid)
            && let Some(buff) = buffs.iter_mut().find(|b| b.uid == buff_uid)
        {
            buff.duration = duration.max(0);
            return true;
        }
        false
    }

    pub fn extend_buff_duration(&mut self, target_uid: i64, buff_id: i32, delta: i32) -> bool {
        if let Some(buffs) = self.active.get_mut(&target_uid)
            && let Some(buff) = buffs.iter_mut().find(|b| b.buff_id == buff_id)
        {
            if buff.duration > 0 { buff.duration += delta; }
            return true;
        }
        false
    }

    #[allow(dead_code)]
    pub fn set_instance_layer(&mut self, target_uid: i64, buff_uid: i64, layer: i32) -> bool {
        if let Some(buffs) = self.active.get_mut(&target_uid)
            && let Some(buff) = buffs.iter_mut().find(|b| b.uid == buff_uid)
        {
            buff.layer = layer.max(0);
            return true;
        }
        false
    }

    pub fn set_instance_count_layer(
        &mut self,
        target_uid: i64,
        buff_uid: i64,
        count: i32,
        layer: i32,
    ) -> bool {
        if let Some(buffs) = self.active.get_mut(&target_uid)
            && let Some(buff) = buffs.iter_mut().find(|b| b.uid == buff_uid)
        {
            buff.stacks = count.max(0);
            buff.layer = layer.max(0);
            return true;
        }
        false
    }

    pub fn set_instance_act_common_params(
        &mut self,
        target_uid: i64,
        buff_uid: i64,
        params: &str,
    ) -> bool {
        if let Some(buffs) = self.active.get_mut(&target_uid)
            && let Some(buff) = buffs.iter_mut().find(|b| b.uid == buff_uid)
        {
            buff.act_common_params = params.to_string();
            return true;
        }
        false
    }

    pub fn has(&self, uid: i64, buff_id: i32) -> bool {
        self.active
            .get(&uid)
            .map(|b| b.iter().any(|x| x.buff_id == buff_id))
            .unwrap_or(false)
    }

    pub fn has_buff(&self, uid: i64, buff_id: i32) -> bool {
        self.active_buff
            .get(&uid)
            .map(|b| b.iter().any(|x| x.buff_id == buff_id))
            .unwrap_or(false)
    }

    #[allow(dead_code)]
    pub fn has_type(&self, uid: i64, type_id: i32) -> bool {
        self.active
            .get(&uid)
            .map(|buffs| buffs.iter().any(|b| b.type_id == type_id))
            .unwrap_or(false)
    }

    pub fn count_buff_ids(&self, uid: i64, buff_ids: &[i32]) -> i32 {
        self.active
            .get(&uid)
            .map(|buffs| {
                buffs
                    .iter()
                    .filter(|b| buff_ids.contains(&b.buff_id))
                    .map(|b| if b.stacks == 0 { 1 } else { b.stacks })
                    .sum()
            })
            .unwrap_or(0)
    }

    pub fn count_type(&self, uid: i64, type_id: i32) -> i32 {
        self.active
            .get(&uid)
            .map(|buffs| {
                buffs
                    .iter()
                    .filter(|b| b.type_id == type_id)
                    .map(|b| if b.stacks == 0 { 1 } else { b.stacks })
                    .sum()
            })
            .unwrap_or(0)
    }

    /// Record a buff instance that was just removed so `BuffIdDel` triggers
    /// firing later in the same step can match against it.
    fn record_deleted(step_deleted: &mut Vec<i32>, inst: &BuffInstance) {
        if inst.buff_id != 0 {
            step_deleted.push(inst.buff_id);
        }
        if inst.type_id != 0 && inst.type_id != inst.buff_id {
            step_deleted.push(inst.type_id);
        }
    }

    /// Buff ids (plus their type ids) removed since the last
    /// `clear_step_deleted_buff_ids` call. Used by mid-step passive-chain
    /// trigger sites that don't get an explicit `TriggerEvent`.
    pub fn step_deleted_buff_ids(&self) -> &[i32] {
        &self.step_deleted_buff_ids
    }

    /// Reset the in-step deleted-buff tracker. Call at step boundaries.
    pub fn clear_step_deleted_buff_ids(&mut self) {
        self.step_deleted_buff_ids.clear();
    }

    pub fn teammate_injury_not_reset(&self, uid: i64) -> i32 {
        self.teammate_injury_not_reset
            .get(&uid)
            .copied()
            .unwrap_or(0)
    }

    pub fn add_teammate_injury_not_reset(&mut self, uid: i64, amount: i32) {
        if uid == 0 || amount <= 0 {
            return;
        }
        let entry = self.teammate_injury_not_reset.entry(uid).or_insert(0);
        *entry = entry.saturating_add(amount);
    }

    pub fn skill_slot_round_usage(&self, caster_uid: i64, skill_effect_id: i32, slot: u8) -> i32 {
        self.skill_slot_round_usage
            .get(&(caster_uid, skill_effect_id, slot))
            .copied()
            .unwrap_or(0)
    }

    pub fn increment_skill_slot_round_usage(
        &mut self,
        caster_uid: i64,
        skill_effect_id: i32,
        slot: u8,
    ) -> i32 {
        let entry = self
            .skill_slot_round_usage
            .entry((caster_uid, skill_effect_id, slot))
            .or_insert(0);
        *entry = entry.saturating_add(1);
        *entry
    }

    pub fn reset_skill_slot_round_usage(&mut self) {
        self.skill_slot_round_usage.clear();
    }

    pub fn clear(&mut self, uid: i64) {
        if let Some(buffs) = self.active.remove(&uid) {
            for inst in &buffs {
                Self::record_deleted(&mut self.step_deleted_buff_ids, inst);
            }
        }
    }

    pub fn remove_by_uid(&mut self, uid: i64, buff_uid: i64) {
        if let Some(buffs) = self.active.get_mut(&uid) {
            let mut step_deleted = std::mem::take(&mut self.step_deleted_buff_ids);
            buffs.retain(|b| {
                if b.uid == buff_uid {
                    Self::record_deleted(&mut step_deleted, b);
                    false
                } else {
                    true
                }
            });
            self.step_deleted_buff_ids = step_deleted;
        }
    }

    pub fn remove_buff(&mut self, target_uid: i64, buff_id: i32) {
        if let Some(buffs) = self.active.get_mut(&target_uid) {
            let mut step_deleted = std::mem::take(&mut self.step_deleted_buff_ids);
            buffs.retain(|b| {
                if b.buff_id == buff_id {
                    Self::record_deleted(&mut step_deleted, b);
                    false
                } else {
                    true
                }
            });
            self.step_deleted_buff_ids = step_deleted;
        }
    }

    #[allow(dead_code)]
    pub fn consume_one_stacked_drop_dmg(&mut self, target_uid: i64) -> Option<StackConsumeResult> {
        let mut should_remove_bucket = false;
        let result = {
            let buffs = self.active.get_mut(&target_uid)?;

            let idx = buffs
                .iter()
                .enumerate()
                .filter(|(_, b)| {
                    utils::is_stacked_include_type(b.buff_id)
                        && utils::is_drop_dmg_attr_buff(b.buff_id)
                })
                .min_by_key(|(_, b)| b.uid)
                .map(|(i, _)| i)?;

            let result = if buffs[idx].layer > 1 {
                buffs[idx].layer -= 1;
                StackConsumeResult::Updated(buffs[idx].clone())
            } else if buffs[idx].layer == 1 {
                let removed = buffs.remove(idx);
                Self::record_deleted(&mut self.step_deleted_buff_ids, &removed);
                StackConsumeResult::Removed(removed)
            } else if buffs[idx].stacks > 1 {
                buffs[idx].stacks -= 1;
                StackConsumeResult::Updated(buffs[idx].clone())
            } else {
                let removed = buffs.remove(idx);
                Self::record_deleted(&mut self.step_deleted_buff_ids, &removed);
                StackConsumeResult::Removed(removed)
            };

            if buffs.is_empty() {
                should_remove_bucket = true;
            }
            result
        };

        if should_remove_bucket {
            self.active.remove(&target_uid);
        }

        Some(result)
    }

    pub fn add_with_uid(
        &mut self,
        target_uid: i64,
        buff_id: i32,
        from_uid: i64,
        from_skill_id: i32,
        count: i32,
        layer: i32,
        buff_uid: i64,
    ) {
        let entry = self.active.entry(target_uid).or_default();
        let cfg = config::configs::get();
        let cfg_buff = cfg.skill_buff.iter().find(|b| b.id == buff_id);
        let buff_type =
            cfg_buff.and_then(|b| cfg.skill_bufftype.iter().find(|t| t.id == b.type_id));
        let instance = BuffInstance {
            uid: buff_uid, // use the uid from the emitted effect
            buff_id,
            type_id: cfg_buff.map(|b| b.type_id).unwrap_or(0),
            from_uid,
            from_skill_id,
            duration: cfg_buff.map(|b| b.during_time).unwrap_or(0),
            stacks: if count > 0 {
                count
            } else {
                cfg_buff.map(|b| b.effect_count).unwrap_or(0)
            },
            layer,
            act_common_params: String::new(),
            refresh_policy: utils::derive_refresh_policy(buff_type),
        };

        if let Some(existing) = entry.iter_mut().find(|b| b.uid == buff_uid) {
            existing.buff_id = instance.buff_id;
            existing.type_id = instance.type_id;
            existing.from_uid = instance.from_uid;
            Self::merge_from_skill_id(existing, from_skill_id);
            existing.duration = existing.duration.max(instance.duration);
            existing.stacks = instance.stacks;
            existing.layer = instance.layer;
            existing.refresh_policy = instance.refresh_policy;
            return;
        }

        if utils::uses_distinct_dot_carrier_instances(buff_id) {
            entry.push(instance);
        } else if utils::uses_single_uid_layer_refresh(buff_id) {
            if let Some(existing) = entry.iter_mut().find(|b| b.buff_id == buff_id) {
                existing.from_uid = instance.from_uid;
                Self::merge_from_skill_id(existing, from_skill_id);
                existing.duration = existing.duration.max(instance.duration);
                existing.stacks = instance.stacks;
                existing.layer = instance.layer;
                existing.refresh_policy = instance.refresh_policy;
            } else {
                entry.push(instance);
            }
        } else if utils::is_stacked_include_type(buff_id) {
            entry.push(instance);
        } else if utils::is_poison_family(buff_id) {
            if let Some(existing) = entry.iter_mut().find(|b| b.buff_id == buff_id) {
                existing.uid = buff_uid;
                Self::merge_from_skill_id(existing, from_skill_id);
                existing.duration = existing.duration.max(instance.duration);
                existing.layer = existing.layer.max(1).saturating_add(instance.layer.max(1));
            } else {
                entry.push(instance);
            }
        } else if let Some(existing) = entry.iter_mut().find(|b| b.buff_id == buff_id) {
            existing.uid = buff_uid;
            Self::merge_from_skill_id(existing, from_skill_id);
            existing.duration = existing.duration.max(instance.duration);
            existing.stacks = instance.stacks;
            existing.layer = instance.layer;
            existing.refresh_policy = instance.refresh_policy;
        } else {
            entry.push(instance);
        }
    }
}

impl BuffMgr {
    pub fn expire_active_buff(&mut self, entity_mgr: &mut crate::state::battle::manager::entity_mgr::EntityMgr) {
        for buffs in self.active_buff.values_mut() {
            for b in buffs.iter().filter(|b| b.duration == 1) {
                for &(uid, attr_id, amount) in &b.attr_bonus_refs {
                    entity_mgr.remove_attr_bonus(uid, attr_id, amount);
                }
            }
            buffs.retain(|b| b.duration != 1);
            for b in buffs.iter_mut() {
                if b.duration > 0 { b.duration -= 1; }
            }
        }
    }
}

impl Manager for BuffMgr {

    fn on_round_end(&mut self, _fight: &mut Fight) {
        self.tick_round_end();
        // active_buff expiry is handled by expire_active_buff called from FightContext
    }

    fn on_battle_end(&mut self) {
        self.active.clear();
        self.teammate_injury_not_reset.clear();
        self.skill_slot_round_usage.clear();
        self.active_buff.clear();
    }
}

impl BuffMgr {
    pub fn tick_round_end(&mut self) {
        let cfg = config::configs::get();
        for buffs in self.active.values_mut() {
            for b in buffs.iter_mut() {
                if b.duration > 0 {
                    b.duration -= 1;
                }
            }
            buffs.retain(|b| {
                // Permanent buffs (cfg duringTime == 0) never expire at round
                // end regardless of stack count — that case is gated by
                // `!was_timed` below. The previous predicate also OR'd in
                // `b.stacks == 0`, intending to protect the same permanent
                // buffs from a stacks-based drop, but `BuffInstance::stacks`
                // is initialized from `cfg.effect_count` which is 0 for most
                // buffs (Poison family, Sotheby's AdvancedCure 30091122 et
                // al.), so the carve-out kept every timed buff alive forever
                // once duration hit 0. Round-end ticks then over-fired by
                // Σ ~+85 across battle3 (30091122/30091111 HoT + 31040005/
                // 300901412/30980145 Poison families). Sentinel/Rubuska's
                // 31260151 stays safe via `!was_timed` (its `duringTime=0`).
                let was_timed = cfg
                    .skill_buff
                    .iter()
                    .find(|c| c.id == b.buff_id)
                    .map(|c| c.during_time > 0)
                    .unwrap_or(false);
                !was_timed || b.duration != 0
            });
        }
        }
}

pub static ATTACKER_BUFF_UID_COUNTER: AtomicI64 = AtomicI64::new(0);
pub static DEFENDER_BUFF_UID_COUNTER: AtomicI64 = AtomicI64::new(DEFENDER_BUFF_UID_START);
pub static ACTIVE_DEFENDER_LANE: AtomicBool = AtomicBool::new(false);

pub const DEFENDER_BUFF_UID_START: i64 = 100000;

#[cfg(test)]
static BUFF_UID_TEST_MUTEX: Mutex<()> = Mutex::new(());
#[cfg(test)]
thread_local! {
    static BUFF_UID_TEST_BYPASS_LOCK: Cell<bool> = const { Cell::new(false) };
}

#[cfg(test)]
fn buff_uid_test_access_lock() -> Option<std::sync::MutexGuard<'static, ()>> {
    if BUFF_UID_TEST_BYPASS_LOCK.with(|bypass| bypass.get()) {
        None
    } else {
        Some(
            BUFF_UID_TEST_MUTEX
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }
}

#[cfg(test)]
struct BuffUidTestBypassGuard {
    previous: bool,
}

#[cfg(test)]
impl BuffUidTestBypassGuard {
    fn enter() -> Self {
        let previous = BUFF_UID_TEST_BYPASS_LOCK.with(|bypass| {
            let previous = bypass.get();
            bypass.set(true);
            previous
        });
        Self { previous }
    }
}

#[cfg(test)]
impl Drop for BuffUidTestBypassGuard {
    fn drop(&mut self) {
        BUFF_UID_TEST_BYPASS_LOCK.with(|bypass| bypass.set(self.previous));
    }
}

#[cfg(test)]
pub fn with_buff_uid_test_lock<T>(f: impl FnOnce() -> T) -> T {
    let _lock = BUFF_UID_TEST_MUTEX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _bypass = BuffUidTestBypassGuard::enter();
    f()
}

pub fn next_buff_uid() -> i64 {
    #[cfg(test)]
    let _test_lock = buff_uid_test_access_lock();
    if ACTIVE_DEFENDER_LANE.load(Ordering::Relaxed) {
        DEFENDER_BUFF_UID_COUNTER.fetch_add(2, Ordering::Relaxed) + 2
    } else {
        ATTACKER_BUFF_UID_COUNTER.fetch_add(2, Ordering::Relaxed) + 2
    }
}

pub fn next_buff_uid_for_target(target_uid: i64) -> i64 {
    #[cfg(test)]
    let _test_lock = buff_uid_test_access_lock();
    if target_uid < 0 {
        DEFENDER_BUFF_UID_COUNTER.fetch_add(2, Ordering::Relaxed) + 2
    } else {
        ATTACKER_BUFF_UID_COUNTER.fetch_add(2, Ordering::Relaxed) + 2
    }
}

pub fn next_slave_buff_uid_for_target(target_uid: i64) -> i64 {
    #[cfg(test)]
    let _test_lock = buff_uid_test_access_lock();
    if target_uid < 0 {
        DEFENDER_BUFF_UID_COUNTER.fetch_add(1, Ordering::Relaxed) + 1
    } else {
        ATTACKER_BUFF_UID_COUNTER.fetch_add(1, Ordering::Relaxed) + 1
    }
}

/// Explicit buff uids can arrive from previewed or replayed effects without
/// going through `next_buff_uid_for_target`. Keep side-local counters ahead of
/// them so later generated ids do not collide with live runtime slots.
pub fn observe_explicit_buff_uid_for_target(target_uid: i64, buff_uid: i64) {
    if buff_uid <= 0 {
        return;
    }
    #[cfg(test)]
    let _test_lock = buff_uid_test_access_lock();
    if target_uid < 0 {
        DEFENDER_BUFF_UID_COUNTER.fetch_max(buff_uid, Ordering::Relaxed);
    } else {
        ATTACKER_BUFF_UID_COUNTER.fetch_max(buff_uid, Ordering::Relaxed);
    }
}

pub fn reset_buff_uid() {
    #[cfg(test)]
    let _test_lock = buff_uid_test_access_lock();
    ATTACKER_BUFF_UID_COUNTER.store(0, Ordering::Relaxed);
    DEFENDER_BUFF_UID_COUNTER.store(DEFENDER_BUFF_UID_START, Ordering::Relaxed);
    ACTIVE_DEFENDER_LANE.store(false, Ordering::Relaxed);
}

pub fn reset_buff_uid_to(value: i64) {
    #[cfg(test)]
    let _test_lock = buff_uid_test_access_lock();
    if value >= DEFENDER_BUFF_UID_START {
        DEFENDER_BUFF_UID_COUNTER.store(value, Ordering::Relaxed);
        ACTIVE_DEFENDER_LANE.store(true, Ordering::Relaxed);
    } else {
        ATTACKER_BUFF_UID_COUNTER.store(value, Ordering::Relaxed);
        ACTIVE_DEFENDER_LANE.store(false, Ordering::Relaxed);
    }
}

pub fn current_buff_uid() -> i64 {
    #[cfg(test)]
    let _test_lock = buff_uid_test_access_lock();
    if ACTIVE_DEFENDER_LANE.load(Ordering::Relaxed) {
        DEFENDER_BUFF_UID_COUNTER.load(Ordering::Relaxed)
    } else {
        ATTACKER_BUFF_UID_COUNTER.load(Ordering::Relaxed)
    }
}

pub fn attacker_buff_uid_checkpoint() -> i64 {
    #[cfg(test)]
    let _test_lock = buff_uid_test_access_lock();
    ATTACKER_BUFF_UID_COUNTER.load(Ordering::Relaxed)
}

pub fn defender_buff_uid_checkpoint() -> i64 {
    #[cfg(test)]
    let _test_lock = buff_uid_test_access_lock();
    DEFENDER_BUFF_UID_COUNTER.load(Ordering::Relaxed)
}

pub fn sync_buff_uid_counters_from_mgr(mgr: &BuffMgr) {
    #[cfg(test)]
    let _test_lock = buff_uid_test_access_lock();
    let mut attacker_max = 0_i64;
    let mut defender_max = DEFENDER_BUFF_UID_START;

    for (target_uid, buffs) in &mgr.active {
        for buff in buffs {
            if *target_uid < 0 {
                defender_max = defender_max.max(buff.uid);
            } else {
                attacker_max = attacker_max.max(buff.uid);
            }
        }
    }

    ATTACKER_BUFF_UID_COUNTER.store(attacker_max, Ordering::Relaxed);
    DEFENDER_BUFF_UID_COUNTER.store(defender_max, Ordering::Relaxed);
}

pub fn sync_from_fight(fight: &Fight, mgr: &mut BuffMgr) {
    mgr.active.clear();

    let mut import_side = |entitys: &[sonettobuf::FightEntityInfo]| {
        for e in entitys {
            let target_uid = e.uid.unwrap_or(0);
            if target_uid == 0 {
                continue;
            }
            for b in e.buffs.iter().chain(e.no_effect_buffs.iter()) {
                let buff_id = b.buff_id.unwrap_or(0);
                let from_uid = b.from_uid.unwrap_or(0);
                let count = b.count.unwrap_or(0);
                let layer = b.layer.unwrap_or(0);
                let buff_uid = b.uid.unwrap_or(0);
                if buff_id == 0 || buff_uid == 0 {
                    continue;
                }
                observe_explicit_buff_uid_for_target(target_uid, buff_uid);
                mgr.add_with_uid(target_uid, buff_id, from_uid, 0, count, layer, buff_uid);
                let _ = mgr.set_instance_act_common_params(
                    target_uid,
                    buff_uid,
                    b.act_common_params.as_deref().unwrap_or_default(),
                );
            }
        }
    };

    if let Some(attacker) = &fight.attacker {
        import_side(&attacker.entitys);
        import_side(&attacker.sub_entitys);
    }
    if let Some(defender) = &fight.defender {
        import_side(&defender.entitys);
        import_side(&defender.sub_entitys);
    }
}

pub fn sync_from_fight_preserve_runtime(fight: &Fight, mgr: &mut BuffMgr) {
    // Start from authoritative fight snapshot, then keep runtime-only entries
    // and runtime-updated entries keyed by uid.
    let runtime = mgr.active.clone();
    sync_from_fight(fight, mgr);

    for (target_uid, buffs) in runtime {
        let entry = mgr.active.entry(target_uid).or_default();
        for buff in buffs {
            if let Some(existing) = entry.iter_mut().find(|b| b.uid == buff.uid) {
                *existing = buff;
            } else {
                entry.push(buff);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BuffInstance, DEFENDER_BUFF_UID_START, RefreshPolicy, current_buff_uid, next_buff_uid,
        next_slave_buff_uid_for_target, reset_buff_uid, reset_buff_uid_to, with_buff_uid_test_lock,
    };
    use std::{path::PathBuf, sync::Once};

    static TEST_CONFIG_INIT: Once = Once::new();

    fn ensure_game_data_initialized() {
        TEST_CONFIG_INIT.call_once(|| {
            if config::configs::try_get().is_some() {
                return;
            }
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            let excel_dir = root.join("data").join("excel2json");
            if excel_dir.exists()
                && let Some(path) = excel_dir.to_str()
            {
                let _ = config::configs::init(path);
            }
        });
    }

    #[test]
    fn buff_uid_policy_normal_adds_increment_by_two() {
        with_buff_uid_test_lock(|| {
            reset_buff_uid();
            let a = next_buff_uid();
            let b = next_buff_uid();
            let c = next_buff_uid();
            assert_eq!(a, 2);
            assert_eq!(b, 4);
            assert_eq!(c, 6);
        });
    }

    #[test]
    fn tuesday_dot_family_add_with_uid_keeps_distinct_instances() {
        ensure_game_data_initialized();
        with_buff_uid_test_lock(|| {
            let mut mgr = super::BuffMgr::new();
            mgr.add_with_uid(-1, 30980111, 230646524, 0, 0, 1, 100002);
            mgr.add_with_uid(-1, 30980111, 230646524, 0, 0, 1, 100004);
            mgr.add_with_uid(-1, 30980111, 230646524, 0, 0, 1, 100006);

            let buffs = mgr.get(-1);
            assert_eq!(buffs.len(), 3);
            assert_eq!(buffs.iter().filter(|b| b.buff_id == 30980111).count(), 3);
            assert_eq!(
                buffs.iter().map(|b| b.uid).collect::<Vec<_>>(),
                vec![100002, 100004, 100006]
            );
            assert!(buffs.iter().all(|b| b.layer == 1));
        });
    }

    #[test]
    fn buff_uid_policy_slave_adds_increment_by_one() {
        with_buff_uid_test_lock(|| {
            reset_buff_uid();
            let a = next_slave_buff_uid_for_target(1);
            let b = next_slave_buff_uid_for_target(1);
            let c = next_slave_buff_uid_for_target(1);
            assert_eq!(a, 1);
            assert_eq!(b, 2);
            assert_eq!(c, 3);
        });
    }

    #[test]
    fn buff_uid_policy_mixed_sequence_matches_live_style_counter() {
        with_buff_uid_test_lock(|| {
            reset_buff_uid();
            let normal_1 = next_buff_uid(); // 2
            let normal_2 = next_buff_uid(); // 4
            let slave_1 = next_slave_buff_uid_for_target(1); // 5
            let normal_3 = next_buff_uid(); // 7

            assert_eq!(normal_1, 2);
            assert_eq!(normal_2, 4);
            assert_eq!(slave_1, 5);
            assert_eq!(normal_3, 7);
        });
    }

    #[test]
    fn buff_uid_policy_defender_range_switch_and_restore() {
        with_buff_uid_test_lock(|| {
            reset_buff_uid();
            assert_eq!(next_buff_uid(), 2);

            let checkpoint = current_buff_uid();
            reset_buff_uid_to(DEFENDER_BUFF_UID_START);
            assert_eq!(next_buff_uid(), DEFENDER_BUFF_UID_START + 2);

            reset_buff_uid_to(checkpoint);
            assert_eq!(next_buff_uid(), checkpoint + 2);
        });
    }

    #[test]
    fn buff_instance_build_defaults_refresh_policy_to_update_in_place() {
        ensure_game_data_initialized();
        with_buff_uid_test_lock(|| {
            reset_buff_uid();
            let instance = BuffInstance::build(30091111, 42, 0);
            assert_eq!(instance.refresh_policy, RefreshPolicy::UpdateInPlace);
        });
    }

    #[test]
    fn duality_potion_regrants_refresh_one_runtime_holder() {
        ensure_game_data_initialized();
        with_buff_uid_test_lock(|| {
            reset_buff_uid();
            let mut mgr = super::BuffMgr::new();

            mgr.add(30137385, 30091120, 30137385, 0, 1, 1);
            mgr.add(30137385, 30091120, 30137385, 0, 1, 2);

            let active = mgr.get(30137385);
            assert_eq!(active.len(), 1);
            assert_eq!(active[0].buff_id, 30091120);
            assert_eq!(active[0].layer, 2);
        });
    }
}
