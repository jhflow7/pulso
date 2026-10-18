use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as Base64Engine;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager, State};
use url::Url;

const APP_VERSION: &str = "0.2.0";
const TORRENT_SOURCE_PREFIX: &str = "torrent-file:";
const MAX_TORRENT_FILE_BYTES: u64 = 32 * 1024 * 1024;
const HISTORY_LIMIT: usize = 1000;

fn default_notify_on_complete() -> bool {
    true
}

fn default_auto_profile() -> bool {
    true
}

struct SharedState {
    engine: Mutex<Option<Arc<Engine>>>,
    startup: Mutex<()>,
}

struct Engine {
    port: u16,
    secret: String,
    client: Client,
    child: Mutex<Option<Child>>,
    settings: Mutex<Preferences>,
    settings_path: PathBuf,
    history: Mutex<Vec<DownloadInfo>>,
    history_path: PathBuf,
    sources: Mutex<HashMap<String, String>>,
    sources_path: PathBuf,
    tuning_profiles: Mutex<HashMap<String, String>>,
    tuning_profiles_path: PathBuf,
    version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Preferences {
    download_dir: String,
    profile: String,
    max_concurrent_downloads: u32,
    connections_per_server: u32,
    split: u32,
    min_split_size: String,
    max_download_speed: String,
    max_upload_speed: String,
    speed_unit: String,
    #[serde(default = "default_notify_on_complete")]
    notify_on_complete: bool,
    #[serde(default = "default_auto_profile")]
    auto_profile: bool,
}

impl Preferences {
    fn defaults(download_dir: &Path) -> Self {
        Self {
            download_dir: download_dir.to_string_lossy().into_owned(),
            profile: "balanced".to_string(),
            max_concurrent_downloads: 5,
            connections_per_server: 8,
            split: 8,
            min_split_size: "1M".to_string(),
            max_download_speed: "0".to_string(),
            max_upload_speed: "0".to_string(),
            speed_unit: "MB".to_string(),
            notify_on_complete: true,
            auto_profile: true,
        }
    }

    fn load(path: &Path, fallback: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|data| serde_json::from_slice::<Preferences>(&data).ok())
            .map(|mut prefs| {
                if prefs.download_dir.trim().is_empty() {
                    prefs.download_dir = fallback.to_string_lossy().into_owned();
                }
                // Older or manually edited settings can exceed the standard aria2
                // per-server limit. Clamp them during load before validating the profile.
                prefs.connections_per_server = prefs.connections_per_server.clamp(1, 16);
                prefs.split = prefs.split.clamp(1, 16);
                prefs
            })
            .unwrap_or_else(|| Self::defaults(fallback))
    }

    fn aria2_options(&self) -> Value {
        json!({
            "dir": self.download_dir,
            "max-concurrent-downloads": self.max_concurrent_downloads.to_string(),
            "max-connection-per-server": self.connections_per_server.to_string(),
            "split": self.split.to_string(),
            "min-split-size": self.min_split_size,
            "max-overall-download-limit": self.max_download_speed,
            "max-overall-upload-limit": self.max_upload_speed,
            "continue": "true",
            "auto-file-renaming": "true",
            "file-allocation": "none",
            "check-certificate": "true",
            "max-tries": "5",
            "retry-wait": "3",
            "connect-timeout": "20",
            "timeout": "60"
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct DownloadInfo {
    gid: String,
    name: String,
    status: String,
    total_length: u64,
    completed_length: u64,
    download_speed: u64,
    upload_speed: u64,
    connections: u64,
    error_message: String,
    directory: String,
    file_path: String,
    source_url: String,
    #[serde(default)]
    tuning_profile: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EngineInfo {
    download_dir: String,
    aria2_version: String,
    app_version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GlobalStats {
    download_speed: u64,
    upload_speed: u64,
    active: u64,
    waiting: u64,
    stopped: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct ResourceHints {
    size_bytes: Option<u64>,
    supports_ranges: Option<bool>,
}

#[derive(Debug, Clone)]
struct DownloadTuning {
    connections: u32,
    split: u32,
    min_split_size: String,
    label: String,
}

fn probe_resource(client: &Client, link: &str) -> ResourceHints {
    let parsed = match Url::parse(link) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => url,
        _ => return ResourceHints::default(),
    };
    // HEAD is advisory only. A server that rejects it still downloads normally using fallback heuristics.
    let response = match client
        .head(parsed.as_str())
        .header(reqwest::header::USER_AGENT, format!("Pulso/{APP_VERSION}"))
        .send()
    {
        Ok(response) if response.status().is_success() => response,
        _ => return ResourceHints::default(),
    };
    let size_bytes = response.content_length().filter(|size| *size > 0);
    let supports_ranges = response
        .headers()
        .get("accept-ranges")
        .and_then(|value| value.to_str().ok())
        .map(|value| match value.trim().to_ascii_lowercase().as_str() {
            "bytes" => Some(true),
            "none" => Some(false),
            _ => None,
        })
        .flatten();
    ResourceHints {
        size_bytes,
        supports_ranges,
    }
}

fn profile_label(profile: &str) -> &'static str {
    match profile {
        "eco" => "Manual · Economia",
        "balanced" => "Manual · Equilibrado",
        "turbo" => "Manual · Turbo",
        _ => "Manual · Personalizado",
    }
}

fn auto_tuning(link: &str, hints: ResourceHints, prefs: &Preferences) -> DownloadTuning {
    if !prefs.auto_profile {
        return DownloadTuning {
            connections: prefs.connections_per_server,
            split: prefs.split,
            min_split_size: prefs.min_split_size.clone(),
            label: profile_label(&prefs.profile).to_string(),
        };
    }

    if link.to_ascii_lowercase().starts_with("magnet:") {
        return DownloadTuning {
            connections: prefs.connections_per_server,
            split: prefs.split,
            min_split_size: prefs.min_split_size.clone(),
            label: "Pulso Auto · BitTorrent".to_string(),
        };
    }

    if hints.supports_ranges == Some(false) {
        return DownloadTuning {
            connections: 1,
            split: 1,
            min_split_size: "1M".to_string(),
            label: "Pulso Auto · conexão única".to_string(),
        };
    }

    let url = match Url::parse(link) {
        Ok(url) => url,
        Err(_) => {
            return DownloadTuning {
                connections: 4,
                split: 4,
                min_split_size: "2M".to_string(),
                label: "Pulso Auto · equilibrado".to_string(),
            }
        }
    };
    let extension = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .and_then(|name| {
            name.rsplit_once('.')
                .map(|(_, ext)| ext.to_ascii_lowercase())
        })
        .unwrap_or_default();
    let likely_large = [
        "iso", "img", "bin", "zip", "7z", "rar", "tar", "gz", "xz", "zst", "bz2", "appimage",
        "deb", "rpm", "exe", "msi", "dmg", "pkg", "mp4", "mkv", "mov", "webm", "flac", "wav",
        "vhd", "vhdx", "qcow2", "ova", "obb", "pak",
    ]
    .contains(&extension.as_str());
    let likely_small = [
        "txt", "md", "json", "xml", "css", "js", "svg", "ico", "png", "jpg", "jpeg", "webp", "gif",
        "woff", "woff2", "ttf", "otf", "csv", "yml", "yaml",
    ]
    .contains(&extension.as_str());

    let (connections, split, min_split_size, label) = match hints.size_bytes {
        Some(size) if size < 8 * 1024 * 1024 => (1, 1, "1M", "Pulso Auto · arquivo pequeno"),
        Some(size) if size < 64 * 1024 * 1024 => (4, 4, "1M", "Pulso Auto · equilibrado"),
        Some(size) if size < 512 * 1024 * 1024 => (8, 8, "2M", "Pulso Auto · multiconexão"),
        Some(_) => (16, 16, "4M", "Pulso Auto · arquivo grande"),
        None if likely_small => (1, 1, "1M", "Pulso Auto · arquivo pequeno"),
        None if likely_large => (8, 8, "2M", "Pulso Auto · arquivo grande provável"),
        None => (4, 4, "2M", "Pulso Auto · equilibrado"),
    };
    DownloadTuning {
        connections,
        split,
        min_split_size: min_split_size.to_string(),
        label: label.to_string(),
    }
}

fn make_probe_client() -> Result<Client, String> {
    Client::builder()
        .connect_timeout(Duration::from_millis(600))
        .timeout(Duration::from_millis(1500))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|error| format!("Não consegui preparar o detector de perfil automático: {error}"))
}

impl Engine {
    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/jsonrpc", self.port)
    }

    fn call(&self, method: &str, mut params: Vec<Value>) -> Result<Value, String> {
        params.insert(0, json!(format!("token:{}", self.secret)));
        let payload = json!({
            "jsonrpc": "2.0",
            "id": "pulso",
            "method": method,
            "params": params
        });

        let response = self
            .client
            .post(self.url())
            .json(&payload)
            .send()
            .map_err(|error| format!("Não consegui falar com o aria2: {error}"))?;
        let status = response.status();
        let body: Value = response
            .json()
            .map_err(|error| format!("Resposta inválida do aria2: {error}"))?;

        if !status.is_success() {
            return Err(format!("O aria2 respondeu com HTTP {status}"));
        }
        if let Some(error) = body.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("erro desconhecido");
            let code = error.get("code").and_then(Value::as_i64);
            return Err(match code {
                Some(code) => format!("aria2 ({code}): {message}"),
                None => message.to_string(),
            });
        }
        body.get("result")
            .cloned()
            .ok_or_else(|| "O aria2 não retornou um resultado.".to_string())
    }

    fn current_preferences(&self) -> Result<Preferences, String> {
        self.settings
            .lock()
            .map(|prefs| prefs.clone())
            .map_err(|_| "As preferências estão temporariamente indisponíveis.".to_string())
    }

    fn download_list(&self) -> Result<Vec<DownloadInfo>, String> {
        let keys = json!([
            "gid",
            "status",
            "totalLength",
            "completedLength",
            "downloadSpeed",
            "uploadSpeed",
            "errorMessage",
            "dir",
            "connections",
            "files",
            "bittorrent"
        ]);
        let active = self.call("aria2.tellActive", vec![keys.clone()])?;
        let waiting = self.call(
            "aria2.tellWaiting",
            vec![json!(0), json!(500), keys.clone()],
        )?;
        let stopped = self.call("aria2.tellStopped", vec![json!(0), json!(500), keys])?;

        let source_map = self
            .sources
            .lock()
            .map(|sources| sources.clone())
            .unwrap_or_default();
        let tuning_map = self
            .tuning_profiles
            .lock()
            .map(|profiles| profiles.clone())
            .unwrap_or_default();
        let mut live = Vec::new();
        // History retains source URLs after the active RPC map has intentionally been
        // cleaned up. Enrich stopped RPC entries from history so retries remain usable.
        let history_snapshot = self
            .history
            .lock()
            .map(|history| history.clone())
            .unwrap_or_default();
        let history_by_gid: HashMap<String, DownloadInfo> = history_snapshot
            .into_iter()
            .map(|item| (item.gid.clone(), item))
            .collect();
        for item in active
            .as_array()
            .into_iter()
            .flatten()
            .chain(waiting.as_array().into_iter().flatten())
            .chain(stopped.as_array().into_iter().flatten())
        {
            let mut mapped = map_download(item);
            if let Some(original) = source_map.get(&mapped.gid) {
                mapped.source_url = original.clone();
            }
            if let Some(profile) = tuning_map.get(&mapped.gid) {
                mapped.tuning_profile = profile.clone();
            }
            if let Some(saved) = history_by_gid.get(&mapped.gid) {
                if mapped.source_url.is_empty() {
                    mapped.source_url = saved.source_url.clone();
                }
                if mapped.tuning_profile.is_empty() {
                    mapped.tuning_profile = saved.tuning_profile.clone();
                }
                if mapped.file_path.is_empty() {
                    mapped.file_path = saved.file_path.clone();
                }
                if mapped.error_message.is_empty() {
                    mapped.error_message = saved.error_message.clone();
                }
            }
            live.push(mapped);
        }

        let notify_enabled = self
            .settings
            .lock()
            .map(|prefs| prefs.notify_on_complete)
            .unwrap_or(false);
        let mut completed_notifications = Vec::new();
        let mut history_changed = false;
        let mut history = self
            .history
            .lock()
            .map_err(|_| "O histórico está temporariamente indisponível.".to_string())?;
        for item in live
            .iter()
            .filter(|item| ["complete", "error"].contains(&item.status.as_str()))
        {
            let already_recorded_complete = history
                .iter()
                .any(|saved| saved.gid == item.gid && saved.status == "complete");
            if notify_enabled && item.status == "complete" && !already_recorded_complete {
                completed_notifications.push(item.name.clone());
            }
            if let Some(existing) = history.iter_mut().find(|saved| saved.gid == item.gid) {
                if existing != item {
                    *existing = item.clone();
                    history_changed = true;
                }
            } else {
                history.insert(0, item.clone());
                history_changed = true;
            }
        }
        if history.len() > HISTORY_LIMIT {
            history.truncate(HISTORY_LIMIT);
            history_changed = true;
        }
        if history_changed {
            let _ = write_json_atomic(&self.history_path, &*history);
        }

        let terminal_ids: HashSet<String> = live
            .iter()
            .filter(|item| ["complete", "error", "removed"].contains(&item.status.as_str()))
            .map(|item| item.gid.clone())
            .collect();
        let live_ids: HashSet<String> = live.iter().map(|item| item.gid.clone()).collect();
        for item in history.iter() {
            if !live_ids.contains(&item.gid) {
                live.push(item.clone());
            }
        }
        // Keep aria2's order inside each group so that the waiting queue is not
        // silently reordered alphabetically by the interface.
        live.sort_by(|a, b| status_order(&a.status).cmp(&status_order(&b.status)));
        drop(history);
        // Terminal items are now stored with their original URL in history.json.
        self.forget_sources(&terminal_ids);
        self.forget_tuning_profiles(&terminal_ids);
        for name in completed_notifications {
            // Notifications are best-effort: the download remains successful even if
            // notify-send is not installed or the desktop notification service is absent.
            let _ = Command::new("notify-send")
                .arg("--app-name=Pulso")
                .arg("--icon=folder-download")
                .arg("Download concluído")
                .arg(name)
                .spawn();
        }
        Ok(live)
    }

    fn save_preferences(&self, prefs: &Preferences) -> Result<(), String> {
        write_json_atomic(&self.settings_path, prefs)
            .map_err(|error| format!("Não consegui salvar as preferências: {error}"))
    }

    fn remember_source(&self, gid: &str, source: &str) -> Result<(), String> {
        let mut sources = self
            .sources
            .lock()
            .map_err(|_| "O registro de origens está temporariamente indisponível.".to_string())?;
        sources.insert(gid.to_string(), source.to_string());
        write_json_atomic(&self.sources_path, &*sources)
            .map_err(|error| format!("Não consegui persistir a origem do download: {error}"))
    }

    fn remember_tuning_profile(&self, gid: &str, profile: &str) -> Result<(), String> {
        let mut profiles = self
            .tuning_profiles
            .lock()
            .map_err(|_| "O registro de perfis está temporariamente indisponível.".to_string())?;
        profiles.insert(gid.to_string(), profile.to_string());
        write_json_atomic(&self.tuning_profiles_path, &*profiles)
            .map_err(|error| format!("Não consegui persistir o perfil do download: {error}"))
    }

    fn forget_tuning_profiles(&self, gids: &HashSet<String>) {
        if let Ok(mut profiles) = self.tuning_profiles.lock() {
            let before = profiles.len();
            profiles.retain(|gid, _| !gids.contains(gid));
            if profiles.len() != before {
                let _ = write_json_atomic(&self.tuning_profiles_path, &*profiles);
            }
        }
    }

    fn forget_sources(&self, gids: &HashSet<String>) {
        if let Ok(mut sources) = self.sources.lock() {
            let before = sources.len();
            sources.retain(|gid, _| !gids.contains(gid));
            if sources.len() != before {
                let _ = write_json_atomic(&self.sources_path, &*sources);
            }
        }
    }

    fn remove_from_history(&self, gids: &HashSet<String>) {
        if let Ok(mut history) = self.history.lock() {
            let before = history.len();
            history.retain(|item| !gids.contains(&item.gid));
            if history.len() != before {
                let _ = write_json_atomic(&self.history_path, &*history);
            }
        }
        self.forget_tuning_profiles(gids);
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // Encerramento gracioso: dá ao aria2 oportunidade de gravar a sessão pendente.
        if let Ok(mut stream) = TcpStream::connect_timeout(
            &format!("127.0.0.1:{}", self.port).parse().unwrap(),
            Duration::from_millis(300),
        ) {
            let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
            let payload = json!({
                "jsonrpc": "2.0",
                "id": "pulso-exit",
                "method": "aria2.shutdown",
                "params": [format!("token:{}", self.secret)]
            })
            .to_string();
            let request = format!(
                "POST /jsonrpc HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                self.port,
                payload.len(),
                payload
            );
            let _ = stream.write_all(request.as_bytes());
            let _ = stream.set_read_timeout(Some(Duration::from_millis(600)));
            let mut response = [0_u8; 512];
            let _ = stream.read(&mut response);
        }

        if let Ok(slot) = self.child.get_mut() {
            if let Some(child) = slot.as_mut() {
                let mut exited = false;
                for _ in 0..30 {
                    if matches!(child.try_wait(), Ok(Some(_))) {
                        exited = true;
                        break;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                if !exited {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            *slot = None;
        }
    }
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())
}

fn value_string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|item| {
            item.as_str()
                .map(ToOwned::to_owned)
                .or_else(|| item.as_u64().map(|number| number.to_string()))
        })
        .unwrap_or_default()
}

fn number(value: &Value, key: &str) -> u64 {
    value_string(value, key).parse::<u64>().unwrap_or(0)
}

fn map_download(value: &Value) -> DownloadInfo {
    let first_file = value
        .get("files")
        .and_then(Value::as_array)
        .and_then(|files| files.first());
    let full_path = first_file
        .and_then(|file| file.get("path"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let torrent_name = value
        .get("bittorrent")
        .and_then(|bt| bt.get("info"))
        .and_then(|info| info.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let source_url = first_file
        .and_then(|file| file.get("uris"))
        .and_then(Value::as_array)
        .and_then(|uris| uris.first())
        .and_then(|uri| uri.get("uri"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let name = (!torrent_name.is_empty())
        .then(|| torrent_name.to_string())
        .or_else(|| {
            Path::new(full_path)
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned)
        })
        .or_else(|| filename_from_url(&source_url))
        .unwrap_or_else(|| "Download sem nome".to_string());
    let status = value_string(value, "status");

    DownloadInfo {
        gid: value_string(value, "gid"),
        name,
        status,
        total_length: number(value, "totalLength"),
        completed_length: number(value, "completedLength"),
        download_speed: number(value, "downloadSpeed"),
        upload_speed: number(value, "uploadSpeed"),
        connections: number(value, "connections"),
        error_message: value_string(value, "errorMessage"),
        directory: value_string(value, "dir"),
        file_path: full_path.to_string(),
        source_url,
        tuning_profile: String::new(),
    }
}

fn filename_from_url(raw: &str) -> Option<String> {
    let parsed = Url::parse(raw).ok()?;
    parsed
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .last()
        .map(|segment| segment.to_string())
}

fn status_order(status: &str) -> u8 {
    match status {
        "active" => 0,
        "waiting" => 1,
        "paused" => 2,
        "complete" => 3,
        "error" => 4,
        "removed" => 5,
        _ => 6,
    }
}

fn current_engine(state: &SharedState) -> Result<Arc<Engine>, String> {
    state
        .engine
        .lock()
        .map_err(|_| "Estado interno bloqueado.".to_string())?
        .as_ref()
        .cloned()
        .ok_or_else(|| "O motor aria2 ainda não foi iniciado.".to_string())
}

#[cfg(unix)]
fn restrict_private_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("Não consegui proteger a pasta de dados do Pulso: {error}"))
}

#[cfg(not(unix))]
fn restrict_private_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn migrate_legacy_app_data(app_data: &Path) -> Result<(), String> {
    // The product identifier changed to dev.pulso.desktop. Preserve existing user data
    // from the prior application identifier only when the new profile has no settings.
    if app_data.join("settings.json").exists() {
        return Ok(());
    }
    let Some(parent) = app_data.parent() else {
        return Ok(());
    };
    let candidates = [
        parent.join("dev.pulso.fdm"),
        parent.join("dev.pulso.app"),
        parent.join("com.pulso.app"),
    ];
    let files = [
        "settings.json",
        "history.json",
        "sources.json",
        "tuning-profiles.json",
        "aria2.session",
        "aria2.log",
    ];
    for legacy in candidates {
        if legacy == app_data || !legacy.is_dir() {
            continue;
        }
        if !files.iter().any(|name| legacy.join(name).is_file()) {
            continue;
        }
        for name in files {
            let source = legacy.join(name);
            let destination = app_data.join(name);
            if source.is_file() && !destination.exists() {
                fs::copy(&source, &destination).map_err(|error| {
                    format!("Não consegui migrar os dados locais ({name}): {error}")
                })?;
            }
        }
        break;
    }
    Ok(())
}

fn torrent_source(path: &Path) -> String {
    format!("{}{}", TORRENT_SOURCE_PREFIX, path.to_string_lossy())
}

fn torrent_path_from_source(source: &str) -> Option<&str> {
    source.strip_prefix(TORRENT_SOURCE_PREFIX)
}

fn has_torrent_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("torrent"))
}

fn home_download_dir(app: &AppHandle) -> PathBuf {
    app.path().download_dir().unwrap_or_else(|_| {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Downloads")
    })
}

#[tauri::command]
fn initialize(app: AppHandle, state: State<'_, SharedState>) -> Result<EngineInfo, String> {
    let _startup_guard = state
        .startup
        .lock()
        .map_err(|_| "Não consegui inicializar o motor.".to_string())?;

    if let Some(engine) = state
        .engine
        .lock()
        .map_err(|_| "Estado interno bloqueado.".to_string())?
        .as_ref()
        .cloned()
    {
        let prefs = engine.current_preferences()?;
        return Ok(EngineInfo {
            download_dir: prefs.download_dir,
            aria2_version: engine.version.clone(),
            app_version: APP_VERSION.to_string(),
        });
    }

    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Não consegui localizar a pasta de dados: {error}"))?;
    fs::create_dir_all(&app_data)
        .map_err(|error| format!("Não consegui criar a pasta de dados: {error}"))?;
    restrict_private_directory(&app_data)?;
    migrate_legacy_app_data(&app_data)?;

    let fallback_dir = home_download_dir(&app);
    let settings_path = app_data.join("settings.json");
    let mut prefs = Preferences::load(&settings_path, &fallback_dir);
    fs::create_dir_all(&prefs.download_dir)
        .map_err(|error| format!("Não consegui preparar a pasta de destino: {error}"))?;
    if !Path::new(&prefs.download_dir).is_dir() {
        return Err("A pasta de destino configurada não é um diretório.".to_string());
    }
    prefs = validate_preferences(prefs, &fallback_dir)?;
    write_json_atomic(&settings_path, &prefs)
        .map_err(|error| format!("Não consegui salvar as preferências iniciais: {error}"))?;

    let session_file = app_data.join("aria2.session");
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&session_file)
        .map_err(|error| format!("Não consegui preparar a sessão: {error}"))?;

    let log_file = app_data.join("aria2.log");
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("Não consegui reservar uma porta local: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    drop(listener);

    let secret = uuid::Uuid::new_v4().simple().to_string();
    let client = Client::builder()
        .connect_timeout(Duration::from_millis(800))
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|error| format!("Não consegui preparar o cliente RPC: {error}"))?;

    let child = Command::new("aria2c")
        .arg("--no-conf=true")
        .arg("--enable-rpc=true")
        .arg("--rpc-listen-all=false")
        .arg(format!("--rpc-listen-port={port}"))
        .arg(format!("--rpc-secret={secret}"))
        .arg(format!("--max-concurrent-downloads={}", prefs.max_concurrent_downloads))
        .arg(format!("--max-connection-per-server={}", prefs.connections_per_server))
        .arg(format!("--split={}", prefs.split))
        .arg(format!("--min-split-size={}", prefs.min_split_size))
        .arg(format!("--max-overall-download-limit={}", prefs.max_download_speed))
        .arg(format!("--max-overall-upload-limit={}", prefs.max_upload_speed))
        .arg("--continue=true")
        .arg("--auto-file-renaming=true")
        .arg("--file-allocation=none")
        .arg("--max-tries=5")
        .arg("--retry-wait=3")
        .arg("--connect-timeout=20")
        .arg("--timeout=60")
        .arg("--check-certificate=true")
        .arg("--save-session-interval=15")
        .arg(format!("--save-session={}", session_file.display()))
        .arg(format!("--input-file={}", session_file.display()))
        .arg(format!("--dir={}", prefs.download_dir))
        .arg(format!("--log={}", log_file.display()))
        .arg("--log-level=notice")
        .arg("--console-log-level=warn")
        .arg("--summary-interval=0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            format!(
                "Não consegui iniciar o aria2c. Confere se está instalado com `sudo pacman -S aria2`. Detalhe: {error}"
            )
        })?;

    let history_path = app_data.join("history.json");
    let history = fs::read(&history_path)
        .ok()
        .and_then(|data| serde_json::from_slice::<Vec<DownloadInfo>>(&data).ok())
        .unwrap_or_default();
    let sources_path = app_data.join("sources.json");
    let sources = fs::read(&sources_path)
        .ok()
        .and_then(|data| serde_json::from_slice::<HashMap<String, String>>(&data).ok())
        .unwrap_or_default();
    let tuning_profiles_path = app_data.join("tuning-profiles.json");
    let tuning_profiles = fs::read(&tuning_profiles_path)
        .ok()
        .and_then(|data| serde_json::from_slice::<HashMap<String, String>>(&data).ok())
        .unwrap_or_default();
    let engine = Arc::new(Engine {
        port,
        secret,
        client,
        child: Mutex::new(Some(child)),
        settings: Mutex::new(prefs.clone()),
        settings_path,
        history: Mutex::new(history),
        history_path,
        sources: Mutex::new(sources),
        sources_path,
        tuning_profiles: Mutex::new(tuning_profiles),
        tuning_profiles_path,
        version: String::new(),
    });

    let mut detected_version = None;
    let mut last_error = String::new();
    for _ in 0..40 {
        match engine.call("aria2.getVersion", vec![]) {
            Ok(result) => {
                detected_version = result
                    .get("version")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                if detected_version.is_some() {
                    break;
                }
            }
            Err(error) => last_error = error,
        }
        if let Ok(mut child_slot) = engine.child.lock() {
            if let Some(child) = child_slot.as_mut() {
                if let Ok(Some(exit)) = child.try_wait() {
                    last_error = format!(
                        "aria2c encerrou com {exit}. Consulte {}",
                        log_file.display()
                    );
                    break;
                }
            }
        }
        thread::sleep(Duration::from_millis(150));
    }

    let version = detected_version.ok_or_else(|| {
        format!(
            "O aria2 não respondeu ao iniciar. {last_error}. Consulte o log em {}",
            log_file.display()
        )
    })?;
    // Version is immutable for this engine instance and was queried from the running process.
    let mut engine =
        Arc::try_unwrap(engine).map_err(|_| "Falha interna ao iniciar o motor.".to_string())?;
    engine.version = version.clone();
    let engine = Arc::new(engine);

    *state
        .engine
        .lock()
        .map_err(|_| "Estado interno bloqueado.".to_string())? = Some(engine.clone());

    Ok(EngineInfo {
        download_dir: prefs.download_dir,
        aria2_version: version,
        app_version: APP_VERSION.to_string(),
    })
}

fn validate_preferences(
    mut prefs: Preferences,
    fallback_dir: &Path,
) -> Result<Preferences, String> {
    if prefs.download_dir.trim().is_empty() {
        prefs.download_dir = fallback_dir.to_string_lossy().into_owned();
    }
    if !(1..=10).contains(&prefs.max_concurrent_downloads) {
        return Err("Downloads simultâneos precisa ficar entre 1 e 10.".to_string());
    }
    // aria2's standard build caps max-connection-per-server at 16. Keep the
    // UI and Rust validation aligned with upstream rather than accepting values
    // that can make changeGlobalOption fail at runtime.
    if !(1..=16).contains(&prefs.connections_per_server) || !(1..=16).contains(&prefs.split) {
        return Err("Conexões e divisões precisam ficar entre 1 e 16 no aria2 padrão.".to_string());
    }
    // Older or manually edited preferences may contain split sizes below aria2's limit.
    if ["256K", "512K"].contains(&prefs.min_split_size.as_str()) {
        prefs.min_split_size = "1M".to_string();
    } else if !["1M", "2M", "4M"].contains(&prefs.min_split_size.as_str()) {
        return Err("Tamanho mínimo de divisão inválido.".to_string());
    }
    let allowed_limits = [
        "0", "256K", "512K", "1M", "2M", "5M", "10M", "25M", "50M", "100M",
    ];
    if !allowed_limits.contains(&prefs.max_download_speed.as_str())
        || !allowed_limits.contains(&prefs.max_upload_speed.as_str())
    {
        return Err("Limite de velocidade inválido.".to_string());
    }
    if !["MB", "Mb"].contains(&prefs.speed_unit.as_str()) {
        prefs.speed_unit = "MB".to_string();
    }
    if !["eco", "balanced", "turbo", "custom"].contains(&prefs.profile.as_str()) {
        prefs.profile = "custom".to_string();
    }
    Ok(prefs)
}

fn validate_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("Há um link vazio na lista.".to_string());
    }

    // Inspect the original text before Url::parse normalizes malformed URLs.
    // In particular, reject forms such as "https:///file.zip" where the
    // parser may treat the first path segment as the host.
    let (raw_scheme, remainder) = raw
        .split_once(':')
        .ok_or_else(|| format!("Link inválido: {raw}"))?;

    match raw_scheme.to_ascii_lowercase().as_str() {
        "http" | "https" | "ftp" | "sftp" => {
            let authority_and_path = remainder
                .strip_prefix("//")
                .ok_or_else(|| format!("O link não contém um servidor válido: {raw}"))?;
            let raw_authority = authority_and_path
                .split(|character: char| matches!(character, '/' | '\\' | '?' | '#'))
                .next()
                .unwrap_or("");
            if raw_authority.is_empty() {
                return Err(format!("O link não contém um servidor válido: {raw}"));
            }

            let parsed = Url::parse(raw).map_err(|_| format!("Link inválido: {raw}"))?;
            if parsed.host_str().filter(|host| !host.is_empty()).is_none() {
                return Err(format!("O link não contém um servidor válido: {raw}"));
            }
        }
        "magnet" => {
            let parsed = Url::parse(raw).map_err(|_| format!("Magnet link inválido: {raw}"))?;
            if !parsed
                .query_pairs()
                .any(|(key, value)| key == "xt" && value.starts_with("urn:btih:"))
            {
                return Err(
                    "Magnet link sem identificador BitTorrent (xt=urn:btih:...).".to_string(),
                );
            }
        }
        _ => return Err("Use links HTTP, HTTPS, FTP, SFTP ou magnet.".to_string()),
    }
    Ok(raw.to_string())
}

fn safe_filename(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("O nome do arquivo não pode estar vazio.".to_string());
    }
    if trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains('\0')
        || trimmed == "."
        || trimmed == ".."
    {
        return Err("Informe apenas o nome do arquivo, sem caminhos ou barras.".to_string());
    }
    if Path::new(trimmed)
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("Esse nome de arquivo não é válido.".to_string());
    }
    Ok(trimmed.to_string())
}

#[tauri::command]
fn get_downloads(state: State<'_, SharedState>) -> Result<Vec<DownloadInfo>, String> {
    current_engine(&state)?.download_list()
}

#[tauri::command]
fn get_stats(state: State<'_, SharedState>) -> Result<GlobalStats, String> {
    let result = current_engine(&state)?.call("aria2.getGlobalStat", vec![])?;
    Ok(GlobalStats {
        download_speed: number(&result, "downloadSpeed"),
        upload_speed: number(&result, "uploadSpeed"),
        active: number(&result, "numActive"),
        waiting: number(&result, "numWaiting"),
        stopped: number(&result, "numStopped"),
    })
}

#[tauri::command]
fn get_preferences(state: State<'_, SharedState>) -> Result<Preferences, String> {
    current_engine(&state)?.current_preferences()
}

#[tauri::command]
fn update_preferences(
    preferences: Preferences,
    state: State<'_, SharedState>,
) -> Result<Preferences, String> {
    let engine = current_engine(&state)?;
    let previous = engine.current_preferences()?;
    let fallback = PathBuf::from(&previous.download_dir);
    let mut next = validate_preferences(preferences, &fallback)?;
    fs::create_dir_all(&next.download_dir)
        .map_err(|error| format!("Não consegui criar a pasta de destino: {error}"))?;
    if !Path::new(&next.download_dir).is_dir() {
        return Err("A pasta de destino não é um diretório.".to_string());
    }
    next.download_dir = fs::canonicalize(&next.download_dir)
        .unwrap_or_else(|_| PathBuf::from(&next.download_dir))
        .to_string_lossy()
        .into_owned();

    engine.call("aria2.changeGlobalOption", vec![next.aria2_options()])?;
    if let Err(error) = engine.save_preferences(&next) {
        // Keep runtime and disk settings in sync if persistence fails.
        let _ = engine.call("aria2.changeGlobalOption", vec![previous.aria2_options()]);
        return Err(error);
    }
    *engine
        .settings
        .lock()
        .map_err(|_| "As preferências estão temporariamente indisponíveis.".to_string())? =
        next.clone();
    Ok(next)
}

#[tauri::command]
fn choose_directory(initial_dir: String) -> Result<String, String> {
    let initial = if initial_dir.trim().is_empty() || !Path::new(&initial_dir).is_dir() {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Downloads")
    } else {
        PathBuf::from(initial_dir)
    };

    match Command::new("kdialog")
        .arg("--getexistingdirectory")
        .arg(&initial)
        .arg("Selecionar pasta de downloads")
        .output()
    {
        Ok(output) if output.status.success() => {
            let chosen = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if chosen.is_empty() {
                Err("Seleção de pasta cancelada.".to_string())
            } else {
                Ok(chosen)
            }
        }
        Ok(_) => Err("Seleção de pasta cancelada.".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match Command::new("zenity")
                .arg("--file-selection")
                .arg("--directory")
                .arg(format!("--filename={}/", initial.display()))
                .output()
            {
                Ok(output) if output.status.success() => {
                    let chosen = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if chosen.is_empty() {
                        Err("Seleção de pasta cancelada.".to_string())
                    } else {
                        Ok(chosen)
                    }
                }
                Ok(_) => Err("Seleção de pasta cancelada.".to_string()),
                Err(zenity_error) if zenity_error.kind() == std::io::ErrorKind::NotFound => Err(
                    "Instale kdialog ou zenity para escolher uma pasta pelo diálogo gráfico."
                        .to_string(),
                ),
                Err(zenity_error) => Err(format!(
                    "Não consegui abrir o seletor de pastas: {zenity_error}"
                )),
            }
        }
        Err(error) => Err(format!("Não consegui abrir o seletor de pastas: {error}")),
    }
}

fn parse_dialog_choice(output: std::process::Output) -> Option<String> {
    if !output.status.success() {
        return None;
    }
    let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!selected.is_empty()).then_some(selected)
}

#[tauri::command]
fn choose_torrent_file() -> Result<Option<String>, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let downloads = home.join("Downloads");
    let initial = if downloads.is_dir() { downloads } else { home };
    match Command::new("kdialog")
        .arg("--getopenfilename")
        .arg(&initial)
        .arg("Torrent files (*.torrent)")
        .arg("Selecionar arquivo .torrent")
        .output()
    {
        Ok(output) => Ok(parse_dialog_choice(output)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match Command::new("zenity")
                .arg("--file-selection")
                .arg("--title=Selecionar arquivo .torrent")
                .arg("--file-filter=Arquivos torrent | *.torrent")
                .arg(format!("--filename={}/", initial.display()))
                .output()
            {
                Ok(output) => Ok(parse_dialog_choice(output)),
                Err(zenity_error) if zenity_error.kind() == std::io::ErrorKind::NotFound => {
                    Err("Para selecionar um torrent, instale kdialog ou zenity.".to_string())
                }
                Err(zenity_error) => Err(format!(
                    "Não consegui abrir o seletor de torrents: {zenity_error}"
                )),
            }
        }
        Err(error) => Err(format!("Não consegui abrir o seletor de torrents: {error}")),
    }
}

fn add_torrent_file_inner(
    engine: &Arc<Engine>,
    torrent_path: &str,
    target_dir: &str,
) -> Result<String, String> {
    let path = fs::canonicalize(torrent_path)
        .map_err(|error| format!("Não consegui acessar o arquivo .torrent: {error}"))?;
    if !path.is_file() {
        return Err("O caminho escolhido não é um arquivo.".to_string());
    }
    if !has_torrent_extension(&path) {
        return Err("Escolha um arquivo com extensão .torrent.".to_string());
    }
    let metadata = fs::metadata(&path)
        .map_err(|error| format!("Não consegui ler os dados do torrent: {error}"))?;
    if metadata.len() == 0 {
        return Err("O arquivo .torrent está vazio.".to_string());
    }
    if metadata.len() > MAX_TORRENT_FILE_BYTES {
        return Err(
            "O arquivo .torrent é maior que 32 MiB; por segurança, o Pulso não o carrega."
                .to_string(),
        );
    }
    let data =
        fs::read(&path).map_err(|error| format!("Não consegui ler o arquivo .torrent: {error}"))?;
    if data.is_empty() || data.len() as u64 > MAX_TORRENT_FILE_BYTES {
        return Err(
            "O tamanho do arquivo .torrent mudou durante a leitura; escolha o arquivo novamente."
                .to_string(),
        );
    }
    let folder = if target_dir.trim().is_empty() {
        engine.current_preferences()?.download_dir
    } else {
        target_dir.to_string()
    };
    fs::create_dir_all(&folder)
        .map_err(|error| format!("Não consegui preparar a pasta de destino: {error}"))?;
    if !Path::new(&folder).is_dir() {
        return Err("A pasta escolhida não é um diretório.".to_string());
    }
    let folder = fs::canonicalize(&folder)
        .unwrap_or_else(|_| PathBuf::from(&folder))
        .to_string_lossy()
        .into_owned();
    let options = json!({
        "dir": folder,
        "continue": "true",
        "auto-file-renaming": "true",
        "file-allocation": "none",
        "max-tries": "5",
        "retry-wait": "3"
    });
    let result = engine.call(
        "aria2.addTorrent",
        vec![json!(BASE64.encode(&data)), json!([]), options],
    )?;
    let gid = result
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| "O aria2 não devolveu o identificador do torrent.".to_string())?;
    // Keep the original local metainfo path so a failed torrent can be retried later.
    let _ = engine.remember_source(&gid, &torrent_source(&path));
    let _ = engine.remember_tuning_profile(&gid, "BitTorrent · arquivo local");
    Ok(gid)
}

#[tauri::command]
fn add_torrent_file(
    file_path: String,
    target_dir: Option<String>,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let engine = current_engine(&state)?;
    add_torrent_file_inner(&engine, &file_path, target_dir.as_deref().unwrap_or(""))
}

#[tauri::command]
fn add_downloads(
    urls: Vec<String>,
    file_name: Option<String>,
    target_dir: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Vec<String>, String> {
    if urls.is_empty() {
        return Err("Cole pelo menos um link.".to_string());
    }
    if urls.len() > 100 {
        return Err("O Pulso aceita até 100 links por lote.".to_string());
    }
    let validated: Vec<String> = urls
        .iter()
        .map(|raw| validate_url(raw))
        .collect::<Result<_, _>>()?;
    let clean_name = match file_name {
        Some(name) if !name.trim().is_empty() => {
            if validated.len() > 1 {
                return Err("Para baixar vários links, deixe o nome do arquivo vazio.".to_string());
            }
            if validated[0].to_ascii_lowercase().starts_with("magnet:") {
                return Err(
                    "O nome personalizado não se aplica a downloads magnet/BitTorrent.".to_string(),
                );
            }
            Some(safe_filename(&name)?)
        }
        _ => None,
    };

    let engine = current_engine(&state)?;
    let prefs = engine.current_preferences()?;
    let folder = target_dir
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| prefs.download_dir.clone());
    fs::create_dir_all(&folder)
        .map_err(|error| format!("Não consegui criar a pasta de destino: {error}"))?;
    if !Path::new(&folder).is_dir() {
        return Err("A pasta escolhida não é um diretório.".to_string());
    }

    let base_options = json!({
        "dir": folder,
        "continue": "true",
        "max-connection-per-server": prefs.connections_per_server.to_string(),
        "split": prefs.split.to_string(),
        "min-split-size": prefs.min_split_size,
        "auto-file-renaming": "true",
        "file-allocation": "none",
        "check-certificate": "true",
        "max-tries": "5",
        "retry-wait": "3",
        "connect-timeout": "20",
        "timeout": "60"
    });

    // Metadata checks only for small batches, in parallel, with a strict timeout.
    // Large batches use URL/type heuristics so pasting a list never stalls for every URL.
    let mut hints_by_index = vec![ResourceHints::default(); validated.len()];
    if prefs.auto_profile && validated.len() <= 8 {
        if let Ok(probe_client) = make_probe_client() {
            let probed = thread::scope(|scope| {
                let handles = validated
                    .iter()
                    .map(|link| {
                        let client = probe_client.clone();
                        scope.spawn(move || probe_resource(&client, link))
                    })
                    .collect::<Vec<_>>();
                handles
                    .into_iter()
                    .map(|handle| handle.join().unwrap_or_default())
                    .collect::<Vec<_>>()
            });
            hints_by_index = probed;
        }
    }

    let mut gids = Vec::with_capacity(validated.len());
    let mut source_pairs = Vec::with_capacity(validated.len());
    for (index, link) in validated.into_iter().enumerate() {
        let tuning = auto_tuning(&link, hints_by_index[index], &prefs);
        let mut options = base_options.clone();
        options["max-connection-per-server"] = json!(tuning.connections.to_string());
        options["split"] = json!(tuning.split.to_string());
        options["min-split-size"] = json!(tuning.min_split_size);
        if let Some(name) = clean_name.as_ref() {
            if !link.to_ascii_lowercase().starts_with("magnet:") {
                options["out"] = json!(name);
            }
        }
        let result = engine.call("aria2.addUri", vec![json!([link.clone()]), options]);
        let gid_value = match result {
            Ok(value) => value,
            Err(error) => {
                // Do not leave a half-added batch behind while the UI reports failure.
                for added_gid in &gids {
                    let _ = engine.call("aria2.remove", vec![json!(added_gid)]);
                    let _ = engine.call("aria2.removeDownloadResult", vec![json!(added_gid)]);
                }
                let ids: HashSet<String> = gids.iter().cloned().collect();
                engine.forget_sources(&ids);
                return Err(format!("O lote não foi adicionado por completo; os itens anteriores foram revertidos. {error}"));
            }
        };
        let gid = match gid_value.as_str() {
            Some(value) => value.to_string(),
            None => {
                for added_gid in &gids {
                    let _ = engine.call("aria2.remove", vec![json!(added_gid)]);
                    let _ = engine.call("aria2.removeDownloadResult", vec![json!(added_gid)]);
                }
                let ids: HashSet<String> = gids.iter().cloned().collect();
                engine.forget_sources(&ids);
                return Err(
                    "O aria2 não devolveu o identificador de um download; o lote foi revertido."
                        .to_string(),
                );
            }
        };
        gids.push(gid.clone());
        source_pairs.push((gid, link, tuning.label));
    }
    // Persist origin and chosen strategy only after all RPC calls in the batch succeed.
    for (gid, link, profile) in source_pairs {
        let _ = engine.remember_source(&gid, &link);
        let _ = engine.remember_tuning_profile(&gid, &profile);
    }
    Ok(gids)
}

#[tauri::command]
fn pause_download(gid: String, state: State<'_, SharedState>) -> Result<(), String> {
    current_engine(&state)?.call("aria2.pause", vec![json!(gid)])?;
    Ok(())
}

#[tauri::command]
fn resume_download(gid: String, state: State<'_, SharedState>) -> Result<(), String> {
    current_engine(&state)?.call("aria2.unpause", vec![json!(gid)])?;
    Ok(())
}

#[tauri::command]
fn retry_download(
    source_url: String,
    file_name: String,
    target_dir: String,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let engine = current_engine(&state)?;
    let prefs = engine.current_preferences()?;
    let folder = if target_dir.trim().is_empty() {
        prefs.download_dir.clone()
    } else {
        target_dir
    };
    if let Some(torrent_path) = torrent_path_from_source(&source_url) {
        return add_torrent_file_inner(&engine, torrent_path, &folder);
    }
    let link = validate_url(&source_url)?;
    fs::create_dir_all(&folder)
        .map_err(|error| format!("Não consegui preparar a pasta de destino: {error}"))?;
    if !Path::new(&folder).is_dir() {
        return Err("A pasta do download anterior não é mais válida.".to_string());
    }
    let hints = if prefs.auto_profile {
        make_probe_client()
            .map(|client| probe_resource(&client, &link))
            .unwrap_or_default()
    } else {
        ResourceHints::default()
    };
    let tuning = auto_tuning(&link, hints, &prefs);
    let mut options = json!({
        "dir": folder,
        "continue": "true",
        "max-connection-per-server": tuning.connections.to_string(),
        "split": tuning.split.to_string(),
        "min-split-size": tuning.min_split_size,
        "auto-file-renaming": "true",
        "file-allocation": "none",
        "check-certificate": "true",
        "max-tries": "5",
        "retry-wait": "3"
    });
    if !link.to_ascii_lowercase().starts_with("magnet:")
        && !file_name.trim().is_empty()
        && file_name != "Download sem nome"
    {
        options["out"] = json!(safe_filename(&file_name)?);
    }
    let result = engine.call("aria2.addUri", vec![json!([link.clone()]), options])?;
    let gid = result
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| "O aria2 não devolveu o identificador do novo download.".to_string())?;
    let _ = engine.remember_source(&gid, &link);
    let _ = engine.remember_tuning_profile(&gid, &tuning.label);
    Ok(gid)
}

#[tauri::command]
fn remove_download(
    gid: String,
    status: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let engine = current_engine(&state)?;
    if ["complete", "error", "removed"].contains(&status.as_str()) {
        // The terminal item may exist only in Pulso's persistent history after a restart.
        let _ = engine.call("aria2.removeDownloadResult", vec![json!(gid.clone())]);
    } else {
        engine.call("aria2.remove", vec![json!(gid)])?;
        let _ = engine.call("aria2.removeDownloadResult", vec![json!(gid)]);
    }
    let remove_ids = HashSet::from([gid]);
    engine.remove_from_history(&remove_ids);
    engine.forget_sources(&remove_ids);
    Ok(())
}

#[tauri::command]
fn pause_all(state: State<'_, SharedState>) -> Result<(), String> {
    current_engine(&state)?.call("aria2.pauseAll", vec![])?;
    Ok(())
}

#[tauri::command]
fn resume_all(state: State<'_, SharedState>) -> Result<(), String> {
    current_engine(&state)?.call("aria2.unpauseAll", vec![])?;
    Ok(())
}

#[tauri::command]
fn clear_finished(state: State<'_, SharedState>) -> Result<u64, String> {
    let engine = current_engine(&state)?;
    let stopped = engine.call(
        "aria2.tellStopped",
        vec![json!(0), json!(1000), json!(["gid", "status"])],
    )?;
    let mut to_clear = HashSet::new();
    if let Some(items) = stopped.as_array() {
        for item in items {
            let status = value_string(item, "status");
            if ["complete", "error", "removed"].contains(&status.as_str()) {
                let gid = value_string(item, "gid");
                if !gid.is_empty() {
                    to_clear.insert(gid);
                }
            }
        }
    }
    if let Ok(history) = engine.history.lock() {
        for item in history.iter() {
            to_clear.insert(item.gid.clone());
        }
    }
    for gid in &to_clear {
        // Some old history entries may already have been discarded by aria2; that is fine.
        let _ = engine.call("aria2.removeDownloadResult", vec![json!(gid)]);
    }
    let count = to_clear.len() as u64;
    engine.remove_from_history(&to_clear);
    engine.forget_sources(&to_clear);
    Ok(count)
}

#[tauri::command]
fn open_path(path: String, reveal: bool) -> Result<(), String> {
    let target = PathBuf::from(path);
    let chosen = if reveal && target.is_file() {
        target.parent().map(Path::to_path_buf).unwrap_or(target)
    } else {
        target
    };
    if !chosen.exists() {
        return Err("Esse caminho não existe mais no disco.".to_string());
    }
    Command::new("xdg-open")
        .arg(&chosen)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Não consegui abrir o caminho: {error}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(SharedState {
            engine: Mutex::new(None),
            startup: Mutex::new(()),
        })
        .invoke_handler(tauri::generate_handler![
            initialize,
            get_downloads,
            get_stats,
            get_preferences,
            update_preferences,
            choose_directory,
            choose_torrent_file,
            add_torrent_file,
            add_downloads,
            pause_download,
            resume_download,
            retry_download,
            remove_download,
            pause_all,
            resume_all,
            clear_finished,
            open_path
        ])
        .run(tauri::generate_context!())
        .expect("erro ao executar Pulso");
}

#[cfg(test)]
mod tests {
    use super::{
        auto_tuning, has_torrent_extension, safe_filename, torrent_path_from_source,
        torrent_source, validate_preferences, validate_url, Preferences, ResourceHints,
    };
    use std::path::Path;

    #[test]
    fn accepts_supported_download_links() {
        assert!(validate_url("https://example.com/file.zip").is_ok());
        assert!(validate_url("ftp://example.com/pub/file.iso").is_ok());
        assert!(validate_url("sftp://example.com/backup.tar").is_ok());
        assert!(validate_url("magnet:?xt=urn:btih:0123456789abcdef").is_ok());
    }

    #[test]
    fn rejects_unsupported_or_malformed_links() {
        assert!(validate_url("").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("javascript:alert(1)").is_err());
        assert!(validate_url("https:///file.zip").is_err());
        assert!(validate_url("magnet:?dn=no-info-hash").is_err());
    }

    #[test]
    fn filename_cannot_escape_the_selected_directory() {
        assert_eq!(safe_filename("setup.zip").unwrap(), "setup.zip");
        assert!(safe_filename("../setup.zip").is_err());
        assert!(safe_filename("folder/setup.zip").is_err());
        assert!(safe_filename("..").is_err());
    }

    #[test]
    fn preferences_migrate_legacy_min_split_values() {
        let fallback = Path::new("/tmp");
        let mut prefs = Preferences::defaults(fallback);
        prefs.min_split_size = "256K".to_string();
        let validated = validate_preferences(prefs, fallback).unwrap();
        assert_eq!(validated.min_split_size, "1M");
    }

    #[test]
    fn defaults_enable_desktop_notifications_and_auto_tuning() {
        let prefs = Preferences::defaults(Path::new("/tmp"));
        assert!(prefs.notify_on_complete);
        assert!(prefs.auto_profile);
    }

    #[test]
    fn automatic_tuning_uses_single_connection_when_server_rejects_ranges() {
        let prefs = Preferences::defaults(Path::new("/tmp"));
        let tuning = auto_tuning(
            "https://example.com/disk.iso",
            ResourceHints {
                size_bytes: Some(2_000_000_000),
                supports_ranges: Some(false),
            },
            &prefs,
        );
        assert_eq!(tuning.connections, 1);
        assert_eq!(tuning.split, 1);
        assert!(tuning.label.contains("conexão única"));
    }

    #[test]
    fn automatic_tuning_scales_up_for_large_downloads() {
        let prefs = Preferences::defaults(Path::new("/tmp"));
        let tuning = auto_tuning(
            "https://example.com/disk.iso",
            ResourceHints {
                size_bytes: Some(2_000_000_000),
                supports_ranges: Some(true),
            },
            &prefs,
        );
        assert_eq!(tuning.connections, 16);
        assert_eq!(tuning.split, 16);
    }

    #[test]
    fn automatic_tuning_keeps_small_files_lightweight() {
        let prefs = Preferences::defaults(Path::new("/tmp"));
        let tuning = auto_tuning(
            "https://example.com/readme.txt",
            ResourceHints {
                size_bytes: Some(200_000),
                supports_ranges: None,
            },
            &prefs,
        );
        assert_eq!(tuning.connections, 1);
        assert_eq!(tuning.split, 1);
    }

    #[test]
    fn standard_aria2_limit_is_enforced() {
        let mut prefs = Preferences::defaults(Path::new("/tmp"));
        prefs.connections_per_server = 32;
        assert!(validate_preferences(prefs, Path::new("/tmp")).is_err());
    }

    #[test]
    fn local_torrent_source_round_trips() {
        let original = Path::new("/tmp/linux-distro.torrent");
        let encoded = torrent_source(original);
        assert_eq!(
            torrent_path_from_source(&encoded),
            Some("/tmp/linux-distro.torrent")
        );
        assert_eq!(
            torrent_path_from_source("https://example.com/file.zip"),
            None
        );
    }

    #[test]
    fn torrent_extension_check_is_case_insensitive_and_path_only() {
        assert!(has_torrent_extension(Path::new("linux.torrent")));
        assert!(has_torrent_extension(Path::new("LINUX.TORRENT")));
        assert!(!has_torrent_extension(Path::new("linux.torrent.zip")));
        assert!(!has_torrent_extension(Path::new("linux.iso")));
    }

    #[test]
    fn magnet_and_torrent_source_are_not_confused() {
        assert!(validate_url("magnet:?xt=urn:btih:0123456789abcdef").is_ok());
        assert_eq!(
            torrent_path_from_source("torrent-file:/tmp/test.torrent"),
            Some("/tmp/test.torrent")
        );
    }

    #[test]
    fn legacy_preferences_enable_auto_profile_by_default() {
        let prefs = Preferences::defaults(Path::new("/tmp"));
        let mut legacy = serde_json::to_value(prefs).unwrap();
        legacy.as_object_mut().unwrap().remove("autoProfile");
        let restored: Preferences = serde_json::from_value(legacy).unwrap();
        assert!(restored.auto_profile);
    }
}
