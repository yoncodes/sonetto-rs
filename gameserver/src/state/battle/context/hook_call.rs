use sonettobuf::Fight;
use crate::state::battle::{
    buff::Buff,
    effect::{SkillEffect, condition::Hook},
    event::Event,
    manager::fight_data_mgr::Managers,
};

enum HookPayload {
    Effect(SkillEffect, i64),
    Buff(Buff, i64),
}

struct HookEntry {
    priority: i32, // TODO: derive from effect/condition config
    payload: HookPayload,
}

impl HookEntry {
    fn fire(self, hook: Hook, fight: &Fight, managers: &mut Managers) -> Vec<Event> {
        match self.payload {
            HookPayload::Effect(mut e, owner_uid) => e.fire_hook(hook, fight, managers, owner_uid),
            HookPayload::Buff(mut b, entity_uid) => {
                let events = b.fire_hook(hook, fight, managers, entity_uid);
                if !b.attr_bonus_refs.is_empty() {
                    if let Some(buffs) = managers.buff_mgr.active_buff.get_mut(&entity_uid) {
                        if let Some(existing) = buffs.iter_mut().find(|x| x.buff_id == b.buff_id) {
                            existing.attr_bonus_refs.extend(b.attr_bonus_refs);
                        }
                    }
                }
                events
            }
        }
    }

    fn fire_attr_fix(
        self,
        hook: Hook,
        fight: &Fight,
        managers: &Managers,
    ) -> std::collections::HashMap<(i64, i32), Vec<i32>> {
        match self.payload {
            HookPayload::Effect(mut e, owner_uid) => {
                e.fire_hook_attr_fix(hook, fight, managers, owner_uid)
            }
            HookPayload::Buff(b, entity_uid) => {
                // b.fire_hook_attr_fix(hook, fight, managers, entity_uid)
                std::collections::HashMap::new()
            }
        }
    }
}

fn sort_entries(entries: &mut Vec<HookEntry>) {
    // TODO: sort by priority once priority field is populated from config
    let _ = entries;
}

fn collect_passive(managers: &Managers, entity_uid: i64) -> Vec<HookEntry> {
    managers.passive_mgr.get(entity_uid).iter().cloned()
        .map(|e| HookEntry { priority: 0, payload: HookPayload::Effect(e, entity_uid) })
        .collect()
}

fn collect_rule(managers: &Managers, entity_uid: i64) -> Vec<HookEntry> {
    managers.rule_mgr.effects.iter().cloned()
        .map(|e| HookEntry { priority: 0, payload: HookPayload::Effect(e, entity_uid) })
        .collect()
}

fn collect_buff(managers: &Managers, entity_uid: i64) -> Vec<HookEntry> {
    managers.buff_mgr.active_buff.get(&entity_uid).cloned().unwrap_or_default()
        .into_iter()
        .map(|b| HookEntry { priority: 0, payload: HookPayload::Buff(b, entity_uid) })
        .collect()
}

fn collect_active(managers: &Managers) -> Vec<HookEntry> {
    let Some(idx) = managers.active_effect_mgr.active_idx else {
        return Vec::new();
    };
    managers.active_effect_mgr.map.get(&idx)
        .into_iter()
        .flat_map(|effects| effects.iter().cloned().map(|e| {
            let owner = e.owner_uid;
            HookEntry { priority: 0, payload: HookPayload::Effect(e, owner) }
        }))
        .collect()
}

pub fn on_buff_add(managers: &mut Managers, fight: &Fight, target_uid: i64) -> Vec<Event> {
    fire_hook(managers, fight, Hook::BuffAdd, target_uid)
}

pub fn on_dead(managers: &mut Managers, fight: &Fight, entity_uid: i64) -> Vec<Event> {
    fire_hook(managers, fight, Hook::Dead, entity_uid)
}

pub fn on_eval_active_skill(managers: &mut Managers, fight: &Fight, caster_uid: i64) -> Vec<Event> {
    fire_hook(managers, fight, Hook::EvalActiveSkill, caster_uid)
}

pub fn on_eval_active_skill_attr_fix(
    managers: &mut Managers,
    fight: &Fight,
    caster_uid: i64,
) -> std::collections::HashMap<(i64, i32), Vec<i32>> {
    fire_hook_attr_fix(managers, fight, Hook::EvalActiveSkill, caster_uid)
}

pub fn on_use_ex_skill(managers: &mut Managers, fight: &Fight, caster_uid: i64) -> Vec<Event> {
    fire_hook(managers, fight, Hook::UseExSkill, caster_uid)
}

pub fn on_eval_being_attacked(managers: &mut Managers, fight: &Fight, defender_uid: i64) -> Vec<Event> {
    fire_hook(managers, fight, Hook::EvalBeingAttacked, defender_uid)
}

pub fn on_eval_being_attacked_attr_fix(
    managers: &mut Managers,
    fight: &Fight,
    defender_uid: i64,
) -> std::collections::HashMap<(i64, i32), Vec<i32>> {
    fire_hook_attr_fix(managers, fight, Hook::EvalBeingAttacked, defender_uid)
}

pub fn on_after_action(managers: &mut Managers, fight: &Fight, caster_uid: i64) -> Vec<Event> {
    fire_hook(managers, fight, Hook::AfterAction, caster_uid)
}

pub fn fire_hook(
    managers: &mut Managers,
    fight: &Fight,
    hook: Hook,
    entity_uid: i64,
) -> Vec<Event> {
    let mut entries = collect_buff(managers, entity_uid);
    entries.extend(collect_rule(managers, entity_uid));
    entries.extend(collect_passive(managers, entity_uid));
    entries.extend(collect_active(managers));
    sort_entries(&mut entries);
    let mut events = Vec::new();
    for entry in entries {
        let mut snapshot = managers.clone();
        events.extend(entry.fire(hook, fight, &mut snapshot));
        *managers = snapshot;
    }
    events
}

pub fn fire_hook_attr_fix(
    managers: &mut Managers,
    fight: &Fight,
    hook: Hook,
    entity_uid: i64,
) -> std::collections::HashMap<(i64, i32), Vec<i32>> {
    let mut entries = collect_buff(managers, entity_uid);
    entries.extend(collect_rule(managers, entity_uid));
    entries.extend(collect_passive(managers, entity_uid));
    entries.extend(collect_active(managers));
    sort_entries(&mut entries);
    let mut acc: std::collections::HashMap<(i64, i32), Vec<i32>> = std::collections::HashMap::new();
    for entry in entries {
        let map = entry.fire_attr_fix(hook, fight, managers);
        for (k, mut v) in map {
            acc.entry(k).or_default().append(&mut v);
        }
    }
    acc
}