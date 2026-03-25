use eframe::egui;

use super::state::{AppState, TaskStatus};
use super::worker::{self, TaskHandle};

pub fn show(ui: &mut egui::Ui, state: &mut AppState, scan_handle: &mut Option<TaskHandle>) {
    let is_scanning = scan_handle.as_ref().map_or(false, |h| !h.is_finished());

    // === Action bar (always visible at top) ===
    ui.add_space(4.0);
    action_bar(ui, state, scan_handle, is_scanning);
    ui.add_space(4.0);
    ui.separator();

    // === Scrollable config area ===
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(4.0);

        // === Scan Targets (with inline rarity) ===
        ui.strong("扫描目标 / Scan Targets");
        ui.add_space(2.0);
        ui.add_enabled_ui(!is_scanning, |ui| {
            scan_target_row(ui, "角色 / Characters", &mut state.scan_characters, None);
            scan_target_row(
                ui,
                "武器 / Weapons",
                &mut state.scan_weapons,
                Some(("最低★ / Min ★", &mut state.weapon_min_rarity)),
            );
            scan_target_row(
                ui,
                "圣遗物 / Artifacts",
                &mut state.scan_artifacts,
                Some(("最低★ / Min ★", &mut state.artifact_min_rarity)),
            );
        });

        ui.add_space(8.0);

        // === Character Names ===
        // Force open when names need attention (first-run, Start pressed with empty names)
        let names_open = state.names_need_attention || !yas_genshin::cli::config_path().exists();
        let header = egui::CollapsingHeader::new(
            if state.names_need_attention {
                egui::RichText::new("⚠ 角色名称 / Character Names").color(egui::Color32::from_rgb(255, 200, 50))
            } else {
                egui::RichText::new("角色名称 / Character Names")
            },
        )
            .default_open(names_open)
            .open(if state.names_need_attention { Some(true) } else { None });

        header.show(ui, |ui| {
                ui.add_enabled_ui(!is_scanning, |ui| {
                    if state.names_need_attention {
                        ui.colored_label(
                            egui::Color32::from_rgb(255, 200, 50),
                            "请输入您的游戏内角色名称，或留空使用默认名称，然后再次点击开始扫描。",
                        );
                        ui.colored_label(
                            egui::Color32::from_rgb(255, 200, 50),
                            "Enter your in-game character names (or leave empty for defaults), then click Start Scan again.",
                        );
                        ui.add_space(4.0);
                    } else {
                        ui.label("自定义名字的角色，留空为默认 / Custom names for renameable characters (empty = default)");
                    }
                    ui.add_space(2.0);
                    let w = (ui.available_width() - 140.0).max(120.0);
                    egui::Grid::new("names_grid")
                        .num_columns(2)
                        .spacing([8.0, 4.0])
                        .show(ui, |ui| {
                            name_field(ui, "旅行者 / Traveler", &mut state.user_config.traveler_name, w);
                            name_field(ui, "流浪者 / Wanderer", &mut state.user_config.wanderer_name, w);
                            name_field(ui, "奇偶·男 / Manekin", &mut state.user_config.manekin_name, w);
                            name_field(ui, "奇偶·女 / Manekina", &mut state.user_config.manekina_name, w);
                        });

                    // Clear the attention flag once the user interacts with a name field
                    if state.names_need_attention {
                        let any_filled = !state.user_config.traveler_name.trim().is_empty()
                            || !state.user_config.wanderer_name.trim().is_empty()
                            || !state.user_config.manekin_name.trim().is_empty()
                            || !state.user_config.manekina_name.trim().is_empty();
                        if any_filled {
                            state.names_need_attention = false;
                        }
                    }
                });
            });

        // === Timing Delays (collapsed) ===
        egui::CollapsingHeader::new("延迟设置 / Timing Delays")
            .default_open(false)
            .show(ui, |ui| {
                ui.add_enabled_ui(!is_scanning, |ui| {
                    delay_group(ui, "char_delays", "角色 / Character", &mut [
                        ("Tab切换 / Tab switch", &mut state.user_config.char_tab_delay),
                        ("打开 / Open", &mut state.user_config.char_open_delay),
                    ]);
                    ui.add_space(4.0);
                    delay_group(ui, "weapon_delays", "武器 / Weapon", &mut [
                        ("格子 / Grid", &mut state.user_config.weapon_grid_delay),
                        ("滚动 / Scroll", &mut state.user_config.weapon_scroll_delay),
                        ("Tab切换 / Tab switch", &mut state.user_config.weapon_tab_delay),
                        ("打开 / Open", &mut state.user_config.weapon_open_delay),
                    ]);
                    ui.add_space(4.0);
                    delay_group(ui, "artifact_delays", "圣遗物 / Artifact", &mut [
                        ("格子 / Grid", &mut state.user_config.artifact_grid_delay),
                        ("滚动 / Scroll", &mut state.user_config.artifact_scroll_delay),
                        ("Tab切换 / Tab switch", &mut state.user_config.artifact_tab_delay),
                        ("打开 / Open", &mut state.user_config.artifact_open_delay),
                    ]);
                });
            });

        // === Advanced Options (collapsed) ===
        egui::CollapsingHeader::new("高级选项 / Advanced Options")
            .default_open(false)
            .show(ui, |ui| {
                ui.add_enabled_ui(!is_scanning, |ui| {
                    ui.checkbox(&mut state.verbose, "详细信息 / Verbose");
                    ui.checkbox(&mut state.continue_on_failure, "失败继续 / Continue on failure");
                    ui.checkbox(&mut state.dump_images, "保存OCR截图 / Dump OCR images");
                    ui.checkbox(
                        &mut state.weapon_skip_delay,
                        "跳过武器面板等待 / Skip weapon panel delay (faster, less accurate)",
                    );
                    ui.checkbox(
                        &mut state.artifact_skip_delay,
                        "跳过圣遗物面板等待 / Skip artifact panel delay (faster, less accurate)",
                    );

                    ui.add_space(4.0);
                    ui.label("最大扫描数 (0 = 全部) / Max scan count (0 = all):");
                    ui.horizontal(|ui| {
                        ui.label("角色:");
                        max_count_field(ui, &mut state.char_max_count);
                        ui.add_space(8.0);
                        ui.label("武器:");
                        max_count_field(ui, &mut state.weapon_max_count);
                        ui.add_space(8.0);
                        ui.label("圣遗物:");
                        max_count_field(ui, &mut state.artifact_max_count);
                    });
                });
            });
    });
}

/// Top action bar: Start/Stop scan + Save Config + status
fn action_bar(
    ui: &mut egui::Ui,
    state: &mut AppState,
    scan_handle: &mut Option<TaskHandle>,
    is_scanning: bool,
) {
    ui.horizontal(|ui| {
        if is_scanning {
            if ui.button("⏹ 停止扫描 / Stop Scan").clicked() {
                yas::utils::set_abort();
            }
            let status = state.scan_status.lock().unwrap().clone();
            if let TaskStatus::Running(phase) = status {
                ui.spinner();
                ui.label(phase);
            }
        } else {
            let any_selected =
                state.scan_characters || state.scan_weapons || state.scan_artifacts;
            if ui
                .add_enabled(
                    any_selected,
                    egui::Button::new("▶ 开始扫描 / Start Scan"),
                )
                .clicked()
            {
                // Check if character names have been configured
                let all_empty = state.user_config.traveler_name.trim().is_empty()
                    && state.user_config.wanderer_name.trim().is_empty()
                    && state.user_config.manekin_name.trim().is_empty()
                    && state.user_config.manekina_name.trim().is_empty();

                if all_empty && !yas_genshin::cli::config_path().exists() {
                    // First run: names never configured — force attention
                    state.names_need_attention = true;
                    log::warn!("请先配置角色名称 / Please configure character names before scanning");
                } else {
                    state.names_need_attention = false;
                    let _ = yas_genshin::cli::save_config(&state.user_config);
                    *scan_handle = Some(worker::spawn_scan(state));
                }
            }

            if ui.button("💾 保存配置 / Save Config").clicked() {
                match yas_genshin::cli::save_config(&state.user_config) {
                    Ok(()) => log::info!("配置已保存 / Config saved"),
                    Err(e) => log::error!("保存失败 / Save failed: {}", e),
                }
            }
        }
    });

    // Status line
    let status = state.scan_status.lock().unwrap().clone();
    match status {
        TaskStatus::Completed(ref msg) => {
            ui.colored_label(egui::Color32::from_rgb(100, 200, 100), msg);
        }
        TaskStatus::Failed(ref msg) => {
            ui.colored_label(egui::Color32::from_rgb(255, 100, 100), msg);
        }
        _ => {}
    }
}

/// A scan target row: checkbox + optional inline rarity slider
fn scan_target_row(
    ui: &mut egui::Ui,
    label: &str,
    checked: &mut bool,
    rarity: Option<(&str, &mut i32)>,
) {
    ui.horizontal(|ui| {
        ui.checkbox(checked, label);
        if let Some((rarity_label, rarity_val)) = rarity {
            if *checked {
                ui.add_space(16.0);
                ui.label(rarity_label);
                ui.add(egui::Slider::new(rarity_val, 1..=5).show_value(true));
            }
        }
    });
}

/// Character name field in a grid row
fn name_field(ui: &mut egui::Ui, label: &str, value: &mut String, width: f32) {
    ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(width));
    ui.end_row();
}

/// A group of delay fields with a category label
fn delay_group(ui: &mut egui::Ui, id: &str, category: &str, fields: &mut [(&str, &mut u64)]) {
    ui.strong(category);
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([8.0, 2.0])
        .show(ui, |ui| {
            for (label, value) in fields.iter_mut() {
                ui.label(format!("  {} (ms):", label));
                let mut v = **value as i64;
                if ui
                    .add(egui::DragValue::new(&mut v).range(0..=5000).speed(10))
                    .changed()
                {
                    **value = v.max(0) as u64;
                }
                ui.end_row();
            }
        });
}

/// Compact max-count drag value
fn max_count_field(ui: &mut egui::Ui, value: &mut usize) {
    let mut v = *value as i64;
    if ui
        .add(
            egui::DragValue::new(&mut v)
                .range(0..=2000)
                .speed(1),
        )
        .changed()
    {
        *value = v.max(0) as usize;
    }
}
