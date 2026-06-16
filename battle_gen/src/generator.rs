use anyhow::{Context, Result};
use gameserver::state::{BattleSimulator, FightDataMgr};
use serde_json::{Value, json};
use sonettobuf::{
    BeginRoundOper, CardInfo, Fight, FightExPointInfo, FightRound, FightStep, StartDungeonReply,
};

use crate::parser::start_dungeon::InitialBuffAddSeed;

pub fn generate_begin_round_reply(
    fight_input: Fight,
    player_deck: Vec<CardInfo>,
    ai_deck: Vec<CardInfo>,
) -> Result<StartDungeonReply> {
    let battle_id = fight_input.battle_id.unwrap_or_default();
    let mut mgr = FightDataMgr::new(fight_input, 0);
    let round = mgr
        .build_initial_round(battle_id, player_deck, ai_deck)
        .context("failed to build initial round")?;

    Ok(StartDungeonReply {
        fight: mgr.pre_fight.clone().or_else(|| Some(mgr.fight().clone())),
        round: Some(round),
    })
}

pub async fn generate_begin_round_sequence(
    fight_input: Fight,
    initial_ex_point_info: Vec<FightExPointInfo>,
    initial_round: Option<FightRound>,
    initial_bloodpool_effects: Vec<(i32, i32, i32)>,
    initial_buff_add_effects: Vec<InitialBuffAddSeed>,
    rounds: Vec<(
        String,
        Vec<CardInfo>,
        Vec<CardInfo>,
        Vec<BeginRoundOper>,
        Vec<FightStep>,
        Vec<CardInfo>,
        Vec<bool>,
        Vec<Fight>,
    )>,
) -> Result<Vec<(String, Value)>> {
    let mut mgr = FightDataMgr::new(fight_input, 0);
    let attacker_uids: std::collections::HashSet<i64> = mgr
        .fight()
        .attacker
        .as_ref()
        .map(|a| {
            a.entitys
                .iter()
                .chain(a.sub_entitys.iter())
                .filter_map(|e| e.uid)
                .collect()
        })
        .unwrap_or_default();
    let has_attacker_buff_seed =
        initial_buff_add_effects
            .iter()
            .any(|(buff_id, target_uid, ..)| {
                attacker_uids.contains(target_uid) && is_bootstrap_relevant_buff(*buff_id)
            });
    if !initial_bloodpool_effects.is_empty() || has_attacker_buff_seed {
        if let Some(initial_round) = initial_round.as_ref() {
            mgr.seed_replay_state(initial_round, &initial_ex_point_info)?;
        } else {
            apply_ex_point_seed_to_fight(mgr.fight_mut(), &initial_ex_point_info);
        }
        mgr.seed_replay_buffs_from_effects(&initial_buff_add_effects);
        mgr.seed_replay_bloodtithe_from_effects(&initial_bloodpool_effects);
    } else {
        apply_ex_point_seed_to_fight(mgr.fight_mut(), &initial_ex_point_info);
    }
    // Begin-round replay mode should follow captured round requests directly.
    let mut simulator = BattleSimulator::new(mgr);

    let mut out_rounds: Vec<(String, Value)> = Vec::new();
    for (
        name,
        deck,
        ai_deck,
        opers,
        ai_steps,
        replay_selected_cards,
        replay_silent_ops,
        replay_wave_snapshots,
    ) in rounds
    {
        let ai_override_steps = if should_replay_enemy_steps(&ai_deck, &ai_steps) {
            Some(ai_steps)
        } else {
            None
        };
        let generated = simulator
            .process_round_with_replay(
                opers.clone(),
                deck,
                ai_deck,
                ai_override_steps,
                Some(replay_selected_cards),
                Some(replay_silent_ops),
                Some(replay_wave_snapshots),
                vec![],
            )
            .await
            .with_context(|| format!("failed simulating {}", name))?;

        let _ = opers; // request metadata intentionally omitted for live-shape compare output.
        out_rounds.push((name, json!({ "round": generated })));
    }

    let _ = simulator.into_data();
    Ok(out_rounds)
}

fn is_bootstrap_relevant_buff(buff_id: i32) -> bool {
    if buff_id <= 0 {
        return false;
    }

    let cfg = config::configs::get();
    let Some(_buff) = cfg.skill_buff.iter().find(|b| b.id == buff_id) else {
        return false;
    };

    let mut stack = vec![buff_id];
    let mut seen = std::collections::HashSet::new();
    while let Some(current_buff_id) = stack.pop() {
        if current_buff_id <= 0 || !seen.insert(current_buff_id) {
            continue;
        }
        let Some(current) = cfg.skill_buff.iter().find(|b| b.id == current_buff_id) else {
            continue;
        };
        for entry in current.features.split('|') {
            let parts: Vec<&str> = entry.split('#').collect();
            let act_id = parts
                .first()
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(0);
            let act_type = cfg
                .buff_act
                .iter()
                .find(|a| a.id == act_id)
                .map(|a| a.r#type.as_str())
                .unwrap_or("");

            if matches!(
                act_type,
                "BloodPoolTag"
                    | "BloodPoolCountAddExPoint"
                    | "ExPointOverflowBank"
                    | "Raspberry"
                    | "MasterHalo"
                    | "SlaveHalo"
            ) {
                return true;
            }

            if act_type == "SubBuff" {
                for raw in parts.iter().skip(1) {
                    for piece in raw.split(',') {
                        if let Ok(child_buff_id) = piece.trim().parse::<i32>() {
                            stack.push(child_buff_id);
                        }
                    }
                }
            }
        }
    }

    false
}

fn has_duplicate_ai_casts(steps: &[FightStep]) -> bool {
    let mut seen = std::collections::HashSet::new();
    for step in steps {
        let key = (
            step.from_id.unwrap_or(0),
            step.act_id.unwrap_or(0),
            step.to_id.unwrap_or(0),
        );
        if !seen.insert(key) {
            return true;
        }
    }
    false
}

fn should_replay_enemy_steps(ai_deck: &[CardInfo], ai_steps: &[FightStep]) -> bool {
    if ai_steps.is_empty() || has_duplicate_ai_casts(ai_steps) {
        return true;
    }

    let ai_skill_ids: std::collections::HashSet<i32> = ai_deck
        .iter()
        .filter_map(|card| card.skill_id)
        .filter(|skill_id| *skill_id > 0)
        .collect();

    ai_steps.iter().any(|step| {
        let skill_id = step.act_id.unwrap_or(0);
        skill_id > 0 && !ai_skill_ids.contains(&skill_id)
    })
}

fn apply_ex_point_seed_to_fight(fight: &mut Fight, ex_infos: &[FightExPointInfo]) {
    if ex_infos.is_empty() {
        return;
    }

    let mut by_uid = std::collections::HashMap::new();
    for info in ex_infos {
        if let Some(uid) = info.uid {
            by_uid.insert(uid, info);
        }
    }

    for team in [&mut fight.attacker, &mut fight.defender] {
        if let Some(side) = team {
            for entity in side.entitys.iter_mut().chain(side.sub_entitys.iter_mut()) {
                let uid = entity.uid.unwrap_or(0);
                if let Some(info) = by_uid.get(&uid) {
                    entity.ex_point = Some(info.ex_point.unwrap_or(0));
                    if let Some(current_hp) = info.current_hp {
                        entity.current_hp = Some(current_hp);
                    }
                }
            }
        }
    }
}
