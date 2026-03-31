/// Downloads and caches `data_cache.json` from ggartifact.com.
///
/// Uses the same TTL/metadata pattern as `mappings.rs`.
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use log::{info, warn};
use serde::{Deserialize, Serialize};

use super::data_types::DataCache;

const DATA_CACHE_URL: &str = "https://ggartifact.com/good/data_cache.json";
const DATA_CACHE_PATH: &str = "data/data_cache.json";
const DATA_CACHE_META_PATH: &str = "data/data_cache_meta.json";
const DATA_CACHE_TTL_SECS: u64 = 24 * 3600;

#[derive(Debug, Deserialize, Serialize)]
struct CacheMeta {
    #[serde(rename = "lastFetchTime")]
    last_fetch_time: u64,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn is_cache_fresh() -> bool {
    let meta_path = Path::new(DATA_CACHE_META_PATH);
    if !meta_path.exists() {
        return false;
    }
    let Ok(content) = fs::read_to_string(meta_path) else {
        return false;
    };
    let Ok(meta) = serde_json::from_str::<CacheMeta>(&content) else {
        return false;
    };
    (now_secs() - meta.last_fetch_time) < DATA_CACHE_TTL_SECS
}

fn write_meta() -> Result<()> {
    let meta = CacheMeta {
        last_fetch_time: now_secs(),
    };
    let content = serde_json::to_string(&meta)?;
    fs::write(DATA_CACHE_META_PATH, content)?;
    Ok(())
}

/// Fetch `data_cache.json` from remote if cache is stale, otherwise load from cache.
/// Returns the parsed `DataCache`.
pub fn load_data_cache() -> Result<DataCache> {
    fs::create_dir_all("data").ok();

    let cache_path = Path::new(DATA_CACHE_PATH);

    if !is_cache_fresh() {
        info!("Fetching data_cache.json from {}", DATA_CACHE_URL);
        match fetch_remote() {
            Ok(data) => {
                // Validate JSON before writing
                let _: DataCache = serde_json::from_str(&data)
                    .context("Failed to parse fetched data_cache.json")?;
                fs::write(cache_path, &data)?;
                write_meta()?;
                info!("data_cache.json updated successfully");
            }
            Err(e) => {
                if cache_path.exists() {
                    warn!(
                        "Failed to fetch data_cache.json ({}), using stale cache",
                        e
                    );
                } else {
                    return Err(e).context("Failed to fetch data_cache.json and no cache exists");
                }
            }
        }
    }

    let content = fs::read_to_string(cache_path).context("Failed to read data_cache.json")?;
    let data_cache: DataCache =
        serde_json::from_str(&content).context("Failed to parse data_cache.json")?;
    Ok(data_cache)
}

fn fetch_remote() -> Result<String> {
    let resp = reqwest::blocking::get(DATA_CACHE_URL)
        .context("HTTP request to ggartifact.com failed")?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {} from {}", status, DATA_CACHE_URL);
    }
    resp.text().context("Failed to read response body")
}
