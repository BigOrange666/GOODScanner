use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use log::{info, warn};

use yas::cancel::CancelToken;

use crate::scanner::common::game_controller::GenshinGameController;
use crate::scanner::common::mappings::MappingManager;
use crate::scanner::common::models::GoodArtifact;

use super::lock_manager::LockManager;
use super::models::*;

pub type ProgressFn = dyn Fn(usize, usize, &str, &str) + Send + Sync;

/// A single lock/unlock target: the artifact to match + desired lock state.
pub struct LockTarget {
    /// Result ID for this target (e.g., "lock:0" or "unlock:2").
    pub result_id: String,
    /// The artifact identity from the client (used for matching).
    pub artifact: GoodArtifact,
    /// Desired lock state: true = lock, false = unlock.
    pub desired_lock: bool,
}

pub struct ArtifactManager {
    mappings: Arc<MappingManager>,
    ocr_backend: String,
    substat_ocr_backend: String,
    delay_grid_item: u64,
    delay_scroll: u64,
    stop_on_all_matched: bool,
}

impl ArtifactManager {
    pub fn new(
        mappings: Arc<MappingManager>,
        ocr_backend: String,
        substat_ocr_backend: String,
        delay_grid_item: u64,
        delay_scroll: u64,
        stop_on_all_matched: bool,
    ) -> Self {
        Self { mappings, ocr_backend, substat_ocr_backend, delay_grid_item, delay_scroll, stop_on_all_matched }
    }

    pub fn execute(
        &self,
        ctrl: &mut GenshinGameController,
        request: LockManageRequest,
        progress_fn: Option<&ProgressFn>,
        cancel_token: CancelToken,
    ) -> (ManageResult, Option<Vec<GoodArtifact>>) {
        let mut targets: Vec<LockTarget> = Vec::new();
        let mut all_results: Vec<InstructionResult> = Vec::new();

        for (idx, artifact) in request.lock.iter().enumerate() {
            let result_id = format!("lock:{}", idx);
            if let Some(err) = validate_artifact(artifact) {
                warn!("[manager] lock[{}] 无效 / lock[{}] invalid: {}", idx, idx, err);
                all_results.push(InstructionResult {
                    id: result_id,
                    status: InstructionStatus::InvalidInput,
                    detail: Some(err),
                });
            } else {
                targets.push(LockTarget { result_id, artifact: artifact.clone(), desired_lock: true });
            }
        }

        for (idx, artifact) in request.unlock.iter().enumerate() {
            let result_id = format!("unlock:{}", idx);
            if let Some(err) = validate_artifact(artifact) {
                warn!("[manager] unlock[{}] 无效 / unlock[{}] invalid: {}", idx, idx, err);
                all_results.push(InstructionResult {
                    id: result_id,
                    status: InstructionStatus::InvalidInput,
                    detail: Some(err),
                });
            } else {
                targets.push(LockTarget { result_id, artifact: artifact.clone(), desired_lock: false });
            }
        }

        if targets.is_empty() {
            info!("[manager] 没有有效目标 / No valid targets to execute");
            let summary = ManageSummary::from_results(&all_results);
            return (ManageResult { results: all_results, summary }, None);
        }

        let total = targets.len() + all_results.len();

        ctrl.focus_game_window();
        ctrl.set_cancel_token(cancel_token.clone());

        let report = |completed: usize, phase: &str| {
            if let Some(f) = progress_fn {
                f(completed, total, "", phase);
            }
        };

        report(all_results.len(), "锁定变更 / Lock changes");

        info!(
            "[manager] 执行 {} 个锁定目标（{} 锁定, {} 解锁）/ Executing {} lock targets ({} lock, {} unlock)",
            targets.len(),
            targets.iter().filter(|t| t.desired_lock).count(),
            targets.iter().filter(|t| !t.desired_lock).count(),
            targets.len(),
            targets.iter().filter(|t| t.desired_lock).count(),
            targets.iter().filter(|t| !t.desired_lock).count(),
        );

        let lock_mgr = LockManager::new(
            self.mappings.clone(),
            self.ocr_backend.clone(),
            self.substat_ocr_backend.clone(),
        );
        let (lock_results, scanned_artifacts, matched_indices, scan_complete) = lock_mgr.execute(
            ctrl,
            &targets,
            self.delay_grid_item,
            self.delay_scroll,
            self.stop_on_all_matched,
        );

        for r in &lock_results {
            all_results.push(r.clone());
            report(all_results.len(), "锁定变更 / Lock changes");
        }

        // Mark unprocessed targets as aborted/skipped
        let processed_ids: HashSet<String> = all_results.iter().map(|r| r.id.clone()).collect();
        let was_cancelled = cancel_token.is_cancelled();
        let cancel_reason = cancel_token.reason();
        for target in &targets {
            if !processed_ids.contains(&target.result_id) {
                all_results.push(InstructionResult {
                    id: target.result_id.clone(),
                    status: if was_cancelled { InstructionStatus::Aborted } else { InstructionStatus::Skipped },
                    detail: Some(if let Some(reason) = cancel_reason {
                        format!("{}", reason)
                    } else {
                        "未处理 / Not processed".to_string()
                    }),
                });
            }
        }

        let summary = ManageSummary::from_results(&all_results);
        info!(
            "[manager] 完成：{} 成功, {} 已正确, {} 未找到, {} 错误, {} 中断 / Done: {} success, {} already correct, {} not found, {} errors, {} aborted",
            summary.success, summary.already_correct, summary.not_found, summary.errors, summary.aborted,
            summary.success, summary.already_correct, summary.not_found, summary.errors, summary.aborted,
        );

        let artifact_snapshot = if scan_complete && !scanned_artifacts.is_empty() {
            Some(build_artifact_snapshot(&scanned_artifacts, &targets, &matched_indices, &all_results))
        } else {
            None
        };

        (ManageResult { results: all_results, summary }, artifact_snapshot)
    }
}

fn validate_artifact(artifact: &GoodArtifact) -> Option<String> {
    if artifact.set_key.trim().is_empty() {
        return Some("setKey 为空 / setKey is empty".to_string());
    }
    if artifact.slot_key.trim().is_empty() {
        return Some("slotKey 为空 / slotKey is empty".to_string());
    }
    if artifact.main_stat_key.trim().is_empty() {
        return Some("mainStatKey 为空 / mainStatKey is empty".to_string());
    }
    if artifact.rarity < 4 || artifact.rarity > 5 {
        return Some(format!(
            "稀有度无效: {} (应为 4-5) / Invalid rarity: {} (must be 4-5)",
            artifact.rarity, artifact.rarity
        ));
    }
    if artifact.level < 0 || artifact.level > 20 {
        return Some(format!(
            "等级无效: {} (应为 0-20) / Invalid level: {} (must be 0-20)",
            artifact.level, artifact.level
        ));
    }
    None
}

fn build_artifact_snapshot(
    scanned_artifacts: &[(usize, GoodArtifact)],
    targets: &[LockTarget],
    matched_indices: &HashMap<usize, usize>,
    results: &[InstructionResult],
) -> Vec<GoodArtifact> {
    let result_success: HashSet<String> = results.iter()
        .filter(|r| r.status == InstructionStatus::Success)
        .map(|r| r.id.clone())
        .collect();

    // Map scanned artifact index -> desired_lock for successful toggles
    let mut toggled_to: HashMap<usize, bool> = HashMap::new();
    for (target_vec_idx, target) in targets.iter().enumerate() {
        if result_success.contains(&target.result_id) {
            if let Some(&scanned_idx) = matched_indices.get(&target_vec_idx) {
                toggled_to.insert(scanned_idx, target.desired_lock);
            }
        }
    }

    scanned_artifacts.iter().map(|(idx, artifact)| {
        if let Some(&desired_lock) = toggled_to.get(idx) {
            let mut updated = artifact.clone();
            updated.lock = desired_lock;
            updated
        } else {
            artifact.clone()
        }
    }).collect()
}
