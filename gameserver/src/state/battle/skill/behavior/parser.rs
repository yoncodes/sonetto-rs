use super::super::super::BehaviorType;
use config::configs;

pub fn parse_behavior(raw: &str) -> BehaviorType {
    let cfg = configs::get();
    if raw.is_empty() {
        return BehaviorType::Unknown {
            raw: raw.to_string(),
        };
    }

    let parts: Vec<&str> = raw.split('#').collect();
    let id: i32 = parts[0].parse().unwrap_or(0);
    let p1: i32 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    let p2: i32 = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);

    // Recoleta temp-card behavior in live captures.
    // Keep this ID-based fallback stable even if behavior type rows drift across data bundles.
    if id == 60175 {
        return BehaviorType::DirectUseBigSkill;
    }
    // Live data bundles can label 60010 as DisperseForce2, but runtime payloads
    // use 60010#<buff_id>[#count] as an AddBuff lane.
    if id == 60010 {
        return BehaviorType::AddBuff {
            buff_id: p1,
            count: p2,
        };
    }
    if id == 60039 {
        return BehaviorType::RealDamageSelfAndAddBuffToTarget {
            amount_permille: p1,
            buff_id: p2,
        };
    }
    if id == 60040 {
        return BehaviorType::ConsumeInjuryBankAndDamage {
            multiplier_permille: p1,
        };
    }
    if id == 30009 && p1 > 0 {
        return BehaviorType::DisperseForce { buff_id: p1 };
    }
    if id == 60228 {
        return BehaviorType::AttrFix {
            attr_id: crate::state::battle::types::attr::AttrId::Cri as i32,
            amount: p1,
        };
    }
    
    if id == 60033 {
        let step_permille = p1;
        let attr_id = p2;
        let bonus_per_stack = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
        let max_stacks = parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0);
        return BehaviorType::AttrFixByLoseHp {
            step_permille,
            attr_id,
            bonus_per_stack,
            max_stacks,
        };
    }
    
    if id == 60073 {
        return BehaviorType::SettleDotAndCostDotDuration { rounds: p1 };
    }
    // `PoisonConvertToTargetBuff` (skill_behavior id 60110) — Willow's
    // basic1 `Hag's Bane Pose` (skill 31040113) carries
    // `60110#<cap>#<buff_id>` per skill_effect (e.g. `60110#5#31040013`
    // for Lv.3). Per the in-game text: "1-target attack. Deals X% Mental
    // DMG; if the target hit already has an instance of [Poison],
    // convert 1 instance of [Poison] into 1 stack of [Hag's Bane] that
    // lasts 2 rounds; up to N instances of Poison can be converted by
    // this effect." LIVE-side the BuffAdd actEffect carries layer = cap,
    // duration = skill_buff.duringTime — no explicit Poison removal
    // event, the engine just stamps the buff. Argument order swaps the
    // `AddBuff` convention (`buff_id#count`) so route by id and re-pack
    // into the existing AddBuff runtime.
    if id == 60110 {
        return BehaviorType::AddBuff {
            buff_id: p2,
            count: p1,
        };
    }
    if id == 60127 {
        let mode = p1;
        let attr_id = p2;
        let permille = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
        let group_id = parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0);
        return BehaviorType::OriginDamageByAttrAndBuffGroupSize {
            mode,
            attr_id,
            permille,
            group_id,
        };
    }
    // Some live data uses 20021#<baseSkillId>#<rank> to direct-cast a derived skill id.
    // Keep AddBuffRanId behavior for true buff pools (small ids), but route skill-like ids.
    if id == 20021 && p1 >= 10000 {
        return BehaviorType::DirectUseGroupAndStarSkill {
            group: p1,
            rank: p2,
        };
    }
    let behavior_type = cfg
        .skill_behavior
        .iter()
        .find(|b| b.id == id)
        .map(|b| b.r#type.as_str())
        .unwrap_or("");

    match behavior_type {
        "Damage" | "Damage2" | "Detonate" | "OriginDamage" | "OriginDamage2" => {
            BehaviorType::Damage { rate: p1 }
        }
        "Detonate2" => BehaviorType::Detonate2 {
            rate: p1,
            granted_buff_id: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        
        "OriginDamageFromInjuryBankBuff" => BehaviorType::OriginDamageFromInjuryBank {
            multiplier_permille: p1,
        },
        "Heal" => BehaviorType::Heal { rate: p1 },
        // `HealCantCrit` (ids 20012 / 20016 / 20018) encodes as
        // `act_id#?#attr_id#permille` and is sibling-emitted at the
        // parent step in LIVE — see the variant doc on
        // `BehaviorType::HealCantCrit`. Routed to a no-op here so the
        // host-wrapper executor stops emitting a wrong-target `et=4
        // num=1` leak; correct sibling-emission lives in future work.
        "HealCantCrit" => BehaviorType::HealCantCrit {
            attr_id: p2,
            permille: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        "HealByTwoAttr" => BehaviorType::HealByTwoAttr {
            missing_percent: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
            caster_hp_percent: parts.get(6).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        "AddBuff" | "AddBuffRound" | "AddBuffRound2" => BehaviorType::AddBuff {
            buff_id: p1,
            count: p2,
        },
        "CatapultBuff" => BehaviorType::CatapultBuff {
            primary_stacks: parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
            duration: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
            buff_id: parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0),
            catapult_stacks: parts.get(5).and_then(|v| v.parse().ok()).unwrap_or(0),
            catapult_cap: parts.get(6).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        "AddTargetBuffByPoison" => BehaviorType::AddTargetBuffByPoison {
            stack_count: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
            duration: parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
            buff_id: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
            max_targets: parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        "CreateAdditionalDamageAddBuff" => BehaviorType::AddBuff {
            buff_id: parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0),
            count: 0,
        },
        "ConsumeBloodAddBuff" => BehaviorType::ConsumeBloodAddBuff {
            consume: p1,
            buff_id: p2,
            count: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        "ConsumeBloodAddBuff2" => BehaviorType::ConsumeBloodAddBuff2 {
            consume: p1,
            buff_id: p2,
            count: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        "AddExPoint" | "AttrFixExPoint" => {
            if id == 20002 {
                BehaviorType::AddExPointWithMax { amount: p1 }
            } else {
                BehaviorType::AddExPoint { amount: p1 }
            }
        }
        "LostLife" => BehaviorType::LostLife {
            mode: p1,
            attr_id: p2,
            permille: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
            behavior_id: id,
        },
        "Bloodlust" => BehaviorType::Bloodlust { amount: p1 },
        "AverageLife" => BehaviorType::AverageLife,
        "BloodPoolValueChange" => BehaviorType::BloodPoolValueChange { amount: p1 },
        "BloodPoolMaxChange" => BehaviorType::BloodPoolMaxChange { amount: p1 },
        "AttrModify" => BehaviorType::AttrModify {
            attr_id: p1,
            amount: p2,
        },
        "BeAttackedAssassinate" => BehaviorType::BeAttackedAssassinate {
            attr_id: p1,
            amount: p2,
        },
        "ConsumeBuffByTypeId" => BehaviorType::ConsumeBuffByTypeId {
            type_id: p1,
            count: p2,
        },
        t if t.starts_with("DisperseForce") => BehaviorType::DisperseForce { buff_id: p1 },
        t if t.starts_with("Disperse") => BehaviorType::Disperse,
        t if t.starts_with("Purify") => BehaviorType::Purify,
        "ChangePower" => BehaviorType::ChangePower { amount: p1 },
        t if t.starts_with("AttrFix") => {
            // Most AttrFix-like behaviors are encoded as:
            //   behavior_id#attr_id#amount
            // Keep the parser permissive because some variants append extra params.
            let attr_id = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(p1);
            let amount = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(p2);
            BehaviorType::AttrFix { attr_id, amount }
        }
        "SkillRateUp" => BehaviorType::SkillRateUp { rate: p1 },
        "ConsumePowerDirectUseSkill" => BehaviorType::ConsumePowerDirectUseSkill {
            count: p1,
            skill_id: p2,
        },
        "DirectUseSkill" => BehaviorType::DirectUseSkill { skill_id: p1 },
        "DirectUseBigSkill" => BehaviorType::DirectUseBigSkill,
        "ConsumeExPointAddAttr" => BehaviorType::ConsumeExPointAddAttr {
            min_consume: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
            max_consume: parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        "SkillRateUpBySelfBuffType" => BehaviorType::SkillRateUpBySelfBuffType {
            buff_type_id: p1,
            rate: p2,
        },
        "SkillRateUpBuffType" => BehaviorType::SkillRateUpByBuffType {
            rate: p1,
            buff_types: parts
                .iter()
                .skip(3)
                .filter_map(|v| v.parse().ok())
                .collect(),
        },
        "RandomUseSkill" => BehaviorType::RandomUseSkill {
            raw: raw.to_string(),
        },
        "MonsterChange" => BehaviorType::MonsterChange {
            new_monster_id: p1,
            probability_permille: p2,
        },
        "Kill" => BehaviorType::Kill,
        "Summon" => BehaviorType::Summon { skill_id: p1 },
        "RaspberryAddCount" => BehaviorType::RaspberryAddCount {
            attr_id: p1,
            rate: p2,
        },
        "AddBuffRanId" => BehaviorType::AddBuffRanId {
            pool_buff_id: p1,
            count: p2,
        },
        "AddMagicCircle" | "MagicCircleAddRound" => BehaviorType::AddMagicCircle { circle_id: p1 },
        "MagicCircleAttr" => {
            // Encoding: `60076#side#attr#permille[#side2#attr2#permille2]...`.
            // `parts[0]` is the behavior id (60076); subsequent parts come
            // in (side, attr, permille) triples. `side`: 1 = caster's team,
            // 2 = opposing team.
            let mut modifiers = Vec::new();
            let mut i = 1;
            while i + 2 < parts.len() {
                let side: i32 = parts.get(i).and_then(|v| v.parse().ok()).unwrap_or(0);
                let attr_id: i32 = parts.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0);
                let permille: i32 = parts.get(i + 2).and_then(|v| v.parse().ok()).unwrap_or(0);
                if attr_id != 0 {
                    modifiers.push((side, attr_id, permille));
                }
                i += 3;
            }
            BehaviorType::MagicCircleAttr { modifiers }
        }
        "DirectUseGroupAndStarSkill" => BehaviorType::DirectUseGroupAndStarSkill {
            group: p1,
            rank: p2,
        },
        "ReplaceBuff2" => BehaviorType::ReplaceBuff2 {
            source_buff_ids: parts
                .get(1)
                .map(|p| p.split(',').filter_map(|v| v.parse().ok()).collect())
                .unwrap_or_default(),
            replacement_buff_id: parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
            duration: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
            count: parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(1),
        },
        "CrystalAddCard" => BehaviorType::CrystalAddCard,

        "ShellUseSkill" => BehaviorType::ShellUseSkill {
            group: p1,
            skill_id: p2,
        },
        "ShellAssign" => BehaviorType::ShellAssign {
            slot: p1,
            skill_id: p2,
        },
        "PurifyX" => BehaviorType::PurifyX {
            type_ids: parts[1..].iter().filter_map(|v| v.parse().ok()).collect(),
        },
        "IgnoreSkillConfigDamageRate" => BehaviorType::IgnoreSkillConfigDamageRate,

        "LostAllLifeByAttr" => BehaviorType::LostAllLifeByAttr {
            caster_attr: p1,
            caster_amount: p2,
            target_attr: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
            target_amount: parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0),
        },

        "DamageRealLostLife" => BehaviorType::DamageRealLostLife {
            buff_id: p1,
            duration: p2,
            rate: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        "NuoDiKaDamage" => BehaviorType::NuoDiKaDamage {
            primary_buff_id: p1,
            primary_rate: p2,
            secondary_buff_id: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
            secondary_rate: parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0),
            self_loss_param: parts.get(5).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        _ => {
            if !behavior_type.is_empty() {
                //tracing::warn!("Unhandled behavior type: {} (id={})", behavior_type, id);
            }
            BehaviorType::Unknown {
                raw: raw.to_string(),
            }
        }
    }
}
