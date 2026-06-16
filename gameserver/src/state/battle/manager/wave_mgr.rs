//! Wave-spawn manager. Owns wave-state mutation and exposes
//! advance_wave plus replay-mode fast-forward helpers.
//!
//! In replay mode, captured boss steps may reference entity uids
//! from wave N+1 before our state has spawned them (because LIVE
//! kills wave N faster than our damage simulation does). The
//! expected_wave_for_uid + fast_forward_to_wave pair lets
//! card_mgr::execute_ai_turn lazy-spawn future waves on demand.

use anyhow::Result;
use sonettobuf::{Fight, FightStep};

use crate::state::battle::{
    buff_actions::{EffectContext, apply_after_buff_add_features},
    context::FightContext,
    fight::defender::Defender,
    fight_step::{ActEffectBuilder, FightStepBuilder},
    manager::{
        buff_mgr::observe_explicit_buff_uid_for_target,
        entity_mgr::{sync_from_fight, seed_ex_point_required_from_fight},
        round_mgr::seed_entry_max_hp_from_fight,
    },
    skill::SkillExecutor,
};

#[derive(Debug, Clone, Default)]
pub struct WaveMgr {}

impl WaveMgr {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn max_wave_for_fight(fight: &Fight) -> i32 {
        let episode_id = fight.episode_id.unwrap_or(0);
        let configs = config::configs::get();

        let battle_id = configs
            .episode
            .iter()
            .find(|episode| episode.id == episode_id)
            .map(|episode| episode.battle_id)
            .unwrap_or(0);

        configs
            .battle
            .iter()
            .find(|battle| battle.id == battle_id)
            .map(|battle| {
                if battle.monster_group_ids.is_empty() {
                    1
                } else {
                    battle.monster_group_ids.split('#').count() as i32
                }
            })
            .unwrap_or(1)
    }

    /// UID assignment formula: uid = -((2 * (wave - 1)) + position),
    /// position in {1, 2}. So abs(uid)={1,2} -> wave 1; {3,4} -> wave 2; etc.
    /// Returns None for non-defender uids (uid >= 0).
    pub fn expected_wave_for_uid(uid: i64) -> Option<i32> {
        if uid >= 0 {
            return None;
        }
        let abs = uid.unsigned_abs() as i32;
        Some((abs + 1) / 2)
    }

    /// Advance to the next wave. Existing behavior, moved from the
    /// free-function advance_wave.
    pub fn advance_wave(
        &mut self,
        ctx: &mut FightContext<'_>,
        executor: &mut SkillExecutor,
    ) -> Result<Vec<FightStep>> {
        self.advance_wave_state(ctx)?;

        let fight = ctx.fight.clone();
        let new_entity_uids: Vec<i64> = fight
            .defender
            .as_ref()
            .into_iter()
            .flat_map(|d| d.entitys.iter())
            .filter_map(|e| e.uid)
            .collect();

        let mut steps = vec![
            FightStepBuilder::effect()
                .with(ActEffectBuilder::new_change_wave(fight.clone()))
                .build(),
        ];

        for uid in new_entity_uids {
            let events = ctx.on_enter_fight(uid);
        }

        if let Some(step) = build_active_circle_enemy_buff_step(&fight, ctx, executor) {
            steps.push(step);
        }

        Ok(steps)
    }

    /// Real-state replay alignment: advance waves without serializing
    /// preview-only wave steps or applying the magic-circle enemy-buff
    /// follow-up that belongs to the preview execution path.
    pub fn fast_forward_state_to_wave(
        &mut self,
        ctx: &mut FightContext<'_>,
        target_wave: i32,
    ) -> Result<()> {
        loop {
            let current = ctx.fight.cur_wave.unwrap_or(1);
            if current >= target_wave {
                break;
            }
            let old_defender_uids: Vec<i64> = ctx
                .fight
                .defender
                .as_ref()
                .into_iter()
                .flat_map(|defender| defender.entitys.iter().chain(defender.sub_entitys.iter()))
                .filter_map(|entity| entity.uid)
                .collect();
            self.advance_wave_state(ctx)?;
            sync_from_fight(ctx.fight, &mut ctx.managers.entity_mgr);
            seed_ex_point_required_from_fight(ctx.fight, &mut ctx.managers.entity_mgr);
            for uid in old_defender_uids {
                ctx.managers.buff_mgr.clear(uid);
            }
            seed_entry_max_hp_from_fight(ctx.fight);
            ctx.sync();
        }
        Ok(())
    }

    /// Replay-mode fast-forward: advance waves until current_wave >= target.
    /// Concatenates wave-spawn steps from each advance.
    pub fn fast_forward_to_wave(
        &mut self,
        ctx: &mut FightContext<'_>,
        executor: &mut SkillExecutor,
        target_wave: i32,
    ) -> Result<Vec<FightStep>> {
        let mut steps = Vec::new();
        loop {
            let current = ctx.fight.cur_wave.unwrap_or(1);
            if current >= target_wave {
                break;
            }
            let old_defender_uids: Vec<i64> = ctx
                .fight
                .defender
                .as_ref()
                .into_iter()
                .flat_map(|defender| defender.entitys.iter().chain(defender.sub_entitys.iter()))
                .filter_map(|entity| entity.uid)
                .collect();
            steps.extend(self.advance_wave(ctx, executor)?);
            sync_from_fight(ctx.fight, &mut ctx.managers.entity_mgr);
            seed_ex_point_required_from_fight(ctx.fight, &mut ctx.managers.entity_mgr);
            for uid in old_defender_uids {
                ctx.managers.buff_mgr.clear(uid);
            }
            seed_entry_max_hp_from_fight(ctx.fight);
            ctx.sync();
        }
        Ok(steps)
    }

    fn advance_wave_state(&mut self, ctx: &mut FightContext<'_>) -> Result<()> {
        let current_wave = ctx.fight.cur_wave.unwrap_or(1);
        let new_wave = current_wave + 1;
        let battle_id = ctx.fight.battle_id.unwrap_or(0);
        let new_entities = Defender::build_wave_entities(battle_id, new_wave, 2)?;

        let defender = ctx
            .fight
            .defender
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Fight missing defender team"))?;
        defender.entitys = new_entities;
        defender.sub_entitys.clear();

        ctx.fight.cur_wave = Some(new_wave);
        ctx.fight.is_finish = Some(false);

        // Phase-shift hook: for each freshly-spawned entity, ask
        // `phase_change::determine_spawn_form` whether the monster
        // should transform based on accumulated Will buffs. If so,
        // apply the form swap immediately so passive triggers fire
        // against the post-transform state.
        let will_buff_ids = ctx.mechanics.phase_change.pending_will_buff_ids();
        let spawn_targets: Vec<(i64, i32)> = ctx
            .fight
            .defender
            .as_ref()
            .into_iter()
            .flat_map(|d| d.entitys.iter())
            .filter_map(|e| {
                let uid = e.uid?;
                let mid = e.model_id?;
                let new_form = crate::state::battle::mechanics::phase_change::determine_spawn_form(
                    mid,
                    &will_buff_ids,
                )?;
                Some((uid, new_form))
            })
            .collect();
        for (uid, new_form) in spawn_targets {
            crate::state::battle::mechanics::phase_change::transform_entity(
                ctx.fight, uid, new_form,
            )?;
        }

        let new_entities: Vec<_> = ctx
            .fight
            .defender
            .as_ref()
            .into_iter()
            .flat_map(|d| d.entitys.iter())
            .collect();
        for entity in new_entities {
            ctx.managers.passive_mgr.seed_entity(entity);
            if let Some(uid) = entity.uid {
                ctx.managers.rule_mgr.seed_entity_uid(uid, ctx.fight);
            }
        }

        Ok(())
    }
}

fn build_active_circle_enemy_buff_step(
    fight: &Fight,
    ctx: &mut FightContext<'_>,
    executor: &mut SkillExecutor,
) -> Option<FightStep> {
    let circle = fight.magic_circle.as_ref()?;
    let circle_id = circle.magic_circle_id.unwrap_or(0);
    let circle_round = circle.round.unwrap_or(0);
    if circle_id == 0 || circle_round <= 0 {
        return None;
    }

    let circle_cfg = config::configs::get().magic_circle.get(circle_id)?;
    let buff_id = circle_cfg.enemy_buff.trim().parse::<i32>().ok()?.max(0);
    if buff_id == 0 {
        return None;
    }

    let caster_uid = circle.create_uid.unwrap_or(0);
    if caster_uid == 0 {
        return None;
    }

    let target_uids: Vec<i64> = fight
        .defender
        .as_ref()
        .into_iter()
        .flat_map(|defender| defender.entitys.iter())
        .filter(|entity| entity.current_hp.unwrap_or(0) > 0)
        .filter_map(|entity| entity.uid)
        .filter(|uid| *uid != 0)
        .collect();

    if target_uids.is_empty() {
        return None;
    }

    let mut effects = Vec::new();
    for target_uid in target_uids {
        let mut effect_ctx =
            EffectContext::new(fight, ctx.managers, ctx.mechanics, caster_uid, target_uid);
        let effect = crate::state::battle::fight_step::ActEffectBuilder::buff_add(
            target_uid, caster_uid, buff_id, 1,
        );
        if let Some(buff_uid) = effect.buff.as_ref().and_then(|buff| buff.uid) {
            observe_explicit_buff_uid_for_target(target_uid, buff_uid);
            effect_ctx
                .buff_mgr_mut()
                .add_with_uid(target_uid, buff_id, caster_uid, 0, 0, 1, buff_uid);
        }
        effects.push(effect);
        effects.extend(apply_after_buff_add_features(
            &mut effect_ctx,
            executor,
            buff_id,
            false,
        ));
        effects.extend(executor.side_effects.drain(..));
    }

    if effects.is_empty() {
        None
    } else {
        Some(FightStepBuilder::effect().with_many(effects).build())
    }
}
