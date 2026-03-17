use std::collections::HashMap;
use std::sync::Arc;

use log::{info, warn};

use crate::scanner::artifact::GoodArtifactScanner;
use crate::scanner::common::backpack_scanner::{BackpackScanConfig, BackpackScanner, GridEvent, ScanAction};
use crate::scanner::common::game_controller::GenshinGameController;
use crate::scanner::common::mappings::MappingManager;
use crate::scanner::common::models::GoodArtifact;
use crate::scanner::common::ocr_factory;

use super::matching;
use super::models::*;
use super::ui_actions;

/// A pending lock toggle: grid index + desired lock state + elixir y_shift.
struct PendingToggle {
    instr_id: String,
    /// Grid index of the artifact in the backpack (for navigating back).
    #[allow(dead_code)]
    grid_index: usize,
    /// Desired lock state after toggle.
    #[allow(dead_code)]
    desired_lock: bool,
    y_shift: f64,
}

/// Phase 1: Iterate the artifact backpack and toggle locks as needed.
///
/// Uses a two-pass approach:
/// 1. Scan all artifacts, identify matches, collect pending toggles
/// 2. Navigate back to matched positions and toggle locks
///
/// 阶段一：遍历圣遗物背包，按需切换锁定状态。
pub struct LockManager {
    mappings: Arc<MappingManager>,
    ocr_backend: String,
    substat_ocr_backend: String,
}

impl LockManager {
    pub fn new(
        mappings: Arc<MappingManager>,
        ocr_backend: String,
        substat_ocr_backend: String,
    ) -> Self {
        Self { mappings, ocr_backend, substat_ocr_backend }
    }

    /// Execute lock change instructions by scanning the artifact backpack.
    ///
    /// Returns results for all provided instructions plus the list of scanned
    /// artifacts (for Phase 2 reuse).
    ///
    /// 执行锁定变更指令。返回所有指令的执行结果，以及扫描到的圣遗物列表。
    pub fn execute(
        &self,
        ctrl: &mut GenshinGameController,
        instructions: &[ArtifactInstruction],
        delay_grid_item: u64,
        delay_scroll: u64,
    ) -> (Vec<InstructionResult>, Vec<(usize, GoodArtifact)>) {
        let mut results: HashMap<String, InstructionResult> = HashMap::new();
        let mut scanned_artifacts: Vec<(usize, GoodArtifact)> = Vec::new();

        // Create OCR models
        let ocr_model = match ocr_factory::create_ocr_model(&self.ocr_backend) {
            Ok(m) => m,
            Err(e) => {
                warn!("OCR模型创建失败 / OCR model creation failed: {}", e);
                return (
                    instructions.iter().map(|i| InstructionResult {
                        id: i.id.clone(),
                        status: InstructionStatus::OcrError,
                        detail: Some(format!("OCR模型创建失败 / OCR model creation failed: {}", e)),
                    }).collect(),
                    scanned_artifacts,
                );
            }
        };

        let substat_ocr_model = if self.substat_ocr_backend != self.ocr_backend {
            match ocr_factory::create_ocr_model(&self.substat_ocr_backend) {
                Ok(m) => m,
                Err(e) => {
                    warn!("副属性OCR模型创建失败 / Substat OCR model creation failed: {}", e);
                    return (
                        instructions.iter().map(|i| InstructionResult {
                            id: i.id.clone(),
                            status: InstructionStatus::OcrError,
                            detail: Some(format!("OCR模型创建失败 / OCR model creation failed: {}", e)),
                        }).collect(),
                        scanned_artifacts,
                    );
                }
            }
        } else {
            ocr_factory::create_ocr_model(&self.ocr_backend).unwrap()
        };

        // Build targets for matching (only instructions with lock changes)
        let targets: Vec<(usize, &ArtifactTarget)> = instructions.iter()
            .enumerate()
            .filter(|(_, i)| i.changes.lock.is_some())
            .map(|(idx, i)| (idx, &i.target))
            .collect();

        if targets.is_empty() {
            return (Vec::new(), scanned_artifacts);
        }

        // Track which instructions have been matched
        let mut matched: HashMap<usize, usize> = HashMap::new();
        let mut pending_toggles: Vec<PendingToggle> = Vec::new();

        // ---- Pass 1: Scan all artifacts and identify matches ----
        {
            let mut scanner = BackpackScanner::new(ctrl);
            scanner.open_backpack(400);
            scanner.select_tab("artifact", 400);

            // Read total item count
            let (current, _max) = match scanner.read_item_count(ocr_model.as_ref()) {
                Ok(counts) => counts,
                Err(e) => {
                    warn!("无法读取圣遗物数量 / Cannot read artifact count: {}", e);
                    return (
                        instructions.iter().map(|i| InstructionResult {
                            id: i.id.clone(),
                            status: InstructionStatus::OcrError,
                            detail: Some(format!("无法读取数量 / Cannot read count: {}", e)),
                        }).collect(),
                        scanned_artifacts,
                    );
                }
            };
            let total = current as usize;
            info!("[lock_manager] 共 {} 个圣遗物 / {} artifacts total", total, total);

            let scaler = scanner.scaler().clone();
            let config = BackpackScanConfig {
                delay_grid_item,
                delay_scroll,
                delay_after_panel: 100,
            };

            let mappings = &self.mappings;

            scanner.scan_grid(total, &config, 0, |event| {
                match event {
                    GridEvent::Item(index, image) => {
                        if yas::utils::is_rmb_down() {
                            return ScanAction::Stop;
                        }

                        // Identify this artifact via OCR
                        let artifact = match GoodArtifactScanner::identify_artifact(
                            ocr_model.as_ref(),
                            substat_ocr_model.as_ref(),
                            &image,
                            &scaler,
                            mappings,
                        ) {
                            Ok(Some(a)) => a,
                            Ok(None) => return ScanAction::Continue,
                            Err(e) => {
                                warn!("[lock_manager] OCR失败 #{}: {} / OCR failed", index, e);
                                return ScanAction::Continue;
                            }
                        };

                        // Store for Phase 2 reuse
                        scanned_artifacts.push((index, artifact.clone()));

                        // Match against pending instructions
                        let unmatched_targets: Vec<(usize, &ArtifactTarget)> = targets.iter()
                            .filter(|(idx, _)| !matched.contains_key(idx))
                            .map(|&(idx, t)| (idx, t))
                            .collect();

                        if let Some((instr_idx, _score)) = matching::find_best_match(&artifact, &unmatched_targets) {
                            matched.insert(instr_idx, index);
                            let instr = &instructions[instr_idx];
                            let desired_lock = instr.changes.lock.unwrap();

                            if artifact.lock == desired_lock {
                                results.insert(instr.id.clone(), InstructionResult {
                                    id: instr.id.clone(),
                                    status: InstructionStatus::AlreadyCorrect,
                                    detail: Some(format!(
                                        "锁定状态已正确 / Lock already {}",
                                        if desired_lock { "locked" } else { "unlocked" }
                                    )),
                                });
                            } else {
                                // Queue for toggle in Pass 2
                                let y_shift = if artifact.elixir_crafted { 40.0 } else { 0.0 };
                                pending_toggles.push(PendingToggle {
                                    instr_id: instr.id.clone(),
                                    grid_index: index,
                                    desired_lock,
                                    y_shift,
                                });
                            }

                            // Early stop if all instructions matched
                            if matched.len() == targets.len() {
                                return ScanAction::Stop;
                            }
                        }

                        ScanAction::Continue
                    }
                    GridEvent::PageScrolled => ScanAction::Continue,
                }
            });
        }
        // BackpackScanner dropped here — `ctrl` is free again

        // ---- Pass 2: Toggle locks on matched artifacts ----
        // For now, toggling requires clicking the lock button while the artifact
        // panel is displayed. We need to navigate back to each matched grid position.
        //
        // TODO: When ui_actions::click_lock_button is implemented, this pass will:
        // 1. Re-open backpack if needed
        // 2. Navigate to each grid position (using scroll + click)
        // 3. Click the lock button
        // 4. Verify the toggle
        //
        // For now, all pending toggles get UiError since the placeholder will bail.
        for toggle in &pending_toggles {
            match ui_actions::click_lock_button(ctrl, toggle.y_shift) {
                Ok(()) => {
                    results.insert(toggle.instr_id.clone(), InstructionResult {
                        id: toggle.instr_id.clone(),
                        status: InstructionStatus::Success,
                        detail: None,
                    });
                }
                Err(e) => {
                    results.insert(toggle.instr_id.clone(), InstructionResult {
                        id: toggle.instr_id.clone(),
                        status: InstructionStatus::UiError,
                        detail: Some(format!("锁定切换失败 / Lock toggle failed: {}", e)),
                    });
                }
            }
        }

        // Mark unmatched instructions
        let was_aborted = yas::utils::was_aborted();
        for instr in instructions {
            if instr.changes.lock.is_some() && !results.contains_key(&instr.id) {
                results.insert(instr.id.clone(), InstructionResult {
                    id: instr.id.clone(),
                    status: if was_aborted {
                        InstructionStatus::Aborted
                    } else {
                        InstructionStatus::NotFound
                    },
                    detail: Some(if was_aborted {
                        "用户中断 / User aborted".to_string()
                    } else {
                        "背包中未找到匹配圣遗物 / Matching artifact not found in inventory".to_string()
                    }),
                });
            }
        }

        // Collect results in instruction order
        let ordered_results: Vec<InstructionResult> = instructions.iter()
            .filter_map(|i| results.remove(&i.id))
            .collect();

        (ordered_results, scanned_artifacts)
    }
}
