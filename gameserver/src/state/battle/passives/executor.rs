use super::super::{
    buff_actions::blood_pool_ex::{
        buff_get_blood_pool_ex_point_params, build_blood_pool_ex_point_step,
    },
    buff_actions::raspberry::buff_get_raspberry_params,
    context::FightContext,
    fight_step::{FightStepBuilder, wrap_step},
    manager::buff_mgr::{
        DEFENDER_BUFF_UID_START, current_buff_uid, reset_buff_uid, reset_buff_uid_to,
    },
    skill::{
        PhaseFilter,
        cache::{SKILL_CACHE, resolve_skill_effect_id},
        condition::parser::parse_condition,
    },
    types::condition::ConditionType,
    utils::buff_has_bloodpool,
};
use super::collector::collect;
use super::steps::{build_battle_rule_step, build_passive_step, cards, temp_card};
use sonettobuf::FightStep;

pub fn execute_battle_start_passives(ctx: &mut FightContext<'_>, battle_id: i32) -> Vec<FightStep> {
    let collected = collect(ctx.fight, battle_id);
    if collected.is_empty() {
        return vec![];
    }
    tracing::info!(
        "execute_battle_start_passives: {} attackers {} defenders",
        collected.attacker.len(),
        collected.defender.len()
    );

    reset_buff_uid();
    ctx.mechanics.init(ctx.fight);
    let mut steps = Vec::new();

    enum Side {
        Attacker,
        Defender,
    }
    enum Pass {
        BattleRulesAttacker,
        BattleRulesDefender,
        BattleRulesUnconditionalAttacker,
        Passive { side: Side, phase: PhaseFilter },
        BloodpoolInit,
        BloodtitheSync,
        Raspberry,
        ShadowCloakSync,
        DefenderUidReset,
        AttackerUidRestore,
        DealCards,
        CardDeckNum,
        TempCard,
        TempCardCleanup,
        BloodPoolExPoint,
        CardDeckNumFinal,
    }

    let passes = vec![
        Pass::BattleRulesAttacker,
        Pass::BattleRulesDefender,
        Pass::Passive {
            side: Side::Attacker,
            phase: PhaseFilter::battle_start(),
        },
        Pass::BloodpoolInit,
        Pass::Passive {
            side: Side::Attacker,
            phase: PhaseFilter::enter_fight(),
        },
        Pass::DefenderUidReset,
        Pass::Passive {
            side: Side::Defender,
            phase: PhaseFilter::enter_fight(),
        },
        Pass::AttackerUidRestore,
        Pass::BattleRulesUnconditionalAttacker,
        Pass::Passive {
            side: Side::Attacker,
            phase: PhaseFilter::unconditional(),
        },
        Pass::BloodtitheSync,
        Pass::Raspberry,
        Pass::ShadowCloakSync,
        Pass::Passive {
            side: Side::Attacker,
            phase: PhaseFilter::consume_blood(),
        },
        Pass::DealCards,
        Pass::CardDeckNum,
        Pass::TempCard,
        Pass::TempCardCleanup,
        Pass::BloodPoolExPoint,
        Pass::CardDeckNumFinal,
    ];

    let mut attacker_uid_checkpoint: i64 = 0;
    let mut defender_uid_checkpoint: i64 = DEFENDER_BUFF_UID_START;

    for pass in passes {
        match pass {
            Pass::BattleRulesAttacker => {
                let battle_skills: Vec<i32> = collected
                    .battle_attacker
                    .iter()
                    .rev()
                    .copied()
                    .filter(|&sid| !is_unconditional_battle_rule_skill(sid))
                    .collect();
                for skill_id in battle_skills {
                    let inner = build_battle_rule_step(
                        ctx,
                        &collected.attacker_uids(),
                        &[skill_id],
                        &PhaseFilter::enter_fight(),
                    );
                    if !inner.is_empty() {
                        let outer_effects = inner.into_iter().map(wrap_step).collect();
                        steps.push(FightStepBuilder::effect().with_many(outer_effects).build());
                    }
                }
            }
            Pass::BattleRulesDefender => {
                // Defender battle-rule passives should allocate buff UIDs from defender space.
                // Use a temporary switch here because this pass runs before the main
                // DefenderUidReset/AttackerUidRestore window.
                let attacker_uid_before_defender_rules = current_buff_uid();
                reset_buff_uid_to(DEFENDER_BUFF_UID_START);

                let battle_skills: Vec<i32> = collected
                    .battle_defender
                    .iter()
                    .rev()
                    .copied()
                    .filter(|&sid| !is_unconditional_battle_rule_skill(sid))
                    .collect();
                for skill_id in battle_skills {
                    let inner = build_battle_rule_step(
                        ctx,
                        &collected.defender_uids(),
                        &[skill_id],
                        &PhaseFilter::enter_fight(),
                    );
                    if !inner.is_empty() {
                        let outer_effects = inner.into_iter().map(wrap_step).collect();
                        steps.push(FightStepBuilder::effect().with_many(outer_effects).build());
                    }
                }

                // Keep defender lane progress so later defender passive pass continues
                // from the last defender-issued uid (e.g. 100006 -> 100007).
                defender_uid_checkpoint = current_buff_uid();
                reset_buff_uid_to(attacker_uid_before_defender_rules);
            }
            Pass::BattleRulesUnconditionalAttacker => {
                let battle_skills: Vec<i32> = collected
                    .battle_attacker
                    .iter()
                    .rev()
                    .copied()
                    .filter(|&sid| is_unconditional_battle_rule_skill(sid))
                    .collect();
                for skill_id in battle_skills {
                    let inner = build_battle_rule_step(
                        ctx,
                        &collected.attacker_uids(),
                        &[skill_id],
                        &PhaseFilter::unconditional(),
                    );
                    if !inner.is_empty() {
                        let outer_effects = inner.into_iter().map(wrap_step).collect();
                        steps.push(FightStepBuilder::effect().with_many(outer_effects).build());
                    }
                }
            }
            Pass::Passive { side, phase } => {
                let uids = match side {
                    Side::Attacker => collected.attacker_uids(),
                    Side::Defender => collected.defender_uids(),
                };
                let inner = build_passive_step(ctx, &uids, &collected, &phase);
                if !inner.is_empty() {
                    let outer_effects = inner.into_iter().map(wrap_step).collect();
                    steps.push(FightStepBuilder::effect().with_many(outer_effects).build());
                }
            }
            Pass::BloodpoolInit => {
                if let Some(s) = ctx.mechanics.on_bloodpool_init() {
                    steps.push(s);
                }
            }
            Pass::DefenderUidReset => {
                if !collected.defender.is_empty() {
                    attacker_uid_checkpoint = current_buff_uid();
                    reset_buff_uid_to(defender_uid_checkpoint);
                }
            }
            Pass::AttackerUidRestore => {
                if attacker_uid_checkpoint > 0 {
                    reset_buff_uid_to(attacker_uid_checkpoint);
                }
            }
            Pass::BloodtitheSync => {
                seed_bloodtithe_from_fight(ctx);
                if let Some(s) = ctx.mechanics.on_pre_raspberry() {
                    steps.push(s);
                }
            }
            Pass::Raspberry => {
                if let Some(s) = ctx.mechanics.on_raspberry(
                    ctx.fight,
                    &ctx.managers.buff_mgr,
                    &mut ctx.managers.entity_mgr,
                ) {
                    steps.push(s);
                }
            }
            Pass::ShadowCloakSync => {
                if let Some(s) = ctx.mechanics.on_post_raspberry(
                    ctx.fight,
                    &ctx.managers.buff_mgr,
                    &ctx.managers.entity_mgr,
                ) {
                    steps.push(s);
                }
            }
            Pass::DealCards => {
                steps.push(cards::build_deal_cards_step());
            }
            Pass::CardDeckNum => {
                steps.push(cards::build_card_deck_num_step(collected.attacker.len()));
            }
            Pass::TempCard => {
                if let Some(s) = temp_card::build_temp_card_step(ctx, &collected.attacker_uids()) {
                    steps.push(s);
                }
            }
            Pass::TempCardCleanup => {
                if let Some(s) =
                    temp_card::build_temp_card_cleanup_step(ctx, &collected.attacker_uids())
                {
                    steps.push(s);
                }
            }
            Pass::BloodPoolExPoint => {
                if let Some(s) = build_blood_pool_ex_point_step(
                    &mut ctx.mechanics.bloodtithe,
                    ctx.fight,
                    &ctx.managers.buff_mgr,
                    &mut ctx.managers.entity_mgr,
                ) {
                    steps.push(s);
                }
            }
            Pass::CardDeckNumFinal => {
                steps.push(cards::build_card_deck_num_final_step(
                    collected.attacker.len(),
                ));
            }
        }
    }

    steps
}

fn condition_uses_bloodtithe(condition: &ConditionType) -> bool {
    let mut stack = vec![condition];
    while let Some(cond) = stack.pop() {
        match cond {
            ConditionType::BloodPool | ConditionType::BloodPoolMax { .. } => return true,
            ConditionType::EnterFightAnd(conds) | ConditionType::EnterFightOr(conds) => {
                stack.extend(conds.iter());
            }
            _ => {}
        }
    }
    false
}

fn behavior_uses_bloodtithe(raw: &str) -> bool {
    let behavior_id = raw
        .split('#')
        .next()
        .and_then(|v| v.trim().parse::<i32>().ok())
        .unwrap_or(0);
    if behavior_id <= 0 {
        return false;
    }

    let cfg = config::configs::get();
    let behavior_type = cfg
        .skill_behavior
        .iter()
        .find(|row| row.id == behavior_id)
        .map(|row| row.r#type.as_str())
        .unwrap_or("");

    matches!(
        behavior_type,
        "LostLife"
            | "Bloodlust"
            | "BloodPoolValueChange"
            | "BloodPoolMaxChange"
            | "ConsumeBloodAddBuff"
            | "ConsumeBloodAddBuff2"
            | "RaspberryAddCount"
    )
}

fn skill_uses_bloodtithe(skill_id: i32) -> bool {
    if skill_id <= 0 {
        return false;
    }

    let cfg = config::configs::get();
    let effect_id = resolve_skill_effect_id(skill_id);
    let Some(effect) = cfg.skill_effect.iter().find(|row| row.id == effect_id) else {
        return false;
    };

    let conditions = [
        effect.condition1.as_str(),
        effect.condition2.as_str(),
        effect.condition3.as_str(),
        effect.condition4.as_str(),
        effect.condition5.as_str(),
        effect.condition6.as_str(),
        effect.condition7.as_str(),
        effect.condition8.as_str(),
        effect.condition9.as_str(),
        effect.condition10.as_str(),
    ];
    if conditions.iter().any(|raw| {
        let trimmed = raw.trim();
        !trimmed.is_empty() && condition_uses_bloodtithe(&parse_condition(trimmed).0)
    }) {
        return true;
    }

    let behaviors = [
        effect.behavior1.as_str(),
        effect.behavior2.as_str(),
        effect.behavior3.as_str(),
        effect.behavior4.as_str(),
        effect.behavior5.as_str(),
        effect.behavior6.as_str(),
        effect.behavior7.as_str(),
        effect.behavior8.as_str(),
        effect.behavior9.as_str(),
        effect.behavior10.as_str(),
    ];
    behaviors.iter().any(|raw| {
        let trimmed = raw.trim();
        !trimmed.is_empty() && behavior_uses_bloodtithe(trimmed)
    })
}

fn buff_uses_bloodtithe(buff_id: i32) -> bool {
    buff_has_bloodpool(buff_id)
        || buff_get_raspberry_params(buff_id).is_some()
        || buff_get_blood_pool_ex_point_params(buff_id).is_some()
}

fn seed_bloodtithe_from_fight(ctx: &mut FightContext<'_>) {
    if ctx.mechanics.bloodtithe.initialized {
        return;
    }

    let mut skill_ids = Vec::new();
    let mut buff_ids = Vec::new();

    if let Some(attacker) = ctx.fight.attacker.as_ref() {
        for entity in attacker.entitys.iter().chain(attacker.sub_entitys.iter()) {
            if entity.position.unwrap_or(-1) <= 0 {
                continue;
            }
            skill_ids.extend(entity.passive_skill.iter().copied().filter(|id| *id > 0));
            skill_ids.extend(entity.skill_group1.iter().copied().filter(|id| *id > 0));
            skill_ids.extend(entity.skill_group2.iter().copied().filter(|id| *id > 0));
            if let Some(ex_skill) = entity.ex_skill
                && ex_skill > 0
            {
                skill_ids.push(ex_skill);
            }
            buff_ids.extend(entity.buffs.iter().filter_map(|b| b.buff_id));
            buff_ids.extend(entity.no_effect_buffs.iter().filter_map(|b| b.buff_id));
        }
    }

    if buff_ids.into_iter().any(buff_uses_bloodtithe)
        || skill_ids.into_iter().any(skill_uses_bloodtithe)
    {
        ctx.mechanics.bloodtithe.initialized = true;
    }
}

fn is_unconditional_battle_rule_skill(skill_id: i32) -> bool {
    let effect_id = resolve_skill_effect_id(skill_id);
    let Some(behaviors) = SKILL_CACHE.get(&effect_id) else {
        return false;
    };
    !behaviors.is_empty()
        && behaviors
            .iter()
            .all(|b| matches!(b.condition, ConditionType::None))
}
