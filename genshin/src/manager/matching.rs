use std::collections::HashSet;

use crate::scanner::common::models::GoodArtifact;

use super::models::ArtifactTarget;

/// Match a scanned `GoodArtifact` against an `ArtifactTarget`.
///
/// Returns `None` if hard fields (set, slot, rarity, level, main stat) don't match.
/// Returns `Some(score)` where higher score means better match.
/// Substats are used for disambiguation when multiple artifacts share the same hard fields.
///
/// 将扫描到的 `GoodArtifact` 与 `ArtifactTarget` 进行匹配。
/// 如果硬性字段不匹配则返回 None，否则返回匹配分数（越高越好）。
pub fn match_score(scanned: &GoodArtifact, target: &ArtifactTarget) -> Option<f64> {
    // Hard-reject on any mismatch in highly reliable fields
    if scanned.rarity != target.rarity {
        return None;
    }
    if scanned.slot_key != target.slot_key {
        return None;
    }
    if scanned.set_key != target.set_key {
        return None;
    }
    if scanned.level != target.level {
        return None;
    }
    if scanned.main_stat_key != target.main_stat_key {
        return None;
    }

    // Base score for matching all hard fields
    let mut score: f64 = 50.0;

    // Substat key matching (order-independent)
    let scanned_keys: HashSet<&str> = scanned.substats.iter().map(|s| s.key.as_str()).collect();
    let target_keys: HashSet<&str> = target.substats.iter().map(|s| s.key.as_str()).collect();

    if scanned_keys == target_keys {
        score += 20.0;

        // Value matching for disambiguation
        for ts in &target.substats {
            if let Some(ss) = scanned.substats.iter().find(|s| s.key == ts.key) {
                let diff = (ss.value - ts.value).abs();
                // Percent stats (keys ending with _) have smaller values, use tighter tolerance
                let tolerance = if ts.key.ends_with('_') {
                    0.2
                } else {
                    1.5
                };
                if diff <= tolerance {
                    score += 5.0;
                }
            }
        }
    } else {
        // Partial substat key match
        let overlap = scanned_keys.intersection(&target_keys).count();
        score += overlap as f64 * 3.0;
    }

    Some(score)
}

/// Find the best matching instruction index for a scanned artifact.
/// Returns `(index, score)` of the best match, or None if no instruction matches.
///
/// 找到与扫描到的圣遗物最佳匹配的指令索引。
pub fn find_best_match(
    scanned: &GoodArtifact,
    targets: &[(usize, &ArtifactTarget)],
) -> Option<(usize, f64)> {
    let mut best: Option<(usize, f64)> = None;
    for &(idx, target) in targets {
        if let Some(score) = match_score(scanned, target) {
            if best.map_or(true, |(_, best_score)| score > best_score) {
                best = Some((idx, score));
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::common::models::GoodSubStat;

    fn make_artifact(set: &str, slot: &str, rarity: i32, level: i32, main: &str, subs: &[(&str, f64)]) -> GoodArtifact {
        GoodArtifact {
            set_key: set.to_string(),
            slot_key: slot.to_string(),
            rarity,
            level,
            main_stat_key: main.to_string(),
            substats: subs.iter().map(|(k, v)| GoodSubStat {
                key: k.to_string(),
                value: *v,
                initial_value: None,
            }).collect(),
            location: String::new(),
            lock: false,
            astral_mark: false,
            elixir_crafted: false,
            unactivated_substats: Vec::new(),
            total_rolls: None,
        }
    }

    fn make_target(set: &str, slot: &str, rarity: i32, level: i32, main: &str, subs: &[(&str, f64)]) -> ArtifactTarget {
        ArtifactTarget {
            set_key: set.to_string(),
            slot_key: slot.to_string(),
            rarity,
            level,
            main_stat_key: main.to_string(),
            substats: subs.iter().map(|(k, v)| GoodSubStat {
                key: k.to_string(),
                value: *v,
                initial_value: None,
            }).collect(),
        }
    }

    #[test]
    fn test_exact_match() {
        let artifact = make_artifact("GladiatorsFinale", "flower", 5, 20, "hp",
            &[("critRate_", 3.9), ("critDMG_", 7.8), ("atk_", 5.8), ("def", 23.0)]);
        let target = make_target("GladiatorsFinale", "flower", 5, 20, "hp",
            &[("critRate_", 3.9), ("critDMG_", 7.8), ("atk_", 5.8), ("def", 23.0)]);

        let score = match_score(&artifact, &target);
        assert!(score.is_some());
        // All hard fields + all substat keys + all values within tolerance
        assert!(score.unwrap() > 80.0);
    }

    #[test]
    fn test_hard_field_mismatch() {
        let artifact = make_artifact("GladiatorsFinale", "flower", 5, 20, "hp", &[]);
        let target = make_target("GladiatorsFinale", "plume", 5, 20, "hp", &[]);
        assert!(match_score(&artifact, &target).is_none());

        let target2 = make_target("GladiatorsFinale", "flower", 4, 20, "hp", &[]);
        assert!(match_score(&artifact, &target2).is_none());

        let target3 = make_target("GladiatorsFinale", "flower", 5, 16, "hp", &[]);
        assert!(match_score(&artifact, &target3).is_none());
    }

    #[test]
    fn test_partial_substat_match() {
        let artifact = make_artifact("GladiatorsFinale", "flower", 5, 20, "hp",
            &[("critRate_", 3.9), ("critDMG_", 7.8), ("atk_", 5.8), ("def", 23.0)]);
        // Target has 3 matching substats and 1 different
        let target = make_target("GladiatorsFinale", "flower", 5, 20, "hp",
            &[("critRate_", 3.9), ("critDMG_", 7.8), ("atk_", 5.8), ("hp_", 4.7)]);

        let score = match_score(&artifact, &target);
        assert!(score.is_some());
        // Partial match = base(50) + 3*3 = 59
        assert!(score.unwrap() < 70.0);
    }

    #[test]
    fn test_disambiguation_by_value() {
        // Two artifacts with same hard fields and substat keys but different values
        let art1 = make_artifact("GladiatorsFinale", "flower", 5, 20, "hp",
            &[("critRate_", 3.9), ("critDMG_", 7.8)]);
        let art2 = make_artifact("GladiatorsFinale", "flower", 5, 20, "hp",
            &[("critRate_", 10.5), ("critDMG_", 15.6)]);

        let target = make_target("GladiatorsFinale", "flower", 5, 20, "hp",
            &[("critRate_", 10.5), ("critDMG_", 15.6)]);

        let score1 = match_score(&art1, &target).unwrap();
        let score2 = match_score(&art2, &target).unwrap();
        assert!(score2 > score1, "Closer values should score higher");
    }
}
