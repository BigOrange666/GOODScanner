/// Capture monitor: orchestrates packet capture, decryption, and data accumulation.
///
/// Ported from irminsul's `monitor.rs`, simplified for yas integration.
/// The monitor runs on a tokio runtime and communicates via channels.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Result, anyhow};
use auto_artifactarium::{
    GamePacket, GameSniffer, matches_avatar_packet, matches_item_packet,
};
use base64::prelude::*;
use log::{error, info, warn};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::data_cache::load_data_cache;
use super::data_types::DataCache;
use super::packet_capture::PacketCapture;
use super::player_data::{CaptureExportSettings, PlayerData};
use crate::scanner::common::models::GoodExport;

/// Timestamps tracking when each data type was last received.
#[derive(Clone, Debug, Default)]
pub struct CaptureTimestamps {
    pub characters_updated: Option<Instant>,
    pub items_updated: Option<Instant>,
}

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
    pub timestamps: CaptureTimestamps,
    pub has_data: bool,
    pub error: Option<String>,
}

impl Default for CaptureState {
    fn default() -> Self {
        Self {
            capturing: false,
            timestamps: CaptureTimestamps::default(),
            has_data: false,
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
}

impl CaptureMonitor {
    /// Initialize the monitor: load data cache, set up sniffer.
    pub fn new(state: Arc<Mutex<CaptureState>>) -> Result<Self> {
        let data_cache = load_data_cache()?;
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
                    warn!("Capture start request while already capturing");
                    return false;
                }
                let cancel_token = CancellationToken::new();
                tokio::spawn(capture_task(cancel_token.clone(), self.packet_tx.clone()));
                self.capture_cancel_token = Some(cancel_token);
                if let Ok(mut state) = self.state.lock() {
                    state.capturing = true;
                }
            }
            CaptureCommand::StopCapture => {
                if let Some(token) = self.capture_cancel_token.take() {
                    token.cancel();
                }
                if let Ok(mut state) = self.state.lock() {
                    state.capturing = false;
                }
            }
            CaptureCommand::Export { settings, reply } => {
                let result = self.player_data.export(&settings);
                let _ = reply.send(result);
            }
        }
        false
    }

    fn handle_packet(&mut self, packet: Vec<u8>) {
        let Some(GamePacket::Commands(commands)) = self.sniffer.receive_packet(packet) else {
            return;
        };

        let mut timestamps_changed = false;

        for command in commands {
            if let Some(items) = matches_item_packet(&command) {
                info!("Captured item packet with {} items", items.len());
                self.player_data.process_items(&items);
                if let Ok(mut state) = self.state.lock() {
                    state.timestamps.items_updated = Some(Instant::now());
                    state.has_data = true;
                }
                timestamps_changed = true;
            } else if let Some(avatars) = matches_avatar_packet(&command) {
                info!("Captured avatar packet with {} avatars", avatars.len());
                self.player_data.process_characters(&avatars);
                if let Ok(mut state) = self.state.lock() {
                    state.timestamps.characters_updated = Some(Instant::now());
                    state.has_data = true;
                }
                timestamps_changed = true;
            }
            // Note: achievement packets are intentionally not handled
        }

        let _ = timestamps_changed; // suppress unused warning
    }
}

async fn capture_task(
    cancel_token: CancellationToken,
    packet_tx: mpsc::UnboundedSender<Vec<u8>>,
) -> Result<()> {
    let mut capture =
        PacketCapture::new().map_err(|e| anyhow!("Error creating packet capture: {e}"))?;
    info!("Starting packet capture");
    loop {
        let packet = tokio::select!(
            packet = capture.next_packet() => packet,
            _ = cancel_token.cancelled() => break,
        );
        let packet = match packet {
            Ok(packet) => packet,
            Err(e) => {
                error!("Error receiving packet: {e}");
                continue;
            }
        };
        if let Err(e) = packet_tx.send(packet) {
            error!("Error sending captured packet: {e}");
        }
    }
    info!("Packet capture stopped");
    Ok(())
}

fn load_keys() -> Result<HashMap<u16, Vec<u8>>> {
    let keys: HashMap<u16, String> =
        serde_json::from_slice(include_bytes!("../../keys/gi.json"))?;

    keys.iter()
        .map(|(key, value)| -> Result<_, _> { Ok((*key, BASE64_STANDARD.decode(value)?)) })
        .collect::<Result<HashMap<_, _>>>()
}
