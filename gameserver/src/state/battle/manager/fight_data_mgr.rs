use super::super::{
    context::{FightContext, RoundContext},
    deck::DeckManager,
    event_queue::{BattleEvent, EventContext, EventQueue, drain_to_fight_steps},
    manager::{
        buff_mgr::{BuffMgr, observe_explicit_buff_uid_for_target},
        calculate_mgr::FightCalculateDataMgr,
        cloth_mgr::ClothMgr,
        entity_mgr::EntityMgr,
        passive_mgr::PassiveMgr,
        active_effect_mgr::ActiveEffectMgr,
        rule_mgr::RuleMgr,
        wave_mgr::WaveMgr,
    },
    mechanics::Mechanics,
    round::processor,
    round_state::set_simulated_round,
    skill::SkillExecutor,
};
use super::round_mgr::seed_entry_max_hp_from_fight;

use anyhow::Result;
use rand::{SeedableRng, rngs::StdRng};
use sonettobuf::{BeginRoundOper, BuffInfo, CardInfoPush, Fight, FightExPointInfo, FightRound, FightStep};

use crate::state::battle::{
    buff_actions::{blood_pool_ex::seed_blood_pool_ex_tracker, raspberry::BUFF_ACT_ID_RASPBERRY},
    heroes::rubuska,
    mechanics::bloodtithe::BloodtitheState,
    types::effects::EffectType,
};

#[derive(Debug, Clone, Default)]
pub struct Managers {
    pub entity_mgr: EntityMgr,
    pub calculate_mgr: FightCalculateDataMgr,
    pub buff_mgr: BuffMgr,
    pub wave_mgr: WaveMgr,
    pub deck_mgr: DeckManager,
    pub cloth_mgr: ClothMgr,
    pub rule_mgr: RuleMgr,
    pub passive_mgr: PassiveMgr,
    pub active_effect_mgr: ActiveEffectMgr,
}

impl Managers {
    pub fn new(fight: &Fight) -> Self {
        tracing::info!("FightDataMgr::new battle_id={:?}", fight.battle_id);
        Self {
            entity_mgr: EntityMgr::new(fight),
            calculate_mgr: FightCalculateDataMgr::new(fight),
            buff_mgr: BuffMgr::new(),
            wave_mgr: WaveMgr::new(),
            deck_mgr: DeckManager::default(),
            cloth_mgr: ClothMgr::default(),
            rule_mgr: RuleMgr::new(fight),
            passive_mgr: PassiveMgr::new(fight),
            active_effect_mgr: ActiveEffectMgr::default(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct FightDataMgr {
    pub(crate) fight: Fight,
    pub pre_fight: Option<Fight>,
    mechanics: Mechanics,
    pub managers: Managers,
    pub last_round: Option<FightRound>,
    pub initial_card_push: Option<CardInfoPush>,
}

impl FightDataMgr {
    pub fn new(fight: Fight, max_ap: i32) -> Self {
        tracing::info!("FightDataMgr::new battle_id={:?}", fight.battle_id);
        seed_entry_max_hp_from_fight(&fight);
        let mechanics = Mechanics::new();
        let pre_fight = Some(fight.clone());
        let mut fight_mgr = Self {
            managers: Managers::new(&fight),
            pre_fight,
            fight,
            mechanics,
            last_round: None,
            initial_card_push: None,
        };
        fight_mgr.managers.entity_mgr.init(&fight_mgr.fight);
        let card_push = fight_mgr.managers.deck_mgr.init_player(&fight_mgr.fight, max_ap);
        fight_mgr.managers.deck_mgr.init_enemy(&fight_mgr.fight);
        fight_mgr.initial_card_push = Some(card_push);
        fight_mgr
    }

    pub async fn process_round(
        &mut self,
        operations: Vec<BeginRoundOper>,
        ai_override_steps: Option<Vec<FightStep>>,
    ) -> Result<FightRound> {
        let round_index = self.fight.cur_round.unwrap_or(1);
        set_simulated_round(round_index);
        let mut rng = rand::rngs::StdRng::seed_from_u64(round_index as u64);
        let mut skill_executor = SkillExecutor::new();
        let mut fight_ctx = self.ctx_with_rng(&mut rng);
        let mut round_ctx = RoundContext::new(&mut fight_ctx, round_index);
        processor::process_round(
            &mut rng,
            &mut round_ctx,
            &mut skill_executor,
            operations,
            ai_override_steps,
            None,
            None,
            None,
        )
        .await
    }

    #[inline]
    pub fn fight(&self) -> &Fight {
        &self.fight
    }

    #[inline]
    pub fn get_fight(&self) -> &Fight {
        &self.fight
    }

    // Replay-bootstrap surface consumed by battle_gen; clippy can't see the
    // cross-crate callers so silence the per-method dead_code lint here.
    #[allow(dead_code)]
    #[inline]
    pub fn fight_mut(&mut self) -> &mut Fight {
        &mut self.fight
    }

    pub fn execute_cloth_skill(&mut self, skill_id: i32, rng: &mut StdRng) -> anyhow::Result<sonettobuf::FightRound> {
        let steps = self.managers.cloth_mgr.execute_skill(skill_id, &mut self.fight, &mut self.managers.deck_mgr, rng)?;
        let mut round = self.last_round.clone().unwrap_or_default();
        round.fight_step = steps;
        round.power = self.fight.attacker.as_ref().and_then(|a| a.power);
        round.team_a_cards1 = self.managers.deck_mgr.player_hand.clone();
        Ok(round)
    }

    pub fn ctx(&mut self) -> FightContext<'_> {
        FightContext::new(&mut self.fight, &mut self.managers, &mut self.mechanics)
    }

    pub fn check_battle_result(&self) -> i32 {
        let enemies_alive = self.fight.defender.as_ref()
            .map(|d| d.entitys.iter().any(|e| e.current_hp.unwrap_or(0) > 0))
            .unwrap_or(false);
        let heroes_alive = self.fight.attacker.as_ref()
            .map(|a| a.entitys.iter().any(|e| e.current_hp.unwrap_or(0) > 0))
            .unwrap_or(false);
        if !heroes_alive { 0 } else if !enemies_alive { 1 } else { 1 }
    }

    pub fn ctx_with_rng<'a>(&'a mut self, rng: &mut StdRng) -> FightContext<'a> {
        self.ctx().with_rng(rng)
    }

    #[allow(dead_code)]
    pub fn seed_replay_state(
        &mut self,
        initial_round: &FightRound,
        ex_point_info: &[FightExPointInfo],
    ) -> Result<()> {
        self.managers.entity_mgr.init(&self.fight);
        self.mechanics.init(&self.fight);

        for step in &initial_round.fight_step {
            self.managers
                .calculate_mgr
                .play_step_data(
                    step,
                    &mut self.fight,
                    &mut self.mechanics.bloodtithe,
                    &mut self.managers.buff_mgr,
                    &mut self.managers.entity_mgr,
                )
                .map_err(anyhow::Error::msg)?;
        }

        if !ex_point_info.is_empty() {
            let by_uid: std::collections::HashMap<i64, &FightExPointInfo> = ex_point_info
                .iter()
                .filter_map(|info| info.uid.map(|uid| (uid, info)))
                .collect();

            for side in [&mut self.fight.attacker, &mut self.fight.defender]
                .into_iter()
                .flatten()
            {
                for entity in side.entitys.iter_mut().chain(side.sub_entitys.iter_mut()) {
                    let uid = entity.uid.unwrap_or(0);
                    let Some(info) = by_uid.get(&uid) else {
                        continue;
                    };

                    let ex_point = info.ex_point.unwrap_or(0);
                    entity.ex_point = Some(ex_point);
                    self.managers.entity_mgr.set_ex_point(uid, ex_point);

                    if let Some(current_hp) = info.current_hp {
                        entity.current_hp = Some(current_hp);
                        self.managers.entity_mgr.set_hp(uid, current_hp);
                    }
                }
            }
        }

        self.managers.entity_mgr.rebuild_cache(&self.fight);
        self.managers.calculate_mgr.update_cache(&self.fight);
        // Replay seeding mutates max HP during initial-round step playback.
        // Refresh mechanics after that replay so Rubuska Shadow Cloak reads
        // the effective post-bootstrap baseline rather than the raw fight payload.
        self.mechanics.init(&self.fight);
        // Some buff-act state (e.g. Kakania's Empathy `actCommonParams`)
        // only lives in `BuffMgr` after the initial-round replay; the
        // entity buff snapshot in `fight` doesn't carry it forward. Pull
        // it into the mechanics caches so cumulative totals like the
        // Empathy injury bank don't regress between rounds.
        self.mechanics.sync_from_buff_mgr(&self.managers.buff_mgr);
        self.reseed_bloodtithe_from_round(initial_round);
        self.reseed_shadow_cloak_from_round(initial_round);
        seed_blood_pool_ex_tracker(
            &self.mechanics.bloodtithe,
            &self.fight,
            &self.managers.buff_mgr,
        );
        Ok(())
    }

    #[allow(dead_code)]
    pub fn seed_replay_bloodtithe_from_effects(&mut self, effects: &[(i32, i32, i32)]) {
        let mut rebuilt = BloodtitheState::new();
        for &(effect_type, team_type, amount) in effects {
            match EffectType::from(effect_type) {
                EffectType::BloodPoolMaxCreate => {
                    rebuilt.initialized = true;
                }
                EffectType::BloodPoolMaxChange => {
                    rebuilt.initialized = true;
                    let current_max = rebuilt.get_max(team_type);
                    if amount > current_max {
                        let mut queue = EventQueue::new();
                        queue.push(BattleEvent::BloodpoolMaxChange {
                            team_type,
                            max: current_max.max(amount),
                        });
                        let mut local_fight = Fight::default();
                        let mut local_buff_mgr = BuffMgr::new();
                        let mut local_entity_mgr = EntityMgr::default();
                        let mut event_ctx = EventContext {
                            fight: &mut local_fight,
                            buff_mgr: &mut local_buff_mgr,
                            entity_mgr: &mut local_entity_mgr,
                            bloodtithe: &mut rebuilt,
                        };
                        let _ = drain_to_fight_steps(queue.drain(), &mut event_ctx);
                    }
                }
                EffectType::BloodPoolValueChange => {
                    rebuilt.initialized = true;
                    let current = rebuilt.get_value(team_type);
                    rebuilt.set_value(team_type, (current + amount).max(0));
                }
                _ => {}
            }
        }
        if rebuilt.initialized {
            self.mechanics.bloodtithe = rebuilt;
            seed_blood_pool_ex_tracker(
                &self.mechanics.bloodtithe,
                &self.fight,
                &self.managers.buff_mgr,
            );
        }
    }

    #[allow(dead_code)]
    pub fn seed_replay_buffs_from_effects(
        &mut self,
        effects: &[(i32, i64, i64, i32, i32, i64, i32)],
    ) {
        if effects.is_empty() {
            return;
        }

        for &(buff_id, target_uid, from_uid, count, layer, buff_uid, duration) in effects {
            if buff_id <= 0 || target_uid == 0 || buff_uid <= 0 {
                continue;
            }

            observe_explicit_buff_uid_for_target(target_uid, buff_uid);
            self.managers
                .buff_mgr
                .add_with_uid(target_uid, buff_id, from_uid, 0, count, layer, buff_uid);
            let _ = self
                .managers
                .buff_mgr
                .set_instance_duration(target_uid, buff_uid, duration);

            let mut seeded = false;
            for side in [&mut self.fight.attacker, &mut self.fight.defender]
                .into_iter()
                .flatten()
            {
                for entity in side.entitys.iter_mut().chain(side.sub_entitys.iter_mut()) {
                    if entity.uid != Some(target_uid) {
                        continue;
                    }

                    if let Some(existing) =
                        entity.buffs.iter_mut().find(|b| b.uid == Some(buff_uid))
                    {
                        existing.buff_id = Some(buff_id);
                        existing.from_uid = Some(from_uid);
                        existing.count = Some(count);
                        existing.layer = Some(layer);
                        existing.duration = Some(duration);
                    } else {
                        entity.buffs.push(BuffInfo {
                            uid: Some(buff_uid),
                            buff_id: Some(buff_id),
                            from_uid: Some(from_uid),
                            count: Some(count),
                            layer: Some(layer),
                            duration: Some(duration),
                            ..Default::default()
                        });
                    }
                    seeded = true;
                    break;
                }
                if seeded {
                    break;
                }
            }
        }
    }

    #[allow(dead_code)]
    fn reseed_bloodtithe_from_round(&mut self, round: &FightRound) {
        #[derive(Clone, Copy)]
        struct Frame<'a> {
            step: &'a FightStep,
            next_effect: usize,
        }

        let mut rebuilt = BloodtitheState::new();
        let mut stack: Vec<Frame<'_>> = round
            .fight_step
            .iter()
            .rev()
            .map(|step| Frame {
                step,
                next_effect: 0,
            })
            .collect();

        while let Some(frame) = stack.last_mut() {
            if frame.next_effect >= frame.step.act_effect.len() {
                stack.pop();
                continue;
            }

            let effect = &frame.step.act_effect[frame.next_effect];
            frame.next_effect += 1;

            if let Some(nested) = effect.fight_step.as_ref() {
                stack.push(Frame {
                    step: nested,
                    next_effect: 0,
                });
                continue;
            }

            let team_type = effect.effect_num.or(effect.team_type).unwrap_or(1);
            match EffectType::from(effect.effect_type.unwrap_or(0)) {
                EffectType::BloodPoolMaxCreate => {
                    rebuilt.initialized = true;
                }
                EffectType::BloodPoolMaxChange => {
                    rebuilt.initialized = true;
                    let next_max = effect.effect_num1.unwrap_or(0);
                    let current_max = rebuilt.get_max(team_type);
                    if next_max > current_max {
                        let mut queue = EventQueue::new();
                        queue.push(BattleEvent::BloodpoolMaxChange {
                            team_type,
                            max: current_max.max(next_max),
                        });
                        let mut local_fight = Fight::default();
                        let mut local_buff_mgr = BuffMgr::new();
                        let mut local_entity_mgr = EntityMgr::default();
                        let mut event_ctx = EventContext {
                            fight: &mut local_fight,
                            buff_mgr: &mut local_buff_mgr,
                            entity_mgr: &mut local_entity_mgr,
                            bloodtithe: &mut rebuilt,
                        };
                        let _ = drain_to_fight_steps(queue.drain(), &mut event_ctx);
                    }
                }
                EffectType::BloodPoolValueChange => {
                    rebuilt.initialized = true;
                    let delta = effect.effect_num1.unwrap_or(0);
                    let current = rebuilt.get_value(team_type);
                    rebuilt.set_value(team_type, (current + delta).max(0));
                }
                _ => {}
            }
        }

        if rebuilt.initialized {
            self.mechanics.bloodtithe = rebuilt;
        }
    }

    #[allow(dead_code)]
    fn reseed_shadow_cloak_from_round(&mut self, round: &FightRound) {
        #[derive(Clone, Copy)]
        struct Frame<'a> {
            step: &'a FightStep,
            next_effect: usize,
        }

        let mut seeded_max = 0;
        let mut stack: Vec<Frame<'_>> = round
            .fight_step
            .iter()
            .rev()
            .map(|step| Frame {
                step,
                next_effect: 0,
            })
            .collect();

        while let Some(frame) = stack.last_mut() {
            if frame.next_effect >= frame.step.act_effect.len() {
                stack.pop();
                continue;
            }

            let effect = &frame.step.act_effect[frame.next_effect];
            frame.next_effect += 1;

            if let Some(nested) = effect.fight_step.as_ref() {
                stack.push(Frame {
                    step: nested,
                    next_effect: 0,
                });
                continue;
            }

            if EffectType::from(effect.effect_type.unwrap_or(0)) != EffectType::BuffActInfoUpdate {
                continue;
            }
            let Some(info) = effect.buff_act_info.as_ref() else {
                continue;
            };
            if info.act_id != Some(BUFF_ACT_ID_RASPBERRY) {
                continue;
            }
            let Some(max_cap) = info.param.get(1).copied() else {
                continue;
            };
            if max_cap > seeded_max {
                seeded_max = max_cap;
            }
        }

        if seeded_max > 0 {
            rubuska::seed_replay_raspberry_max(&self.fight, seeded_max);
            rubuska::apply_seeded_shadow_cloak_capacity(
                &mut self.mechanics.shadow_cloak,
                seeded_max,
            );
        }
    }
}
