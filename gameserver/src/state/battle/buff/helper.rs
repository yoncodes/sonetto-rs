use crate::state::battle::manager::buff_mgr::BuffMgr;

pub fn has_include_type(include_types: &str, wanted: &str) -> bool {
    include_types
        .split([',', '，', '#'])
        .any(|v| v.trim() == wanted)
}

pub fn first_feature_is_slave_halo(buff_id: i32) -> bool {
    let cfg = config::configs::get();
    cfg.skill_buff
        .iter()
        .find(|b| b.id == buff_id)
        .map(|b| {
            b.features
                .split('|')
                .next()
                .and_then(|e| e.split('#').next())
                .and_then(|id| id.trim().parse::<i32>().ok())
                .map(|act_id| {
                    cfg.buff_act
                        .iter()
                        .find(|a| a.id == act_id)
                        .map(|a| a.r#type == "SlaveHalo")
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

fn buff_include_types(buff_id: i32) -> Option<String> {
    let cfg = config::configs::get();
    let type_id = cfg.skill_buff.iter().find(|b| b.id == buff_id)?.type_id;
    cfg.skill_bufftype
        .iter()
        .find(|t| t.id == type_id)
        .map(|t| t.include_types.clone())
}

fn target_has_include_type(buff_mgr: &BuffMgr, target: i64, wanted: &str) -> bool {
    buff_mgr.get(target).iter().any(|b| {
        buff_include_types(b.buff_id).is_some_and(|types| has_include_type(&types, wanted))
    })
}

pub fn uses_slave_uid(
    buff_id: i32,
    include_types: &str,
    behavior_count: i32,
    buff_mgr: &BuffMgr,
    target: i64,
) -> bool {
    // Live behavior hint:
    // - explicit SlaveHalo feature buffs use slave UID lane
    // - includeType 10 family also consumes slave UID lane
    if first_feature_is_slave_halo(buff_id) {
        return true;
    }

    if has_include_type(include_types, "10") {
        // IncludeType 10 buffs normally use slave lane, except when the target
        // already carries an IncludeType 2 stack-family buff (e.g. 30631)
        // and this add is an explicit stack grant (count>0).
        if behavior_count > 0 && target_has_include_type(buff_mgr, target, "2") {
            return false;
        }
        return true;
    }

    false
}
