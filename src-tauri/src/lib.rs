use reqwest::blocking::Client;
use serde::Serialize;
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager, State};

struct SharedState {
    engine: Mutex<Option<Arc<Engine>>>,
    startup: Mutex<()>,
}

struct Engine {
    port: u16,
    secret: String,
    client: Client,
    child: Mutex<Option<Child>>,
    download_dir: PathBuf,
    version: Mutex<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EngineInfo {
    download_dir: String,
    aria2_version: String,
}

#[derive(Serialize, Clone)]
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
            return Err(message.to_string());
        }
        body.get("result")
            .cloned()
            .ok_or_else(|| "O aria2 não retornou um resultado.".to_string())
    }

    fn download_list(&self) -> Result<Vec<DownloadInfo>, String> {
        let keys = json!([
            "gid", "status", "totalLength", "completedLength", "downloadSpeed",
            "uploadSpeed", "errorMessage", "dir", "connections", "files"
        ]);
        let active = self.call("aria2.tellActive", vec![keys.clone()])?;
        let waiting = self.call(
            "aria2.tellWaiting",
            vec![json!(0), json!(200), keys.clone()],
        )?;
        let stopped = self.call(
            "aria2.tellStopped",
            vec![json!(0), json!(500), keys],
        )?;

        let mut downloads = Vec::new();
        for item in active
            .as_array()
            .into_iter()
            .flatten()
            .chain(waiting.as_array().into_iter().flatten())
            .chain(stopped.as_array().into_iter().flatten())
        {
            downloads.push(map_download(item));
        }

        downloads.sort_by(|a, b| {
            status_order(&a.status)
                .cmp(&status_order(&b.status))
                .then_with(|| b.download_speed.cmp(&a.download_speed))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(downloads)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // Solicita encerramento gracioso para o aria2 gravar a sessão antes de sair.
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
            let mut exited = false;
            if let Some(child) = slot.as_mut() {
                for _ in 0..20 {
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
            if exited {
                *slot = None;
            }
        }
    }
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
    let full_path = value
        .get("files")
        .and_then(Value::as_array)
        .and_then(|files| files.first())
        .and_then(|file| file.get("path"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let name = Path::new(full_path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("Download sem nome")
        .to_string();

    DownloadInfo {
        gid: value_string(value, "gid"),
        name,
        status: value_string(value, "status"),
        total_length: number(value, "totalLength"),
        completed_length: number(value, "completedLength"),
        download_speed: number(value, "downloadSpeed"),
        upload_speed: number(value, "uploadSpeed"),
        connections: number(value, "connections"),
        error_message: value_string(value, "errorMessage"),
        directory: value_string(value, "dir"),
    }
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
        let version = engine
            .version
            .lock()
            .map_err(|_| "Não consegui ler a versão do aria2.".to_string())?
            .clone();
        return Ok(EngineInfo {
            download_dir: engine.download_dir.to_string_lossy().into_owned(),
            aria2_version: version,
        });
    }

    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Não consegui localizar a pasta de dados: {error}"))?;
    fs::create_dir_all(&app_data)
        .map_err(|error| format!("Não consegui criar a pasta de dados: {error}"))?;

    let download_dir = app.path().download_dir().unwrap_or_else(|_| {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Downloads")
    });
    fs::create_dir_all(&download_dir)
        .map_err(|error| format!("Não consegui preparar a pasta Downloads: {error}"))?;

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
    let child = Command::new("aria2c")
        .arg("--no-conf=true")
        .arg("--enable-rpc=true")
        .arg("--rpc-listen-all=false")
        .arg(format!("--rpc-listen-port={port}"))
        .arg(format!("--rpc-secret={secret}"))
        .arg("--max-concurrent-downloads=5")
        .arg("--max-connection-per-server=16")
        .arg("--split=16")
        .arg("--min-split-size=1M")
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
        .arg(format!("--dir={}", download_dir.display()))
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

    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|error| format!("Não consegui preparar o cliente RPC: {error}"))?;

    let engine = Arc::new(Engine {
        port,
        secret,
        client,
        child: Mutex::new(Some(child)),
        download_dir: download_dir.clone(),
        version: Mutex::new("detectando…".to_string()),
    });

    let mut detected_version = None;
    let mut last_error = String::new();
    for _ in 0..30 {
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
        thread::sleep(Duration::from_millis(150));
    }

    let version = detected_version.ok_or_else(|| {
        format!("O aria2 não respondeu ao iniciar. {last_error}. Veja o log em {}", log_file.display())
    })?;

    *engine
        .version
        .lock()
        .map_err(|_| "Não consegui atualizar a versão do aria2.".to_string())? = version.clone();

    *state
        .engine
        .lock()
        .map_err(|_| "Estado interno bloqueado.".to_string())? = Some(engine);

    Ok(EngineInfo {
        download_dir: download_dir.to_string_lossy().into_owned(),
        aria2_version: version,
    })
}

#[tauri::command]
fn get_downloads(state: State<'_, SharedState>) -> Result<Vec<DownloadInfo>, String> {
    current_engine(&state)?.download_list()
}

#[tauri::command]
fn get_stats(state: State<'_, SharedState>) -> Result<GlobalStats, String> {
    let engine = current_engine(&state)?;
    let result = engine.call("aria2.getGlobalStat", vec![])?;
    Ok(GlobalStats {
        download_speed: number(&result, "downloadSpeed"),
        upload_speed: number(&result, "uploadSpeed"),
        active: number(&result, "numActive"),
        waiting: number(&result, "numWaiting"),
        stopped: number(&result, "numStopped"),
    })
}

#[tauri::command]
fn add_download(url: String, state: State<'_, SharedState>) -> Result<String, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("Cola um link primeiro.".to_string());
    }
    let allowed = ["http://", "https://", "ftp://", "sftp://", "magnet:"];
    if !allowed.iter().any(|prefix| url.to_lowercase().starts_with(prefix)) {
        return Err("Link não reconhecido. Use HTTP, HTTPS, FTP, SFTP ou magnet link.".to_string());
    }

    let engine = current_engine(&state)?;
    let options = json!({
        "dir": engine.download_dir.to_string_lossy(),
        "continue": "true",
        "max-connection-per-server": "16",
        "split": "16",
        "min-split-size": "1M",
        "auto-file-renaming": "true"
    });
    let gid = engine.call(
        "aria2.addUri",
        vec![json!([url]), options],
    )?;
    gid.as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| "O aria2 não devolveu o identificador do download.".to_string())
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
fn remove_download(gid: String, status: String, state: State<'_, SharedState>) -> Result<(), String> {
    let engine = current_engine(&state)?;
    if ["complete", "error", "removed"].contains(&status.as_str()) {
        engine.call("aria2.removeDownloadResult", vec![json!(gid)])?;
    } else {
        engine.call("aria2.remove", vec![json!(gid)])?;
        let _ = engine.call("aria2.removeDownloadResult", vec![json!(gid)]);
    }
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
        vec![
            json!(0),
            json!(500),
            json!(["gid", "status"]),
        ],
    )?;
    let mut removed = 0_u64;
    if let Some(items) = stopped.as_array() {
        for item in items {
            let status = value_string(item, "status");
            if ["complete", "error", "removed"].contains(&status.as_str()) {
                let gid = value_string(item, "gid");
                if engine.call("aria2.removeDownloadResult", vec![json!(gid)]).is_ok() {
                    removed += 1;
                }
            }
        }
    }
    Ok(removed)
}

#[tauri::command]
fn open_download_folder(state: State<'_, SharedState>) -> Result<(), String> {
    let engine = current_engine(&state)?;
    Command::new("xdg-open")
        .arg(&engine.download_dir)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Não consegui abrir a pasta: {error}"))
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
            add_download,
            pause_download,
            resume_download,
            remove_download,
            pause_all,
            resume_all,
            clear_finished,
            open_download_folder
        ])
        .run(tauri::generate_context!())
        .expect("erro ao executar Pulso");
}
