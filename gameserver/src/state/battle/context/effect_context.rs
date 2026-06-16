use sonettobuf::{Fight, FightEntityInfo};

use crate::state::battle::{
    manager::{buff_mgr::BuffMgr, fight_data_mgr::Managers},
    mechanics::Mechanics,
    utils::find_entity,
};

/// Combat state passed into all buff_actions handlers.
/// Access everything through managers and mechanics - no manual threading of individual fields.
pub struct EffectContext<'a> {
    pub fight: &'a Fight,
    pub managers: &'a mut Managers,
    pub mechanics: &'a mut Mechanics,
    pub caster_uid: i64,
    pub target: i64,
}

impl EffectContext<'_> {
    #[inline]
    pub fn new<'a>(
        fight: &'a Fight,
        managers: &'a mut Managers,
        mechanics: &'a mut Mechanics,
        caster_uid: i64,
        target: i64,
    ) -> EffectContext<'a> {
        EffectContext {
            fight,
            managers,
            mechanics,
            caster_uid,
            target,
        }
    }

    #[inline]
    pub fn fight(&self) -> &Fight {
        self.fight
    }

    #[inline]
    pub fn caster_uid(&self) -> i64 {
        self.caster_uid
    }

    #[inline]
    pub fn target_uid(&self) -> i64 {
        self.target
    }

    #[inline]
    pub fn get_entity(&self, uid: i64) -> Option<&FightEntityInfo> {
        find_entity(self.fight, uid)
    }

    #[inline]
    pub fn caster_entity(&self) -> Option<&FightEntityInfo> {
        self.get_entity(self.caster_uid)
    }

    #[inline]
    pub fn target_entity(&self) -> Option<&FightEntityInfo> {
        self.get_entity(self.target)
    }

    #[inline]
    pub fn get_hp(&self, uid: i64) -> i32 {
        self.managers.entity_mgr.get_hp(uid)
    }

    #[inline]
    pub fn get_max_hp(&self, uid: i64) -> i32 {
        let tracked = self.managers.entity_mgr.get_max_hp(uid);
        if tracked > 0 {
            return tracked;
        }
        self.get_entity(uid)
            .and_then(|e| e.attr.as_ref().and_then(|a| a.hp))
            .unwrap_or(0)
    }

    #[inline]
    pub fn target_hp(&self) -> i32 {
        self.get_hp(self.target)
    }

    #[inline]
    pub fn target_max_hp(&self) -> i32 {
        self.get_max_hp(self.target)
    }

    #[inline]
    pub fn buff_mgr(&self) -> &BuffMgr {
        &self.managers.buff_mgr
    }

    #[inline]
    pub fn buff_mgr_mut(&mut self) -> &mut BuffMgr {
        &mut self.managers.buff_mgr
    }

    #[inline]
    pub fn entity_mgr(&self) -> &crate::state::battle::manager::entity_mgr::EntityMgr {
        &self.managers.entity_mgr
    }
}
