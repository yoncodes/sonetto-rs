use std::collections::HashMap;

use sonettobuf::{ActEffect, Fight, FightStep};

use crate::state::battle::skill::cache::resolve_skill_effect_id;
use crate::state::battle::types::effects::EffectType;
use crate::state::battle::{
    buff_actions::attr_replace::buff_get_attr_replace_permille,
    buff_actions::nuodika_cast::buff_get_nuodika_channel_params,
    context::FightContext,
    event_queue::{BattleEvent, EventContext, EventQueue, drain_to_fight_steps},
    fight_step::{ActEffectBuilder, effect_container_step, wrap_step},
    manager::{
        buff_mgr::BuffMgr,
        entity_mgr::EntityMgr,
        round_mgr::lookup_entry_max_hp,
    },
    mechanics::{bloodtithe::BloodtitheState, injury_counter, magic_circle},
    passives::{
        collector::CollectedPassives, steps::skill::execute_skill as execute_passive_skill,
    },
    round::step_shape::build_effect_step,
    skill::classification::has_injury_reactive_condition,
    skill::targets,
    trigger::combat::{event_from_step, fire_combat_triggers},
};

pub(crate) fn build_nuodika_channel_steps(
    ctx: &mut FightContext<'_>,
    prior_steps: &[FightStep],
    collected: &CollectedPassives,
) -> Vec<FightStep> {
    let mut out = Vec::new();
    let mut simulated_pool = std::collections::HashMap::<i32, i32>::new();
    let mut simulated_hp = build_simulated_hp_map(ctx.fight);
    for step in prior_steps {
        apply_step_to_simulated_hp(step, &mut simulated_hp);
    }
    let holder_uids: Vec<i64> = ctx
        .fight
        .attacker
        .as_ref()
        .into_iter()
        .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter()))
        .chain(
            ctx.fight
                .defender
                .as_ref()
                .into_iter()
                .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter())),
        )
        .filter(|entity| entity.current_hp.unwrap_or(0) > 0)
        .filter_map(|entity| entity.uid)
        .collect();
    for holder_uid in holder_uids {
        let Some(holder) = crate::state::battle::skill::get_entity(ctx.fight, holder_uid) else {
            continue;
        };
        let team_type = holder
            .team_type
            .unwrap_or(if holder_uid > 0 { 1 } else { 2 });
        let pool_value = *simulated_pool
            .entry(team_type)
            .or_insert_with(|| ctx.mechanics.bloodtithe.get_value(team_type).max(0));
        if pool_value <= 0 {
            continue;
        }

        let holder_buffs = ctx.managers.buff_mgr.get(holder_uid).to_vec();
        for instance in holder_buffs {
            let Some((
                _duration,
                threshold,
                _max_points,
                points_per_trigger,
                output_skill_id,
                _counter_buff_id,
            )) = buff_get_nuodika_channel_params(instance.buff_id)
            else {
                continue;
            };
            if threshold <= 0 || output_skill_id <= 0 {
                continue;
            }

            let available = *simulated_pool.get(&team_type).unwrap_or(&0);
            let consume = (available / threshold) * threshold;
            if consume <= 0 {
                continue;
            }
            simulated_pool.insert(team_type, available - consume);

            let granted_points = (consume / threshold) * points_per_trigger;
            let target_uid = targets::collect_team(ctx.fight, Some(team_type), false)
                .into_iter()
                .find(|uid| {
                    simulated_hp.get(uid).copied().unwrap_or_else(|| {
                        crate::state::battle::skill::get_entity(ctx.fight, *uid)
                            .map(|entity| entity.current_hp.unwrap_or(0))
                            .unwrap_or(0)
                    }) > 0
                })
                .unwrap_or(holder_uid);
            let alive_enemy_targets: Vec<i64> =
                targets::collect_team(ctx.fight, Some(team_type), false)
                    .into_iter()
                    .filter(|uid| {
                        simulated_hp.get(uid).copied().unwrap_or_else(|| {
                            crate::state::battle::skill::get_entity(ctx.fight, *uid)
                                .map(|entity| entity.current_hp.unwrap_or(0))
                                .unwrap_or(0)
                        }) > 0
                    })
                    .collect();

            let phase = crate::state::battle::skill::PhaseFilter::combat_with(
                crate::state::battle::skill::TriggerState::default()
                    .with_buff_mgr(&ctx.managers.buff_mgr),
            );
            let Ok(mut channel_effects) =
                execute_passive_skill(ctx, holder_uid, target_uid, output_skill_id, &phase)
            else {
                continue;
            };
            let skill_event = if let Some(skill_step) =
                injury_counter::find_nested_skill_step_mut(&mut channel_effects, output_skill_id)
            {
                let circle_embeds = magic_circle::build_magic_circle_self_skill_embeds(
                    ctx,
                    &skill_step.clone(),
                    holder_uid,
                );
                if !circle_embeds.is_empty() {
                    let insert_at =
                        crate::state::battle::steps::trigger_embed::find_trigger_insert_index(
                            &skill_step.act_effect,
                        );
                    skill_step
                        .act_effect
                        .splice(insert_at..insert_at, circle_embeds);
                }
                Some(event_from_step(
                    ctx.fight,
                    skill_step.from_id.unwrap_or(0),
                    skill_step.to_id.unwrap_or(0),
                    skill_step.act_id.unwrap_or(0),
                    &skill_step.act_effect,
                ))
            } else {
                None
            };

            if let Some(skill_event) = skill_event {
                let injury_only = collected.filter(has_injury_reactive_condition);
                let trigger_steps = fire_combat_triggers(ctx, &injury_only, &skill_event);
                if !trigger_steps.is_empty()
                    && let Some(skill_step) = injury_counter::find_nested_skill_step_mut(
                        &mut channel_effects,
                        output_skill_id,
                    )
                {
                    for ts in trigger_steps {
                        let embedded =
                            crate::state::battle::steps::trigger_embed::trigger_step_to_embedded_effect(ts);
                        skill_step.act_effect.push(embedded);
                    }
                }
            }
            rewrite_nuodika_channel_body(
                ctx.fight,
                &ctx.managers.buff_mgr,
                &ctx.managers.entity_mgr,
                &mut channel_effects,
                holder_uid,
                output_skill_id,
                target_uid,
                &alive_enemy_targets,
                granted_points,
                available - consume,
            );

            let mut queue = EventQueue::new();
            queue.push(BattleEvent::BloodpoolValueChange {
                team_type,
                target: holder_uid,
                delta: -consume,
            });
            let mut local_fight = Fight::default();
            let mut local_buff_mgr = BuffMgr::new();
            let mut local_entity_mgr = EntityMgr::default();
            let mut local_bloodtithe = BloodtitheState::new();
            let mut event_ctx = EventContext {
                fight: &mut local_fight,
                buff_mgr: &mut local_buff_mgr,
                entity_mgr: &mut local_entity_mgr,
                bloodtithe: &mut local_bloodtithe,
            };
            let mut step_effects = drain_to_fight_steps(queue.drain(), &mut event_ctx);
            step_effects.push(ActEffectBuilder::nuodika_random_attack_num(
                holder_uid,
                granted_points,
                1,
            ));

            step_effects.append(&mut channel_effects);
            let inner =
                effect_container_step(holder_uid, holder_uid, instance.buff_id, step_effects);
            let wrapped = build_effect_step(vec![wrap_step(inner)]);
            apply_step_to_simulated_hp(&wrapped, &mut simulated_hp);
            out.push(wrapped);
        }
    }

    out
}

fn build_simulated_hp_map(fight: &Fight) -> HashMap<i64, i32> {
    let mut hp = HashMap::new();
    for entity in fight
        .attacker
        .as_ref()
        .into_iter()
        .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter()))
        .chain(
            fight
                .defender
                .as_ref()
                .into_iter()
                .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter())),
        )
    {
        if let Some(uid) = entity.uid {
            hp.insert(uid, entity.current_hp.unwrap_or(0));
        }
    }
    hp
}

fn apply_step_to_simulated_hp(step: &FightStep, simulated_hp: &mut HashMap<i64, i32>) {
    let mut stack = vec![step];
    while let Some(cur) = stack.pop() {
        for effect in &cur.act_effect {
            if let Some(child) = effect.fight_step.as_ref() {
                stack.push(child);
            }
            let target_id = effect.target_id.unwrap_or(0);
            if target_id == 0 {
                continue;
            }
            match effect.effect_type.unwrap_or(0) {
                x if injury_counter::is_damage_effect_type(x) => {
                    let amount = effect.effect_num.unwrap_or(0).max(0);
                    let entry = simulated_hp.entry(target_id).or_insert(0);
                    *entry = (*entry - amount).max(0);
                }
                x if x == EffectType::Dead as i32 => {
                    simulated_hp.insert(target_id, 0);
                }
                _ => {}
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn rewrite_nuodika_channel_body(
    fight: &Fight,
    buff_mgr: &BuffMgr,
    entity_mgr: &EntityMgr,
    skill_effects: &mut [ActEffect],
    caster_uid: i64,
    output_skill_id: i32,
    primary_target_uid: i64,
    alive_enemy_targets: &[i64],
    blood_sacrifice_points: i32,
    current_blood_value: i32,
) {
    if blood_sacrifice_points <= 0 {
        return;
    }
    let Some(skill_step) =
        injury_counter::find_nested_skill_step_mut(skill_effects, output_skill_id)
    else {
        return;
    };
    let cfg = config::configs::get();
    let Some(skill_cfg) = cfg
        .skill_effect
        .iter()
        .find(|s| s.id == resolve_skill_effect_id(output_skill_id))
    else {
        return;
    };

    let mut nuodika_behavior = None;
    let behavior_slots = [
        &skill_cfg.behavior1,
        &skill_cfg.behavior2,
        &skill_cfg.behavior3,
        &skill_cfg.behavior4,
        &skill_cfg.behavior5,
        &skill_cfg.behavior6,
        &skill_cfg.behavior7,
        &skill_cfg.behavior8,
        &skill_cfg.behavior9,
        &skill_cfg.behavior10,
        &skill_cfg.behavior11,
        &skill_cfg.behavior12,
        &skill_cfg.behavior13,
        &skill_cfg.behavior14,
        &skill_cfg.behavior15,
        &skill_cfg.behavior16,
        &skill_cfg.behavior17,
        &skill_cfg.behavior18,
        &skill_cfg.behavior19,
        &skill_cfg.behavior20,
    ];
    for behavior in behavior_slots {
        let behavior = behavior.trim();
        if behavior.is_empty() {
            continue;
        }
        let Some(behavior_id) = behavior
            .split('#')
            .next()
            .and_then(|v| v.trim().parse::<i32>().ok())
        else {
            continue;
        };
        let is_nuodika = cfg
            .skill_behavior
            .iter()
            .find(|b| b.id == behavior_id)
            .map(|b| b.r#type == "NuoDiKaDamage")
            .unwrap_or(false);
        if is_nuodika {
            nuodika_behavior = Some((behavior_id, behavior.to_string()));
            break;
        }
    }
    let Some((behavior_id, raw_behavior)) = nuodika_behavior else {
        return;
    };
    let parts: Vec<&str> = raw_behavior.split('#').collect();
    let primary_buff_id = parts
        .get(1)
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0);
    let primary_rate = parts
        .get(2)
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0);
    let secondary_buff_id = parts
        .get(3)
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0);
    let secondary_rate = parts
        .get(4)
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0);
    let primary_permille = buff_get_attr_replace_permille(primary_buff_id).unwrap_or(0);
    let secondary_permille = buff_get_attr_replace_permille(secondary_buff_id).unwrap_or(0);
    let Some(caster) = crate::state::battle::skill::get_entity(fight, caster_uid) else {
        return;
    };
    let pending_attr_bonus = collect_pre_hit_attr_bonus(&skill_step.act_effect, caster_uid);
    let max_hp = lookup_entry_max_hp(fight, caster_uid)
        .max(entity_mgr.get_max_hp(caster_uid))
        .max(
            caster
                .base_attr
                .as_ref()
                .and_then(|a| a.hp)
                .unwrap_or(caster.current_hp.unwrap_or(0)),
        )
        .max(
            caster
                .attr
                .as_ref()
                .and_then(|a| a.hp)
                .unwrap_or(caster.current_hp.unwrap_or(0)),
        )
        .max(caster.current_hp.unwrap_or(0))
        .max(0);
    if max_hp <= 0 {
        return;
    }
    let blood_sacrifice_hp_bonus_permille =
        resolve_blood_sacrifice_hp_bonus_permille(buff_mgr, caster_uid);
    let blood_sacrifice_point_count =
        resolve_blood_sacrifice_point_count(buff_mgr, caster_uid, current_blood_value);
    let scaled_max_hp = apply_blood_sacrifice_hp_bonus(
        max_hp,
        blood_sacrifice_point_count,
        blood_sacrifice_hp_bonus_permille,
    );
    let team_hit_extra_hp_permille =
        resolve_team_hit_extra_max_hp_permille(&skill_step.act_effect, caster_uid);
    let team_hit_max_hp = apply_permille_bonus(scaled_max_hp, team_hit_extra_hp_permille);
    let calc_scaled_damage = |base_max_hp: i32, permille: i32, rate: i32| -> i32 {
        let scaled = (base_max_hp as i64)
            .saturating_mul(permille as i64)
            .saturating_mul(rate as i64)
            / 1000
            / 1000;
        scaled.clamp(1, i32::MAX as i64) as i32
    };
    let apply_pending_attr_bonus = |base_damage: i32| -> i32 {
        let rate_bonus = pending_attr_bonus.get(&205).copied().unwrap_or(0)
            + pending_attr_bonus.get(&214).copied().unwrap_or(0)
            + pending_attr_bonus.get(&215).copied().unwrap_or(0)
            + pending_attr_bonus.get(&219).copied().unwrap_or(0)
            - pending_attr_bonus.get(&206).copied().unwrap_or(0);
        let aux_bonus = pending_attr_bonus.get(&201).copied().unwrap_or(0)
            + pending_attr_bonus.get(&203).copied().unwrap_or(0);
        let scaled = (base_damage as i64)
            .saturating_mul((1000 + rate_bonus).max(0) as i64)
            .saturating_mul((1000 + aux_bonus).max(0) as i64)
            / 1000
            / 1000;
        scaled.clamp(1, i32::MAX as i64) as i32
    };
    let mut preserved = Vec::new();
    let mut aggregate_damage = std::collections::HashMap::<i64, i32>::new();
    let mut team_targets = Vec::new();
    for effect in skill_step.act_effect.drain(..) {
        let target_id = effect.target_id.unwrap_or(0);
        let is_enemy_damage =
            injury_counter::is_damage_effect_type(effect.effect_type.unwrap_or(0))
                && target_id != 0
                && target_id != caster_uid;
        if is_enemy_damage {
            if !team_targets.contains(&target_id) {
                team_targets.push(target_id);
            }
            continue;
        }
        preserved.push(effect);
    }
    if !alive_enemy_targets.is_empty() {
        team_targets.retain(|uid| alive_enemy_targets.contains(uid));
    }
    if team_targets.is_empty() && !alive_enemy_targets.is_empty() {
        team_targets.extend_from_slice(alive_enemy_targets);
    }
    if team_targets.is_empty() && primary_target_uid != 0 {
        team_targets.push(primary_target_uid);
    }
    if team_targets.is_empty() {
        return;
    }

    let random_target = primary_target_uid;
    // Damage per hit is derived from the caster's Max HP times the buff's
    // attrReplace permille times the behavior-slot rate (both parts-per-1000).
    let random_hit_damage = apply_pending_attr_bonus(calc_scaled_damage(
        scaled_max_hp,
        primary_permille,
        primary_rate,
    ));
    let team_hit_damage = apply_pending_attr_bonus(calc_scaled_damage(
        team_hit_max_hp,
        secondary_permille,
        secondary_rate,
    ));
    let variant_config = resolve_dying_heal_disperse_variant(buff_mgr, caster_uid);
    let variant_random_hit_damage = variant_config
        .as_ref()
        .map(|variant| {
            let base_variant_damage = calc_scaled_damage(max_hp, primary_permille, primary_rate);
            let reduced_variant_damage = (base_variant_damage as i64).saturating_mul(1000)
                / (1000 + variant.reduction_permille.max(0)) as i64;
            apply_pending_attr_bonus(reduced_variant_damage.clamp(1, i32::MAX as i64) as i32)
        })
        .unwrap_or(random_hit_damage);
    let random_hit_pattern: Vec<(i32, i32)> = (0..blood_sacrifice_points.max(0) as usize)
        .map(|index| {
            let is_variant = variant_config
                .as_ref()
                .map(|variant| index % variant.hit_stride == 0)
                .unwrap_or(false);
            if is_variant {
                (variant_random_hit_damage, 2)
            } else {
                (random_hit_damage, 3)
            }
        })
        .collect();
    if random_hit_pattern.is_empty() && team_hit_damage <= 0 {
        return;
    }
    for (damage, _) in &random_hit_pattern {
        if random_target != 0 {
            *aggregate_damage.entry(random_target).or_insert(0) += *damage;
        }
    }
    for target_id in &team_targets {
        *aggregate_damage.entry(*target_id).or_insert(0) += team_hit_damage;
    }

    let mut rebuilt = preserved;
    rebuilt.push(ActEffectBuilder::nuodika_random_attack_num(
        caster_uid,
        blood_sacrifice_points,
        1,
    ));
    for (index, (damage, effect_num1)) in random_hit_pattern.iter().enumerate() {
        if random_target != 0 {
            rebuilt.push(ActEffectBuilder::nuodika_random_attack(
                random_target,
                *damage,
                *effect_num1,
                behavior_id,
                output_skill_id,
                format!("{}#{}", index + 1, blood_sacrifice_points.max(0) as usize),
            ));
        }
    }
    for target_id in &team_targets {
        rebuilt.push(ActEffectBuilder::nuodika_team_attack(
            *target_id,
            team_hit_damage,
            1,
            behavior_id,
            output_skill_id,
        ));
    }
    for (target_id, damage) in aggregate_damage {
        rebuilt.push(ActEffectBuilder::damage_skill(
            target_id,
            damage.max(1),
            behavior_id,
            output_skill_id,
            caster_uid,
        ));
    }
    let dead_effects = injury_counter::collect_dead_effects_after_damage(fight, &rebuilt);
    if !dead_effects.is_empty() {
        rebuilt.extend(dead_effects);
    }
    skill_step.act_effect = rebuilt;
}

fn collect_pre_hit_attr_bonus(effects: &[ActEffect], caster_uid: i64) -> HashMap<i32, i32> {
    fn walk(effects: &[ActEffect], caster_uid: i64, out: &mut HashMap<i32, i32>) {
        for effect in effects {
            let Some(step) = effect.fight_step.as_ref() else {
                continue;
            };
            if step.from_id.unwrap_or(0) == caster_uid {
                collect_skill_attr_bonus(step.act_id.unwrap_or(0), out);
            }
            walk(&step.act_effect, caster_uid, out);
        }
    }

    fn collect_skill_attr_bonus(skill_id: i32, out: &mut HashMap<i32, i32>) {
        if skill_id <= 0 {
            return;
        }
        let cfg = config::configs::get();
        let Some(skill_cfg) = cfg
            .skill_effect
            .iter()
            .find(|s| s.id == resolve_skill_effect_id(skill_id))
        else {
            return;
        };
        let behavior_slots = [
            &skill_cfg.behavior1,
            &skill_cfg.behavior2,
            &skill_cfg.behavior3,
            &skill_cfg.behavior4,
            &skill_cfg.behavior5,
            &skill_cfg.behavior6,
            &skill_cfg.behavior7,
            &skill_cfg.behavior8,
            &skill_cfg.behavior9,
            &skill_cfg.behavior10,
            &skill_cfg.behavior11,
            &skill_cfg.behavior12,
            &skill_cfg.behavior13,
            &skill_cfg.behavior14,
            &skill_cfg.behavior15,
            &skill_cfg.behavior16,
            &skill_cfg.behavior17,
            &skill_cfg.behavior18,
            &skill_cfg.behavior19,
            &skill_cfg.behavior20,
        ];
        for raw in behavior_slots {
            if let Some((attr_id, amount)) = parse_attr_fix_like_behavior(raw.trim()) {
                {
                    *out.entry(attr_id).or_insert(0) += amount;
                }
            }
        }
    }

    let mut out = HashMap::new();
    walk(effects, caster_uid, &mut out);
    out
}

fn apply_blood_sacrifice_hp_bonus(
    base_max_hp: i32,
    point_count: i32,
    per_point_permille: i32,
) -> i32 {
    if base_max_hp <= 0 || point_count <= 0 || per_point_permille <= 0 {
        return base_max_hp.max(0);
    }
    let scaled = (base_max_hp as i64).saturating_mul(
        1000_i64.saturating_add((point_count as i64).saturating_mul(per_point_permille as i64)),
    ) / 1000;
    scaled.clamp(1, i32::MAX as i64) as i32
}

fn apply_permille_bonus(base_value: i32, bonus_permille: i32) -> i32 {
    if base_value <= 0 || bonus_permille <= 0 {
        return base_value.max(0);
    }
    let scaled = (base_value as i64).saturating_mul((1000 + bonus_permille) as i64) / 1000;
    scaled.clamp(1, i32::MAX as i64) as i32
}

struct DyingHealDisperseVariant {
    reduction_permille: i32,
    hit_stride: usize,
}

fn resolve_dying_heal_disperse_variant(
    buff_mgr: &BuffMgr,
    caster_uid: i64,
) -> Option<DyingHealDisperseVariant> {
    let cfg = config::configs::get();
    buff_mgr.get(caster_uid).iter().find_map(|instance| {
        let buff = cfg.skill_buff.iter().find(|b| b.id == instance.buff_id)?;
        buff.features.split('|').find_map(|feature| {
            let parts: Vec<&str> = feature.split('#').collect();
            let act_id = parts.first()?.trim().parse::<i32>().ok()?;
            if act_id != 1010 {
                return None;
            }
            let reduction_permille = parts
                .get(1)
                .and_then(|value| value.trim().parse::<i32>().ok())
                .unwrap_or(0);
            let hit_stride = parts
                .get(2)
                .map(|value| {
                    value
                        .split(',')
                        .filter_map(|part| part.trim().parse::<usize>().ok())
                        .sum::<usize>()
                })
                .unwrap_or(0);
            if reduction_permille <= 0 || hit_stride == 0 {
                return None;
            }
            Some(DyingHealDisperseVariant {
                reduction_permille,
                hit_stride,
            })
        })
    })
}

fn resolve_team_hit_extra_max_hp_permille(effects: &[ActEffect], caster_uid: i64) -> i32 {
    fn walk(effects: &[ActEffect], caster_uid: i64, out: &mut i32) {
        let cfg = config::configs::get();
        for effect in effects {
            if effect.target_id.unwrap_or(0) == caster_uid {
                let Some(buff_id) = effect.buff.as_ref().and_then(|buff| buff.buff_id) else {
                    if let Some(step) = effect.fight_step.as_ref() {
                        walk(&step.act_effect, caster_uid, out);
                    }
                    continue;
                };
                let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == buff_id) else {
                    if let Some(step) = effect.fight_step.as_ref() {
                        walk(&step.act_effect, caster_uid, out);
                    }
                    continue;
                };
                let is_stacked = cfg
                    .skill_bufftype
                    .iter()
                    .find(|t| t.id == buff_cfg.type_id)
                    .and_then(|t| t.include_types.split('#').next())
                    .map(|include_type| matches!(include_type.trim(), "10" | "12" | "14" | "15"))
                    .unwrap_or(false);
                if is_stacked {
                    for entry in buff_cfg.features.split('|') {
                        let parts: Vec<&str> = entry.split('#').collect();
                        let Some(act_id) = parts.first().and_then(|v| v.trim().parse::<i32>().ok())
                        else {
                            continue;
                        };
                        let act_type = cfg
                            .buff_act
                            .iter()
                            .find(|a| a.id == act_id)
                            .map(|a| a.r#type.as_str())
                            .unwrap_or("");
                        if act_type != "Attr" {
                            continue;
                        }
                        let attr_id = parts
                            .get(1)
                            .and_then(|v| v.trim().parse::<i32>().ok())
                            .unwrap_or(0);
                        let amount = parts
                            .get(2)
                            .and_then(|v| v.trim().parse::<i32>().ok())
                            .unwrap_or(0);
                        if attr_id == 101 && amount > 0 {
                            *out = (*out).max(amount);
                        }
                    }
                }
            }
            if let Some(step) = effect.fight_step.as_ref() {
                walk(&step.act_effect, caster_uid, out);
            }
        }
    }

    let mut out = 0;
    walk(effects, caster_uid, &mut out);
    out
}

fn resolve_blood_sacrifice_point_count(
    buff_mgr: &BuffMgr,
    caster_uid: i64,
    current_blood_value: i32,
) -> i32 {
    let cfg = config::configs::get();
    let stack_bonus: i32 = buff_mgr
        .get(caster_uid)
        .iter()
        .filter_map(|instance| {
            let buff = cfg.skill_buff.iter().find(|b| b.id == instance.buff_id)?;
            let is_blood_sacrifice_counter = buff.features.split('|').any(|feature| {
                let parts: Vec<&str> = feature.split('#').collect();
                let Some(act_id) = parts.first().and_then(|v| v.trim().parse::<i32>().ok()) else {
                    return false;
                };
                cfg.buff_act
                    .iter()
                    .find(|a| a.id == act_id)
                    .map(|a| a.r#type == "DyingHealDisperse1")
                    .unwrap_or(false)
            });
            if !is_blood_sacrifice_counter {
                return None;
            }
            Some(instance.stacks.max(instance.layer).max(0))
        })
        .sum();
    current_blood_value.max(0) + stack_bonus
}

fn resolve_blood_sacrifice_hp_bonus_permille(buff_mgr: &BuffMgr, caster_uid: i64) -> i32 {
    let cfg = config::configs::get();
    let carrier_buff_ids: Vec<i32> = buff_mgr
        .get(caster_uid)
        .iter()
        .filter_map(|instance| {
            let buff = cfg.skill_buff.iter().find(|b| b.id == instance.buff_id)?;
            buff.features.split('|').find_map(|feature| {
                let parts: Vec<&str> = feature.split('#').collect();
                let act_id = parts.first()?.trim().parse::<i32>().ok()?;
                let act_type = cfg
                    .buff_act
                    .iter()
                    .find(|a| a.id == act_id)
                    .map(|a| a.r#type.as_str())
                    .unwrap_or("");
                if act_type != "BloodValueUseSkill" {
                    return None;
                }
                parts.get(1)?.trim().parse::<i32>().ok()
            })
        })
        .collect();

    carrier_buff_ids
        .into_iter()
        .filter(|buff_id| buff_mgr.has(caster_uid, *buff_id))
        .filter_map(extract_blood_sacrifice_permille_from_carrier_buff)
        .max()
        .unwrap_or(0)
}

fn extract_blood_sacrifice_permille_from_carrier_buff(buff_id: i32) -> Option<i32> {
    let cfg = config::configs::get();
    let buff = cfg.skill_buff.iter().find(|b| b.id == buff_id)?;
    buff.features.split('|').find_map(|feature| {
        let parts: Vec<&str> = feature.split('#').collect();
        let act_id = parts.first()?.trim().parse::<i32>().ok()?;
        let act_type = cfg
            .buff_act
            .iter()
            .find(|a| a.id == act_id)
            .map(|a| a.r#type.as_str())
            .unwrap_or("");
        if act_type != "CureUpByLostHp" {
            return None;
        }
        parts.last()?.trim().parse::<i32>().ok()
    })
}

fn parse_attr_fix_like_behavior(raw: &str) -> Option<(i32, i32)> {
    if raw.is_empty() {
        return None;
    }
    let parts: Vec<&str> = raw.split('#').collect();
    let behavior_id = parts.first()?.trim().parse::<i32>().ok()?;
    let behavior_type = config::configs::get()
        .skill_behavior
        .iter()
        .find(|b| b.id == behavior_id)
        .map(|b| b.r#type.as_str())
        .unwrap_or("");
    if behavior_type != "AttrFix" && behavior_type != "AttrModify" {
        return None;
    }
    let attr_id = parts.get(1)?.trim().parse::<i32>().ok()?;
    let amount = parts.get(2)?.trim().parse::<i32>().ok()?;
    Some((attr_id, amount))
}
