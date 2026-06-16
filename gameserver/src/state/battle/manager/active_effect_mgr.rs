use std::collections::HashMap;
use crate::state::battle::effect::SkillEffect;

#[derive(Default, Debug, Clone)]
pub struct ActiveEffectMgr {
    next_idx: usize,
    pub map: HashMap<usize, Vec<SkillEffect>>,
    /// Names the single `map` entry eligible to participate in the current
    /// hook fanout. Set by `SkillExecutor::execute_skill` for the duration
    /// of the call and restored on exit by `ActiveEffectGuard`.
    pub active_idx: Option<usize>,
}

impl ActiveEffectMgr {
    pub fn push(&mut self, effects: Vec<SkillEffect>) -> usize {
        let idx = self.next_idx;
        self.next_idx += 1;
        self.map.insert(idx, effects);
        idx
    }

    pub fn void(&mut self, idx: usize) {
        self.map.remove(&idx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::battle::effect::SkillEffect;

    #[test]
    fn push_returns_increasing_indices() {
        let mut m = ActiveEffectMgr::default();
        let a = m.push(vec![SkillEffect::empty(1)]);
        let b = m.push(vec![SkillEffect::empty(2)]);
        assert_ne!(a, b);
        assert!(m.map.contains_key(&a));
        assert!(m.map.contains_key(&b));
    }

    #[test]
    fn void_removes_entry() {
        let mut m = ActiveEffectMgr::default();
        let idx = m.push(vec![SkillEffect::empty(1)]);
        m.void(idx);
        assert!(!m.map.contains_key(&idx));
    }

    #[test]
    fn void_unknown_idx_is_noop() {
        let mut m = ActiveEffectMgr::default();
        m.void(99); // no panic
        assert!(m.map.is_empty());
    }

    #[test]
    fn active_idx_defaults_to_none() {
        let m = ActiveEffectMgr::default();
        assert!(m.active_idx.is_none());
    }
}
