use sonettobuf::Fight;
use crate::state::battle::event::Event;
use crate::state::battle::manager::fight_data_mgr::Managers;
mod r#type;
use r#type::{behaviour_type, BehaviourType};
use super::target::Target;

mod ex_point;
mod add_buff;
mod add_act;
mod attr_modify;
pub(crate) mod stats;
mod disperse;
mod skill_rate;
mod misc;
mod catapult;
pub(crate) mod random;
mod magic_circle;
mod nuodika_damage;
mod poison_priority;
pub(crate) mod damage;
mod heal;
mod empathy;
mod dot_settle;
mod lost_life;
mod bloodtithe;
mod direct_skill;

#[derive(Debug, Clone)]
pub struct Behaviour {
    pub raw: String,
    pub target: i32,
}

pub fn is_attr_fix(raw: &str) -> bool {
    let id: i32 = raw.split('#').next().and_then(|v| v.parse().ok()).unwrap_or(0);
    matches!(
        behaviour_type(id),
        Some(BehaviourType::_10004AttrFix)
            | Some(BehaviourType::_10011AttrFixBuff)
            | Some(BehaviourType::_60033AttrFixByLoseHp)
    )
}

pub fn calculate_bonus(
    fight: &Fight,
    managers: &Managers,
    entity_uid: i64,
    raw: &str,
    beh_target: i32,
    count: i32,
) -> std::collections::HashMap<(i64, i32), i32> {
    let targets = Target::from_id(beh_target).entities(fight, entity_uid);
    let mut out = std::collections::HashMap::new();
    for uid in targets {
        let map = attr_modify::calculate_bonus(fight, managers, uid, raw, count);
        for (k, v) in map {
            *out.entry(k).or_insert(0) += v;
        }
    }
    out
}

pub fn parse(raw: &str, beh_target: i32) -> Vec<Behaviour> {
    raw.split('|')
        .filter(|s| !s.is_empty())
        .map(|seg| Behaviour { raw: seg.to_string(), target: beh_target })
        .collect()
}

pub fn execute(
    fight: &Fight,
    managers: &mut Managers,
    mechanics: &mut crate::state::battle::mechanics::Mechanics,
    executor: &mut crate::state::battle::skill::SkillExecutor,
    rng: &mut rand::rngs::StdRng,
    entity_uid: i64,
    skill_id: i32,
    raw: &str,
    beh_target: i32,
    count: i32,
) -> Vec<Event> {
    let targets = Target::from_id(beh_target).entities(fight, entity_uid);
    let id: i32 = raw.split('#').next().and_then(|v| v.parse().ok()).unwrap_or(0);
    match behaviour_type(id) {
        Some(beh @ (BehaviourType::_20002AddExPoint
        | BehaviourType::_60174ConsumeExPointAddAttr)) => ex_point::execute(fight, managers, mechanics, executor, rng, targets, entity_uid, raw, count, beh, skill_id),
        Some(beh @ (BehaviourType::_1AddBuff
        | BehaviourType::_20005AddBuffRound
        | BehaviourType::_20017AddBuffRound2
        | BehaviourType::_60210ConsumeBloodAddBuff
        | BehaviourType::_60211ConsumeBloodAddBuff2)) => add_buff::execute(fight, managers, mechanics, executor, rng, targets, entity_uid, raw, count, beh, skill_id),
        Some(BehaviourType::_40003AddAct) | Some(BehaviourType::_50006AddActHero) => add_act::execute(managers, targets, raw, count),
        Some(beh @ (BehaviourType::_20010Bloodlust
        | BehaviourType::_20011AverageLife
        | BehaviourType::_50017ChangePower
        | BehaviourType::_50037ChangePower)) => {
            stats::execute(fight, managers, mechanics, executor, rng, targets, raw, count, beh)
        }
        Some(beh @ (BehaviourType::_30003Disperse1
        | BehaviourType::_30004Disperse2
        | BehaviourType::_30008Disperse1
        | BehaviourType::_30009Disperse2
        | BehaviourType::_30016Disperse3
        | BehaviourType::_30017Disperse4
        | BehaviourType::_90002Disperse2
        | BehaviourType::_60010DisperseForce2
        | BehaviourType::_60011DisperseForce1
        | BehaviourType::_20003Purify1
        | BehaviourType::_20004Purify2
        | BehaviourType::_20020PurifyX
        | BehaviourType::_50014ConsumeBuffByTypeId
        | BehaviourType::_50016ConsumeBuffByTypeId2
        | BehaviourType::_60176ReplaceBuff2)) => {
            disperse::execute(fight, managers, mechanics, executor, rng, targets, entity_uid, raw, count, beh, skill_id)
        }
        Some(beh @ (BehaviourType::_10001SkillRateUp
        | BehaviourType::_10002SkillRateUp1
        | BehaviourType::_10003SkillRateUp2
        | BehaviourType::_10009SkillRateUpExPoint
        | BehaviourType::_10012SkillRateUpBuffType
        | BehaviourType::_40015Rouge2MusicBlueBallSkillRateUp
        | BehaviourType::_60028ConsumePowerSkillRateUp)) => {
            skill_rate::execute(fight, managers, mechanics, executor, rng, entity_uid, targets, raw, count, beh)
        }
        Some(beh @ (BehaviourType::_60008Summon
        | BehaviourType::_60013SummonSp
        | BehaviourType::_60056SummonSp2
        | BehaviourType::_40006MonsterChange
        | BehaviourType::_40008MonsterChangeClearSelfCard
        | BehaviourType::_60015Kill
        | BehaviourType::_60018Kill
        | BehaviourType::_60019KillTargets
        | BehaviourType::_100017IgnoreSkillConfigDamageRate)) => {
            misc::execute(fight, managers, mechanics, executor, rng, entity_uid, targets, raw, count, beh)
        }
        Some(beh @ BehaviourType::_60074CatapultBuff) => {
            catapult::execute(fight, managers, mechanics, executor, rng, targets, entity_uid, raw, count, beh)
        }
        Some(beh @ (BehaviourType::_20021AddBuffRanId | BehaviourType::_20022AddBuffRanTypeId | BehaviourType::_20023AddBuffRanTypeGroup)) => {
            random::execute(fight, managers, mechanics, executor, rng, targets, entity_uid, raw, beh)
        }
        Some(beh @ (BehaviourType::_50019AddMagicCircle | BehaviourType::_60163MagicCircleAddRound | BehaviourType::_60076MagicCircleAttr | BehaviourType::_50020RemoveAllMagicCircle | BehaviourType::_50021RemoveMagicCircleById | BehaviourType::_60270UpdateWangQiMagicCircle | BehaviourType::_60272ChangeElectricMagicCircleProgress)) => {
            magic_circle::execute(fight, managers, mechanics, executor, rng, targets, entity_uid, raw, count, beh)
        }
        Some(beh @ BehaviourType::_60209NuoDiKaDamage) => {
            nuodika_damage::execute(fight, managers, mechanics, executor, rng, targets, entity_uid, skill_id, raw, count, beh)
        }
        Some(beh @ (BehaviourType::_10006Damage
        | BehaviourType::_10008Damage2
        | BehaviourType::_20008Detonate
        | BehaviourType::_20009Detonate2
        | BehaviourType::_60237Detonate3
        | BehaviourType::_30014OriginDamage
        | BehaviourType::_60215InjurySaveDamage
        | BehaviourType::_60127OriginDamageByAttrAndBuffGroupSize)) => {
            damage::execute(fight, managers, mechanics, executor, rng, targets, entity_uid, skill_id, raw, count, beh)
        }
        Some(beh @ (BehaviourType::_20001Heal
        | BehaviourType::_90001Heal
        | BehaviourType::_60232HealByTwoAttr
        | BehaviourType::_20012HealCantCrit
        | BehaviourType::_20016HealCantCrit
        | BehaviourType::_20018HealCantCrit)) => {
            heal::execute(fight, managers, mechanics, executor, rng, targets, entity_uid, raw, count, beh)
        }
        Some(beh @ (BehaviourType::_60038OriginDamageFromInjuryBankBuff
        | BehaviourType::_60052OriginDamageFromInjuryBankBuff
        | BehaviourType::_60039RealDamageSelfAndAddBuffToTarget
        | BehaviourType::_60040ClearInjuryBankBuffOriginDamage)) => {
            empathy::execute(fight, managers, mechanics, executor, rng, targets, entity_uid, raw, count, beh, skill_id)
        }
        Some(beh @ BehaviourType::_60073SettleDotAndCostDotDuration) => {
            dot_settle::execute(fight, managers, mechanics, executor, rng, targets, entity_uid, raw, count, beh)
        }
        Some(beh @ (BehaviourType::_30005LostLife
        | BehaviourType::_30006LostLife
        | BehaviourType::_30010LostLifeNotFixed
        | BehaviourType::_30018LostLife
        | BehaviourType::_60226LostLife2
        | BehaviourType::_60216DamageRealLostLife
        | BehaviourType::_60213SurvivalHealth
        | BehaviourType::_60146OriginDamageByTeamAttr)) => {
            lost_life::execute(fight, managers, mechanics, executor, rng, targets, entity_uid, skill_id, raw, count, beh)
        }
        Some(beh @ (BehaviourType::_60190BloodPoolMaxChange
        | BehaviourType::_60191BloodPoolValueChange
        | BehaviourType::_60199ConsumeBloodPoolHeal)) => {
            bloodtithe::execute(fight, managers, mechanics, executor, rng, targets, entity_uid, raw, count, beh)
        }
        Some(BehaviourType::_60112AddTargetBuffByPoison) => {
            poison_priority::execute(fight, managers, mechanics, executor, rng, entity_uid, skill_id, raw)
        }
        Some(beh @ (BehaviourType::_50008DirectUseSkill
        | BehaviourType::_60053DirectUseSkill2
        | BehaviourType::_60014DirectUseSkillPrev
        | BehaviourType::_50012DirectUseSkillNoAct
        | BehaviourType::_50038DirectUseSkillNoAct2
        | BehaviourType::_60223DirectUseSkillNotExtra
        | BehaviourType::_50039DirectUseSkillCard
        | BehaviourType::_60156DirectUseSkillByBuff
        | BehaviourType::_60175DirectUseBigSkill
        | BehaviourType::_50010DirectUseGroupAndStarSkill
        | BehaviourType::_50036ConsumePowerDirectUseSkill
        | BehaviourType::_60188ConsumePowerUseSkill
        | BehaviourType::_60196DirectUseExSkillNoConsumeExPoint
        | BehaviourType::_60262PerConsumeExPointDirectUseSkill
        | BehaviourType::_100021ConsumeBloodPoolDirectUseSkill
        | BehaviourType::_60225RandomUseSkill
        | BehaviourType::_60239RandomUseSkillWithDice)) => {
            direct_skill::execute(fight, managers, mechanics, executor, rng, entity_uid, skill_id, targets, raw, beh)
        }
        Some(other) => { tracing::warn!("unimplemented behaviour type: {:?}", other); vec![] }
        None => vec![],
    }
}