use std::collections::HashMap;
use std::sync::Arc;

use crossbeam_channel;

use log::{debug, info, warn};

use crate::scanner::artifact::GoodArtifactScanner;
use crate::scanner::common::backpack_scanner;
use crate::scanner::common::constants::*;
use crate::scanner::common::game_controller::GenshinGameController;
use crate::scanner::common::mappings::MappingManager;
use crate::scanner::common::models::GoodArtifact;
use crate::scanner::common::ocr_factory;
use crate::scanner::common::ocr_pool::OcrPool;
use crate::scanner::common::pixel_utils;

use super::matching;
use super::models::*;
use super::orchestrator::LockTarget;
use super::ui_actions;

/// Single-pass artifact backpack scan with per-page lock toggling.
///
/// Accepts `LockTarget` slices from the orchestrator. Each target specifies
/// an artifact to match and the desired lock state. Rarity early-stop:
/// when a scanned artifact has rarity < 4, the current page is finished
/// but no further items are dispatched.
///
/// 单次遍历圣遗物背包，每页扫描后直接切换锁定。
/// 当检测到稀有度 < 4 时，完成当前页后停止扫描。
pub struct LockManager {
    mappings: Arc<MappingManager>,
    ocr_backend: String,
    substat_ocr_backend: String,
}

/// An artifact identified on the current page that needs a lock toggle.
struct PageToggle {
    result_id: String,
    /// Row within the current visible page (0-based).
    row: usize,
    /// Column (0-based).
    col: usize,
    /// Desired lock state.
    desired_lock: bool,
    /// Y-shift for elixir artifacts.
    y_shift: f64,
}

/// Panel pool rect for wait_until_panel_loaded (same as backpack_scanner).
const PANEL_POOL_RECT: (f64, f64, f64, f64) = (1400.0, 300.0, 300.0, 200.0);

impl LockManager {
    pub fn new(
        mappings: Arc<MappingManager>,
        ocr_backend: String,
        substat_ocr_backend: String,
    ) -> Self {
        Self { mappings, ocr_backend, substat_ocr_backend }
    }

    /// Execute lock change targets by scanning the artifact backpack.
    ///
    /// Returns:
    /// - Results for processed targets
    /// - Scanned artifacts as (index, artifact)
    /// - Map from target vec index -> scanned artifact index (for snapshot building)
    /// - Whether the scan completed fully (all items visited, no interruption)
    ///
    /// 执行锁定变更目标。返回：
    /// - 已处理目标的结果
    /// - 扫描到的圣遗物列表 (index, artifact)
    /// - 目标向量索引→圣遗物索引的映射
    /// - 扫描是否完整
    pub fn execute(
        &self,
        ctrl: &mut GenshinGameController,
        targets: &[LockTarget],
        delay_grid_item: u64,
        delay_scroll: u64,
        stop_on_all_matched: bool,
        max_target_level: i32,
    ) -> (Vec<InstructionResult>, Vec<(usize, GoodArtifact)>, HashMap<usize, usize>, bool) {
        let mut results: HashMap<String, InstructionResult> = HashMap::new();
        let mut scanned_artifacts: Vec<(usize, GoodArtifact)> = Vec::new();

        let make_error_results = |targets: &[LockTarget], status: InstructionStatus| -> Vec<InstructionResult> {
            targets.iter().map(|t| InstructionResult {
                id: t.result_id.clone(),
                status: status.clone(),
            }).collect()
        };

        // Create OCR pools (same sizes as artifact scanner: 2 main + 5 substat)
        let ocr_backend = self.ocr_backend.clone();
        let ocr_pool = match OcrPool::new(
            move || ocr_factory::create_ocr_model(&ocr_backend),
            2,
        ) {
            Ok(p) => Arc::new(p),
            Err(e) => {
                warn!("OCR模型创建失败 / OCR model creation failed: {}", e);
                return (
                    make_error_results(targets, InstructionStatus::OcrError),
                    scanned_artifacts,
                    HashMap::new(),
                    false,
                );
            }
        };
        let substat_backend = self.substat_ocr_backend.clone();
        let substat_pool = match OcrPool::new(
            move || ocr_factory::create_ocr_model(&substat_backend),
            5,
        ) {
            Ok(p) => Arc::new(p),
            Err(e) => {
                warn!("副属性OCR模型创建失败 / Substat OCR model creation failed: {}", e);
                return (
                    make_error_results(targets, InstructionStatus::OcrError),
                    scanned_artifacts,
                    HashMap::new(),
                    false,
                );
            }
        };
        // Single model for count OCR (used once during backpack open)
        let count_ocr = match ocr_factory::create_ocr_model(&self.ocr_backend) {
            Ok(m) => m,
            Err(e) => {
                warn!("OCR模型创建失败 / OCR model creation failed: {}", e);
                return (
                    make_error_results(targets, InstructionStatus::OcrError),
                    scanned_artifacts,
                    HashMap::new(),
                    false,
                );
            }
        };

        if targets.is_empty() {
            return (Vec::new(), scanned_artifacts, HashMap::new(), false);
        }

        // Track which targets have been matched (target vec index -> scanned artifact index)
        let mut matched: HashMap<usize, usize> = HashMap::new();

        // --- Open backpack to artifact tab (same as artifact scanner) ---
        let total = match backpack_scanner::open_backpack_to_tab(
            ctrl, "artifact", 1200, 400, count_ocr.as_ref(),
        ) {
            Ok((count, _max)) => {
                info!("[lock_manager] 共 {} 个圣遗物 / {} artifacts total", count, count);
                count as usize
            }
            Err(e) => {
                warn!("无法读取圣遗物数量 / Cannot read artifact count: {}", e);
                return (
                    make_error_results(targets, InstructionStatus::OcrError),
                    scanned_artifacts,
                    HashMap::new(),
                    false,
                );
            }
        };

        if total == 0 {
            info!("[lock_manager] 背包中没有圣遗物 / No artifacts in backpack");
            return (
                targets.iter().map(|t| InstructionResult {
                    id: t.result_id.clone(),
                    status: InstructionStatus::NotFound,
                }).collect(),
                scanned_artifacts,
                HashMap::new(),
                true, // empty backpack is a "complete" scan
            );
        }

        let scaler = ctrl.scaler.clone();
        let total_rows = (total + GRID_COLS - 1) / GRID_COLS;
        let last_row_cols = if total % GRID_COLS == 0 { GRID_COLS } else { total % GRID_COLS };

        // Timing: same two-phase capture as artifact scanner
        let delay_after_panel = delay_grid_item; // same as scanner
        let first_delay = delay_after_panel / 2;
        let retry_delay = delay_after_panel;

        // Click first item to ensure focus
        ctrl.click_at(GRID_FIRST_X, GRID_FIRST_Y);
        yas::utils::sleep(300);

        let mut scanned_count: usize = 0;
        let mut scanned_row: usize = 0;
        let mut pages_scrolled: u32 = 0;
        let mut stop_after_page = false;
        let mut page_skipped: bool;

        let scaler_arc = Arc::new(scaler.clone());

        // --- Per-page scan + lock toggle loop ---
        'outer: while scanned_count < total {
            let visible_rows = GRID_ROWS.min(total_rows - scanned_row);

            // Page-skip optimization: in fast mode, click the last item on this page
            // and OCR its level. If it's strictly higher than the highest target level,
            // all items on this page are too high (inventory is sorted by level descending)
            // and we can skip the entire page.
            page_skipped = false;
            if max_target_level >= 0 {
                let last_row = visible_rows - 1;
                let last_col = if scanned_row + last_row == total_rows - 1 {
                    last_row_cols - 1
                } else {
                    GRID_COLS - 1
                };
                let x = GRID_FIRST_X + last_col as f64 * GRID_OFFSET_X;
                let y = GRID_FIRST_Y + last_row as f64 * GRID_OFFSET_Y;
                ctrl.click_at(x, y);
                let _ = ctrl.wait_until_panel_loaded(PANEL_POOL_RECT, 400);
                yas::utils::sleep(first_delay as u32);

                if let Ok(image) = ctrl.capture_game() {
                    let ocr_guard = ocr_pool.get();
                    let level = GoodArtifactScanner::scan_level_only(
                        &ocr_guard as &dyn yas::ocr::ImageToText<image::RgbImage>,
                        &image,
                        &scaler,
                    );
                    if level > max_target_level {
                        let page_items = (0..visible_rows).map(|r| {
                            if scanned_row + r == total_rows - 1 { last_row_cols } else { GRID_COLS }
                        }).sum::<usize>();
                        info!(
                            "[lock_manager] 页面跳过：末尾等级={} > 最高目标等级={}，跳过 {} 个 / Page skip: last level={} > max target={}, skipping {} items",
                            level, max_target_level, page_items, level, max_target_level, page_items
                        );
                        scanned_count += page_items;
                        scanned_row += visible_rows;
                        page_skipped = true;
                    }
                }
            }

            if !page_skipped {

            // Result channel for pipelined OCR
            let (result_tx, result_rx) = crossbeam_channel::unbounded::<(usize, usize, usize, Option<GoodArtifact>)>();
            let mut dispatched: usize = 0;

            // Step 1: Click through items, capture, dispatch OCR to rayon immediately
            for row in 0..visible_rows {
                let row_cols = if scanned_row + row == total_rows - 1 {
                    last_row_cols
                } else {
                    GRID_COLS
                };

                for col in 0..row_cols {
                    if ctrl.check_rmb() || scanned_count >= total {
                        break 'outer;
                    }

                    // If rarity early-stop triggered, don't dispatch more items
                    if stop_after_page {
                        break 'outer;
                    }

                    let x = GRID_FIRST_X + col as f64 * GRID_OFFSET_X;
                    let y = GRID_FIRST_Y + row as f64 * GRID_OFFSET_Y;
                    ctrl.click_at(x, y);

                    // Wait for panel to load (same as scan_grid)
                    let _ = ctrl.wait_until_panel_loaded(PANEL_POOL_RECT, 400);

                    // Two-phase capture with retry for lock/astral animation
                    if first_delay > 0 {
                        yas::utils::sleep(first_delay as u32);
                    }

                    let mut image = match ctrl.capture_game() {
                        Ok(img) => img,
                        Err(e) => {
                            warn!("[lock_manager] 截图失败 #{}: {} / [lock_manager] capture failed #{}: {}", scanned_count, e, scanned_count, e);
                            scanned_count += 1;
                            continue;
                        }
                    };

                    // Retry if icon brightness is ambiguous (mid-animation)
                    if retry_delay > 0 && pixel_utils::is_artifact_icon_ambiguous(&image, &scaler) {
                        yas::utils::sleep(retry_delay as u32);
                        image = match ctrl.capture_game() {
                            Ok(img) => img,
                            Err(e) => {
                                warn!("[lock_manager] 重试截图失败 #{}: {} / [lock_manager] retry capture failed #{}: {}", scanned_count, e, scanned_count, e);
                                scanned_count += 1;
                                continue;
                            }
                        };
                    }

                    // Rarity early-stop: check if artifact rarity is below minimum (4)
                    if pixel_utils::artifact_below_min_rarity(&image, &scaler, 4) {
                        info!(
                            "[lock_manager] 检测到低稀有度圣遗物，当前页后停止 / Low rarity artifact detected, stopping after current page"
                        );
                        stop_after_page = true;
                        // Don't dispatch this item, don't increment scanned_count
                        // Break inner loops to go to page collection
                        break 'outer;
                    }

                    // Dispatch OCR to rayon immediately (runs in parallel with next capture)
                    let tx = result_tx.clone();
                    let pool = ocr_pool.clone();
                    let sub_pool = substat_pool.clone();
                    let sc = scaler_arc.clone();
                    let mp = self.mappings.clone();
                    let idx = scanned_count;
                    let r = row;
                    let c = col;
                    rayon::spawn(move || {
                        let ocr = pool.get();
                        let sub_ocr = sub_pool.get();
                        let artifact = match GoodArtifactScanner::identify_artifact(
                            &ocr as &dyn yas::ocr::ImageToText<image::RgbImage>,
                            &sub_ocr as &dyn yas::ocr::ImageToText<image::RgbImage>,
                            &image, &sc, &mp,
                        ) {
                            Ok(a) => a,
                            Err(e) => {
                                warn!("[lock_manager] OCR失败 #{}: {} / OCR failed", idx, e);
                                None
                            }
                        };
                        let _ = tx.send((idx, r, c, artifact));
                    });
                    dispatched += 1;

                    scanned_count += 1;
                }

                scanned_row += 1;
            }

            // Step 2: Collect all OCR results for this page
            drop(result_tx); // close sender so recv terminates
            let mut page_results: Vec<(usize, usize, usize, Option<GoodArtifact>)> =
                Vec::with_capacity(dispatched);
            for result in result_rx {
                page_results.push(result);
            }
            // Sort by index to maintain grid order
            page_results.sort_by_key(|(idx, _, _, _)| *idx);

            // Step 3: Match against targets, collect per-page toggles
            let mut page_toggles: Vec<PageToggle> = Vec::new();

            for (idx, row, col, artifact_opt) in &page_results {
                if let Some(ref artifact) = artifact_opt {
                    scanned_artifacts.push((*idx, artifact.clone()));

                    // Build unmatched target list for matching
                    let unmatched: Vec<(usize, &GoodArtifact)> = targets.iter()
                        .enumerate()
                        .filter(|(i, _)| !matched.contains_key(i))
                        .map(|(i, t)| (i, &t.artifact))
                        .collect();

                    if let Some((target_idx, _score)) = matching::find_best_match(artifact, &unmatched) {
                        matched.insert(target_idx, *idx);
                        let target = &targets[target_idx];

                        if artifact.lock == target.desired_lock {
                            results.insert(target.result_id.clone(), InstructionResult {
                                id: target.result_id.clone(),
                                status: InstructionStatus::AlreadyCorrect,
                            });
                        } else {
                            let y_shift = if artifact.elixir_crafted { 40.0 } else { 0.0 };
                            page_toggles.push(PageToggle {
                                result_id: target.result_id.clone(),
                                row: *row,
                                col: *col,
                                desired_lock: target.desired_lock,
                                y_shift,
                            });
                        }
                    }
                }
            }

            // Step 4: Apply lock toggles for this page (before scrolling)
            for toggle in &page_toggles {
                if ctrl.check_rmb() {
                    results.insert(toggle.result_id.clone(), InstructionResult {
                        id: toggle.result_id.clone(),
                        status: InstructionStatus::Aborted,
                    });
                    continue;
                }

                // Click back to the grid position
                let x = GRID_FIRST_X + toggle.col as f64 * GRID_OFFSET_X;
                let y = GRID_FIRST_Y + toggle.row as f64 * GRID_OFFSET_Y;
                ctrl.click_at(x, y);
                let _ = ctrl.wait_until_panel_loaded(PANEL_POOL_RECT, 400);
                yas::utils::sleep(100);

                // Toggle lock
                if let Err(e) = ui_actions::click_lock_button(ctrl, toggle.y_shift) {
                    warn!("[lock_manager] 锁定切换失败 / Lock toggle failed: {}", e);
                    results.insert(toggle.result_id.clone(), InstructionResult {
                        id: toggle.result_id.clone(),
                        status: InstructionStatus::UiError,
                    });
                    continue;
                }

                // Verify lock state changed
                yas::utils::sleep(200);
                let image = match ctrl.capture_game() {
                    Ok(img) => img,
                    Err(e) => {
                        warn!("[lock_manager] 截图失败 / Capture failed: {}", e);
                        results.insert(toggle.result_id.clone(), InstructionResult {
                            id: toggle.result_id.clone(),
                            status: InstructionStatus::UiError,
                        });
                        continue;
                    }
                };
                let new_lock = pixel_utils::detect_artifact_lock(&image, &scaler, toggle.y_shift);

                if new_lock == toggle.desired_lock {
                    info!(
                        "[lock_manager] 锁定切换成功 ({},{}) / Lock toggle success",
                        toggle.row, toggle.col
                    );
                    results.insert(toggle.result_id.clone(), InstructionResult {
                        id: toggle.result_id.clone(),
                        status: InstructionStatus::Success,
                    });
                } else {
                    warn!(
                        "[lock_manager] 锁定验证失败 ({},{}): 期望={} 实际={} / Lock verify failed",
                        toggle.row, toggle.col, toggle.desired_lock, new_lock
                    );
                    results.insert(toggle.result_id.clone(), InstructionResult {
                        id: toggle.result_id.clone(),
                        status: InstructionStatus::UiError,
                    });
                }
            }

            // If rarity early-stop was triggered, break after processing this page
            if stop_after_page {
                info!("[lock_manager] 低稀有度停止：当前页已处理完毕 / Rarity early-stop: current page processed");
                break;
            }

            // Early stop if all targets matched (only when configured)
            if stop_on_all_matched && matched.len() == targets.len() {
                info!("[lock_manager] 所有目标已匹配，提前停止 / All targets matched, stopping early");
                break;
            }

            } // end else (page was not skipped)

            // Step 5: Scroll to next page
            let remain = total.saturating_sub(scanned_count);
            if remain == 0 {
                break;
            }
            let remain_rows = (remain + GRID_COLS - 1) / GRID_COLS;
            let scroll_rows = remain_rows.min(GRID_ROWS);

            // Move mouse to grid center for scroll
            let center_x = GRID_FIRST_X + 3.0 * GRID_OFFSET_X;
            let center_y = GRID_FIRST_Y + 2.0 * GRID_OFFSET_Y;
            ctrl.move_to(center_x, center_y);
            yas::utils::sleep(30);

            let mut ticks = (SCROLL_TICKS_PER_PAGE as f64 / GRID_ROWS as f64 * scroll_rows as f64).round() as i32;
            if scroll_rows == GRID_ROWS {
                pages_scrolled += 1;
                if SCROLL_CORRECTION_INTERVAL > 0
                    && pages_scrolled % SCROLL_CORRECTION_INTERVAL as u32 == 0
                {
                    ticks -= 1;
                    debug!("[lock_manager] 滚动修正在第{}页（-1 tick） / [lock_manager] scroll correction at page {} (-1 tick)", pages_scrolled, pages_scrolled);
                }
            }

            for i in 0..ticks {
                if ctrl.check_rmb() {
                    break 'outer;
                }
                ctrl.mouse_scroll(1);
                if (i + 1) % 5 == 0 {
                    yas::utils::sleep(10);
                }
            }
            yas::utils::sleep(delay_scroll as u32);
        }

        // Scan is complete only if we visited every item without interruption and no early-stop
        // Rarity early-stop is a complete scan — we visited all relevant (4★+) artifacts.
        // Only cancellation or RMB abort counts as incomplete.
        let scan_complete = (scanned_count >= total || stop_after_page) && !ctrl.is_cancelled();

        // Mark unmatched targets
        let was_cancelled = ctrl.is_cancelled();
        for target in targets {
            if !results.contains_key(&target.result_id) {
                results.insert(target.result_id.clone(), InstructionResult {
                    id: target.result_id.clone(),
                    status: if was_cancelled {
                        InstructionStatus::Aborted
                    } else {
                        InstructionStatus::NotFound
                    },
                });
            }
        }

        // Collect results in target order
        let ordered_results: Vec<InstructionResult> = targets.iter()
            .filter_map(|t| results.remove(&t.result_id))
            .collect();

        (ordered_results, scanned_artifacts, matched, scan_complete)
    }
}
