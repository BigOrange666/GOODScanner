use eframe::egui;

use super::state::{AppState, TaskStatus};
use super::worker::{self, TaskHandle};

pub fn show(
    ui: &mut egui::Ui,
    state: &mut AppState,
    scan_handle: &mut Option<TaskHandle>,
    game_busy: bool,
) {
    let is_scanning = scan_handle.as_ref().map_or(false, |h| !h.is_finished());
    let l = state.lang;

    // === Action bar (always visible at top) ===
    ui.add_space(4.0);
    action_bar(ui, state, scan_handle, is_scanning, game_busy);
    if !is_scanning {
        ui.colored_label(
            egui::Color32::from_rgb(120, 120, 120),
            l.t(
                "请确认游戏已运行，扫描过程中可按鼠标右键终止。",
                "Make sure the game is running. Right-click to abort during scanning.",
            ),
        );
    }
    ui.add_space(4.0);
    ui.separator();

    // === Scrollable config area ===
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(4.0);

        // === Scan Targets ===
        ui.strong(l.t("扫描目标", "Scan Targets"));
        ui.add_space(2.0);
        ui.add_enabled_ui(!is_scanning, |ui| {
            ui.checkbox(&mut state.scan_characters, l.t("角色", "Characters"));
            ui.checkbox(&mut state.scan_weapons, l.t("武器", "Weapons"));
            ui.checkbox(&mut state.scan_artifacts, l.t("圣遗物", "Artifacts"));
        });

        ui.add_space(8.0);

        // === Character Names ===
        let names_open = state.names_need_attention || !yas_genshin::cli::config_path().exists();
        let header_text = if state.names_need_attention {
            egui::RichText::new(l.t("⚠ 角色名称", "⚠ Character Names"))
                .color(egui::Color32::from_rgb(255, 200, 50))
        } else {
            egui::RichText::new(l.t("角色名称", "Character Names"))
        };
        let header = egui::CollapsingHeader::new(header_text)
            .default_open(names_open)
            .open(if state.names_need_attention { Some(true) } else { None });

        header.show(ui, |ui| {
            ui.add_enabled_ui(!is_scanning, |ui| {
                if state.names_need_attention {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 200, 50),
                        l.t(
                            "请输入您的游戏内角色名称，或留空使用默认名称，然后再次点击开始扫描。",
                            "Enter your in-game character names (or leave empty for defaults), then click Start Scan again.",
                        ),
                    );
                    ui.add_space(4.0);
                } else {
                    ui.label(l.t(
                        "自定义名字的角色，留空为默认",
                        "Custom names for renameable characters (empty = default)",
                    ));
                }
                ui.add_space(2.0);
                let w = (ui.available_width() - 140.0).max(120.0);
                egui::Grid::new("names_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        name_field(ui, l.t("旅行者", "Traveler"), &mut state.user_config.traveler_name, w);
                        name_field(ui, l.t("流浪者", "Wanderer"), &mut state.user_config.wanderer_name, w);
                        name_field(ui, l.t("奇偶·男", "Manekin"), &mut state.user_config.manekin_name, w);
                        name_field(ui, l.t("奇偶·女", "Manekina"), &mut state.user_config.manekina_name, w);
                    });

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

        // === Timing Delays ===
        egui::CollapsingHeader::new(l.t("延迟设置", "Timing Delays"))
            .default_open(false)
            .show(ui, |ui| {
                ui.add_enabled_ui(!is_scanning, |ui| {
                    delay_group(ui, "char_delays", l.t("角色", "Character"), &mut [
                        (l.t("Tab切换", "Tab switch"), &mut state.user_config.char_tab_delay),
                        (l.t("打开", "Open"), &mut state.user_config.char_open_delay),
                    ]);
                    ui.add_space(4.0);
                    delay_group(ui, "weapon_delays", l.t("武器", "Weapon"), &mut [
                        (l.t("格子", "Grid"), &mut state.user_config.weapon_grid_delay),
                        (l.t("滚动", "Scroll"), &mut state.user_config.weapon_scroll_delay),
                        (l.t("Tab切换", "Tab switch"), &mut state.user_config.weapon_tab_delay),
                        (l.t("打开", "Open"), &mut state.user_config.weapon_open_delay),
                    ]);
                    ui.add_space(4.0);
                    delay_group(ui, "artifact_delays", l.t("圣遗物", "Artifact"), &mut [
                        (l.t("格子", "Grid"), &mut state.user_config.artifact_grid_delay),
                        (l.t("滚动", "Scroll"), &mut state.user_config.artifact_scroll_delay),
                        (l.t("Tab切换", "Tab switch"), &mut state.user_config.artifact_tab_delay),
                        (l.t("打开", "Open"), &mut state.user_config.artifact_open_delay),
                    ]);
                });
            });

        // === Advanced Options ===
        egui::CollapsingHeader::new(l.t("高级选项", "Advanced Options"))
            .default_open(false)
            .show(ui, |ui| {
                ui.add_enabled_ui(!is_scanning, |ui| {
                    ui.checkbox(&mut state.verbose, l.t("详细信息", "Verbose"));
                    ui.checkbox(&mut state.continue_on_failure, l.t("失败继续", "Continue on failure"));
                    ui.checkbox(&mut state.dump_images, l.t("保存OCR截图", "Dump OCR images"));
                    ui.checkbox(
                        &mut state.weapon_skip_delay,
                        l.t("跳过武器面板等待（更快但检测不太准）", "Skip weapon panel delay (faster, less accurate)"),
                    );
                    ui.checkbox(
                        &mut state.artifact_skip_delay,
                        l.t("跳过圣遗物面板等待（更快但检测不太准）", "Skip artifact panel delay (faster, less accurate)"),
                    );

                    ui.add_space(4.0);
                    ui.label(l.t("最大扫描数 (0 = 全部):", "Max scan count (0 = all):"));
                    ui.horizontal(|ui| {
                        ui.label(l.t("角色:", "Char:"));
                        max_count_field(ui, &mut state.char_max_count);
                        ui.add_space(8.0);
                        ui.label(l.t("武器:", "Wpn:"));
                        max_count_field(ui, &mut state.weapon_max_count);
                        ui.add_space(8.0);
                        ui.label(l.t("圣遗物:", "Art:"));
                        max_count_field(ui, &mut state.artifact_max_count);
                    });
                });
            });
    });
}

/// Top action bar
fn action_bar(
    ui: &mut egui::Ui,
    state: &mut AppState,
    scan_handle: &mut Option<TaskHandle>,
    is_scanning: bool,
    game_busy: bool,
) {
    let l = state.lang;

    if game_busy && !is_scanning {
        ui.colored_label(
            egui::Color32::from_rgb(255, 200, 50),
            l.t(
                "管理器正在运行，请先停止后再扫描",
                "Manager is running. Stop it before scanning.",
            ),
        );
    }

    ui.horizontal(|ui| {
        if is_scanning {
            if ui.button(l.t("⏹ 停止扫描", "⏹ Stop Scan")).clicked() {
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
            let can_scan = any_selected && !game_busy;
            if ui
                .add_enabled(can_scan, egui::Button::new(l.t("▶ 开始扫描", "▶ Start Scan")))
                .clicked()
            {
                let all_empty = state.user_config.traveler_name.trim().is_empty()
                    && state.user_config.wanderer_name.trim().is_empty()
                    && state.user_config.manekin_name.trim().is_empty()
                    && state.user_config.manekina_name.trim().is_empty();
                let first_run = !yas_genshin::cli::config_path().exists();

                if all_empty && first_run && !state.names_need_attention {
                    state.names_need_attention = true;
                    log::warn!("{}", l.t(
                        "请先确认角色名称配置",
                        "Please review character name settings before scanning",
                    ));
                } else {
                    state.names_need_attention = false;
                    let _ = yas_genshin::cli::save_config(&state.user_config);
                    *scan_handle = Some(worker::spawn_scan(state));
                }
            }

            if ui.button(l.t("💾 保存配置", "💾 Save Config")).clicked() {
                match yas_genshin::cli::save_config(&state.user_config) {
                    Ok(()) => log::info!("{}", l.t("配置已保存", "Config saved")),
                    Err(e) => log::error!("{}: {}", l.t("保存失败", "Save failed"), e),
                }
            }
        }
    });

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

fn name_field(ui: &mut egui::Ui, label: &str, value: &mut String, width: f32) {
    ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(width));
    ui.end_row();
}

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

fn max_count_field(ui: &mut egui::Ui, value: &mut usize) {
    let mut v = *value as i64;
    if ui
        .add(egui::DragValue::new(&mut v).range(0..=2000).speed(1))
        .changed()
    {
        *value = v.max(0) as usize;
    }
}
