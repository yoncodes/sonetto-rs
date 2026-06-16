use sonettobuf::FightStep;

use crate::state::battle::{
    buff_actions::probability_add_buff::probability_add_buff_specs,
    context::FightContext,
    event_queue::{BattleEvent, EventContext, EventQueue, SkillEmitKind, drain_to_fight_steps},
    manager::buff_mgr::BuffInstance,
    passives::collector::CollectedPassives,
    round::step_shape::build_effect_step,
    skill::{SkillExecutor, buff, cache::resolve_skill_effect_id},
    trigger::combat::{SyntheticEmissionKind, TriggerEvent},
    types::behavior::BehaviorType,
};

use super::TriggerPass;

pub struct BuffFeatureReactivesPass;

impl TriggerPass for BuffFeatureReactivesPass {
    fn run(
        &self,
        ctx: &mut FightContext<'_>,
        event: &TriggerEvent,
        _collected: &CollectedPassives,
    ) -> Vec<FightStep> {
        run_probability_add_buff_reactives(ctx, event)
    }
}

fn run_probability_add_buff_reactives(
    ctx: &mut FightContext<'_>,
    event: &TriggerEvent,
) -> Vec<FightStep> {
    if event.caster_uid == 0
        || event.synthetic_emission == SyntheticEmissionKind::SothebyHolderConsume
        || !skill_is_hurt(event.skill_id)
    {
        return Vec::new();
    }

    let trigger_targets = trigger_targets(event);
    if trigger_targets.is_empty() {
        return Vec::new();
    }

    let mut sources = ctx
        .managers
        .buff_mgr
        .all_instances()
        .into_iter()
        .filter(|(_, instance)| {
            instance.from_uid != 0
                && instance.from_uid.signum() == event.caster_uid.signum()
                && probability_add_buff_specs(instance.buff_id)
                    .iter()
                    .any(|spec| spec.buff_id > 0 && spec.permille > 0)
        })
        .map(|(_, instance)| (instance.from_uid, instance.buff_id))
        .collect::<Vec<_>>();
    sources.sort_unstable();
    sources.dedup();

    let mut steps = Vec::new();
    for (source_uid, carrier_buff_id) in sources {
        let has_bloodpool = ctx.mechanics.bloodtithe.has_bloodpool();
        let mut executor = SkillExecutor::new();
        let mut inner_effects = Vec::new();

        for spec in probability_add_buff_specs(carrier_buff_id) {
            if spec.buff_id <= 0 || spec.permille <= 0 {
                continue;
            }
            for &target_uid in &trigger_targets {
                for _ in 0..spec.stack_count.max(1) {
                    let mut effects = buff::apply(
                        buff::BuffApplySpec::new(spec.buff_id)
                            .caster(event.caster_uid)
                            .target(target_uid)
                            .bloodpool(has_bloodpool)
                            .skill(event.skill_id),
                        &mut executor,
                        ctx.fight,
                        ctx.managers,
                        ctx.mechanics,
                    );
                    hydrate_buff_effects(
                        ctx.managers.buff_mgr.get(target_uid),
                        spec.buff_id,
                        event.caster_uid,
                        &mut effects,
                    );
                    inner_effects.extend(effects);
                }
            }
        }

        if inner_effects.is_empty() {
            continue;
        }

        let mut queue = EventQueue::new();
        queue.push(BattleEvent::SkillEmit {
            skill_id: carrier_buff_id,
            from: source_uid,
            to: event.caster_uid,
            children: inner_effects
                .into_iter()
                .map(|effect| BattleEvent::SerializedActEffect { effect })
                .collect(),
            kind: SkillEmitKind::EventTriggered,
        });
        let mut event_ctx = EventContext {
            fight: ctx.fight,
            buff_mgr: &mut ctx.managers.buff_mgr,
            entity_mgr: &mut ctx.managers.entity_mgr,
            bloodtithe: &mut ctx.mechanics.bloodtithe,
        };
        let drained_act_effect = drain_to_fight_steps(queue.drain(), &mut event_ctx)
            .into_iter()
            .next()
            .expect("event-triggered skill emission should serialize to a single ActEffect");

        let mut step = build_effect_step(vec![drained_act_effect]);
        step.from_id = Some(source_uid);
        steps.push(step);
    }

    steps
}

fn trigger_targets(event: &TriggerEvent) -> Vec<i64> {
    let mut out = Vec::new();
    for &target_uid in &event.damaged_uids {
        if target_uid == 0 || target_uid.signum() == event.caster_uid.signum() {
            continue;
        }
        if !out.contains(&target_uid) {
            out.push(target_uid);
        }
    }
    out
}

fn skill_is_hurt(skill_id: i32) -> bool {
    if skill_id <= 0 {
        return false;
    }

    let cfg = config::configs::get();
    let effect_id = resolve_skill_effect_id(skill_id);
    if cfg
        .skill_effect
        .iter()
        .find(|row| row.id == effect_id)
        .map(|row| row.damage_rate > 0)
        .unwrap_or(false)
    {
        return true;
    }

    crate::state::battle::skill::cache::SKILL_CACHE
        .get(&effect_id)
        .map(|rows| {
            rows.iter()
                .any(|row| matches!(row.behavior, BehaviorType::Damage { .. }))
        })
        .unwrap_or(false)
}

fn hydrate_buff_effects(
    runtime_buffs: &[BuffInstance],
    buff_id: i32,
    from_uid: i64,
    effects: &mut [sonettobuf::ActEffect],
) {
    let runtime = runtime_buffs
        .iter()
        .filter(|instance| instance.buff_id == buff_id && instance.from_uid == from_uid)
        .max_by_key(|instance| instance.uid);
    let Some(runtime) = runtime else {
        return;
    };

    for effect in effects {
        let effect_type = effect.effect_type.unwrap_or(0);
        if !matches!(
            effect_type,
            x if x == crate::state::battle::types::effects::EffectType::BuffAdd as i32
                || x == crate::state::battle::types::effects::EffectType::BuffUpdate as i32
        ) {
            continue;
        }
        let Some(buff) = effect.buff.as_mut() else {
            continue;
        };
        if buff.buff_id != Some(buff_id) || buff.from_uid != Some(from_uid) {
            continue;
        }
        if buff.uid.unwrap_or(0) == 0 {
            buff.uid = Some(runtime.uid);
        }
        if buff.duration.unwrap_or(0) == 0 && runtime.duration > 0 {
            buff.duration = Some(runtime.duration);
        }
    }
}
