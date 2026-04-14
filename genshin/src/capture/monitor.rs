/// Capture monitor: orchestrates packet capture, decryption, and data accumulation.
///
/// Ported from irminsul's `monitor.rs`, simplified for yas integration.
/// The monitor runs on a tokio runtime and communicates via channels.
///
/// ## Version resilience
///
/// Command matching uses heuristic protobuf parsing instead of hardcoded command IDs.
/// Every decrypted command is tentatively parsed as both `PacketWithItems` and
/// `AvatarDataNotify`; only structurally valid packets are accepted.  This
/// eliminates the most frequent breakage when the game client updates (command
/// ID rotation).
///
/// To avoid false positives from loose protobuf parsing, the heuristic requires:
/// - Items: ≥5 items with equip data (weapon or reliquary)
/// - Avatars: ≥4 characters AND ≥2 with non-empty equip_guid_list
///
/// Dispatch keys are loaded from an external `keys/gi.json` file first (next to
/// the exe), falling back to the compile-time embedded copy.  This allows key
/// updates without recompiling.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use auto_artifactarium::r#gen::protos::{AvatarInfo, Item};
use auto_artifactarium::{
    GamePacket, GameSniffer, matches_avatars_all_data_notify, matches_items_all_data_notify,
};
use base64::prelude::*;
use log::{debug, error, info, warn};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::data_cache::load_data_cache;
use super::data_types::DataCache;
use super::packet_capture::PacketCapture;
use super::player_data::{CaptureExportSettings, PlayerData};
use crate::scanner::common::models::GoodExport;

/// Commands the UI can send to the monitor.
pub enum CaptureCommand {
    StartCapture,
    StopCapture,
    Export {
        settings: CaptureExportSettings,
        reply: tokio::sync::oneshot::Sender<Result<GoodExport>>,
    },
}

/// State shared between the monitor and UI.
#[derive(Clone, Debug)]
pub struct CaptureState {
    pub capturing: bool,
    /// Both characters and items have been received; capture auto-stopped.
    pub complete: bool,
    pub has_characters: bool,
    pub has_items: bool,
    pub character_count: usize,
    pub weapon_count: usize,
    pub artifact_count: usize,
    pub error: Option<String>,
}

impl Default for CaptureState {
    fn default() -> Self {
        Self {
            capturing: false,
            complete: false,
            has_characters: false,
            has_items: false,
            character_count: 0,
            weapon_count: 0,
            artifact_count: 0,
            error: None,
        }
    }
}

/// The capture monitor. Runs on a tokio runtime.
pub struct CaptureMonitor {
    player_data: PlayerData,
    sniffer: GameSniffer,
    state: Arc<Mutex<CaptureState>>,
    capture_cancel_token: Option<CancellationToken>,
    packet_tx: mpsc::UnboundedSender<Vec<u8>>,
    packet_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    dump_packets: bool,
    dump_dir: std::path::PathBuf,
    dump_counter: u32,
}

impl CaptureMonitor {
    /// Initialize the monitor: load data cache, set up sniffer.
    pub fn new(state: Arc<Mutex<CaptureState>>, dump_packets: bool) -> Result<Self> {
        let data_cache = load_data_cache()?;
        let player_data = PlayerData::new(data_cache);
        let keys = load_keys()?;
        let sniffer = GameSniffer::new().set_initial_keys(keys);
        let (packet_tx, packet_rx) = mpsc::unbounded_channel();

        let dump_dir = crate::cli::exe_dir().join("debug_capture");
        if dump_packets {
            std::fs::create_dir_all(&dump_dir).ok();
            info!(
                "数据包转储已开启 → {} / Packet dump enabled → {}",
                dump_dir.display(),
                dump_dir.display(),
            );
        }

        Ok(Self {
            player_data,
            sniffer,
            state,
            capture_cancel_token: None,
            packet_tx,
            packet_rx,
            dump_packets,
            dump_dir,
            dump_counter: 0,
        })
    }

    /// Initialize with a pre-loaded DataCache (for testing or custom sources).
    pub fn new_with_data(data_cache: DataCache, state: Arc<Mutex<CaptureState>>) -> Result<Self> {
        let player_data = PlayerData::new(data_cache);
        let keys = load_keys()?;
        let sniffer = GameSniffer::new().set_initial_keys(keys);
        let (packet_tx, packet_rx) = mpsc::unbounded_channel();

        Ok(Self {
            player_data,
            sniffer,
            state,
            capture_cancel_token: None,
            packet_tx,
            packet_rx,
            dump_packets: false,
            dump_dir: crate::cli::exe_dir().join("debug_capture"),
            dump_counter: 0,
        })
    }

    /// Main event loop. Processes packets and UI commands.
    pub async fn run(mut self, mut cmd_rx: mpsc::UnboundedReceiver<CaptureCommand>) {
        loop {
            tokio::select! {
                Some(packet) = self.packet_rx.recv() => {
                    self.handle_packet(packet);
                }
                Some(cmd) = cmd_rx.recv() => {
                    if self.handle_command(cmd) {
                        break;
                    }
                }
                else => break,
            }
        }
    }

    /// Returns true if the loop should exit.
    fn handle_command(&mut self, cmd: CaptureCommand) -> bool {
        match cmd {
            CaptureCommand::StartCapture => {
                if self.capture_cancel_token.is_some() {
                    return false;
                }
                let cancel_token = CancellationToken::new();
                tokio::spawn(capture_task(cancel_token.clone(), self.packet_tx.clone()));
                self.capture_cancel_token = Some(cancel_token);
                if let Ok(mut state) = self.state.lock() {
                    state.capturing = true;
                    state.complete = false;
                    state.error = None;
                }
            }
            CaptureCommand::StopCapture => {
                self.stop_capture();
            }
            CaptureCommand::Export { settings, reply } => {
                let result = self.player_data.export(&settings);
                let _ = reply.send(result);
            }
        }
        false
    }

    fn stop_capture(&mut self) {
        if let Some(token) = self.capture_cancel_token.take() {
            token.cancel();
        }
        if let Ok(mut state) = self.state.lock() {
            state.capturing = false;
        }
    }

    fn handle_packet(&mut self, packet: Vec<u8>) {
        let Some(GamePacket::Commands(commands)) = self.sniffer.receive_packet(packet) else {
            return;
        };

        // Heuristic matching: try parsing every command as item/avatar packets
        // regardless of command_id.  This survives command ID rotation across
        // game versions.
        for command in commands {
            // Dump raw decrypted commands when enabled
            if self.dump_packets {
                let path = self.dump_dir.join(format!(
                    "{:06}_cmd{}.bin",
                    self.dump_counter, command.command_id
                ));
                if let Err(e) = std::fs::write(&path, &command.proto_data) {
                    warn!("转储失败 / Dump failed: {}", e);
                }
                self.dump_counter += 1;
            }

            if let Some(items) = try_match_items(&command.proto_data) {
                info!(
                    "捕获到物品数据包 (cmd={})，共 {} 个物品 / \
                     Captured item packet (cmd={}), {} items",
                    command.command_id,
                    items.len(),
                    command.command_id,
                    items.len(),
                );
                self.player_data.process_items(&items);
                if let Ok(mut state) = self.state.lock() {
                    state.has_items = true;
                    state.weapon_count = self.player_data.weapon_count();
                    state.artifact_count = self.player_data.artifact_count();
                }
            } else if let Some(avatars) = try_match_avatars(&command.proto_data) {
                info!(
                    "捕获到角色数据包 (cmd={})，共 {} 个角色 / \
                     Captured avatar packet (cmd={}), {} avatars",
                    command.command_id,
                    avatars.len(),
                    command.command_id,
                    avatars.len(),
                );
                self.player_data.process_characters(&avatars);
                if let Ok(mut state) = self.state.lock() {
                    state.has_characters = true;
                    state.character_count = self.player_data.character_count();
                }
            }
        }

        // Auto-stop when we have both characters and items
        let should_stop = self
            .state
            .lock()
            .map_or(false, |s| s.has_characters && s.has_items && s.capturing);
        if should_stop {
            info!(
                "已收集到所有数据，自动停止抓包 / All data collected, stopping capture automatically"
            );
            self.stop_capture();
            if let Ok(mut state) = self.state.lock() {
                state.complete = true;
            }
        }
    }
}

/// Strict heuristic match for item packets (PlayerStoreNotify).
///
/// The upstream `matches_items_all_data_notify` only requires ≥10 items with
/// non-zero ids — too loose when applied to every decrypted command.  We require
/// items with actual weapon or reliquary data (not just the `Equip` wrapper),
/// and enough of them to be a real inventory dump.
fn try_match_items(proto_data: &[u8]) -> Option<Vec<Item>> {
    let items = matches_items_all_data_notify(proto_data)?;
    let gear_count = items
        .iter()
        .filter(|i| i.has_equip() && (i.equip().has_weapon() || i.equip().has_reliquary()))
        .count();
    if gear_count < 5 {
        debug!(
            "物品数据包候选被拒（{} 个物品，{} 个武器/圣遗物）/ \
             Item packet candidate rejected ({} items, {} weapons/artifacts)",
            items.len(),
            gear_count,
            items.len(),
            gear_count,
        );
        return None;
    }
    Some(items)
}

/// Strict heuristic match for avatar packets (AvatarDataNotify).
///
/// Requires ≥4 avatars (every account has Traveler + 3 free characters) and
/// ≥2 avatars with non-empty `equip_guid_list` (active characters have weapons).
fn try_match_avatars(proto_data: &[u8]) -> Option<Vec<AvatarInfo>> {
    let avatars = matches_avatars_all_data_notify(proto_data)?;
    if avatars.len() < 4 {
        debug!(
            "角色数据包候选被拒（仅 {} 个角色）/ \
             Avatar packet candidate rejected (only {} avatars)",
            avatars.len(),
            avatars.len(),
        );
        return None;
    }
    let equipped = avatars
        .iter()
        .filter(|a| !a.equip_guid_list.is_empty())
        .count();
    if equipped < 2 {
        debug!(
            "角色数据包候选被拒（{} 个角色，仅 {} 个有装备）/ \
             Avatar packet candidate rejected ({} avatars, only {} equipped)",
            avatars.len(),
            equipped,
            avatars.len(),
            equipped,
        );
        return None;
    }
    Some(avatars)
}

async fn capture_task(
    cancel_token: CancellationToken,
    packet_tx: mpsc::UnboundedSender<Vec<u8>>,
) -> Result<()> {
    let mut capture =
        PacketCapture::new().map_err(|e| anyhow!("创建抓包失败 / Error creating packet capture: {e}"))?;
    info!("开始抓包 / Starting packet capture");
    loop {
        let packet = tokio::select!(
            packet = capture.next_packet() => packet,
            _ = cancel_token.cancelled() => break,
        );
        let packet = match packet {
            Ok(packet) => packet,
            Err(e) => {
                error!("接收数据包出错 / Error receiving packet: {e}");
                continue;
            }
        };
        if let Err(e) = packet_tx.send(packet) {
            error!("发送数据包出错 / Error sending captured packet: {e}");
        }
    }
    info!("抓包已停止 / Packet capture stopped");
    Ok(())
}

/// Load dispatch keys from external file first, then merge with embedded keys.
///
/// External keys (in `keys/gi.json` next to the exe) override embedded ones for
/// the same version, allowing key updates without recompiling.
fn load_keys() -> Result<HashMap<u16, Vec<u8>>> {
    let mut all_keys = HashMap::new();

    // 1. Embedded keys (compile-time fallback)
    let embedded: HashMap<u16, String> =
        serde_json::from_slice(include_bytes!("../../keys/gi.json"))?;
    for (version, b64) in &embedded {
        all_keys.insert(*version, BASE64_STANDARD.decode(b64)?);
    }

    // 2. External key file next to the exe (overrides embedded for same version)
    let external_path = crate::cli::exe_dir().join("keys").join("gi.json");
    match std::fs::read_to_string(&external_path) {
        Ok(content) => match serde_json::from_str::<HashMap<u16, String>>(&content) {
            Ok(external) => {
                let mut added = 0usize;
                for (version, b64) in &external {
                    if let Ok(decoded) = BASE64_STANDARD.decode(b64) {
                        if !all_keys.contains_key(version) {
                            added += 1;
                        }
                        all_keys.insert(*version, decoded);
                    }
                }
                info!(
                    "已加载外部密钥文件（{} 个密钥，{} 个新增）/ Loaded external key file ({} keys, {} new)",
                    external.len(), added, external.len(), added,
                );
            }
            Err(e) => warn!(
                "外部密钥文件格式错误: {} / External key file parse error: {}",
                e, e
            ),
        },
        Err(_) => {} // No external file — use embedded only
    }

    Ok(all_keys)
}
