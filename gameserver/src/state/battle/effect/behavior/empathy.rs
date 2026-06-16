use crate::state::battle::effect::behavior::r#type::BehaviourType;
use crate::state::battle::{
    event::Event,
    fight_step::ActEffectBuilder,
    heroes::kakania::build_insight_iii_threshold_heals,
    manager::fight_data_mgr::Managers,
    mechanics::{
        Mechanics,
        empathy::{EmpathyState, empathy_default_buff_id, empathy_type_id},
    },
    skill::{SkillExecutor, buff},
    types::condition::ConditionType,
    utils::apply_real_hurt_fix,
};
use rand::rngs::StdRng;
use sonettobuf::Fight;

const SOLACE_SKILL_IDS: [i32; 3] = [30800121, 30800122, 30800123];
const SOLACE_CONFIG_EFFECT: i32 = 60039;
pub const SUBCONSCIOUS_BONUS_CONFIG_EFFECT: i32 = 60038;
pub const EX_CONSUME_CONFIG_EFFECT: i32 = 60040;

fn find_max_hp(fight: &Fight, uid: i64) -> i32 {
    fight
        .attacker
        .as_ref()
        .into_iter()
        .flat_map(|s| s.entitys.iter().chain(s.sub_entitys.iter()))
        .chain(
            fight
                .defender
                .as_ref()
                .into_iter()
                .flat_map(|s| s.entitys.iter().chain(s.sub_entitys.iter())),
        )
        .find(|e| e.uid == Some(uid))
        .and_then(|e| e.attr.as_ref())
        .and_then(|a| a.hp)
        .unwrap_or(0)
}

pub fn execute(
    fight: &Fight,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    executor: &mut SkillExecutor,
    _rng: &mut StdRng,
    targets: Vec<i64>,
    entity_uid: i64,
    raw: &str,
    _count: i32,
    beh_type: BehaviourType,
    skill_id: i32,
) -> Vec<Event> {
    let parts: Vec<&str> = raw.split('#').collect();
    let p1: i32 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);

    let target = targets.into_iter().next().unwrap_or(0);

    let effects: Vec<_> = match beh_type {
        // Kakania's Empathy Genesis bonus family. Both Subconscious's
        // basic (`60038#multiplier`) and the Insight III heal-trigger
        // reactive (skill 30800161/2/3 with `60052#multiplier`) share
        // the same skill_behavior `type`. Per the in-game ability
        // description for Subconscious: "1-target attack. Deals X%
        // Mental DMG plus (Current [Empathy] × multiplier%) Genesis
        // DMG." The Insight III variant fires the same bonus
        // emission off a heal trigger via the standard
        // OriginDamageFromInjuryBank path. Multiplier is permille
        // (1800 / 2200 / 2600 / 1000 / 1200 across Lv1-3 + Insight
        // ranks).
        BehaviourType::_60038OriginDamageFromInjuryBankBuff
        | BehaviourType::_60052OriginDamageFromInjuryBankBuff => {
            let current = mechanics.empathy.current(entity_uid);
            if current <= 0 || p1 <= 0 || target == 0 {
                return vec![];
            }
            let bonus = current.saturating_mul(p1) / 1000;
            if bonus <= 0 {
                return vec![];
            }
            vec![ActEffectBuilder::origin_damage(
                target,
                apply_real_hurt_fix(&managers.buff_mgr, target, bonus),
                Some(SUBCONSCIOUS_BONUS_CONFIG_EFFECT),
            )]
        }
        // Kakania's EX `Id, Ego and Superego` (skill 30800131,
        // `behavior1 = 60040#10000#1#0`) is the consume-and-bonus version
        // of `60038`: same Genesis-bonus formula, but the caster's
        // stored Empathy is reset to 0 once the bonus has been computed.
        // Per the in-game ability text: "1-target attack. Deals X% Mental
        // DMG plus (Current [Empathy] × multiplier%) Genesis DMG to the
        // target, resets [Empathy] to zero, and then starts recording
        // the damage the target takes for the round."
        BehaviourType::_60040ClearInjuryBankBuffOriginDamage => {
            let current = mechanics.empathy.current(entity_uid);
            if current <= 0 || p1 <= 0 || target == 0 {
                return vec![];
            }
            let bonus = current.saturating_mul(p1) / 1000;
            let max_hp = find_max_hp(fight, entity_uid);
            let cap = EmpathyState::storage_cap(&managers.buff_mgr, entity_uid, max_hp);
            let (empathy_buff_id, buff_uid) = managers
                .buff_mgr
                .find_instance_by_type_id(entity_uid, empathy_type_id())
                .map(|b| (b.buff_id, b.uid))
                .unwrap_or((empathy_default_buff_id(), 0));
            let mut effects = vec![
                ActEffectBuilder::storage_injury(
                    entity_uid,
                    0,
                    empathy_buff_id,
                    buff_uid,
                    entity_uid,
                    format!("770#0#{}", cap.max(0)),
                    Some(EX_CONSUME_CONFIG_EFFECT),
                ),
                ActEffectBuilder::origin_damage(
                    target,
                    apply_real_hurt_fix(&managers.buff_mgr, target, bonus),
                    Some(EX_CONSUME_CONFIG_EFFECT),
                ),
            ];
            mechanics
                .empathy
                .sync_buff_state(&mut managers.buff_mgr, entity_uid, 0, max_hp);
            effects
        }
        BehaviourType::_60039RealDamageSelfAndAddBuffToTarget => {
            let buff_id: i32 = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
            if !SOLACE_SKILL_IDS.contains(&skill_id) || target == entity_uid {
                return vec![];
            }
            let max_hp = find_max_hp(fight, entity_uid);
            if max_hp <= 0 {
                return vec![];
            }
            let self_damage = max_hp.saturating_mul(p1) / 1000;
            let storage_amount = EmpathyState::compute_storage_amount(self_damage);
            let cap = EmpathyState::storage_cap(&managers.buff_mgr, entity_uid, max_hp);
            let (current_total, thresholds_crossed) =
                mechanics.empathy.apply_storage_with_threshold(
                    &mut managers.buff_mgr,
                    entity_uid,
                    storage_amount,
                    max_hp,
                );
            let (empathy_buff_id, buff_uid) = managers
                .buff_mgr
                .find_instance_by_type_id(entity_uid, empathy_type_id())
                .map(|b| (b.buff_id, b.uid))
                .unwrap_or((empathy_default_buff_id(), 0));
            let mut effects = vec![mechanics.empathy.emit_storage_injury(
                entity_uid,
                current_total,
                empathy_buff_id,
                buff_uid,
                entity_uid,
                cap,
            )];
            effects.extend(build_insight_iii_threshold_heals(
                &mechanics.empathy,
                &managers.buff_mgr,
                fight,
                entity_uid,
                max_hp,
                thresholds_crossed,
            ));
            effects.push(ActEffectBuilder::origin_damage(
                entity_uid,
                apply_real_hurt_fix(&managers.buff_mgr, entity_uid, self_damage),
                Some(SOLACE_CONFIG_EFFECT),
            ));
            effects.extend(buff::apply(
                buff::BuffApplySpec::new(buff_id)
                    .caster(entity_uid)
                    .target(target)
                    .bloodpool(mechanics.bloodtithe.has_bloodpool())
                    .skill(skill_id)
                    .condition(0, &ConditionType::None),
                executor,
                fight,
                managers,
                mechanics,
            ));
            effects.push(ActEffectBuilder::effect_none(target));
            effects
        }
        _ => {
            tracing::warn!("unimplemented empathy behaviour: {:?}", beh_type);
            return vec![];
        }
    };
    effects
        .into_iter()
        .map(|e| Event::SerializedActEffect { effect: e })
        .collect()
}
