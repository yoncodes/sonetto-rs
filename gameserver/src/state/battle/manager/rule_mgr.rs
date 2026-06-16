use sonettobuf::Fight;

use super::super::rule::collect::collect_rules;
use super::traits::Manager;
use super::fight_data_mgr::Managers;
use crate::state::battle::effect;

#[derive(Default, Clone)]
pub struct RuleMgr {
    pub effects: Vec<effect::SkillEffect>,
}

impl std::fmt::Debug for RuleMgr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuleMgr").field("effects_count", &self.effects.len()).finish()
    }
}

fn all_entity_uids(fight: &Fight) -> Vec<i64> {
    let attacker = fight.attacker.as_ref().into_iter()
        .flat_map(|a| a.entitys.iter());
    let defender = fight.defender.as_ref().into_iter()
        .flat_map(|d| d.entitys.iter());
    attacker.chain(defender).filter_map(|e| e.uid).collect()
}

impl RuleMgr {
    pub fn new(fight: &Fight) -> Self {
        let mut mgr = Self::default();
        let rules = collect_rules(fight);
        for uid in all_entity_uids(fight) {
            mgr.seed_entity_uid_with_rules(uid, &rules);
        }
        mgr
    }

    pub fn seed_entity_uid(&mut self, uid: i64, fight: &Fight) {
        let rules = collect_rules(fight);
        self.seed_entity_uid_with_rules(uid, &rules);
    }

    fn seed_entity_uid_with_rules(&mut self, uid: i64, rules: &[(i32, i32)]) {
        let new_effects: Vec<effect::SkillEffect> = rules
            .iter()
            .filter_map(|&(prefix, effect_id)| {
                let matched = match prefix {
                    1 => uid >= 0,
                    2 => uid < 0,
                    _ => true,
                };
                if !matched { return None; }
                tracing::info!(prefix, effect_id, uid, "rule_mgr: parsing rule for matched uid");
                effect::parser::parse(effect_id, uid)
            })
            .collect();
        self.effects.extend(new_effects);
    }

}

impl Manager for RuleMgr {}
