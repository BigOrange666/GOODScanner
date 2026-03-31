use std::collections::HashMap;
use std::sync::Arc;

use log::{info, warn};

use crate::scanner::common::coord_scaler::CoordScaler;
use crate::scanner::common::game_controller::GenshinGameController;
use crate::scanner::common::mappings::MappingManager;
use crate::scanner::common::models::GoodArtifact;
use crate::scanner::common::ocr_factory;

use super::models::*;
use super::ui_actions;

/// A single equip/unequip target with a result ID for tracking.
pub struct EquipTarget {
    pub result_id: String,
    pub artifact: GoodArtifact,
    pub target_location: String,
}

/// Executes equip/unequip operations by navigating character screens.
pub struct EquipManager {
    mappings: Arc<MappingManager>,
    ocr_backend: String,
    #[allow(dead_code)]
    substat_ocr_backend: String,
}

impl EquipManager {
    pub fn new(
        mappings: Arc<MappingManager>,
        ocr_backend: String,
        substat_ocr_backend: String,
    ) -> Self {
        Self { mappings, ocr_backend, substat_ocr_backend }
    }

    /// Execute a list of equip/unequip targets.
    ///
    /// Groups targets by destination character, processes unequip first,
    /// then equip groups one character at a time.
    pub fn execute(
        &self,
        ctrl: &mut GenshinGameController,
        targets: &[EquipTarget],
    ) -> Vec<InstructionResult> {
        let ocr = match ocr_factory::create_ocr_model(&self.ocr_backend) {
            Ok(m) => m,
            Err(e) => {
                warn!("OCR模型创建失败 / OCR model creation failed: {}", e);
                return targets.iter().map(|t| InstructionResult {
                    id: t.result_id.clone(),
                    status: InstructionStatus::OcrError,
                }).collect();
            }
        };

        let scaler = ctrl.scaler.clone();
        let mut results: HashMap<String, InstructionResult> = HashMap::new();

        // Group targets by target_location
        let mut groups: HashMap<&str, Vec<&EquipTarget>> = HashMap::new();
        for target in targets {
            groups.entry(target.target_location.as_str())
                .or_default()
                .push(target);
        }

        // Process unequip group first (location = "")
        if let Some(unequip_targets) = groups.remove("") {
            for target in &unequip_targets {
                if ctrl.check_rmb() {
                    results.insert(target.result_id.clone(), InstructionResult {
                        id: target.result_id.clone(),
                        status: InstructionStatus::Aborted,
                    });
                    continue;
                }

                // If artifact has no current owner, it's already unequipped
                if target.artifact.location.is_empty() {
                    results.insert(target.result_id.clone(), InstructionResult {
                        id: target.result_id.clone(),
                        status: InstructionStatus::AlreadyCorrect,
                    });
                    continue;
                }

                let result = self.do_unequip(ctrl, target, &target.artifact.location);
                results.insert(target.result_id.clone(), result);
            }
        }

        // Process equip groups (one character at a time)
        for (char_key, equip_targets) in &groups {
            for target in equip_targets {
                if ctrl.check_rmb() {
                    results.insert(target.result_id.clone(), InstructionResult {
                        id: target.result_id.clone(),
                        status: InstructionStatus::Aborted,
                    });
                    continue;
                }

                // If artifact is already at the target location, skip
                if target.artifact.location == *char_key {
                    results.insert(target.result_id.clone(), InstructionResult {
                        id: target.result_id.clone(),
                        status: InstructionStatus::AlreadyCorrect,
                    });
                    continue;
                }

                let result = self.do_equip(ctrl, target, char_key, ocr.as_ref(), &scaler);
                results.insert(target.result_id.clone(), result);
            }
        }

        // Leave character screen after all operations
        let _ = ui_actions::leave_character_screen(ctrl);

        // Return results in target order
        targets.iter().map(|t| {
            results.remove(&t.result_id).unwrap_or(InstructionResult {
                id: t.result_id.clone(),
                status: InstructionStatus::Skipped,
            })
        }).collect()
    }

    fn do_equip(
        &self,
        ctrl: &mut GenshinGameController,
        target: &EquipTarget,
        char_key: &str,
        ocr: &dyn yas::ocr::ImageToText<image::RgbImage>,
        scaler: &CoordScaler,
    ) -> InstructionResult {
        if let Err(e) = ui_actions::open_character_screen(ctrl, char_key, &self.mappings) {
            warn!("[equip_manager] 打开角色界面失败: {} / open character screen failed: {}", e, e);
            return InstructionResult {
                id: target.result_id.clone(),
                status: InstructionStatus::UiError,
            };
        }

        if let Err(e) = ui_actions::click_equipment_slot(ctrl, &target.artifact.slot_key) {
            warn!("[equip_manager] 点击装备栏位失败: {} / click equipment slot failed: {}", e, e);
            return InstructionResult {
                id: target.result_id.clone(),
                status: InstructionStatus::UiError,
            };
        }

        match ui_actions::find_and_click_artifact_in_selection(
            ctrl,
            &target.artifact,
            ocr,
            scaler,
            &self.mappings,
        ) {
            Ok(true) => {
                info!("[equip_manager] 装备成功: {} -> {} / equip success", target.result_id, char_key);
                InstructionResult {
                    id: target.result_id.clone(),
                    status: InstructionStatus::Success,
                }
            }
            Ok(false) => {
                warn!("[equip_manager] 未找到目标圣遗物: {} / artifact not found", target.result_id);
                InstructionResult {
                    id: target.result_id.clone(),
                    status: InstructionStatus::NotFound,
                }
            }
            Err(e) => {
                warn!("[equip_manager] 装备操作失败: {} / equip failed: {}", e, e);
                InstructionResult {
                    id: target.result_id.clone(),
                    status: InstructionStatus::UiError,
                }
            }
        }
    }

    fn do_unequip(
        &self,
        ctrl: &mut GenshinGameController,
        target: &EquipTarget,
        current_owner: &str,
    ) -> InstructionResult {
        if let Err(e) = ui_actions::open_character_screen(ctrl, current_owner, &self.mappings) {
            warn!("[equip_manager] 打开角色界面失败: {} / open character screen failed: {}", e, e);
            return InstructionResult {
                id: target.result_id.clone(),
                status: InstructionStatus::UiError,
            };
        }

        if let Err(e) = ui_actions::click_equipment_slot(ctrl, &target.artifact.slot_key) {
            warn!("[equip_manager] 点击装备栏位失败: {} / click equipment slot failed: {}", e, e);
            return InstructionResult {
                id: target.result_id.clone(),
                status: InstructionStatus::UiError,
            };
        }

        if let Err(e) = ui_actions::click_unequip_button(ctrl) {
            warn!("[equip_manager] 卸下操作失败: {} / unequip failed: {}", e, e);
            return InstructionResult {
                id: target.result_id.clone(),
                status: InstructionStatus::UiError,
            };
        }

        info!("[equip_manager] 卸下成功: {} (from {}) / unequip success", target.result_id, current_owner);
        InstructionResult {
            id: target.result_id.clone(),
            status: InstructionStatus::Success,
        }
    }
}
