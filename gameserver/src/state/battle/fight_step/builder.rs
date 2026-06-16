use crate::state::battle::types::effects::EffectType as BattleEffectType;
use crate::state::battle::{
    event_queue::{BattleEvent, serialize_leaf_event},
    manager::buff_mgr::{next_buff_uid_for_target, next_slave_buff_uid_for_target},
    types::buff::BuffLayerType,
    utils::buff_get_act_common_params,
};
use sonettobuf::{
    ActEffect, BuffActInfo, BuffInfo, CardInfo, Fight, FightHurtInfo, FightStep, MagicCircleInfo,
    effect_type_enum::EffectType, fight_hurt_info::DamageFromType, fight_step,
};

pub struct ActEffectBuilder {
    effect: ActEffect,
}

#[allow(dead_code)]
impl ActEffectBuilder {
    pub fn new(effect_type: i32, target_id: i64) -> Self {
        Self {
            effect: ActEffect {
                effect_type: Some(effect_type),
                target_id: Some(target_id),
                ..Default::default()
            },
        }
    }

    pub fn skill_wrapper(step: FightStep) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "skill_wrapper",
            with_effect_num = true,
            inner_act = step.act_id.unwrap_or(0),
            inner_from = step.from_id.unwrap_or(0)
        );
        Self::bare(EffectType::Fightstep as i32)
            .target_id(0)
            .effect_num(0)
            .fight_step(step)
            .build()
    }

    pub fn skill_wrapper_without_num(step: FightStep) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "skill_wrapper",
            with_effect_num = false,
            inner_act = step.act_id.unwrap_or(0),
            inner_from = step.from_id.unwrap_or(0)
        );
        Self::bare(EffectType::Fightstep as i32)
            .target_id(0)
            .fight_step(step)
            .build()
    }

    pub fn ex_point_change(target: i64, delta: i32) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "ex_point_change", target, delta);
        Self::new(EffectType::Expointchange as i32, target)
            .effect_num(delta)
            .build()
    }

    pub fn ex_point_change_with_config_effect(
        target: i64,
        delta: i32,
        config_effect: i32,
    ) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "ex_point_change",
            target,
            delta,
            config_effect
        );
        Self::new(EffectType::Expointchange as i32, target)
            .effect_num(delta)
            .config_effect(config_effect)
            .build()
    }

    pub fn moxie_change(target: i64, delta: i32) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "moxie_change", target, delta);
        Self::ex_point_change_with_config_effect(target, delta, 20002)
    }

    pub fn bloodpool_value_change(target: i64, team: i32, delta: i32) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "bloodpool_value_change",
            target,
            team,
            delta
        );
        Self::new(EffectType::Bloodpoolvaluechange as i32, target)
            .effect_num(team)
            .effect_num1(delta)
            .build()
    }

    pub fn bloodpool_max_change(team: i32, amount: i32) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "bloodpool_max_change",
            team,
            amount
        );
        Self::new(EffectType::Bloodpoolmaxchange as i32, 0)
            .effect_num(team)
            .effect_num1(amount)
            .build()
    }

    pub fn bloodpool_max_create(effect_num: i32) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "bloodpool_max_create",
            effect_num
        );
        Self::bare(EffectType::Bloodpoolmaxcreate as i32)
            .effect_num(effect_num)
            .build()
    }

    pub fn effect_none(target: i64) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "effect_none", target);
        Self::new(EffectType::None as i32, target).build()
    }

    pub fn effect_none_with_num(target: i64, effect_num: i32) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "effect_none_with_num",
            target,
            effect_num
        );
        Self::new(EffectType::None as i32, target)
            .effect_num(effect_num)
            .build()
    }

    pub fn storage_injury(
        target_uid: i64,
        amount: i32,
        buff_id: i32,
        buff_uid: i64,
        from_uid: i64,
        act_common_params: impl Into<String>,
        config_effect: Option<i32>,
    ) -> ActEffect {
        let act_common_params = act_common_params.into();
        tracing::trace!(
            target: "act_effects",
            kind = "storage_injury",
            target_uid,
            amount,
            buff_id,
            buff_uid,
            from_uid,
            ?config_effect,
            act_common_params = act_common_params.as_str()
        );
        let mut builder = Self::new(EffectType::Storageinjury as i32, target_uid)
            .effect_num(amount.max(0))
            .buff(BuffInfo {
                buff_id: Some(buff_id),
                duration: Some(0),
                uid: Some(buff_uid),
                ex_info: Some(0),
                from_uid: Some(from_uid),
                count: Some(0),
                act_common_params: Some(act_common_params),
                layer: Some(0),
                r#type: Some(BuffLayerType::Normal as i32),
                act_info: vec![],
            });
        if let Some(config_effect) = config_effect {
            builder = builder.config_effect(config_effect);
        }
        builder.build()
    }

    pub fn buff_add(target_uid: i64, from_uid: i64, buff_id: i32, layer: i32) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "buff_add",
            target_uid,
            from_uid,
            buff_id,
            layer
        );
        Self::build_buff_add(target_uid, from_uid, buff_id, layer, 0, false)
    }

    pub fn buff_add_with_snapshot(
        target_uid: i64,
        from_uid: i64,
        buff_id: i32,
        buff_uid: i64,
        duration: i32,
        count: i32,
        act_common_params: String,
        layer: i32,
        buff_type: i32,
        config_effect: Option<i32>,
    ) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "buff_add_with_snapshot",
            target_uid,
            from_uid,
            buff_id,
            buff_uid,
            duration,
            count,
            layer,
            buff_type,
            ?config_effect,
            act_common_params = act_common_params.as_str()
        );
        let mut builder = Self::new(EffectType::Buffadd as i32, target_uid)
            .effect_num(buff_id)
            .buff(BuffInfo {
                buff_id: Some(buff_id),
                duration: Some(duration),
                uid: Some(buff_uid),
                ex_info: Some(0),
                from_uid: Some(from_uid),
                count: Some(count),
                act_common_params: Some(act_common_params),
                layer: Some(layer),
                r#type: Some(buff_type),
                act_info: vec![],
            });
        if let Some(config_effect) = config_effect {
            builder = builder.config_effect(config_effect);
        }
        builder.build()
    }

    pub fn buff_add_with_count(
        target_uid: i64,
        from_uid: i64,
        buff_id: i32,
        layer: i32,
        count: i32,
    ) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "buff_add_with_count",
            target_uid,
            from_uid,
            buff_id,
            layer,
            count
        );
        Self::build_buff_add(target_uid, from_uid, buff_id, layer, count, false)
    }

    pub fn buff_add_slave(target_uid: i64, from_uid: i64, buff_id: i32, layer: i32) -> ActEffect {
        let cfg = config::configs::get();
        let initial_count = if layer > 1 {
            1
        } else {
            cfg.skill_buff
                .iter()
                .find(|b| b.id == buff_id)
                .map(|b| b.effect_count)
                .unwrap_or(0)
        };
        tracing::trace!(
            target: "act_effects",
            kind = "buff_add_slave",
            target_uid,
            from_uid,
            buff_id,
            layer,
            initial_count
        );
        Self::build_buff_add(target_uid, from_uid, buff_id, layer, initial_count, true)
    }

    pub fn buff_update(
        target_uid: i64,
        from_uid: i64,
        buff_id: i32,
        buff_uid: i64,
        count: i32,
        layer: i32,
    ) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "buff_update",
            target_uid,
            from_uid,
            buff_id,
            buff_uid,
            count,
            layer
        );
        Self::buff_update_with_snapshot(
            target_uid,
            from_uid,
            buff_id,
            buff_uid,
            0,
            count,
            buff_get_act_common_params(buff_id),
            layer,
            BuffLayerType::Normal as i32,
        )
    }

    pub fn buff_update_with_snapshot(
        target_uid: i64,
        from_uid: i64,
        buff_id: i32,
        buff_uid: i64,
        duration: i32,
        count: i32,
        act_common_params: String,
        layer: i32,
        buff_type: i32,
    ) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "buff_update_with_snapshot",
            target_uid,
            from_uid,
            buff_id,
            buff_uid,
            duration,
            count,
            layer,
            buff_type,
            act_common_params = act_common_params.as_str()
        );
        Self::new(EffectType::Buffupdate as i32, target_uid)
            .effect_num(0)
            .buff(BuffInfo {
                buff_id: Some(buff_id),
                duration: Some(duration),
                uid: Some(buff_uid),
                ex_info: Some(0),
                from_uid: Some(from_uid),
                count: Some(count),
                act_common_params: Some(act_common_params),
                layer: Some(layer),
                r#type: Some(buff_type),
                act_info: vec![],
            })
            .build()
    }

    pub fn buff_del(target: i64, buff_uid: i64, buff_id: i32, from_uid: i64) -> ActEffect {
        let cfg = config::configs::get();
        let layer = cfg
            .skill_buff
            .iter()
            .find(|b| b.id == buff_id)
            .map(|b| if b.features.is_empty() { 0 } else { 1 })
            .unwrap_or(0);
        tracing::trace!(
            target: "act_effects",
            kind = "buff_del",
            target,
            buff_uid,
            buff_id,
            from_uid,
            layer
        );
        Self::new(EffectType::Buffdel as i32, target)
            .buff(BuffInfo {
                uid: Some(buff_uid),
                buff_id: Some(buff_id),
                from_uid: Some(from_uid),
                layer: Some(layer),
                ..Default::default()
            })
            .build()
    }

    pub fn buff_del_with_snapshot(
        target: i64,
        buff_uid: i64,
        buff_id: i32,
        from_uid: i64,
        duration: i32,
        count: i32,
        act_common_params: String,
        layer: i32,
        buff_type: i32,
    ) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "buff_del_with_snapshot",
            target,
            buff_uid,
            buff_id,
            from_uid,
            duration,
            count,
            layer,
            buff_type,
            act_common_params = act_common_params.as_str()
        );
        Self::new(EffectType::Buffdel as i32, target)
            .buff(BuffInfo {
                buff_id: Some(buff_id),
                duration: Some(duration),
                uid: Some(buff_uid),
                ex_info: Some(0),
                from_uid: Some(from_uid),
                count: Some(count),
                act_common_params: Some(act_common_params),
                layer: Some(layer),
                r#type: Some(buff_type),
                act_info: vec![],
            })
            .build()
    }

    pub fn damage(target: i64, amount: i32, config_effect: Option<i32>) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "damage",
            target,
            amount,
            ?config_effect
        );
        Self::build_numeric_effect(
            EffectType::Damage as i32,
            target,
            amount,
            config_effect,
            None,
            None,
        )
    }

    pub fn damage_default(target: i64, amount: i32) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "damage_default", target, amount);
        Self::damage(target, amount, Some(30006))
    }

    pub fn damage_with_buff_act(target: i64, amount: i32, buff_act_id: i32) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "damage_with_buff_act",
            target,
            amount,
            buff_act_id
        );
        Self::build_numeric_effect(
            EffectType::Damage as i32,
            target,
            amount,
            None,
            Some(buff_act_id),
            None,
        )
    }

    pub fn damage_skill(
        target: i64,
        amount: i32,
        config_effect: i32,
        skill_id: i32,
        from_uid: i64,
    ) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "damage_skill",
            target,
            amount,
            config_effect,
            skill_id,
            from_uid
        );
        let effect = serialize_leaf_event(BattleEvent::Damage {
            target,
            amount,
            is_crit: false,
            hurt_info: FightHurtInfo {
                damage: Some(amount),
                reduce_hp: Some(0),
                hurt_effect: Some(EffectType::Damage as i32),
                damage_from_type: Some(DamageFromType::SkillEffect as i32),
                config_effect: Some(config_effect),
                effect_id: Some(skill_id),
                skill_id: Some(skill_id),
                from_uid: Some(from_uid),
                ..Default::default()
            },
            from: from_uid,
            skill_id: Some(skill_id),
        });
        Self::damage_with_hurt(
            target,
            amount,
            Some(config_effect),
            effect.hurt_info.unwrap_or_default(),
        )
    }

    pub fn damage_with_hurt(
        target: i64,
        amount: i32,
        config_effect: Option<i32>,
        hurt_info: FightHurtInfo,
    ) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "damage_with_hurt",
            target,
            amount,
            ?config_effect,
            ?hurt_info
        );
        Self::build_numeric_effect(
            EffectType::Damage as i32,
            target,
            amount,
            config_effect,
            None,
            Some(hurt_info),
        )
    }

    pub fn damage_with_buff_hurt(
        target: i64,
        amount: i32,
        buff_act_id: i32,
        hurt_info: FightHurtInfo,
    ) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "damage_with_buff_hurt",
            target,
            amount,
            buff_act_id,
            ?hurt_info
        );
        Self::build_numeric_effect(
            EffectType::Damage as i32,
            target,
            amount,
            None,
            Some(buff_act_id),
            Some(hurt_info),
        )
    }

    pub fn damage_buff_with_uid(
        target: i64,
        amount: i32,
        buff_act_id: i32,
        from_uid: i64,
        buff_uid: i64,
    ) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "damage_buff_with_uid",
            target,
            amount,
            buff_act_id,
            from_uid,
            buff_uid
        );
        Self::damage_with_buff_hurt(
            target,
            amount,
            buff_act_id,
            FightHurtInfo {
                damage: Some(amount),
                hurt_effect: Some(EffectType::Damage as i32),
                damage_from_type: Some(DamageFromType::Buff as i32),
                buff_act_id: Some(buff_act_id),
                from_uid: Some(from_uid),
                buff_uid: Some(buff_uid as i32),
                ..Default::default()
            },
        )
    }

    pub fn crit(target: i64, amount: i32, config_effect: Option<i32>) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "crit",
            target,
            amount,
            ?config_effect
        );
        Self::build_numeric_effect(
            EffectType::Crit as i32,
            target,
            amount,
            config_effect,
            None,
            None,
        )
    }

    pub fn crit_with_hurt(
        target: i64,
        amount: i32,
        config_effect: Option<i32>,
        hurt_info: FightHurtInfo,
    ) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "crit_with_hurt",
            target,
            amount,
            ?config_effect,
            ?hurt_info
        );
        Self::build_numeric_effect(
            EffectType::Crit as i32,
            target,
            amount,
            config_effect,
            None,
            Some(hurt_info),
        )
    }

    pub fn heal(target: i64, amount: i32, config_effect: Option<i32>) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "heal",
            target,
            amount,
            ?config_effect
        );
        Self::build_numeric_effect(
            EffectType::Heal as i32,
            target,
            amount,
            config_effect,
            None,
            None,
        )
    }

    pub fn origin_damage(target: i64, amount: i32, config_effect: Option<i32>) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "origin_damage",
            target,
            amount,
            ?config_effect
        );
        Self::build_numeric_effect(
            EffectType::Origindamage as i32,
            target,
            amount,
            config_effect,
            None,
            None,
        )
    }

    pub fn origin_damage_with_hurt(
        target: i64,
        amount: i32,
        buff_act_id: Option<i32>,
        hurt_info: FightHurtInfo,
    ) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "origin_damage_with_hurt",
            target,
            amount,
            ?buff_act_id,
            ?hurt_info
        );
        Self::build_numeric_effect(
            EffectType::Origindamage as i32,
            target,
            amount,
            None,
            buff_act_id,
            Some(hurt_info),
        )
    }

    pub fn origin_crit(target: i64, amount: i32, config_effect: Option<i32>) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "origin_crit",
            target,
            amount,
            ?config_effect
        );
        Self::build_numeric_effect(
            EffectType::Origincrit as i32,
            target,
            amount,
            config_effect,
            None,
            None,
        )
    }

    pub fn dead(target: i64) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "dead", target);
        Self::new(EffectType::Dead as i32, target)
            .effect_num(0)
            .build()
    }

    pub fn current_hp_change(target: i64, amount: i32) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "current_hp_change",
            target,
            amount
        );
        Self::new(EffectType::Currenthpchange as i32, target)
            .effect_num(amount)
            .build()
    }

    pub fn max_hp_change(target: i64, amount: i32, buff_act_id: Option<i32>) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "max_hp_change",
            target,
            amount,
            ?buff_act_id
        );
        let mut builder = Self::new(EffectType::Maxhpchange as i32, target).effect_num(amount);
        if let Some(buff_act_id) = buff_act_id {
            builder = builder.buff_act_id(buff_act_id);
        }
        builder.build()
    }

    pub fn attr(target: i64) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "attr", target);
        Self::new(EffectType::Attr as i32, target)
            .effect_num(0)
            .build()
    }

    pub fn power_change(target: Option<i64>, amount: i32, config_effect: Option<i32>) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "power_change",
            ?target,
            amount,
            ?config_effect
        );
        let mut builder = if let Some(target) = target {
            Self::new(EffectType::Powerchange as i32, target)
        } else {
            Self::bare(EffectType::Powerchange as i32)
        }
        .effect_num(amount);
        if let Some(config_effect) = config_effect {
            builder = builder.config_effect(config_effect);
        }
        builder.build()
    }

    pub fn damage_from_absorb(target: i64, amount: i32) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "damage_from_absorb",
            target,
            amount
        );
        Self::new(EffectType::Damagefromabsorb as i32, target)
            .effect_num(amount)
            .build()
    }

    pub fn injury_bank_heal(target: i64, amount: i32) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "injury_bank_heal",
            target,
            amount
        );
        Self::new(EffectType::Injurybankheal as i32, target)
            .effect_num(amount)
            .build()
    }

    pub fn card_deck_num(deck_num: i32) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "card_deck_num", deck_num);
        Self::bare(EffectType::Carddecknum as i32)
            .effect_num(deck_num)
            .team_type(1)
            .build()
    }

    pub fn card_deck_num_with_target(target: i64, deck_num: i32) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "card_deck_num_with_target",
            target,
            deck_num
        );
        Self::new(BattleEffectType::CardDeckNum as i32, target)
            .effect_num(deck_num)
            .team_type(1)
            .build()
    }


    pub fn cards_push(card_info_list: Vec<CardInfo>, team_type: Option<i32>) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "cards_push",
            card_count = card_info_list.len(),
            ?team_type
        );
        let mut builder = Self::bare(EffectType::Cardspush as i32).card_info_list(card_info_list);
        if let Some(team_type) = team_type {
            builder = builder.team_type(team_type);
        }
        builder.build()
    }

    pub fn remove_entity_cards(target_uid: i64, team_type: Option<i32>) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "remove_entity_cards", target_uid);
        let inner = Self::new(BattleEffectType::RemoveEntityCards as i32, target_uid)
            .team_type(team_type.unwrap_or(1))
            .build();
        let step = effect_container_step(0, 0, 0, vec![inner]);
        Self::skill_wrapper(step)
    }

    pub fn new_change_wave(fight: Fight) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "new_change_wave",
            wave = fight.cur_wave.unwrap_or(0)
        );
        Self::bare(EffectType::Newchangewave as i32)
            .effect_num(0)
            .fight(fight)
            .build()
    }

    pub fn fight_hurt_detail(target: i64, hurt_info: FightHurtInfo) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "fight_hurt_detail",
            target,
            damage = hurt_info.damage.unwrap_or(0),
            hurt_effect = hurt_info.hurt_effect.unwrap_or(0),
            damage_from_type = hurt_info.damage_from_type.unwrap_or(0)
        );
        Self::new(EffectType::Fighthurtdetail as i32, target)
            .hurt_info(hurt_info)
            .build()
    }

    pub fn hurt_detail_skill(
        target: i64,
        damage: i32,
        skill_id: i32,
        damage_type: i32,
        hurt_effect: i32,
    ) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "hurt_detail_skill",
            target,
            damage,
            skill_id,
            damage_type,
            hurt_effect
        );
        Self::fight_hurt_detail(
            target,
            FightHurtInfo {
                damage: Some(damage),
                reduce_hp: Some(damage),
                reduce_shield: Some(0),
                career_restraint: Some(false),
                critical: Some(false),
                assassinate: Some(false),
                hurt_effect: Some(hurt_effect),
                damage_from_type: Some(damage_type),
                config_effect: Some(30006),
                effect_id: Some(skill_id),
                skill_id: Some(skill_id),
                from_uid: Some(target),
                ..Default::default()
            },
        )
    }

    pub fn hurt_detail_buff(
        target: i64,
        damage: i32,
        skill_id: i32,
        damage_type: i32,
        buff_id: i32,
    ) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "hurt_detail_buff",
            target,
            damage,
            skill_id,
            damage_type,
            buff_id
        );
        Self::fight_hurt_detail(
            target,
            FightHurtInfo {
                damage: Some(damage),
                reduce_hp: Some(damage),
                reduce_shield: Some(0),
                career_restraint: Some(false),
                critical: Some(false),
                assassinate: Some(false),
                hurt_effect: Some(EffectType::Damage as i32),
                damage_from_type: Some(damage_type),
                config_effect: Some(0),
                buff_act_id: Some(buff_id),
                effect_id: Some(skill_id),
                skill_id: Some(skill_id),
                from_uid: Some(target),
                ..Default::default()
            },
        )
    }

    pub fn heal_crit(target: i64, amount: i32) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "heal_crit", target, amount);
        Self::new(EffectType::Healcrit as i32, target)
            .effect_num(amount)
            .build()
    }

    pub fn cure(target: i64) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "cure", target);
        Self::new(BattleEffectType::Cure as i32, target)
            .effect_num(0)
            .build()
    }

    pub fn cure_up_by_lost_hp(target: i64) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "cure_up_by_lost_hp", target);
        Self::new(BattleEffectType::CureUpByLostHp as i32, target)
            .effect_num(0)
            .build()
    }

    pub fn shield(target: i64, amount: i32) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "shield", target, amount);
        Self::new(BattleEffectType::Shield as i32, target)
            .effect_num(amount)
            .build()
    }

    pub fn shield_change(target: i64, current: i32) -> ActEffect {
        Self::new(BattleEffectType::ShieldChange as i32, target)
            .effect_num(current)
            .build()
    }

    pub fn shield_broken(target: i64) -> ActEffect {
        Self::new(BattleEffectType::ShieldBroken as i32, target).build()
    }

    pub fn shield_del(target: i64) -> ActEffect {
        Self::new(BattleEffectType::ShieldDel as i32, target).build()
    }

    pub fn poison(target: i64) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "poison", target);
        Self::new(BattleEffectType::Poison as i32, target)
            .effect_num(0)
            .build()
    }

    pub fn burn(target: i64, effect_num: i32, buff_act_id: Option<i32>) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "burn",
            target,
            effect_num,
            ?buff_act_id
        );
        let mut builder = Self::new(BattleEffectType::Burn as i32, target).effect_num(effect_num);
        if let Some(buff_act_id) = buff_act_id {
            builder = builder.buff_act_id(buff_act_id);
        }
        builder.build()
    }

    pub fn bloodlust(target: i64, amount: i32) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "bloodlust", target, amount);
        Self::new(BattleEffectType::Bloodlust as i32, target)
            .effect_num(amount)
            .build()
    }

    pub fn average_life(target: i64) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "average_life", target);
        Self::new(BattleEffectType::AverageLife as i32, target).build()
    }

    pub fn fight_counter(target: i64, amount: i32) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "fight_counter", target, amount);
        Self::new(BattleEffectType::FightCounter as i32, target)
            .effect_num(amount)
            .build()
    }

    pub fn buff_act_info_update(
        target: i64,
        reserve_id: i64,
        buff_act_info: BuffActInfo,
    ) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "buff_act_info_update",
            target,
            reserve_id,
            act_id = buff_act_info.act_id.unwrap_or(0)
        );
        Self::new(EffectType::Buffactinfoupdate as i32, target)
            .reserve_id(reserve_id)
            .buff_act_info(buff_act_info)
            .build()
    }

    /// Nautika-only marker for how many random-target follow-up hits will fire.
    pub fn nuodika_random_attack_num(target: i64, amount: i32, target_count: i32) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "nuodika_random_attack_num",
            target,
            amount,
            target_count
        );
        Self::new(EffectType::Nuodikarandomattacknum as i32, target)
            .effect_num(amount)
            .effect_num1(target_count)
            .build()
    }

    /// Nautika-only random-target follow-up attack marker.
    pub fn nuodika_random_attack(
        target: i64,
        amount: i32,
        hit_kind: i32,
        config_effect: i32,
        buff_act_id: i32,
        reserve_str: impl Into<String>,
    ) -> ActEffect {
        let reserve_str = reserve_str.into();
        tracing::trace!(
            target: "act_effects",
            kind = "nuodika_random_attack",
            target,
            amount,
            hit_kind,
            config_effect,
            buff_act_id,
            reserve_str = reserve_str.as_str()
        );
        Self::new(EffectType::Nuodikarandomattack as i32, target)
            .effect_num(amount)
            .effect_num1(hit_kind)
            .config_effect(config_effect)
            .buff_act_id(buff_act_id)
            .reserve_str(reserve_str)
            .build()
    }

    /// Nautika-only team-wide follow-up attack marker.
    pub fn nuodika_team_attack(
        target: i64,
        amount: i32,
        team_hit_kind: i32,
        config_effect: i32,
        buff_act_id: i32,
    ) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "nuodika_team_attack",
            target,
            amount,
            team_hit_kind,
            config_effect,
            buff_act_id
        );
        Self::new(EffectType::Nuodikateamattack as i32, target)
            .effect_num(amount)
            .effect_num1(team_hit_kind)
            .config_effect(config_effect)
            .buff_act_id(buff_act_id)
            .build()
    }

    pub fn magic_circle_add(target: i64, circle_id: i32, circle: MagicCircleInfo) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "magic_circle_add",
            target,
            circle_id,
            ?circle
        );
        Self::new(EffectType::Magiccircleadd as i32, target)
            .effect_num(0)
            .reserve_id(circle_id as i64)
            .magic_circle(circle)
            .build()
    }

    pub fn magic_circle_delete(target: i64, circle_id: i32) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "magic_circle_delete",
            target,
            circle_id
        );
        Self::new(EffectType::Magiccircledelete as i32, target)
            .reserve_id(circle_id as i64)
            .effect_num(0)
            .build()
    }

    pub fn magic_circle_update(
        target: i64,
        circle_id: i32,
        reserve_str: impl Into<String>,
        circle: MagicCircleInfo,
    ) -> ActEffect {
        let reserve_str = reserve_str.into();
        tracing::trace!(
            target: "act_effects",
            kind = "magic_circle_update",
            target,
            circle_id,
            reserve_str = reserve_str.as_str(),
            ?circle
        );
        Self::new(EffectType::Magiccircleupdate as i32, target)
            .reserve_id(circle_id as i64)
            .reserve_str(reserve_str)
            .magic_circle(circle)
            .effect_num(0)
            .build()
    }

    pub fn additional_damage(target: i64, amount: i32) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "additional_damage",
            target,
            amount,
            crit = false
        );
        Self::new(EffectType::Additionaldamage as i32, target)
            .effect_num(amount)
            .build()
    }

    pub fn additional_damage_crit(target: i64, amount: i32) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "additional_damage",
            target,
            amount,
            crit = true
        );
        Self::new(EffectType::Additionaldamagecrit as i32, target)
            .effect_num(amount)
            .build()
    }

    pub fn select(actor: i64, card_idx: i32) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "select", actor, card_idx);
        Self::bare(EffectType::Cardinvalid as i32)
            .effect_num(card_idx)
            .build()
    }

    pub fn click_effect(target: i64) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "click_effect", target);
        Self::new(EffectType::Expointchange as i32, target)
            .effect_num(1)
            .build()
    }

    pub fn master_halo(target: i64) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "master_halo", target);
        Self::new(BattleEffectType::MasterHalo as i32, target).build()
    }

    pub fn slave_halo(target: i64) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "slave_halo", target);
        Self::new(BattleEffectType::SlaveHalo as i32, target).build()
    }

    pub fn attr_with_num(target: i64, effect_num: i32) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "attr_with_num",
            target,
            effect_num
        );
        Self::new(BattleEffectType::Attr as i32, target)
            .effect_num(effect_num)
            .build()
    }

    pub fn marker(effect_type: i32, target: i64, effect_num: i32) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "marker",
            effect_type,
            target,
            effect_num
        );
        Self::new(effect_type, target)
            .effect_num(effect_num)
            .build()
    }

    pub fn effect_none_with_buff_act(target: i64, effect_num: i32, buff_act_id: i32) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "effect_none_with_buff_act",
            target,
            effect_num,
            buff_act_id
        );
        Self::new(EffectType::None as i32, target)
            .effect_num(effect_num)
            .buff_act_id(buff_act_id)
            .build()
    }

    pub fn round_end(target: Option<i64>, effect_num: Option<i32>) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "round_end",
            ?target,
            ?effect_num
        );
        let mut builder = Self::bare(BattleEffectType::RoundEnd as i32);
        if let Some(target) = target {
            builder = builder.target_id(target);
        }
        if let Some(effect_num) = effect_num {
            builder = builder.effect_num(effect_num);
        }
        builder.build()
    }

    pub fn change_hero(dead_uid: i64, sub_entity: sonettobuf::FightEntityInfo, position: i32) -> ActEffect {
        ActEffect {
            effect_type: Some(BattleEffectType::ChangeHero as i32),
            target_id: Some(dead_uid),
            effect_num: Some(position),
            entity: Some(sub_entity),
            ..Default::default()
        }
    }

    pub fn small_round_end(target: Option<i64>, effect_num: i32) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "small_round_end",
            ?target,
            effect_num
        );
        let mut builder = Self::bare(BattleEffectType::SmallRoundEnd as i32).effect_num(effect_num);
        if let Some(target) = target {
            builder = builder.target_id(target);
        }
        builder.build()
    }

    pub fn clear_universal_card(
        target: Option<i64>,
        effect_num: Option<i32>,
        team_type: Option<i32>,
    ) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "clear_universal_card",
            ?target,
            ?effect_num,
            ?team_type
        );
        let mut builder = Self::bare(BattleEffectType::ClearUniversalCard as i32);
        if let Some(target) = target {
            builder = builder.target_id(target);
        }
        if let Some(effect_num) = effect_num {
            builder = builder.effect_num(effect_num);
        }
        if let Some(team_type) = team_type {
            builder = builder.team_type(team_type);
        }
        builder.build()
    }

    pub fn after_redeal_card(card_info_list: Vec<CardInfo>) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "after_redeal_card", card_count = card_info_list.len());
        Self::new(BattleEffectType::AfterReDealCard as i32, 0)
            .card_info_list(card_info_list)
            .team_type(1)
            .build()
    }

    pub fn change_round(target: Option<i64>, effect_num: Option<i32>) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "change_round",
            ?target,
            ?effect_num
        );
        let mut builder = Self::bare(BattleEffectType::ChangeRound as i32);
        if let Some(target) = target {
            builder = builder.target_id(target);
        }
        if let Some(effect_num) = effect_num {
            builder = builder.effect_num(effect_num);
        }
        builder.build()
    }

    pub fn use_cards(card_info_list: Vec<CardInfo>) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "use_cards",
            card_count = card_info_list.len()
        );
        Self::bare(BattleEffectType::UseCards as i32)
            .card_info_list(card_info_list)
            .build()
    }

    pub fn enter_fight_deal() -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "enter_fight_deal");
        Self::new(BattleEffectType::EnterFightDeal as i32, 0)
            .effect_num(0)
            .team_type(0)
            .build()
    }

    pub fn direct_use_ex_skill(target: i64) -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "direct_use_ex_skill", target);
        Self::new(BattleEffectType::DirectUseExSkill as i32, target)
            .effect_num(0)
            .build()
    }

    pub fn allocate_card_energy(card_info_list: Vec<CardInfo>) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "allocate_card_energy",
            card_count = card_info_list.len()
        );
        Self::bare(BattleEffectType::AllocateCardEnergy as i32)
            .effect_num(1)
            .card_info_list(card_info_list)
            .build()
    }

    pub fn deal_card1() -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "deal_card1");
        Self::new(BattleEffectType::DealCard1 as i32, 0).build()
    }

    pub fn deal_card2() -> ActEffect {
        tracing::trace!(target: "act_effects", kind = "deal_card2");
        Self::new(BattleEffectType::DealCard2 as i32, 0).build()
    }

    pub fn sp_card_add(target: i64, effect_num: i32, reserve_id: i64, team_type: i32) -> ActEffect {
        tracing::trace!(
            target: "act_effects",
            kind = "sp_card_add",
            target,
            effect_num,
            reserve_id,
            team_type
        );
        Self::new(BattleEffectType::SpCardAdd as i32, target)
            .effect_num(effect_num)
            .reserve_id(reserve_id)
            .team_type(team_type)
            .build()
    }

    pub fn change_to_temp_card(
        target: i64,
        reserve_str: impl Into<String>,
        team_type: i32,
    ) -> ActEffect {
        let reserve_str = reserve_str.into();
        tracing::trace!(
            target: "act_effects",
            kind = "change_to_temp_card",
            target,
            reserve_str = reserve_str.as_str(),
            team_type
        );
        Self::new(BattleEffectType::ChangeToTempCard as i32, target)
            .reserve_str(reserve_str)
            .team_type(team_type)
            .build()
    }

    pub fn effect_num(mut self, value: i32) -> Self {
        self.effect.effect_num = Some(value);
        self
    }

    pub fn target_id(mut self, value: i64) -> Self {
        self.effect.target_id = Some(value);
        self
    }

    pub fn effect_num1(mut self, value: i32) -> Self {
        self.effect.effect_num1 = Some(value);
        self
    }

    pub fn buff(mut self, value: BuffInfo) -> Self {
        self.effect.buff = Some(value);
        self
    }

    pub fn card_info_list(mut self, value: Vec<CardInfo>) -> Self {
        self.effect.card_info_list = value;
        self
    }

    pub fn config_effect(mut self, value: i32) -> Self {
        self.effect.config_effect = Some(value);
        self
    }

    pub fn buff_act_id(mut self, value: i32) -> Self {
        self.effect.buff_act_id = Some(value);
        self
    }

    pub fn reserve_id(mut self, value: i64) -> Self {
        self.effect.reserve_id = Some(value);
        self
    }

    pub fn reserve_str(mut self, value: impl Into<String>) -> Self {
        self.effect.reserve_str = Some(value.into());
        self
    }

    pub fn team_type(mut self, value: i32) -> Self {
        self.effect.team_type = Some(value);
        self
    }

    pub fn hurt_info(mut self, value: FightHurtInfo) -> Self {
        self.effect.hurt_info = Some(value);
        self
    }

    pub fn buff_act_info(mut self, value: BuffActInfo) -> Self {
        self.effect.buff_act_info = Some(value);
        self
    }

    pub fn magic_circle(mut self, value: MagicCircleInfo) -> Self {
        self.effect.magic_circle = Some(value);
        self
    }

    pub fn fight(mut self, value: Fight) -> Self {
        self.effect.fight = Some(value);
        self
    }

    pub fn fight_step(mut self, value: FightStep) -> Self {
        self.effect.fight_step = Some(value);
        self
    }

    pub fn build(self) -> ActEffect {
        self.effect
    }

    fn build_buff_add(
        target_uid: i64,
        from_uid: i64,
        buff_id: i32,
        layer: i32,
        count: i32,
        slave: bool,
    ) -> ActEffect {
        let params = buff_get_act_common_params(buff_id);
        Self::new(EffectType::Buffadd as i32, target_uid)
            .effect_num(buff_id)
            .buff(Self::create_buff(
                target_uid, buff_id, from_uid, count, layer, &params, slave,
            ))
            .build()
    }

    fn create_buff(
        target_uid: i64,
        buff_id: i32,
        from_uid: i64,
        count: i32,
        layer: i32,
        params: &str,
        slave: bool,
    ) -> BuffInfo {
        let cfg = config::configs::get();
        let buff_cfg = cfg.skill_buff.iter().find(|b| b.id == buff_id);
        let duration = buff_cfg.map(|b| b.during_time).unwrap_or(0);
        let uid = if slave {
            next_slave_buff_uid_for_target(target_uid)
        } else {
            next_buff_uid_for_target(target_uid)
        };
        BuffInfo {
            buff_id: Some(buff_id),
            duration: Some(duration),
            uid: Some(uid),
            ex_info: Some(0),
            from_uid: Some(from_uid),
            count: Some(count),
            act_common_params: Some(params.to_string()),
            layer: Some(layer),
            r#type: Some(BuffLayerType::Normal as i32),
            act_info: vec![],
        }
    }

    fn build_numeric_effect(
        effect_type: i32,
        target: i64,
        amount: i32,
        config_effect: Option<i32>,
        buff_act_id: Option<i32>,
        hurt_info: Option<FightHurtInfo>,
    ) -> ActEffect {
        let mut builder = Self::new(effect_type, target).effect_num(amount);
        if let Some(config_effect) = config_effect {
            builder = builder.config_effect(config_effect);
        }
        if let Some(buff_act_id) = buff_act_id {
            builder = builder.buff_act_id(buff_act_id);
        }
        if let Some(hurt_info) = hurt_info {
            builder = builder.hurt_info(hurt_info);
        }
        builder.build()
    }

    fn bare(effect_type: i32) -> Self {
        Self {
            effect: ActEffect {
                effect_type: Some(effect_type),
                ..Default::default()
            },
        }
    }
}

pub struct FightStepBuilder {
    act_type: fight_step::ActType,
    from_id: i64,
    to_id: i64,
    act_id: i32,
    effects: Vec<ActEffect>,
}

#[allow(dead_code)]
impl FightStepBuilder {
    pub fn effect() -> Self {
        Self {
            act_type: fight_step::ActType::Effect,
            from_id: 0,
            to_id: 0,
            act_id: 0,
            effects: vec![],
        }
    }

    pub fn effect_from(from_id: i64) -> Self {
        Self {
            act_type: fight_step::ActType::Effect,
            from_id,
            to_id: 0,
            act_id: 0,
            effects: vec![],
        }
    }

    pub fn skill(from_id: i64, to_id: i64, skill_id: i32) -> Self {
        Self {
            act_type: fight_step::ActType::Skill,
            from_id,
            to_id,
            act_id: skill_id,
            effects: vec![],
        }
    }

    pub fn with(mut self, effect: ActEffect) -> Self {
        self.effects.push(effect);
        self
    }

    pub fn with_many(mut self, effects: Vec<ActEffect>) -> Self {
        self.effects.extend(effects);
        self
    }

    pub fn with_nested(mut self, step: FightStep) -> Self {
        self.effects
            .push(ActEffectBuilder::skill_wrapper_without_num(step));
        self
    }

    pub fn with_skill_container(
        mut self,
        from_uid: i64,
        skill_id: i32,
        effects: Vec<ActEffect>,
    ) -> Self {
        let step = FightStepBuilder::skill(from_uid, from_uid, skill_id)
            .with_many(effects)
            .build();
        self.effects.push(wrap_step(step));
        self
    }

    pub fn with_effect_container(
        mut self,
        from_uid: i64,
        to_uid: i64,
        act_id: i32,
        effects: Vec<ActEffect>,
    ) -> Self {
        let step = effect_container_step(from_uid, to_uid, act_id, effects);
        self.effects.push(wrap_step(step));
        self
    }

    pub fn with_bloodtithe_sync(mut self, team: i32, display_uid: i64, cur: i32, max: i32) -> Self {
        self.effects.push(ActEffect {
            effect_type: Some(EffectType::Bloodpoolmaxcreate as i32),
            target_id: Some(0),
            team_type: Some(team),
            effect_num: Some(1),
            ..Default::default()
        });
        self.effects.push(
            ActEffectBuilder::new(EffectType::Bloodpoolmaxchange as i32, 0)
                .team_type(team)
                .effect_num(1)
                .effect_num1(max)
                .build(),
        );
        self.effects.push(
            ActEffectBuilder::new(EffectType::Bloodpoolvaluechange as i32, display_uid)
                .team_type(team)
                .effect_num(cur)
                .effect_num1(1)
                .build(),
        );
        self
    }

    pub fn build(self) -> FightStep {
        FightStep {
            act_type: Some(self.act_type.into()),
            from_id: Some(self.from_id),
            to_id: Some(self.to_id),
            act_id: Some(self.act_id),
            act_effect: self.effects,
            card_index: Some(0),
            support_hero_id: Some(0),
            fake_timeline: Some(false),
            real_skill_type: Some(0),
            real_skin_id: Some(0),
        }
    }

    pub fn ex_point_change(uid: i64, delta: i32) -> FightStep {
        Self::effect()
            .with(ActEffectBuilder::ex_point_change(uid, delta))
            .build()
    }

    pub fn wrap(self) -> ActEffect {
        wrap_step(self.build())
    }
}

pub fn effect_container_step(
    from_uid: i64,
    to_uid: i64,
    act_id: i32,
    effects: Vec<ActEffect>,
) -> FightStep {
    FightStep {
        act_type: Some(fight_step::ActType::Effect.into()),
        from_id: Some(from_uid),
        to_id: Some(to_uid),
        act_id: Some(act_id),
        act_effect: effects,
        card_index: Some(0),
        support_hero_id: Some(0),
        fake_timeline: Some(false),
        real_skill_type: Some(0),
        real_skin_id: Some(0),
    }
}

pub fn wrap_step(step: FightStep) -> ActEffect {
    ActEffectBuilder::skill_wrapper(step)
}
