use rand::Rng;
use sonettobuf::CardInfo;
use std::collections::HashMap;

fn skill_key(card: &CardInfo) -> i32 {
    card.skill_id.unwrap_or(0)
}

pub fn draw_deck_guaranteed_by_uid_with_rng<R: Rng + ?Sized>(
    cards: &[CardInfo],
    required_uids: &[i64],
    count: usize,
    rng: &mut R,
) -> Vec<CardInfo> {
    if cards.is_empty() || count == 0 {
        return vec![];
    }

    let mut by_uid: HashMap<i64, Vec<&CardInfo>> = HashMap::new();
    for card in cards {
        by_uid.entry(card.uid.unwrap_or(0)).or_default().push(card);
    }

    loop {
        let mut out: Vec<CardInfo> = Vec::with_capacity(count);

        for &uid in required_uids {
            if out.len() >= count {
                break;
            }
            if let Some(options) = by_uid.get(&uid)
                && !options.is_empty()
            {
                let prev_key = out.last().map(skill_key);
                let non_touching: Vec<&CardInfo> = match prev_key {
                    Some(prev) => options.iter().copied().filter(|c| skill_key(c) != prev).collect(),
                    None => options.clone(),
                };
                let pool = if non_touching.is_empty() { options.as_slice() } else { &non_touching };
                out.push(pool[rng.gen_range(0..pool.len())].clone());
            }
        }

        while out.len() < count {
            let prev_key = out.last().map(skill_key);
            let non_touching: Vec<&CardInfo> = match prev_key {
                Some(prev) => cards.iter().filter(|c| skill_key(c) != prev).collect(),
                None => cards.iter().collect(),
            };
            let pool = if non_touching.is_empty() { cards.iter().collect::<Vec<_>>() } else { non_touching };
            out.push(pool[rng.gen_range(0..pool.len())].clone());
        }

        if count % 2 == 1 {
            out[count - 1] = out[0].clone();
        }

        if !out.windows(2).any(|w| skill_key(&w[0]) == skill_key(&w[1])) {
            return out;
        }
    }
}
