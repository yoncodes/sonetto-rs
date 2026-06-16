use sonettobuf::{ActEffect, CardInfo, FightStep, fight_step};

use crate::state::battle::{context::FightContext, fight_step::ActEffectBuilder};

pub fn build_temp_card_step(ctx: &mut FightContext<'_>, uids: &[i64]) -> Option<FightStep> {
    let cfg = config::configs::get();
    let mut effects: Vec<ActEffect> = Vec::new();

    for &uid in uids {
        for instance in ctx.managers.buff_mgr.get(uid).to_vec() {
            let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == instance.buff_id) else {
                continue;
            };
            for entry in buff_cfg.features.split('|') {
                let parts: Vec<&str> = entry.split('#').collect();
                let act_id: i32 = parts.first().and_then(|v| v.parse().ok()).unwrap_or(0);
                let act_type = cfg
                    .buff_act
                    .iter()
                    .find(|a| a.id == act_id)
                    .map(|a| a.r#type.as_str())
                    .unwrap_or("");
                if act_type != "AddSpTempCard" {
                    continue;
                }

                let ex_skill_id: i32 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                let model_id = ctx
                    .fight
                    .attacker
                    .as_ref()
                    .and_then(|a| a.entitys.iter().find(|e| e.uid == Some(uid)))
                    .and_then(|e| e.model_id)
                    .unwrap_or(0);
                let ex_max = ctx.managers.entity_mgr.get_ex_max(uid);

                let inner = FightStep {
                    act_type: Some(fight_step::ActType::Effect.into()),
                    from_id: Some(uid),
                    to_id: Some(uid),
                    act_id: Some(instance.buff_id),
                    act_effect: vec![
                        ActEffectBuilder::sp_card_add(uid, ex_skill_id, model_id as i64, 1),
                        ActEffectBuilder::change_to_temp_card(uid, ex_max.to_string(), 1),
                    ],
                    ..Default::default()
                };
                if ex_skill_id == 0 {
                    continue;
                }
                ctx.managers.deck_mgr.player_hand.push(CardInfo {
                    uid: Some(0),
                    skill_id: Some(ex_skill_id),
                    card_effect: Some(0),
                    temp_card: Some(true),
                    enchants: vec![],
                    card_type: Some(0),
                    hero_id: Some(0),
                    status: Some(0),
                    target_uid: Some(0),
                    extra_info: None,
                    energy: Some(0),
                    extra_infos: vec![],
                    area_red_or_blue: Some(0),
                    heat_id: Some(0),
                    music_note: None,
                });
                effects
                    .push(crate::state::battle::fight_step::ActEffectBuilder::skill_wrapper(inner));
            }
        }
    }

    if effects.is_empty() {
        None
    } else {
        Some(
            crate::state::battle::fight_step::FightStepBuilder::effect()
                .with_many(effects)
                .build(),
        )
    }
}

pub fn build_temp_card_cleanup_step(ctx: &mut FightContext<'_>, uids: &[i64]) -> Option<FightStep> {
    let cfg = config::configs::get();
    let mut effects: Vec<ActEffect> = Vec::new();

    for &uid in uids {
        let to_remove: Vec<_> = ctx
            .managers
            .buff_mgr
            .get(uid)
            .iter()
            .filter(|instance| {
                cfg.skill_buff
                    .iter()
                    .find(|b| b.id == instance.buff_id)
                    .map(|b| {
                        b.features.split('|').any(|entry| {
                            let parts: Vec<&str> = entry.split('#').collect();
                            let act_id: i32 =
                                parts.first().and_then(|v| v.parse().ok()).unwrap_or(0);
                            cfg.buff_act
                                .iter()
                                .find(|a| a.id == act_id)
                                .map(|a| a.r#type == "AddSpTempCard")
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            })
            .map(|i| (i.uid, i.buff_id, i.from_uid))
            .collect();

        for (buff_uid, buff_id, from_uid) in to_remove {
            effects.push(
                crate::state::battle::fight_step::ActEffectBuilder::buff_del(
                    uid, buff_uid, buff_id, from_uid,
                ),
            );
            // intentionally NOT removing from buff_mgr - buff persists for ReplaceBuff2
        }
    }

    if effects.is_empty() {
        None
    } else {
        Some(
            crate::state::battle::fight_step::FightStepBuilder::effect()
                .with_many(effects)
                .build(),
        )
    }
}
