use std::collections::HashMap;

use sonettobuf::{Fight, FightEntityInfo};

use crate::state::battle::{
    effect::{self, SkillEffect},
};

#[derive(Default, Debug, Clone)]
pub struct PassiveMgr {
    map: HashMap<i64, Vec<SkillEffect>>,
}

impl PassiveMgr {
    pub fn new(fight: &Fight) -> Self {
        let mut mgr = Self::default();
        let sides = [fight.attacker.as_ref(), fight.defender.as_ref()];
        for side in sides.into_iter().flatten() {
            for entity in side.entitys.iter() {
                mgr.seed_entity(entity);
            }
        }
        mgr
    }

    pub fn seed_entity(&mut self, entity: &FightEntityInfo) {
        let Some(uid) = entity.uid else { return };
        if entity.passive_skill.is_empty() { return }

        let effects: Vec<SkillEffect> = entity
            .passive_skill
            .iter()
            .filter_map(|&sid| {
                tracing::info!(uid, sid, "passive_mgr: parsing passive for entity");
                effect::parser::parse(sid, uid)
            })
            .collect();

        if !effects.is_empty() {
            tracing::info!(uid, count = effects.len(), "passive_mgr: loaded passives for entity");
            self.map.insert(uid, effects);
        }
    }

    pub fn get(&self, uid: i64) -> &[SkillEffect] {
        self.map.get(&uid).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn get_mut(&mut self, uid: i64) -> &mut [SkillEffect] {
        self.map.get_mut(&uid).map(Vec::as_mut_slice).unwrap_or(&mut [])
    }


}
