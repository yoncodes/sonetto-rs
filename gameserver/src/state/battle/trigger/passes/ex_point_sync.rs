use sonettobuf::{ActEffect, FightStep, FightStep as ProtoFightStep, fight_step};

use crate::state::battle::{
    context::FightContext,
    event_queue::{BattleEvent, EventContext, EventQueue, drain_to_fight_steps},
    fight_step::{ActEffectBuilder, FightStepBuilder},
    manager::buff_mgr::observe_explicit_buff_uid_for_target,
    passives::collector::CollectedPassives,
    trigger::combat::TriggerEvent,
};

use super::TriggerPass;

pub struct ExPointSyncPass;

impl TriggerPass for ExPointSyncPass {
    fn run(
        &self,
        ctx: &mut FightContext<'_>,
        event: &TriggerEvent,
        _collected: &CollectedPassives,
    ) -> Vec<FightStep> {
        let cfg = config::configs::get();
        let mut out = Vec::new();

        for target_uid in &event.damaged_uids {
            let buffs = ctx.managers.buff_mgr.get(*target_uid).to_vec();
            for buff in buffs {
                if event.added_buff_uids.contains(&buff.uid) {
                    continue;
                }
                let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == buff.buff_id) else {
                    continue;
                };

                let mut ex_gain = 0_i32;
                let mut has_ex_on_hit = false;
                for entry in buff_cfg.features.split('|') {
                    let parts: Vec<&str> = entry.split('#').collect();
                    let Some(act_id) = parts.first().and_then(|v| v.trim().parse::<i32>().ok())
                    else {
                        continue;
                    };
                    let is_ex_on_hit = cfg
                        .buff_act
                        .iter()
                        .find(|a| a.id == act_id)
                        .map(|a| a.r#type == "ExPointAddByHit")
                        .unwrap_or(false);
                    if !is_ex_on_hit {
                        continue;
                    }
                    has_ex_on_hit = true;
                    ex_gain += parts
                        .get(1)
                        .and_then(|v| v.trim().parse::<i32>().ok())
                        .unwrap_or(0);
                }

                if !has_ex_on_hit {
                    continue;
                }

                let mut act_effect = Vec::<ActEffect>::new();

                if buff.layer > 1 {
                    let new_layer = buff.layer - 1;
                    observe_explicit_buff_uid_for_target(*target_uid, buff.uid);
                    ctx.managers.buff_mgr.add_with_uid(
                        *target_uid,
                        buff.buff_id,
                        buff.from_uid,
                        buff.from_skill_id,
                        buff.stacks,
                        new_layer,
                        buff.uid,
                    );
                    act_effect.push(
                        crate::state::battle::fight_step::ActEffectBuilder::buff_update(
                            *target_uid,
                            buff.from_uid,
                            buff.buff_id,
                            buff.uid,
                            buff.stacks,
                            new_layer,
                        ),
                    );
                } else if buff.stacks > 1 {
                    let new_count = buff.stacks - 1;
                    observe_explicit_buff_uid_for_target(*target_uid, buff.uid);
                    ctx.managers.buff_mgr.add_with_uid(
                        *target_uid,
                        buff.buff_id,
                        buff.from_uid,
                        buff.from_skill_id,
                        new_count,
                        buff.layer,
                        buff.uid,
                    );
                    act_effect.push(
                        crate::state::battle::fight_step::ActEffectBuilder::buff_update(
                            *target_uid,
                            buff.from_uid,
                            buff.buff_id,
                            buff.uid,
                            new_count,
                            buff.layer,
                        ),
                    );
                } else {
                    let mut queue = EventQueue::new();
                    queue.push(BattleEvent::BuffRemove {
                        target: *target_uid,
                        buff_uid: buff.uid,
                    });
                    let mut event_ctx = EventContext {
                        fight: ctx.fight,
                        buff_mgr: &mut ctx.managers.buff_mgr,
                        entity_mgr: &mut ctx.managers.entity_mgr,
                        bloodtithe: &mut ctx.mechanics.bloodtithe,
                    };
                    act_effect.extend(drain_to_fight_steps(queue.drain(), &mut event_ctx));
                }

                act_effect.push(ActEffectBuilder::effect_none_with_num(*target_uid, 0));

                if ex_gain != 0 {
                    // Don't mutate entity_mgr directly — the emitted ExPointChange
                    // effect is applied by calculate_mgr::play_effect_add_ex_point during
                    // play_step_data. Direct mutation + replay = double-apply.
                    act_effect.push(ActEffectBuilder::moxie_change(buff.from_uid, ex_gain));
                }

                let inner = ProtoFightStep {
                    act_type: Some(fight_step::ActType::Effect.into()),
                    from_id: Some(buff.from_uid),
                    to_id: Some(*target_uid),
                    act_id: Some(buff.buff_id),
                    act_effect,
                    card_index: Some(0),
                    support_hero_id: Some(0),
                    fake_timeline: Some(false),
                    real_skill_type: Some(0),
                    real_skin_id: Some(0),
                };

                let wrapped =
                    crate::state::battle::fight_step::ActEffectBuilder::skill_wrapper(inner);

                out.push(FightStepBuilder::effect().with(wrapped).build());
            }
        }

        out
    }
}
