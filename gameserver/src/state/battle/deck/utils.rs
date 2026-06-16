pub(crate) fn card_limit(alive_count: usize, has_support: bool) -> usize {
    match alive_count {
        1 => 4,
        2 => 5,
        3 => {
            if has_support {
                7
            } else {
                6
            }
        }
        4 => 8,
        _ => (alive_count + 4).min(9),
    }
}

pub fn default_max_ap(episode_id: i32, hero_count: usize) -> i32 {
    let game_data = config::configs::get();

    let battle_id = game_data
        .episode
        .iter()
        .find(|t| t.id == episode_id)
        .map(|t| t.battle_id)
        .unwrap_or(0);

    let base_ap = game_data
        .battle
        .iter()
        .find(|t| t.id == battle_id)
        .map(|t| t.player_max)
        .unwrap_or(0);

    let hero_ap = match hero_count {
        0..=2 => 2,
        _ => 4,
    };
    base_ap.min(hero_ap)
}
