use std::sync::Arc;

use log::info;

use crate::scanner::common::game_controller::GenshinGameController;
use crate::scanner::common::mappings::MappingManager;

use super::lock_manager::LockManager;
use super::equip_manager::EquipManager;
use super::models::*;

/// Top-level artifact manager that orchestrates lock and equip operations.
///
/// 顶层圣遗物管理器，编排锁定和装备操作。
pub struct ArtifactManager {
    mappings: Arc<MappingManager>,
    ocr_backend: String,
    substat_ocr_backend: String,
    /// Delay (ms) between grid item clicks during backpack scan.
    pub delay_grid_item: u64,
    /// Delay (ms) after scrolling during backpack scan.
    pub delay_scroll: u64,
}

impl ArtifactManager {
    pub fn new(
        mappings: Arc<MappingManager>,
        ocr_backend: String,
        substat_ocr_backend: String,
    ) -> Self {
        Self {
            mappings,
            ocr_backend,
            substat_ocr_backend,
            delay_grid_item: 60,
            delay_scroll: 200,
        }
    }

    /// Execute all instructions in the request.
    ///
    /// Runs Phase 1 (lock changes via backpack scan) then Phase 2 (equip changes
    /// via character screens). The backpack scan results from Phase 1 are reused
    /// in Phase 2 for pre-flight validation.
    ///
    /// 执行请求中的所有指令。先执行阶段一（通过背包扫描更改锁定），
    /// 再执行阶段二（通过角色界面更改装备）。
    pub fn execute(
        &self,
        ctrl: &mut GenshinGameController,
        request: ArtifactManageRequest,
    ) -> ManageResult {
        let mut all_results: Vec<InstructionResult> = Vec::new();

        // Focus the game window before any UI interactions
        ctrl.focus_game_window();

        // Partition instructions
        let has_lock_changes: Vec<&ArtifactInstruction> = request.instructions.iter()
            .filter(|i| i.changes.lock.is_some())
            .collect();
        let has_equip_changes: Vec<&ArtifactInstruction> = request.instructions.iter()
            .filter(|i| i.changes.location.is_some())
            .collect();

        info!(
            "[manager] 收到 {} 条指令：{} 条锁定变更，{} 条装备变更 / \
             Received {} instructions: {} lock changes, {} equip changes",
            request.instructions.len(),
            has_lock_changes.len(),
            has_equip_changes.len(),
            request.instructions.len(),
            has_lock_changes.len(),
            has_equip_changes.len(),
        );

        // Phase 1: Lock changes (iterate artifact backpack)
        let mut scanned_artifacts = Vec::new();
        let lock_instructions: Vec<ArtifactInstruction> = request.instructions.iter()
            .filter(|i| i.changes.lock.is_some())
            .cloned()
            .collect();

        if !lock_instructions.is_empty() && !yas::utils::was_aborted() {
            let lock_mgr = LockManager::new(
                self.mappings.clone(),
                self.ocr_backend.clone(),
                self.substat_ocr_backend.clone(),
            );
            let (lock_results, artifacts) = lock_mgr.execute(
                ctrl,
                &lock_instructions,
                self.delay_grid_item,
                self.delay_scroll,
            );
            scanned_artifacts = artifacts;
            all_results.extend(lock_results);

            // Return to main UI between phases
            if !yas::utils::was_aborted() {
                ctrl.return_to_main_ui(4);
            }
        }

        // Phase 2: Equip changes (navigate character screens)
        let equip_instructions: Vec<ArtifactInstruction> = request.instructions.iter()
            .filter(|i| i.changes.location.is_some())
            .cloned()
            .collect();

        if !equip_instructions.is_empty() && !yas::utils::was_aborted() {
            let equip_mgr = EquipManager::new(
                self.mappings.clone(),
                self.ocr_backend.clone(),
                self.substat_ocr_backend.clone(),
            );
            let equip_results = equip_mgr.execute(
                ctrl, &equip_instructions, &scanned_artifacts,
            );
            all_results.extend(equip_results);
        }

        // Mark any remaining instructions as aborted
        let processed_ids: std::collections::HashSet<String> = all_results.iter()
            .map(|r| r.id.clone())
            .collect();

        for instr in &request.instructions {
            if !processed_ids.contains(&instr.id) {
                all_results.push(InstructionResult {
                    id: instr.id.clone(),
                    status: if yas::utils::was_aborted() {
                        InstructionStatus::Aborted
                    } else {
                        InstructionStatus::Skipped
                    },
                    detail: Some(if yas::utils::was_aborted() {
                        "用户中断 / User aborted".to_string()
                    } else {
                        "未处理 / Not processed".to_string()
                    }),
                });
            }
        }

        let summary = ManageSummary::from_results(&all_results);
        info!(
            "[manager] 完成：{} 成功, {} 已正确, {} 未找到, {} 错误, {} 中断 / \
             Done: {} success, {} already correct, {} not found, {} errors, {} aborted",
            summary.success, summary.already_correct, summary.not_found, summary.errors, summary.aborted,
            summary.success, summary.already_correct, summary.not_found, summary.errors, summary.aborted,
        );

        ManageResult {
            results: all_results,
            summary,
        }
    }
}
