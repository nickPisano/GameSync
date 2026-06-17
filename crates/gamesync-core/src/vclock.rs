//! Version-vector comparison — the core of conflict detection.
//!
//! A version vector maps `device_id → counter`. When comparing two snapshots'
//! vectors we get one of four relations; only `Concurrent` is a real conflict.

use crate::model::VectorClock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ordering {
    /// Identical histories.
    Equal,
    /// `a` is strictly newer than `b` (safe fast-forward up: push).
    Dominates,
    /// `a` is strictly older than `b` (safe fast-forward down: pull).
    DominatedBy,
    /// Neither dominates — both sides diverged. This is a conflict.
    Concurrent,
}

/// Compare two version vectors.
pub fn compare(a: &VectorClock, b: &VectorClock) -> Ordering {
    let mut a_greater = false;
    let mut b_greater = false;
    for key in a.keys().chain(b.keys()) {
        let av = a.get(key).copied().unwrap_or(0);
        let bv = b.get(key).copied().unwrap_or(0);
        if av > bv {
            a_greater = true;
        }
        if bv > av {
            b_greater = true;
        }
    }
    match (a_greater, b_greater) {
        (false, false) => Ordering::Equal,
        (true, false) => Ordering::Dominates,
        (false, true) => Ordering::DominatedBy,
        (true, true) => Ordering::Concurrent,
    }
}

/// Pointwise maximum of two vectors — the causal join used when resolving a
/// conflict (the result dominates both inputs).
pub fn merge(a: &VectorClock, b: &VectorClock) -> VectorClock {
    let mut out = a.clone();
    for (k, v) in b {
        let e = out.entry(k.clone()).or_insert(0);
        *e = (*e).max(*v);
    }
    out
}

/// Derive the vector for a new snapshot from a base vector, by incrementing this
/// device's counter.
pub fn bump(base: &VectorClock, device_id: &str) -> VectorClock {
    let mut out = base.clone();
    *out.entry(device_id.to_string()).or_insert(0) += 1;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vc(pairs: &[(&str, u64)]) -> VectorClock {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn relations() {
        assert_eq!(compare(&vc(&[("a", 1)]), &vc(&[("a", 1)])), Ordering::Equal);
        assert_eq!(
            compare(&vc(&[("a", 2)]), &vc(&[("a", 1)])),
            Ordering::Dominates
        );
        assert_eq!(
            compare(&vc(&[("a", 1)]), &vc(&[("a", 2)])),
            Ordering::DominatedBy
        );
        // a went one way, b another from a common ancestor {a:1}
        assert_eq!(
            compare(&vc(&[("a", 2)]), &vc(&[("a", 1), ("b", 1)])),
            Ordering::Concurrent
        );
    }

    #[test]
    fn merge_and_bump() {
        let m = merge(&vc(&[("a", 2)]), &vc(&[("a", 1), ("b", 1)]));
        assert_eq!(m, vc(&[("a", 2), ("b", 1)]));
        assert_eq!(
            bump(&vc(&[("a", 2), ("b", 1)]), "b"),
            vc(&[("a", 2), ("b", 2)])
        );
        // The merged+bumped vector dominates both originals.
        let resolved = bump(&m, "a");
        assert_eq!(compare(&resolved, &vc(&[("a", 2)])), Ordering::Dominates);
        assert_eq!(
            compare(&resolved, &vc(&[("a", 1), ("b", 1)])),
            Ordering::Dominates
        );
    }
}
