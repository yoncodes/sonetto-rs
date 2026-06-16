use crate::state::battle::effect::condition::Hook;
use crate::state::battle::event::Event;
use crate::state::battle::manager::{fight_data_mgr::Managers, traits::Manager};
use crate::state::battle::mechanics::Mechanics;
use super::hook_call;
use crate::state::battle::skill::{PhaseFilter, TriggerState};

use rand::rngs::StdRng;
use sonettobuf::Fight;
use std::{collections::HashSet, ptr::NonNull};

#[derive(Copy, Clone)]
struct RngPtr(NonNull<StdRng>);

// SAFETY: this is a non-owning pointer to the round-local RNG. Access still requires
// `&mut FightContext`, so callers maintain exclusivity when dereferencing it.
unsafe impl Send for RngPtr {}

pub struct FightContext<'a> {
    pub fight: &'a mut Fight,
    pub managers: &'a mut Managers,
    pub mechanics: &'a mut Mechanics,
    pub active_card_cast_uids: HashSet<i64>,
    rng: Option<RngPtr>,
}

impl<'a> FightContext<'a> {
    pub fn new(
        fight: &'a mut Fight,
        managers: &'a mut Managers,
        mechanics: &'a mut Mechanics,
    ) -> Self {
        Self {
            fight,
            managers,
            mechanics,
            active_card_cast_uids: HashSet::new(),
            rng: None,
        }
    }

    pub fn with_rng(mut self, rng: &mut StdRng) -> Self {
        self.rng = Some(RngPtr(NonNull::from(rng)));
        self
    }

    pub fn rng_ptr(&self) -> Option<NonNull<StdRng>> {
        self.rng.map(|ptr| ptr.0)
    }

    pub fn sync(&mut self) {
        self.managers.entity_mgr.rebuild_cache(self.fight);
        self.managers.calculate_mgr.update_cache(self.fight);
    }

    pub fn clear_round_active_card_casts(&mut self) {
        self.active_card_cast_uids.clear();
    }

    pub fn mark_round_active_card_cast(&mut self, uid: i64) {
        if uid != 0 {
            self.active_card_cast_uids.insert(uid);
        }
    }

    pub fn set_round_active_card_cast_uids(&mut self, uids: &HashSet<i64>) {
        self.active_card_cast_uids = uids.clone();
    }

    pub fn combat_trigger_state(&self) -> TriggerState {
        TriggerState::default().with_round_active_card_cast_uids(&self.active_card_cast_uids)
    }

    pub fn combat_phase(&self) -> PhaseFilter {
        PhaseFilter::combat_with(self.combat_trigger_state())
    }

    pub fn active_use_trigger_state(&self, skill_id: i32) -> TriggerState {
        TriggerState::on_active_use_skill(skill_id)
            .with_round_active_card_cast_uids(&self.active_card_cast_uids)
    }

    pub fn on_round_end(&mut self, entity_uid: i64) -> Vec<Event> {
        self.managers.entity_mgr.on_round_end(self.fight);
        self.managers.buff_mgr.on_round_end(self.fight);
        self.managers.calculate_mgr.on_round_end();
        self.managers.cloth_mgr.on_round_end(self.fight);
        self.clear_round_active_card_casts();
        self.sync();
        hook_call::fire_hook(self.managers, self.fight, Hook::RoundEnd, entity_uid)
    }

    pub fn on_round_start(&mut self, entity_uid: i64) -> Vec<Event> {
        tracing::info!(entity_uid, "hook: on_round_start");
        hook_call::fire_hook(self.managers, self.fight, Hook::RoundStart, entity_uid)
    }

    pub fn on_battle_start(&mut self, entity_uid: i64) -> Vec<Event> {
        tracing::info!(entity_uid, "hook: on_battle_start");
        self.managers.cloth_mgr.on_battle_start(self.fight);
        hook_call::fire_hook(self.managers, self.fight, Hook::BattleStart, entity_uid)
    }

    pub fn on_use_card(&mut self, event: &Event) -> Vec<Event> {
        let Event::CardPlayed { card, .. } = event else { return vec![]; };
        let uid = card.uid.unwrap_or(0);
        let mut events = self.managers.cloth_mgr.on_use_card(self.fight,
            vec![Event::ExPointChange { target: uid, delta: 1, emit_step: false }]);
        events.extend(hook_call::fire_hook(self.managers, self.fight, Hook::UseCard, uid));
        events
    }

    pub fn on_move_card(&mut self, event: &Event) -> Vec<Event> {
        let Event::CardMoved { card } = event else { return vec![]; };
        let uid = card.uid.unwrap_or(0);
        let mut events = self.managers.cloth_mgr.on_move_card(self.fight,
            vec![Event::ExPointChange { target: uid, delta: 1, emit_step: false }]);
        events.extend(hook_call::fire_hook(self.managers, self.fight, Hook::MoveCard, uid));
        events
    }

    pub fn on_card_upgrade(&mut self, event: &Event) -> Vec<Event> {
        let Event::CardUpgrade { card } = event else { return vec![]; };
        let uid = card.uid.unwrap_or(0);
        let mut events = self.managers.cloth_mgr.on_card_upgrade(self.fight,
            vec![Event::ExPointChange { target: uid, delta: 1, emit_step: false }]);
        events.extend(hook_call::fire_hook(self.managers, self.fight, Hook::CardUpgrade, uid));
        events
    }

    pub fn on_enter_fight(&mut self, entity_uid: i64) -> Vec<Event> {
        tracing::info!(entity_uid, "hook: on_enter_fight");
        let mut events = self.managers.cloth_mgr.on_enter_fight(self.fight, entity_uid);
        events.extend(hook_call::fire_hook(self.managers, self.fight, Hook::EnterFight, entity_uid));
        events
    }

    pub fn on_dead(&mut self, entity_uid: i64) -> Vec<Event> {
        tracing::info!(entity_uid, "hook: on_dead");
        let mut events = vec![
            Event::Dead { entity_uid },
            Event::RemoveEntityCards { entity_uid },
        ];
        events.extend(self.managers.cloth_mgr.on_dead(self.fight, entity_uid));
        events.extend(hook_call::fire_hook(self.managers, self.fight, Hook::Dead, entity_uid));
        self.managers.entity_mgr.action_points.remove(&entity_uid);
        events
    }

    pub fn on_buff_add(&mut self, target_uid: i64) -> Vec<Event> {
        hook_call::fire_hook(self.managers, self.fight, Hook::BuffAdd, target_uid)
    }

    pub fn on_eval_active_skill(&mut self, caster_uid: i64) -> Vec<Event> {
        tracing::info!(caster_uid, "hook: on_eval_active_skill");
        let attr_map = hook_call::on_eval_active_skill_attr_fix(self.managers, self.fight, caster_uid);
        self.managers.entity_mgr.merge_attr_bonus(attr_map);
        hook_call::on_eval_active_skill(self.managers, self.fight, caster_uid)
    }

    pub fn on_use_ex_skill(&mut self, caster_uid: i64) -> Vec<Event> {
        tracing::info!(caster_uid, "hook: on_use_ex_skill");
        hook_call::on_use_ex_skill(self.managers, self.fight, caster_uid)
    }

    pub fn on_eval_being_attacked(&mut self, defender_uid: i64) -> Vec<Event> {
        tracing::info!(defender_uid, "hook: on_eval_being_attacked");
        let attr_map = hook_call::on_eval_being_attacked_attr_fix(self.managers, self.fight, defender_uid);
        self.managers.entity_mgr.merge_attr_bonus(attr_map);
        hook_call::on_eval_being_attacked(self.managers, self.fight, defender_uid)
    }

    pub fn on_after_action(&mut self, caster_uid: i64) -> Vec<Event> {
        tracing::info!(caster_uid, "hook: on_after_action");
        hook_call::on_after_action(self.managers, self.fight, caster_uid)
    }
}
