use sonettobuf::{Fight, FightEntityInfo, FightExPointInfo, FightStep};
use std::collections::{HashMap, HashSet};

use super::super::{
    event::Event,
    fight_step::{ActEffectBuilder, FightStepBuilder},
    types::ex_point::ExPointType,
};
use super::traits::Manager;

#[derive(Debug, Clone, Copy)]
pub struct EntityLocation {
    pub is_attacker: bool,
    pub index: usize,
}

pub type FightEntityDataMgr = EntityMgr;

#[derive(Default, Debug, Clone)]
pub struct EntityMgr {
    entity_cache: HashMap<i64, EntityLocation>,
    ex_points: HashMap<i64, i32>,
    ex_max: HashMap<i64, i32>,
    ex_point_required: HashMap<i64, i32>,
    pub current_hp: HashMap<i64, i32>,
    pub max_hp: HashMap<i64, i32>,
    recent_decr_ex_point: HashMap<i64, i32>,
    pub action_points: HashMap<i64, i32>,
    pub attr_bonus: HashMap<(i64, i32), Vec<i32>>,
}

impl EntityMgr {
    pub fn new(fight: &Fight) -> Self {
        let mut mgr = Self::default();
        mgr.rebuild_cache(fight);
        mgr.init(fight);
        mgr
    }

    pub fn rebuild_cache(&mut self, fight: &Fight) {
        self.entity_cache.clear();
        if let Some(attacker) = &fight.attacker {
            for (idx, entity) in attacker.entitys.iter().enumerate() {
                if let Some(uid) = entity.uid {
                    self.entity_cache.insert(uid, EntityLocation { is_attacker: true, index: idx });
                }
            }
            /* 
            When a hero die, sub_entity will move to entity list
            So we may only cache the main entity list
            for (idx, entity) in attacker.sub_entitys.iter().enumerate() {
                if let Some(uid) = entity.uid {
                    self.entity_cache.insert(uid, EntityLocation { is_attacker: true, index: idx });
                }
            }
            */
        }
        if let Some(defender) = &fight.defender {
            for (idx, entity) in defender.entitys.iter().enumerate() {
                if let Some(uid) = entity.uid {
                    self.entity_cache.insert(uid, EntityLocation { is_attacker: false, index: idx });
                }
            }
        }
    }

    pub fn get_location(&self, entity_id: i64) -> Option<EntityLocation> {
        self.entity_cache.get(&entity_id).copied()
    }

    pub fn all_positioned_uids(fight: &Fight) -> Vec<i64> {
        [fight.attacker.as_ref(), fight.defender.as_ref()]
            .into_iter()
            .flatten()
            .flat_map(|s| s.entitys.iter().chain(s.sub_entitys.iter()))
            .filter_map(|e| e.uid.filter(|_| e.position.unwrap_or(-1) > 0))
            .collect()
    }

    pub fn alive_hero_uids(&self) -> HashSet<i64> {
        self.entity_cache
            .iter()
            .filter(|(uid, loc)| loc.is_attacker && self.current_hp.get(uid).copied().unwrap_or(0) > 0)
            .map(|(uid, _)| *uid)
            .collect()
    }

    pub fn alive_enemy_uids(&self) -> HashSet<i64> {
        self.entity_cache
            .iter()
            .filter(|(uid, loc)| !loc.is_attacker && self.current_hp.get(uid).copied().unwrap_or(0) > 0)
            .map(|(uid, _)| *uid)
            .collect()
    }

    /// If any active hero is dead and a sub is available, substitutes the first such hero.
    /// Returns `(dead_uid, new_entity, position)` where position is 1-based.
    pub fn sub_hero(&mut self, fight: &mut Fight) -> Option<(i64, FightEntityInfo, i32)> {
        let attacker = fight.attacker.as_mut()?;
        if attacker.sub_entitys.is_empty() {
            return None;
        }
        let (slot, dead_uid, position) = attacker.entitys.iter().enumerate().find_map(|(i, e)| {
            let uid = e.uid?;
            let pos = e.position.unwrap_or(0);
            if e.current_hp.unwrap_or(0) <= 0 && pos > 0 {
                Some((i, uid, pos))
            } else {
                None
            }
        })?;
        let mut sub = attacker.sub_entitys.remove(0);
        sub.position = Some(position);
        attacker.entitys[slot] = sub.clone();
        self.rebuild_cache(fight);
        Some((dead_uid, sub, position))
    }

    #[allow(dead_code)]
    pub fn get_team_entities<'a>(&self, fight: &'a Fight, team_type: i32) -> Vec<&'a FightEntityInfo> {
        let mut entities = Vec::new();
        if let Some(attacker) = &fight.attacker {
            entities.extend(attacker.entitys.iter().filter(|e| e.team_type == Some(team_type)));
        }
        if let Some(defender) = &fight.defender {
            entities.extend(defender.entitys.iter().filter(|e| e.team_type == Some(team_type)));
        }
        entities
    }

    pub fn init(&mut self, fight: &Fight) {
        let cfg = config::configs::get();
        let iter = fight
            .attacker
            .iter()
            .chain(fight.defender.iter())
            .flat_map(|t| t.entitys.iter().chain(t.sub_entitys.iter()));
        for e in iter {
            let Some(uid) = e.uid else { continue };
            self.ex_points.insert(uid, e.ex_point.unwrap_or(0));
            self.ex_max.insert(uid, if e.ex_point_type == Some(1) { 8 } else { 5 });
            let required = e.model_id
                .and_then(|mid| cfg.monster_skill_template.iter().find(|t| t.id == mid))
                .map(|t| t.unique_skill_point)
                .unwrap_or(0);
            self.ex_point_required.insert(uid, required);
            let hp = e.current_hp.unwrap_or(0);
            self.current_hp.insert(uid, hp);
            let mhp = e.attr.as_ref().and_then(|a| a.hp).unwrap_or(hp);
            self.max_hp.insert(uid, mhp);
            self.action_points.insert(uid, 1);
        }
    }

    pub fn add_ex_point(&mut self, uid: i64, amount: i32) {
        let v = self.ex_points.entry(uid).or_insert(0);
        *v = (*v + amount).max(0);
    }

    pub fn on_use_card(&mut self, uid: i64) -> Option<FightStep> {
        self.add_ex_point(uid, 1);
        Some(FightStepBuilder::effect().with(ActEffectBuilder::ex_point_change(uid, 1)).build())
    }

    pub fn on_move_card(&mut self, uid: i64) -> Option<FightStep> {
        self.add_ex_point(uid, 1);
        Some(FightStepBuilder::effect().with(ActEffectBuilder::ex_point_change(uid, 1)).build())
    }

    pub fn on_card_upgrade(&mut self, uid: i64) -> Option<FightStep> {
        self.add_ex_point(uid, 1);
        Some(FightStepBuilder::effect().with(ActEffectBuilder::ex_point_change(uid, 1)).build())
    }

    pub fn set_recent_decr_ex_point(&mut self, uid: i64, amount: i32) {
        self.recent_decr_ex_point.insert(uid, amount.max(0));
    }

    pub fn get_recent_decr_ex_point(&self, uid: i64) -> i32 {
        self.recent_decr_ex_point.get(&uid).copied().unwrap_or(0)
    }

    pub fn clear_recent_decr_ex_point(&mut self, uid: i64) {
        self.recent_decr_ex_point.remove(&uid);
    }

    #[allow(dead_code)]
    pub fn add_ex_max(&mut self, uid: i64, amount: i32) {
        let v = self.ex_max.entry(uid).or_insert(0);
        *v += amount;
    }

    pub fn get_ex_max(&self, uid: i64) -> i32 {
        self.ex_max.get(&uid).copied().unwrap_or(0)
    }

    pub fn get_ex_point_required(&self, uid: i64) -> i32 {
        self.ex_point_required.get(&uid).copied().unwrap_or(0)
    }

    #[allow(dead_code)]
    pub fn consume_ex_point(&mut self, uid: i64, amount: i32) -> bool {
        let v = self.ex_points.entry(uid).or_insert(0);
        if *v >= amount {
            *v -= amount;
            true
        } else {
            false
        }
    }

    pub fn set_ex_point(&mut self, uid: i64, value: i32) {
        self.ex_points.insert(uid, value.max(0));
    }

    pub fn get_ex_point(&self, uid: i64) -> i32 {
        self.ex_points.get(&uid).copied().unwrap_or(0)
    }

    pub fn get_ac_point(&self, fight: &Fight, is_attacker: bool) -> i32 {
        tracing::info!(" action point hashmap: {:?}", self.action_points);
        let side = if is_attacker {
            fight.attacker.as_ref()
        } else {
            fight.defender.as_ref()
        };

        side.into_iter()
            .flat_map(|team| team.entitys.iter())
            .filter_map(|entity| entity.uid)
            .map(|uid| self.action_points.get(&uid).copied().unwrap_or(0))
            .sum()
    }

    pub fn set_action_point(&mut self, uid: i64, val: i32) {
        self.action_points.insert(uid, val);
    }

    pub fn add_action_point(&mut self, uid: i64, delta: i32) -> Event {
        let v = self.action_points.entry(uid).or_insert(0);
        *v += delta;
        Event::AddActionPoint { entity_uid: uid, delta }
    }

    #[allow(dead_code)]
    pub fn set_hp(&mut self, uid: i64, hp: i32) {
        self.current_hp.insert(uid, hp.max(0));
    }

    pub fn get_hp(&self, uid: i64) -> i32 {
        self.current_hp.get(&uid).copied().unwrap_or(0)
    }

    pub fn set_max_hp(&mut self, uid: i64, hp: i32) {
        self.max_hp.insert(uid, hp.max(0));
    }

    pub fn get_max_hp(&self, uid: i64) -> i32 {
        self.max_hp.get(&uid).copied().unwrap_or(0)
    }

    pub fn apply_damage(&mut self, uid: i64, amount: i32) {
        let hp = self.current_hp.entry(uid).or_insert(0);
        *hp = (*hp - amount).max(0);
    }

    #[allow(dead_code)]
    pub fn apply_heal(&mut self, uid: i64, amount: i32, max_hp: i32) {
        let hp = self.current_hp.entry(uid).or_insert(0);
        *hp = (*hp + amount).min(max_hp);
    }

    pub fn merge_attr_bonus(&mut self, contributions: HashMap<(i64, i32), Vec<i32>>) {
        for (key, values) in contributions {
            let entry = self.attr_bonus.entry(key).or_default();
            for v in values {
                if v == 0 {
                    continue;
                }
                entry.push(v);
            }
        }
    }

    pub fn sum_attr_bonus(&self, uid: i64, attr_id: i32) -> i32 {
        self.attr_bonus
            .get(&(uid, attr_id))
            .map(|v| v.iter().copied().fold(0i32, i32::saturating_add))
            .unwrap_or(0)
    }

    pub fn clear_attr_bonus(&mut self) {
        self.attr_bonus.clear();
    }

    pub fn remove_attr_bonus(&mut self, uid: i64, attr_id: i32, amount: i32) {
        if let Some(vec) = self.attr_bonus.get_mut(&(uid, attr_id)) {
            if let Some(pos) = vec.iter().position(|&v| v == amount) {
                vec.swap_remove(pos);
            }
            if vec.is_empty() {
                self.attr_bonus.remove(&(uid, attr_id));
            }
        }
    }
}

impl Manager for EntityMgr {
    fn on_enter_fight(&mut self, _fight: &Fight, entity_uid: i64) -> Vec<crate::state::battle::event::Event> {
        self.set_action_point(entity_uid, 1);
        vec![]
    }

    fn on_round_end(&mut self, _fight: &mut Fight) {
        self.recent_decr_ex_point.clear();
        self.action_points.values_mut().for_each(|ap| *ap = 1);
    }

    fn on_battle_end(&mut self) {
        self.ex_points.clear();
        self.current_hp.clear();
        self.recent_decr_ex_point.clear();
    }
}

pub fn get_entity_mut_by_location(
    fight: &mut Fight,
    location: EntityLocation,
) -> Option<&mut sonettobuf::FightEntityInfo> {
    if location.is_attacker {
        fight.attacker.as_mut()?.entitys.get_mut(location.index)
    } else {
        fight.defender.as_mut()?.entitys.get_mut(location.index)
    }
}

pub fn build_ex_point_info(fight: &Fight, mgr: &EntityMgr) -> Vec<FightExPointInfo> {
    fight
        .attacker
        .iter()
        .chain(fight.defender.iter())
        .flat_map(|t| t.entitys.iter().chain(t.sub_entitys.iter()))
        .map(|e| {
            let uid = e.uid.unwrap_or(0);
            let hp = mgr.get_hp(uid);
            let ex = mgr.get_ex_point(uid);
            tracing::debug!("build_ex_point_info uid={} hp={} ex={}", uid, hp, ex);
            let ex_point_type = match e.model_id {
                Some(3120) => Some(ExPointType::Belief as i32),
                Some(3123) => Some(ExPointType::Synchronization as i32),
                Some(3124) | Some(3122) => Some(ExPointType::Adrenaline as i32),
                _ => e.ex_point_type.or(Some(ExPointType::Common as i32)),
            };
            FightExPointInfo {
                uid: e.uid,
                ex_point: Some(mgr.get_ex_point(uid)),
                power_infos: e.power_infos.clone(),
                current_hp: Some(hp),
                ex_point_type,
            }
        })
        .collect()
}

pub fn sync_to_fight(fight: &mut Fight, mgr: &EntityMgr) {
    for e in fight
        .attacker
        .iter_mut()
        .chain(fight.defender.iter_mut())
        .flat_map(|t| t.entitys.iter_mut().chain(t.sub_entitys.iter_mut()))
    {
        let Some(uid) = e.uid else { continue };
        e.ex_point = Some(mgr.get_ex_point(uid));
        e.current_hp = Some(mgr.get_hp(uid));
        let max_hp = mgr.get_max_hp(uid);
        if max_hp > 0 {
            if let Some(attr) = e.attr.as_mut() {
                attr.hp = Some(max_hp);
            }
            if let Some(base) = e.base_attr.as_mut() {
                base.hp = Some(max_hp);
            }
        }
    }
}

pub fn sync_from_fight(fight: &Fight, mgr: &mut EntityMgr) {
    for e in fight
        .attacker
        .iter()
        .chain(fight.defender.iter())
        .flat_map(|t| t.entitys.iter().chain(t.sub_entitys.iter()))
    {
        let Some(uid) = e.uid else { continue };
        mgr.set_ex_point(uid, e.ex_point.unwrap_or(0));
        mgr.set_hp(uid, e.current_hp.unwrap_or(0));
        if let Some(max) = e.attr.as_ref().and_then(|a| a.hp) {
            mgr.set_max_hp(uid, max);
        }
    }
}

pub fn seed_ex_point_required_from_fight(fight: &Fight, mgr: &mut EntityMgr) {
    let cfg = config::configs::get();
    for e in fight
        .attacker
        .iter()
        .chain(fight.defender.iter())
        .flat_map(|t| t.entitys.iter().chain(t.sub_entitys.iter()))
    {
        let Some(uid) = e.uid else { continue };
        let required = e.model_id
            .and_then(|mid| cfg.monster_skill_template.iter().find(|t| t.id == mid))
            .map(|t| t.unique_skill_point)
            .unwrap_or(0);
        mgr.ex_point_required.insert(uid, required);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn merge_attr_bonus_appends_each_value_individually() {
        let mut mgr = EntityMgr::default();
        let mut first = HashMap::new();
        first.insert((1i64, 102i32), vec![50]);
        mgr.merge_attr_bonus(first);

        let mut second = HashMap::new();
        second.insert((1i64, 102i32), vec![30]);
        mgr.merge_attr_bonus(second);

        assert_eq!(mgr.attr_bonus.get(&(1, 102)).unwrap(), &vec![50, 30]);
    }

    #[test]
    fn merge_attr_bonus_drops_zero_amounts() {
        let mut mgr = EntityMgr::default();
        let mut m = HashMap::new();
        m.insert((1i64, 102i32), vec![0, 25, 0]);
        mgr.merge_attr_bonus(m);
        assert_eq!(mgr.attr_bonus.get(&(1, 102)).unwrap(), &vec![25]);
    }

    #[test]
    fn sum_attr_bonus_sums_vec_with_saturation() {
        let mut mgr = EntityMgr::default();
        let mut m = HashMap::new();
        m.insert((1i64, 102i32), vec![i32::MAX, 100]);
        mgr.merge_attr_bonus(m);
        assert_eq!(mgr.sum_attr_bonus(1, 102), i32::MAX);
    }

    #[test]
    fn sum_attr_bonus_returns_zero_for_absent_key() {
        let mgr = EntityMgr::default();
        assert_eq!(mgr.sum_attr_bonus(99, 102), 0);
    }

    #[test]
    fn clear_attr_bonus_empties_the_map() {
        let mut mgr = EntityMgr::default();
        let mut m = HashMap::new();
        m.insert((1i64, 102i32), vec![50]);
        mgr.merge_attr_bonus(m);
        assert!(!mgr.attr_bonus.is_empty());
        mgr.clear_attr_bonus();
        assert!(mgr.attr_bonus.is_empty());
    }
}

