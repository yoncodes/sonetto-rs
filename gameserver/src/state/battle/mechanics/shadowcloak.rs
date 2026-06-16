use super::super::{
    fight_step::{ActEffectBuilder, FightStepBuilder},
    manager::{buff_mgr::BuffMgr, entity_mgr::EntityMgr},
    utils::find_entity,
};
use sonettobuf::{ActEffect, Fight, FightStep};

use crate::state::battle::heroes::rubuska;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ShadowCloakState {
    total_shared: i32,     // total accumulated across all ticks
    last_gain_shared: i32, // gain from this tick
    pub rubuska_entry_max_hp: i32,
    pub rubuska_uid: i64,
    pub raspberry_accum: i32,
    pub raspberry_max: i32,
}

impl ShadowCloakState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn init(&mut self, fight: &Fight) {
        rubuska::init_shadow_cloak_state(self, fight);
    }

    pub fn is_active(&self) -> bool {
        self.rubuska_uid != 0
    }

    pub fn add(&mut self, uid: i64, hp_lost: i32) -> i32 {
        tracing::warn!(
            "shadow_cloak add: uid={} rubuska_uid={} hp_lost={}",
            uid,
            self.rubuska_uid,
            hp_lost
        );
        // only Rubuska's own HP loss drives shadow cloak
        if uid != self.rubuska_uid {
            return 0;
        }
        let gain = rubuska::shadow_friend_hp_to_cloak_gain(hp_lost, self.rubuska_entry_max_hp);
        if gain <= 0 {
            return 0;
        }
        self.last_gain_shared = gain;
        self.total_shared += gain;
        gain
    }

    pub fn reset_tick(&mut self) {
        self.last_gain_shared = 0;
    }

    pub fn build_sync_effects(
        &self,
        fight: &Fight,
        buff_mgr: &BuffMgr,
        entity_mgr: &EntityMgr,
    ) -> Vec<ActEffect> {
        let gain = self.last_gain_shared;
        tracing::warn!(
            "shadow_cloak sync: gain={} total={}",
            self.last_gain_shared,
            self.total_shared
        );
        if gain == 0 {
            return vec![];
        }
        let total = self.total_shared;
        let max_capacity = rubuska::shadow_cloak_capacity(self.rubuska_entry_max_hp);
        let mut effects = Vec::new();

        let uids: Vec<i64> = fight
            .attacker
            .as_ref()
            .map(|a| a.entitys.iter().filter_map(|e| e.uid).collect())
            .unwrap_or_default();

        for uid in uids {
            let current_hp = entity_mgr.get_hp(uid);
            let base_max_hp = find_entity(fight, uid)
                .and_then(|e| e.attr.as_ref().and_then(|a| a.hp))
                .unwrap_or(0);
            let new_max_hp = base_max_hp + total;
            let buff_uid = buff_mgr
                .get(uid)
                .iter()
                .find(|b| b.buff_id == rubuska::SHADOW_CLOAK_ACCUMULATOR_BUFF_ID)
                .map(|b| b.uid)
                .unwrap_or(0);

            effects.push(ActEffectBuilder::current_hp_change(uid, current_hp));
            effects.push(ActEffectBuilder::buff_act_info_update(
                uid,
                buff_uid,
                sonettobuf::BuffActInfo {
                    act_id: Some(1042),
                    param: vec![gain, max_capacity],
                    ..Default::default()
                },
            ));
            effects.push(ActEffectBuilder::max_hp_change(uid, new_max_hp, Some(1042)));
        }

        effects
    }
}

impl ShadowCloakState {
    pub fn sync_step(
        &mut self,
        fight: &Fight,
        buff_mgr: &BuffMgr,
        entity_mgr: &EntityMgr,
    ) -> Option<FightStep> {
        if !self.is_active() {
            return None;
        }
        let effects = self.build_sync_effects(fight, buff_mgr, entity_mgr);
        self.reset_tick();
        if effects.is_empty() {
            return None;
        }
        Some(FightStepBuilder::effect().with_many(effects).build())
    }
}
