use sonettobuf::{
    ActEffect, Fight, FightHeroSpAttributeInfo, FightStep, HeroSpAttribute, PlayerSkillInfo,
};

use super::super::{
    buff_actions::{
        ex_point_overflow_bank::buff_get_ex_point_overflow, raspberry::BUFF_ACT_ID_RASPBERRY,
    },
    event_queue::{BattleEvent, EventContext, EventQueue, drain_to_fight_steps},
    fight_step::ActEffectBuilder,
    manager::{
        buff_mgr::{BuffMgr, observe_explicit_buff_uid_for_target},
        entity_mgr::{EntityLocation, FightEntityDataMgr, get_entity_mut_by_location},
        entity_mgr::EntityMgr,
    },
    mechanics::bloodtithe::BloodtitheState,
    types::{buff::BuffLayerType, effects::EffectType},
};

use super::traits::Manager;

#[derive(Default, Debug, Clone)]
pub struct FightCalculateDataMgr {
    entity_mgr: FightEntityDataMgr,
    buff_mgr: BuffMgr,
    pub pending_effects: Vec<ActEffect>,
}

impl FightCalculateDataMgr {
    const MAX_NESTED_STEP_DEPTH: usize = 256;

    fn entity_location_or_skip(
        &self,
        target_id: i64,
        effect_name: &str,
    ) -> Result<Option<EntityLocation>, String> {
        match self.entity_mgr.get_location(target_id) {
            Some(location) => Ok(Some(location)),
            None if target_id < 0 => {
                tracing::warn!(
                    "calculate_mgr skip effect={} target={} reason=entity_missing",
                    effect_name,
                    target_id
                );
                Ok(None)
            }
            None => Err(format!("Entity {} not found", target_id)),
        }
    }

    fn step_contains_positive_bloodpool_delta(step: &FightStep) -> bool {
        let mut stack: Vec<&FightStep> = vec![step];
        while let Some(current) = stack.pop() {
            for effect in &current.act_effect {
                if effect.effect_type == Some(EffectType::BloodPoolValueChange as i32)
                    && effect.effect_num1.unwrap_or(0) > 0
                {
                    return true;
                }
                if let Some(nested) = effect.fight_step.as_ref() {
                    stack.push(nested);
                }
            }
        }
        false
    }

    pub fn new(fight: &Fight) -> Self {
        Self {
            entity_mgr: FightEntityDataMgr::new(fight),
            buff_mgr: BuffMgr::new(),
            pending_effects: Vec::new(),
        }
    }

    pub fn update_cache(&mut self, fight: &Fight) {
        self.entity_mgr.rebuild_cache(fight);
    }

    #[allow(dead_code)]
    pub fn take_pending_effects(&mut self) -> Vec<ActEffect> {
        std::mem::take(&mut self.pending_effects)
    }

    pub fn play_step_data(
        &mut self,
        step: &FightStep,
        fight: &mut Fight,
        bloodtithe: &mut BloodtitheState,
        buff_mgr: &mut BuffMgr,
        entity_mgr: &mut EntityMgr,
    ) -> Result<(), String> {
        let mut stack: Vec<(&FightStep, usize, bool)> = vec![(step, 0, false)];
        while let Some((current, depth, inherited_bloodtithe_sync)) = stack.pop() {
            let local_bloodtithe_sync = Self::step_contains_positive_bloodpool_delta(current);
            let use_accumulator_only_bloodtithe_sync =
                inherited_bloodtithe_sync || local_bloodtithe_sync;
            for effect in current.act_effect.iter().rev() {
                if let Some(nested) = effect.fight_step.as_ref() {
                    if depth >= Self::MAX_NESTED_STEP_DEPTH {
                        tracing::warn!(
                            "play_step_data nested depth limit reached (depth={} effect_type={:?})",
                            depth,
                            effect.effect_type
                        );
                        continue;
                    }
                    stack.push((nested, depth + 1, use_accumulator_only_bloodtithe_sync));
                    continue;
                }
                let is_bloodpool_delta =
                    effect.effect_type == Some(EffectType::BloodPoolValueChange as i32);
                let pre_pool = is_bloodpool_delta.then(|| {
                    let team_type = effect.team_type.or(effect.effect_num).unwrap_or(1);
                    (
                        team_type,
                        bloodtithe.get_value(team_type),
                        bloodtithe.get_max(team_type),
                        bloodtithe.get_acc(team_type),
                    )
                });
                self.play_non_nested_effect_data(
                    effect,
                    fight,
                    bloodtithe,
                    buff_mgr,
                    entity_mgr,
                    use_accumulator_only_bloodtithe_sync,
                )?;
                let _ = pre_pool;
            }
        }
        Ok(())
    }

    fn play_non_nested_effect_data(
        &mut self,
        effect: &ActEffect,
        fight: &mut Fight,
        bloodtithe: &mut BloodtitheState,
        buff_mgr: &mut BuffMgr,
        entity_mgr: &mut EntityMgr,
        use_accumulator_only_bloodtithe_sync: bool,
    ) -> Result<(), String> {
        let effect_type = EffectType::from(effect.effect_type.unwrap_or(0));

        match effect_type {
            // Just ignore
            EffectType::None
            | EffectType::FightStep
            | EffectType::MasterHalo
            | EffectType::SlaveHalo
            | EffectType::Attr
            | EffectType::Cure
            | EffectType::CureUpByLostHp
            | EffectType::MonsterLabelBuff
            | EffectType::TeammateInjuryCount
            | EffectType::ExPointOverflowBank => Ok(()),

            EffectType::Damage
            | EffectType::Crit
            | EffectType::DamageExtra
            | EffectType::OriginDamage
            | EffectType::OriginCrit
            | EffectType::DamageFromAbsorb
            | EffectType::DamageFromLostHp
            | EffectType::EnchantBurnDamage
            | EffectType::DamageShareHp
            | EffectType::DeadlyPoisonOriginDamage
            | EffectType::DeadlyPoisonOriginCrit
            | EffectType::AdditionalDamage
            | EffectType::AdditionalDamageCrit
            | EffectType::ShareHurt
            | EffectType::EnchantDepresseDamage => self.play_effect_damage(
                effect,
                fight,
                bloodtithe,
                buff_mgr,
                entity_mgr,
                use_accumulator_only_bloodtithe_sync,
            ),

            EffectType::Heal
            | EffectType::Bloodlust
            | EffectType::InjuryBankHeal
            | EffectType::SubHeroLifeChange => self.play_effect_heal(effect, fight, entity_mgr),

            EffectType::BuffAdd => self.play_effect_add_buff(effect, buff_mgr),

            EffectType::Dead => self.play_effect_death(effect, fight),
            EffectType::Kill => self.play_effect_kill(effect, fight),

            EffectType::Shield => self.play_effect_shield(effect, fight),
            EffectType::ShieldDel => self.play_effect_shield_del(effect, fight),

            EffectType::AverageLife => self.play_effect_set_hp(effect, fight),
            EffectType::MaxHpChange => self.play_effect_set_max_hp(effect, fight, entity_mgr),
            EffectType::CurrentHpChange => {
                self.play_effect_set_current_hp(effect, fight, bloodtithe, entity_mgr)
            }

            EffectType::AddExPoint | EffectType::ExPointChange => {
                self.play_effect_add_ex_point(effect, fight, buff_mgr, entity_mgr)
            }

            EffectType::ExPointDel => self.play_effect_del_ex_point(effect, fight, entity_mgr),

            EffectType::BloodPoolMaxCreate => {
                self.play_effect_bloodtithe_enable(effect, bloodtithe)
            }
            EffectType::BloodPoolMaxChange => self.play_effect_bloodtithe_max(effect, bloodtithe),
            EffectType::BloodPoolValueChange => {
                self.play_effect_bloodtithe_value(effect, bloodtithe)
            }
            EffectType::MagicCircleAdd => self.play_effect_magic_circle_add(effect, fight),
            EffectType::MagicCircleDelete => self.play_effect_magic_circle_delete(effect, fight),
            EffectType::MagicCircleUpdate => self.play_effect_magic_circle_update(effect, fight),

            EffectType::FightHurtDetail => self.play_effect_fight_hurt_detail(effect, fight),

            EffectType::BuffDel | EffectType::BuffDelNoEffect => {
                self.play_effect_del_buff(effect, buff_mgr)
            }
            EffectType::BuffActInfoUpdate => self.play_effect_buff_act_info_update(effect),
            EffectType::AddToTarget => self.play_effect_add_to_target(effect),
            EffectType::Rebound => self.play_effect_rebound(effect),
            EffectType::BuffUpdate => self.play_effect_update_buff(effect, buff_mgr),
            EffectType::StorageInjury => self.play_effect_storage_injury(effect, buff_mgr),
            EffectType::PowerChange => self.play_effect_power_change(effect, fight),

            // Client-only display effects — no server state change needed
            EffectType::BuffAddNoEffect
            | EffectType::UseCards
            | EffectType::EnterFightDeal
            | EffectType::CardsPush
            | EffectType::DealCard2
            | EffectType::CardDeckNum => Ok(()),

            other => {
                tracing::warn!("Unhandled effect type: {:?}", other);
                Ok(())
            }
        }
    }

    fn play_effect_damage(
        &mut self,
        effect: &ActEffect,
        fight: &mut Fight,
        bloodtithe: &mut BloodtitheState,
        buff_mgr: &mut BuffMgr,
        entity_mgr: &mut EntityMgr,
        use_accumulator_only_bloodtithe_sync: bool,
    ) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;
        let damage = effect.effect_num.ok_or("No damage amount")?;

        let Some(location) = self.entity_location_or_skip(target_id, "damage")? else {
            return Ok(());
        };
        let entity = get_entity_mut_by_location(fight, location)
            .ok_or_else(|| format!("Failed to get entity {} mutably", target_id))?;

        let shield = entity.shield_value.unwrap_or(0);
        let shield_absorbed = damage.min(shield);
        let hp_damage = damage - shield_absorbed;

        entity.shield_value = Some(shield - shield_absorbed);

        if shield_absorbed > 0 {
            buff_mgr.reduce_shield_value(target_id, shield_absorbed);
            let remaining = shield - shield_absorbed;
            if remaining == 0 {
                let shield_buff_id = buff_mgr.active_buff
                    .get(&target_id)
                    .and_then(|bs| bs.iter().find(|b| b.buff_type.as_ref().map(|t| t.r#type) == Some(7)))
                    .map(|b| b.buff_id);
                self.pending_effects.push(ActEffectBuilder::shield_broken(target_id));
                self.pending_effects.push(ActEffectBuilder::shield_del(target_id));
                if let Some(bid) = shield_buff_id {
                    self.pending_effects.push(ActEffectBuilder::buff_del(target_id, 0, bid, 0));
                }
            } else {
                self.pending_effects.push(ActEffectBuilder::shield(target_id, remaining));
            }
        }

        let current_hp = entity.current_hp.unwrap_or(0);
        entity.current_hp = Some((current_hp - hp_damage).max(0));

        // sync to entity_mgr
        entity_mgr.apply_damage(target_id, hp_damage);

        if hp_damage > 0
            && bloodtithe.initialized
            && let Some(team_type) = entity.team_type
        {
            if use_accumulator_only_bloodtithe_sync {
                let previous_value = bloodtithe.get_value(team_type);
                let _ = bloodtithe.on_hp_lost(target_id, team_type, hp_damage);
                bloodtithe.set_value(team_type, previous_value);
            } else {
                let _ = bloodtithe.on_hp_lost(target_id, team_type, hp_damage);
            }
        }

        tracing::trace!(
            "Damage applied: target={} damage={} shield_absorbed={} hp_damage={}",
            target_id,
            damage,
            shield_absorbed,
            hp_damage
        );

        // NOTE:
        // Drop-damage stack decay is driven by passive/effect skills in live flow
        // (e.g. wrapper-triggered consume behaviors), not by a generic damage hook.
        // Applying it here causes double-consume and diverges update-vs-delete order.

        Ok(())
    }

    fn play_effect_heal(
        &mut self,
        effect: &ActEffect,
        fight: &mut Fight,
        entity_mgr: &mut EntityMgr,
    ) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;
        let heal = effect.effect_num.ok_or("No heal amount")?;

        let Some(location) = self.entity_location_or_skip(target_id, "heal")? else {
            return Ok(());
        };

        let entity = get_entity_mut_by_location(fight, location)
            .ok_or_else(|| format!("Failed to get entity {} mutably", target_id))?;

        let current_hp = entity.current_hp.unwrap_or(0);
        let max_hp = entity
            .attr
            .as_ref()
            .and_then(|a| a.hp)
            .unwrap_or(current_hp);
        let new_hp = (current_hp + heal).min(max_hp);
        entity.current_hp = Some(new_hp);

        // sync to entity_mgr
        entity_mgr.set_hp(target_id, new_hp);

        tracing::trace!(
            "Heal applied: target={}, heal={}, new_hp={}",
            target_id,
            heal,
            new_hp
        );
        Ok(())
    }

    fn play_effect_add_buff(
        &mut self,
        effect: &ActEffect,
        buff_mgr: &mut BuffMgr,
    ) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;

        let buff_id = effect.effect_num.ok_or("No buff ID")?;

        let from_uid = effect.buff.as_ref().and_then(|b| b.from_uid).unwrap_or(0);

        let count = effect.buff.as_ref().and_then(|b| b.count).unwrap_or(0);
        let layer = effect.buff.as_ref().and_then(|b| b.layer).unwrap_or(0);
        let buff_uid = effect.buff.as_ref().and_then(|b| b.uid).unwrap_or(0);

        if buff_uid != 0 {
            observe_explicit_buff_uid_for_target(target_id, buff_uid);
            buff_mgr.add_with_uid(target_id, buff_id, from_uid, 0, count, layer, buff_uid);
            let _ = buff_mgr.set_instance_act_common_params(
                target_id,
                buff_uid,
                effect
                    .buff
                    .as_ref()
                    .and_then(|b| b.act_common_params.as_deref())
                    .unwrap_or_default(),
            );
        } else {
            buff_mgr.add(target_id, buff_id, from_uid, 0, count, layer);
        }
        Ok(())
    }

    fn play_effect_update_buff(
        &mut self,
        effect: &ActEffect,
        buff_mgr: &mut BuffMgr,
    ) -> Result<(), String> {
        let Some(target_id) = effect.target_id else {
            return Ok(());
        };
        let Some(buff) = effect.buff.as_ref() else {
            return Ok(());
        };
        let Some(buff_id) = buff.buff_id else {
            return Ok(());
        };
        let Some(buff_uid) = buff.uid else {
            return Ok(());
        };
        let from_uid = buff.from_uid.unwrap_or(0);
        let count = buff.count.unwrap_or(0);
        let layer = buff.layer.unwrap_or(0);

        observe_explicit_buff_uid_for_target(target_id, buff_uid);
        buff_mgr.add_with_uid(target_id, buff_id, from_uid, 0, count, layer, buff_uid);
        let _ = buff_mgr.set_instance_act_common_params(
            target_id,
            buff_uid,
            buff.act_common_params.as_deref().unwrap_or_default(),
        );
        Ok(())
    }

    fn play_effect_storage_injury(
        &mut self,
        effect: &ActEffect,
        buff_mgr: &mut BuffMgr,
    ) -> Result<(), String> {
        let Some(target_id) = effect.target_id else {
            return Ok(());
        };
        let Some(buff) = effect.buff.as_ref() else {
            return Ok(());
        };
        let Some(buff_id) = buff.buff_id else {
            return Ok(());
        };
        let Some(buff_uid) = buff.uid else {
            return Ok(());
        };
        let from_uid = buff.from_uid.unwrap_or(0);
        let count = buff.count.unwrap_or(0);
        let layer = buff.layer.unwrap_or(0);

        observe_explicit_buff_uid_for_target(target_id, buff_uid);
        buff_mgr.add_with_uid(target_id, buff_id, from_uid, 0, count, layer, buff_uid);
        let _ = buff_mgr.set_instance_act_common_params(
            target_id,
            buff_uid,
            buff.act_common_params.as_deref().unwrap_or_default(),
        );
        Ok(())
    }

    fn play_effect_buff_act_info_update(&mut self, effect: &ActEffect) -> Result<(), String> {
        let Some(info) = effect.buff_act_info.as_ref() else {
            return Ok(());
        };
        let act_id = info.act_id.unwrap_or(0);
        if act_id == BUFF_ACT_ID_RASPBERRY {
            // Replay bootstrap pre-scans this payload and seeds Shadow Cloak state
            // from the full step stream in FightDataMgr.
            return Ok(());
        }
        tracing::debug!(
            "Ignoring BuffActInfoUpdate replay payload: target={:?} act_id={} param={:?}",
            effect.target_id,
            act_id,
            info.param
        );
        Ok(())
    }

    fn play_effect_add_to_target(&mut self, _effect: &ActEffect) -> Result<(), String> {
        // Replay state for AddToTarget feature chains is already resolved by passive-skill
        // expansion in trigger/combat.rs and round_mgr.rs.
        Ok(())
    }

    fn play_effect_rebound(&mut self, _effect: &ActEffect) -> Result<(), String> {
        // Reflect math is modeled via hero attributes (ReboundDmg), not effect-36 payloads.
        Ok(())
    }

    fn play_effect_death(&mut self, effect: &ActEffect, fight: &mut Fight) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;

        let Some(location) = self.entity_location_or_skip(target_id, "death")? else {
            return Ok(());
        };

        let entity = get_entity_mut_by_location(fight, location)
            .ok_or_else(|| format!("Failed to get entity {} mutably", target_id))?;

        entity.current_hp = Some(0);
        self.buff_mgr.clear(target_id);

        tracing::trace!("Entity died: target={}", target_id);
        Ok(())
    }

    fn play_effect_kill(&mut self, effect: &ActEffect, fight: &mut Fight) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;

        let Some(location) = self.entity_location_or_skip(target_id, "kill")? else {
            return Ok(());
        };

        let entity = get_entity_mut_by_location(fight, location)
            .ok_or_else(|| format!("Failed to get entity {} mutably", target_id))?;

        entity.current_hp = Some(0);
        self.buff_mgr.clear(target_id);

        tracing::trace!("Entity killed: target={}", target_id);
        Ok(())
    }

    fn play_effect_shield(&mut self, effect: &ActEffect, fight: &mut Fight) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;
        let shield = effect.effect_num.ok_or("No shield amount")?;

        let Some(location) = self.entity_location_or_skip(target_id, "shield")? else {
            return Ok(());
        };
        let entity = get_entity_mut_by_location(fight, location)
            .ok_or_else(|| format!("Failed to get entity {} mutably", target_id))?;

        let current = entity.shield_value.unwrap_or(0);
        entity.shield_value = Some(current + shield);

        tracing::trace!(
            "Shield applied: target={} +{} => {}",
            target_id,
            shield,
            current + shield
        );
        Ok(())
    }

    fn play_effect_shield_del(
        &mut self,
        effect: &ActEffect,
        fight: &mut Fight,
    ) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;
        let Some(location) = self.entity_location_or_skip(target_id, "shield_del")? else {
            return Ok(());
        };
        let entity = get_entity_mut_by_location(fight, location)
            .ok_or_else(|| format!("Failed to get entity {} mutably", target_id))?;
        entity.shield_value = Some(0);
        tracing::trace!("Shield removed: target={}", target_id);
        Ok(())
    }

    fn play_effect_set_hp(&mut self, effect: &ActEffect, fight: &mut Fight) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;
        let hp = effect.effect_num.ok_or("No HP amount")?;

        let Some(location) = self.entity_location_or_skip(target_id, "set_hp")? else {
            return Ok(());
        };

        let entity = get_entity_mut_by_location(fight, location)
            .ok_or_else(|| format!("Failed to get entity {} mutably", target_id))?;

        entity.current_hp = Some(hp);
        tracing::trace!("HP set: target={}, hp={}", target_id, hp);
        Ok(())
    }

    fn play_effect_set_max_hp(
        &mut self,
        effect: &ActEffect,
        fight: &mut Fight,
        entity_mgr: &mut EntityMgr,
    ) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;
        let max_hp = effect.effect_num.ok_or("No max HP amount")?;

        let Some(location) = self.entity_location_or_skip(target_id, "set_max_hp")? else {
            return Ok(());
        };

        let entity = get_entity_mut_by_location(fight, location)
            .ok_or_else(|| format!("Failed to get entity {} mutably", target_id))?;

        if let Some(attr) = entity.attr.as_mut() {
            attr.hp = Some(max_hp);
        }
        if let Some(base) = entity.base_attr.as_mut() {
            base.hp = Some(max_hp);
        }

        entity_mgr.set_max_hp(target_id, max_hp);
        Ok(())
    }

    fn play_effect_set_current_hp(
        &mut self,
        effect: &ActEffect,
        fight: &mut Fight,
        bloodtithe: &mut BloodtitheState,
        entity_mgr: &mut EntityMgr,
    ) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;
        let hp = effect.effect_num.ok_or("No HP amount")?;
        let Some(location) = self.entity_location_or_skip(target_id, "current_hp_change")? else {
            return Ok(());
        };
        let entity = get_entity_mut_by_location(fight, location)
            .ok_or_else(|| format!("Failed to get entity {} mutably", target_id))?;
        let current_hp = entity.current_hp.unwrap_or(0);
        entity.current_hp = Some(hp);
        entity_mgr.set_hp(target_id, hp);
        if bloodtithe.initialized
            && hp < current_hp
            && let Some(team_type) = entity.team_type
        {
            let _ = bloodtithe.on_hp_lost(target_id, team_type, current_hp - hp);
        }
        Ok(())
    }

    fn play_effect_add_ex_point(
        &mut self,
        effect: &ActEffect,
        fight: &mut Fight,
        buff_mgr: &BuffMgr,
        entity_mgr: &mut EntityMgr,
    ) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;
        let offset = effect.effect_num.unwrap_or(0);

        let overflow_bonus = buff_mgr
            .get(target_id)
            .iter()
            .find_map(|b| buff_get_ex_point_overflow(b.buff_id))
            .unwrap_or(0);

        let Some(location) = self.entity_location_or_skip(target_id, "ex_point_change")? else {
            return Ok(());
        };
        let entity = get_entity_mut_by_location(fight, location)
            .ok_or_else(|| format!("Failed to get entity {} mutably", target_id))?;

        let base_max = match entity.ex_point_type.unwrap_or(0) {
            0 => 5,
            1 => 8,
            _ => 0,
        };
        let max_ex = base_max + overflow_bonus;
        let old = entity.ex_point.unwrap_or(0);
        let new = if base_max > 0 {
            (old + offset).min(max_ex)
        } else {
            old + offset
        };
        entity.ex_point = Some(new);

        // sync to entity_mgr
        entity_mgr.add_ex_point(target_id, new - old);

        tracing::info!(
            "EX changed: uid={} {} -> {} (offset={} max={})",
            target_id,
            old,
            new,
            offset,
            max_ex
        );
        Ok(())
    }

    fn play_effect_del_ex_point(
        &mut self,
        effect: &ActEffect,
        fight: &mut Fight,
        entity_mgr: &mut EntityMgr,
    ) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;
        let amount = effect.effect_num.unwrap_or(0);

        let Some(location) = self.entity_location_or_skip(target_id, "ex_point_del")? else {
            return Ok(());
        };
        let entity = get_entity_mut_by_location(fight, location)
            .ok_or_else(|| format!("Failed to get entity {} mutably", target_id))?;

        let old = entity.ex_point.unwrap_or(0);
        let new = (old - amount).max(0);
        entity.ex_point = Some(new);

        entity_mgr.set_ex_point(target_id, new);

        tracing::info!(
            "EX consumed: uid={} {} -> {} (amount={})",
            target_id,
            old,
            new,
            amount
        );
        Ok(())
    }

    fn play_effect_fight_hurt_detail(
        &mut self,
        effect: &ActEffect,
        _fight: &mut Fight,
    ) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;
        let hurt = effect.effect_num.unwrap_or(0);

        tracing::trace!("Hurt detail: target={}, amount={}", target_id, hurt);

        Ok(())
    }

    fn play_effect_bloodtithe_enable(
        &mut self,
        effect: &ActEffect,
        bloodtithe: &mut BloodtitheState,
    ) -> Result<(), String> {
        let team_type = effect.team_type.unwrap_or(1);
        bloodtithe.initialized = true;
        tracing::trace!("Bloodtithe enabled: team={}", team_type);
        Ok(())
    }

    fn play_effect_bloodtithe_max(
        &mut self,
        effect: &ActEffect,
        bloodtithe: &mut BloodtitheState,
    ) -> Result<(), String> {
        let team_type = effect.team_type.unwrap_or(1);
        let max = effect.effect_num1.unwrap_or(0);
        let mut events = EventQueue::new();
        events.push(BattleEvent::BloodpoolMaxChange {
            team_type,
            max: bloodtithe.get_max(team_type).max(max),
        });
        let mut local_fight = Fight::default();
        let mut local_buff_mgr = BuffMgr::new();
        let mut local_entity_mgr = EntityMgr::default();
        let mut event_ctx = EventContext {
            fight: &mut local_fight,
            buff_mgr: &mut local_buff_mgr,
            entity_mgr: &mut local_entity_mgr,
            bloodtithe,
        };
        let _ = drain_to_fight_steps(events.drain(), &mut event_ctx);
        tracing::trace!("Bloodtithe max set: team={}, max={}", team_type, max);
        Ok(())
    }

    fn play_effect_bloodtithe_value(
        &mut self,
        effect: &ActEffect,
        bloodtithe: &mut BloodtitheState,
    ) -> Result<(), String> {
        let team_type = effect.team_type.unwrap_or(1);
        let effect_num = effect.effect_num.unwrap_or(0);
        let effect_num1 = effect.effect_num1.unwrap_or(0);

        // effectNum=team_type, effectNum1=delta (gain or consume)
        // legacy absolute-set packets encode the value in effectNum with effectNum1=0
        if effect_num == 1 && effect_num1 != 0 {
            let mut events = EventQueue::new();
            events.push(BattleEvent::BloodpoolValueChange {
                team_type,
                target: effect.target_id.unwrap_or(0),
                delta: effect_num1,
            });
            let mut local_fight = Fight::default();
            let mut local_buff_mgr = BuffMgr::new();
            let mut local_entity_mgr = EntityMgr::default();
            let mut event_ctx = EventContext {
                fight: &mut local_fight,
                buff_mgr: &mut local_buff_mgr,
                entity_mgr: &mut local_entity_mgr,
                bloodtithe,
            };
            let _ = drain_to_fight_steps(events.drain(), &mut event_ctx);
        } else {
            // legacy absolute set
            bloodtithe.set_value(team_type, effect_num);
        }
        Ok(())
    }

    fn play_effect_magic_circle_add(
        &mut self,
        effect: &ActEffect,
        fight: &mut Fight,
    ) -> Result<(), String> {
        let magic_circle = effect.magic_circle.ok_or("No magic circle info")?;
        fight.magic_circle = Some(magic_circle);
        Ok(())
    }

    fn play_effect_magic_circle_delete(
        &mut self,
        _effect: &ActEffect,
        fight: &mut Fight,
    ) -> Result<(), String> {
        fight.magic_circle = None;
        Ok(())
    }

    fn play_effect_magic_circle_update(
        &mut self,
        effect: &ActEffect,
        fight: &mut Fight,
    ) -> Result<(), String> {
        let new_mc = effect.magic_circle.ok_or("No magic circle info")?;
        if let Some(existing) = fight.magic_circle.as_mut() {
            existing.round = new_mc.round;
            existing.electric_level = new_mc.electric_level;
            existing.electric_progress = new_mc.electric_progress;
            existing.max_electric_progress = new_mc.max_electric_progress;
        }
        Ok(())
    }

    fn play_effect_del_buff(
        &mut self,
        effect: &ActEffect,
        buff_mgr: &mut BuffMgr,
    ) -> Result<(), String> {
        let target_id = effect.target_id.ok_or("No target ID")?;
        let buff_uid = effect.buff.as_ref().and_then(|b| b.uid).unwrap_or(0);
        buff_mgr.remove_by_uid(target_id, buff_uid);
        Ok(())
    }

    fn play_effect_power_change(
        &mut self,
        effect: &ActEffect,
        fight: &mut Fight,
    ) -> Result<(), String> {
        let mut events = EventQueue::new();
        events.push(BattleEvent::PowerChange {
            delta: effect.effect_num.unwrap_or(0),
        });

        let mut local_buff_mgr = BuffMgr::new();
        let mut local_entity_mgr = EntityMgr::default();
        let mut local_bloodtithe = BloodtitheState::new();
        let mut event_ctx = EventContext {
            fight,
            buff_mgr: &mut local_buff_mgr,
            entity_mgr: &mut local_entity_mgr,
            bloodtithe: &mut local_bloodtithe,
        };
        let amount = drain_to_fight_steps(events.drain(), &mut event_ctx)
            .into_iter()
            .next()
            .and_then(|effect| effect.effect_num)
            .unwrap_or(0);

        if let Some(attacker) = fight.attacker.as_mut() {
            let current = attacker.power.unwrap_or(0);
            attacker.power = Some((current + amount).max(0));
            tracing::trace!("Power change: {} -> {}", current, attacker.power.unwrap());
        }
        Ok(())
    }
}

impl FightCalculateDataMgr {
    fn zero_hero_sp_attribute() -> HeroSpAttribute {
        HeroSpAttribute {
            revive: Some(0),
            heal: Some(0),
            absorb: Some(0),
            defense_ignore: Some(0),
            clutch: Some(0),
            final_add_dmg: Some(0),
            final_drop_dmg: Some(0),
            normal_skill_rate: Some(0),
            play_add_rate: Some(0),
            play_drop_rate: Some(0),
            dizzy_resistances: Some(0),
            sleep_resistances: Some(0),
            petrified_resistances: Some(0),
            frozen_resistances: Some(0),
            disarm_resistances: Some(0),
            forbid_resistances: Some(0),
            seal_resistances: Some(0),
            cant_get_exskill_resistances: Some(0),
            del_ex_point_resistances: Some(0),
            stress_up_resistances: Some(0),
            control_resilience: Some(0),
            del_ex_point_resilience: Some(0),
            stress_up_resilience: Some(0),
            charm_resistances: Some(0),
            rebound_dmg: Some(0),
            extra_dmg: Some(0),
            reuse_dmg: Some(0),
            big_skill_rate: Some(0),
            clutch_dmg: Some(0),
            nowmal_dmg: Some(0),
        }
    }

    pub fn build_hero_sp_attributes(&mut self, fight: &Fight) -> Vec<FightHeroSpAttributeInfo> {
        let cfg = config::configs::get();
        let mut attrs = vec![];
        if let Some(ref defender) = fight.defender {
            for entity in defender.entitys.iter().chain(defender.sub_entitys.iter()) {
                let attribute = entity
                    .model_id
                    .and_then(|model_id| {
                        cfg.monster_skill_template.iter().find(|m| m.id == model_id)
                    })
                    .and_then(|m| {
                        cfg.resistances_attribute
                            .iter()
                            .find(|r| r.id == m.resistance)
                    })
                    .map(|r| HeroSpAttribute {
                        dizzy_resistances: Some(r.dizzy),
                        sleep_resistances: Some(r.sleep),
                        petrified_resistances: Some(r.petrified),
                        frozen_resistances: Some(r.frozen),
                        disarm_resistances: Some(r.disarm),
                        forbid_resistances: Some(r.forbid),
                        seal_resistances: Some(r.seal),
                        cant_get_exskill_resistances: Some(r.cant_get_exskill),
                        charm_resistances: Some(r.charm),
                        del_ex_point_resistances: Some(r.del_ex_point),
                        stress_up_resistances: Some(r.stress_up),
                        control_resilience: Some(r.control_resilience),
                        del_ex_point_resilience: Some(r.del_ex_point_resilience),
                        stress_up_resilience: Some(r.stress_up_resilience),
                        revive: Some(0),
                        heal: Some(0),
                        absorb: Some(0),
                        defense_ignore: Some(0),
                        clutch: Some(0),
                        final_add_dmg: Some(0),
                        final_drop_dmg: Some(0),
                        normal_skill_rate: Some(0),
                        play_add_rate: Some(0),
                        play_drop_rate: Some(0),
                        rebound_dmg: Some(0),
                        extra_dmg: Some(0),
                        reuse_dmg: Some(0),
                        big_skill_rate: Some(0),
                        clutch_dmg: Some(0),
                        nowmal_dmg: Some(0),
                    })
                    .unwrap_or_else(Self::zero_hero_sp_attribute);
                attrs.push(FightHeroSpAttributeInfo {
                    uid: entity.uid,
                    attribute: Some(attribute),
                });
            }
        }
        attrs
    }

    pub fn build_player_skills(&mut self) -> Vec<PlayerSkillInfo> {
        vec![
            PlayerSkillInfo {
                skill_id: Some(30010201),
                cd: Some(0),
                need_power: Some(40),
                r#type: Some(BuffLayerType::Normal as i32),
            },
            PlayerSkillInfo {
                skill_id: Some(30010202),
                cd: Some(0),
                need_power: Some(25),
                r#type: Some(BuffLayerType::Normal as i32),
            },
        ]
    }
}

impl FightCalculateDataMgr {
    pub fn on_round_end(&mut self) {
        self.buff_mgr.tick_round_end();
    }
}
