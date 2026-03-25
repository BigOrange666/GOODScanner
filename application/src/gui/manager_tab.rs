use std::sync::atomic::Ordering;

use eframe::egui;

use super::state::{AppState, TaskStatus};
use super::worker::{self, TaskHandle};

pub fn show(
    ui: &mut egui::Ui,
    state: &mut AppState,
    server_handle: &mut Option<TaskHandle>,
    manage_handle: &mut Option<TaskHandle>,
) {
    let is_server_running = server_handle.as_ref().map_or(false, |h| !h.is_finished());
    let is_managing = manage_handle.as_ref().map_or(false, |h| !h.is_finished());

    ui.add_space(4.0);
    ui.label("接收来自网页前端的圣遗物管理指令（装备/锁定/解锁）");
    ui.label("Accept artifact manage instructions (equip/lock/unlock) from a web frontend.");
    ui.add_space(8.0);

    // === HTTP Server Section ===
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.strong("HTTP 服务器 / HTTP Server");
        ui.add_space(4.0);

        // Port + Start/Status on one line
        ui.horizontal(|ui| {
            ui.label("端口 / Port:");
            ui.add_enabled(
                !is_server_running,
                egui::DragValue::new(&mut state.server_port).range(1024..=65535u16),
            );

            ui.add_space(12.0);

            if is_server_running {
                ui.spinner();
                ui.colored_label(
                    egui::Color32::from_rgb(100, 200, 100),
                    format!("运行中 / Running on port {}", state.server_port),
                );
            } else {
                if ui.button("▶ 启动 / Start").clicked() {
                    let _ = yas_genshin::cli::save_config(&state.user_config);
                    state
                        .server_enabled
                        .store(true, Ordering::Relaxed);
                    *server_handle = Some(worker::spawn_server(state));
                }
            }
        });

        // Enabled toggle (only when server is running)
        if is_server_running {
            ui.add_space(2.0);
            let mut enabled = state.server_enabled.load(Ordering::Relaxed);
            if ui
                .checkbox(&mut enabled, "接受管理请求 / Accept manage requests")
                .changed()
            {
                state.server_enabled.store(enabled, Ordering::Relaxed);
                if enabled {
                    log::info!(
                        "管理器已启用 / Manager enabled on port {}",
                        state.server_port
                    );
                } else {
                    log::info!(
                        "管理器已暂停 / Manager paused — requests return 503",
                    );
                }
            }
            if !enabled {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 200, 50),
                    "  已暂停：POST /manage → 503 / Paused: POST /manage → 503",
                );
            }
        }

        // Error from previous run
        if !is_server_running {
            let status = state.server_status.lock().unwrap().clone();
            match status {
                TaskStatus::Failed(ref msg) => {
                    ui.colored_label(egui::Color32::from_rgb(255, 100, 100), msg);
                }
                TaskStatus::Completed(ref msg) => {
                    ui.colored_label(egui::Color32::from_rgb(150, 150, 150), msg);
                }
                _ => {}
            }
        }
    });

    ui.add_space(12.0);

    // === Execute JSON Section ===
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.strong("离线执行 / Offline Execute");
        ui.add_space(2.0);
        ui.label("从JSON文件加载管理指令并执行（无需启动服务器）");
        ui.label("Load instructions from a JSON file and execute directly.");
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            let can_execute = !is_managing && !is_server_running;
            if ui
                .add_enabled(can_execute, egui::Button::new("📁 选择文件 / Choose File..."))
                .clicked()
            {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("JSON", &["json"])
                    .pick_file()
                {
                    match std::fs::read_to_string(&path) {
                        Ok(json_str) => {
                            log::info!("加载文件 / Loaded: {}", path.display());
                            let _ = yas_genshin::cli::save_config(&state.user_config);
                            *manage_handle = Some(worker::spawn_manage_json(
                                state.user_config.clone(),
                                json_str,
                                state.manage_status.clone(),
                            ));
                        }
                        Err(e) => {
                            log::error!("读取文件失败 / Failed to read: {}", e);
                        }
                    }
                }
            }

            // Inline status
            if is_managing {
                ui.spinner();
                let status = state.manage_status.lock().unwrap().clone();
                if let TaskStatus::Running(msg) = status {
                    ui.label(msg);
                }
            }
        });

        // Result
        if !is_managing {
            let status = state.manage_status.lock().unwrap().clone();
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
    });
}
