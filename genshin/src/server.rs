//! HTTP server for the artifact manager.
//!
//! Provides a simple blocking HTTP server using `tiny_http` that accepts
//! artifact management instructions from a web frontend and returns results.
//!
//! HTTP 服务器，接受来自网页前端的圣遗物管理指令并返回结果。

use anyhow::{anyhow, Result};
use log::{error, info};
use tiny_http::{Header, Method, Response, Server};

use crate::manager::models::ArtifactManageRequest;
use crate::manager::orchestrator::ArtifactManager;
use crate::scanner::common::game_controller::GenshinGameController;

/// CORS headers for browser access.
fn cors_headers() -> Vec<Header> {
    vec![
        Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap(),
        Header::from_bytes("Access-Control-Allow-Methods", "GET, POST, OPTIONS").unwrap(),
        Header::from_bytes("Access-Control-Allow-Headers", "Content-Type").unwrap(),
        Header::from_bytes("Content-Type", "application/json; charset=utf-8").unwrap(),
    ]
}

/// Run the artifact manager HTTP server.
///
/// This blocks the current thread and serves requests until the process is killed.
/// Only one manage operation can run at a time (game control is single-threaded).
///
/// 运行圣遗物管理 HTTP 服务器。阻塞当前线程。
/// 同一时间只能执行一个管理操作。
pub fn run_server(
    port: u16,
    ctrl: &mut GenshinGameController,
    manager: &ArtifactManager,
) -> Result<()> {
    let addr = format!("127.0.0.1:{}", port);
    let server = Server::http(&addr)
        .map_err(|e| anyhow!("HTTP服务器启动失败 / HTTP server start failed: {}", e))?;

    info!(
        "HTTP服务器已启动：http://{} / HTTP server running at http://{}",
        addr, addr
    );
    println!(
        "圣遗物管理服务器已启动 / Artifact manager server running at http://{}",
        addr
    );
    println!("按 Ctrl+C 停止 / Press Ctrl+C to stop");

    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let url = request.url().to_string();

        // Handle CORS preflight
        if method == Method::Options {
            let response = Response::empty(204);
            let mut resp = response;
            for header in cors_headers() {
                resp.add_header(header);
            }
            if let Err(e) = request.respond(resp) {
                error!("响应失败 / Response failed: {}", e);
            }
            continue;
        }

        match (method, url.as_str()) {
            (Method::Post, "/manage") => {
                // Read request body
                let mut body = String::new();
                if let Err(e) = request.as_reader().read_to_string(&mut body) {
                    let error_json = format!("{{\"error\":\"读取请求体失败 / Failed to read body: {}\"}}", e);
                    let mut resp = Response::from_string(error_json).with_status_code(400);
                    for header in cors_headers() {
                        resp.add_header(header);
                    }
                    let _ = request.respond(resp);
                    continue;
                }

                // Parse request
                let manage_request: ArtifactManageRequest = match serde_json::from_str(&body) {
                    Ok(r) => r,
                    Err(e) => {
                        let error_json = format!(
                            "{{\"error\":\"JSON解析失败 / JSON parse error: {}\"}}",
                            e
                        );
                        let mut resp = Response::from_string(error_json).with_status_code(400);
                        for header in cors_headers() {
                            resp.add_header(header);
                        }
                        let _ = request.respond(resp);
                        continue;
                    }
                };

                info!(
                    "收到管理请求：{} 条指令 / Received manage request: {} instructions",
                    manage_request.instructions.len(),
                    manage_request.instructions.len()
                );

                // Execute (blocks until complete)
                let result = manager.execute(ctrl, manage_request);

                let json = match serde_json::to_string_pretty(&result) {
                    Ok(j) => j,
                    Err(e) => {
                        let error_json = format!("{{\"error\":\"序列化失败 / Serialization error: {}\"}}", e);
                        let mut resp = Response::from_string(error_json).with_status_code(500);
                        for header in cors_headers() {
                            resp.add_header(header);
                        }
                        let _ = request.respond(resp);
                        continue;
                    }
                };

                let mut resp = Response::from_string(json);
                for header in cors_headers() {
                    resp.add_header(header);
                }
                if let Err(e) = request.respond(resp) {
                    error!("响应失败 / Response failed: {}", e);
                }
            }

            (Method::Get, "/health") => {
                let json = r#"{"status":"ok"}"#;
                let mut resp = Response::from_string(json);
                for header in cors_headers() {
                    resp.add_header(header);
                }
                let _ = request.respond(resp);
            }

            _ => {
                let mut resp = Response::from_string(r#"{"error":"Not Found"}"#).with_status_code(404);
                for header in cors_headers() {
                    resp.add_header(header);
                }
                let _ = request.respond(resp);
            }
        }
    }

    Ok(())
}
