use super::super::{
    ai::select_enemy_cards,
    event::apply::events_to_steps,
    manager::{
        entity_mgr::{build_ex_point_info, sync_to_fight},
        fight_data_mgr::FightDataMgr,
    },
    fight_step::split_step_by_effect_limit,
    passives::execute_battle_start_passives,
};
use anyhow::Result;
use rand::{SeedableRng, rngs::StdRng};
use sonettobuf::FightRound;

pub fn build_initial_round(fight_mgr: &mut FightDataMgr, battle_id: i32, seed: u64) -> Result<FightRound> {
    let mut steps = {
        let seed = fight_mgr.fight.cur_round.unwrap_or(0) as u64;
        let mut rng = StdRng::seed_from_u64(seed);
        let mut ctx = fight_mgr.ctx_with_rng(&mut rng);
        let mut steps = execute_battle_start_passives(&mut ctx, battle_id);

        // ENTER_FIGHT hook
        let initial_uids: Vec<i64> = ctx.fight
            .attacker.iter().chain(ctx.fight.defender.iter())
            .flat_map(|t| t.entitys.iter())
            .filter_map(|e| e.uid)
            .collect();
        for uid in initial_uids {
            let events = ctx.on_battle_start(uid);
            steps.extend(events_to_steps(events));
            let events = ctx.on_enter_fight(uid);
            steps.extend(events_to_steps(events));
        }
        steps
    };
    steps = steps.into_iter().flat_map(split_step_by_effect_limit).collect();

    sync_to_fight(&mut fight_mgr.fight, &fight_mgr.managers.entity_mgr);

    if let Some(pre) = fight_mgr.pre_fight.as_mut() {
        for side in [pre.attacker.as_mut(), pre.defender.as_mut()]
            .into_iter()
            .flatten()
        {
            for e in side.entitys.iter_mut().chain(side.sub_entitys.iter_mut()) {
                if let Some(uid) = e.uid {
                    e.current_hp = Some(fight_mgr.managers.entity_mgr.get_hp(uid));
                }
            }
        }
    }

    fight_mgr.managers.entity_mgr.rebuild_cache(&fight_mgr.fight);
    fight_mgr.managers.calculate_mgr.update_cache(&fight_mgr.fight);

    let act_point = fight_mgr.managers.entity_mgr.get_ac_point(&fight_mgr.fight, true);

    for e in fight_mgr.fight.attacker.iter().flat_map(|t| t.entitys.iter()) {
        let uid = e.uid.unwrap_or(0);
        tracing::warn!(
            "pre-build uid={} mgr_hp={}",
            uid,
            fight_mgr.managers.entity_mgr.get_hp(uid)
        );
    }

    let ex_point_info = build_ex_point_info(&fight_mgr.fight, &fight_mgr.managers.entity_mgr);
    let hero_sp_attributes = fight_mgr.managers.calculate_mgr.build_hero_sp_attributes(&fight_mgr.fight);
    let skill_infos = fight_mgr.fight.attacker.as_ref().map(|a| a.skill_infos.clone()).unwrap_or_default();
    let team_a_cards1 = fight_mgr.managers.deck_mgr.player_hand.clone();

    let ai_use_cards = {
        let mut rng = StdRng::seed_from_u64(seed);
        let cards = select_enemy_cards(
            &mut fight_mgr.managers.deck_mgr,
            &fight_mgr.fight,
            &fight_mgr.managers.entity_mgr,
            &mut rng,
        );
        fight_mgr.managers.deck_mgr.next_ai_use_cards = cards.clone();
        cards
    };

    let round = FightRound {
        fight_step: steps,
        act_point: Some(act_point),
        is_finish: Some(false),
        move_num: Some(0),
        ex_point_info,
        ai_use_cards,
        power: fight_mgr.fight.attacker.as_ref().and_then(|a| a.power),
        skill_infos,
        before_cards1: vec![],
        team_a_cards1,
        before_cards2: vec![],
        team_a_cards2: vec![],
        next_round_begin_step: vec![],
        use_card_list: vec![],
        cur_round: Some(1),
        hero_sp_attributes,
        last_change_hero_uid: Some(0),
    };

    Ok(round)
}
