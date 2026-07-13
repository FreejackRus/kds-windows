use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Utc};
use reqwest::blocking::Client;
use rodio::{Decoder, OutputStream, Sink, Source};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, File},
    io::BufReader,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuItem},
    path::BaseDirectory,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WindowEvent,
};

const SETTINGS_FILE: &str = "settings.json";
const HTTP_TIMEOUT_SECS: u64 = 15;
const AUDIO_START_TIMEOUT_SECS: u64 = 5;
const WATCHDOG_RESTART_DELAY_SECS: u64 = 1;
const ALARM_ENDPOINT: &str = "/rest/staff/alarm";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub server_url: String,
    pub poll_interval_secs: u64,
    pub cookie_header: String,
    pub basic_auth_user: String,
    pub basic_auth_password: String,
    pub sleep_guard_enabled: bool,
    pub start_monitor_on_launch: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server_url: "https://staff.tkgse.ru".to_string(),
            poll_interval_secs: 2,
            cookie_header: String::new(),
            basic_auth_user: String::new(),
            basic_auth_password: String::new(),
            sleep_guard_enabled: true,
            start_monitor_on_launch: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub running: bool,
    pub sleep_guard_active: bool,
    pub alarm_active: bool,
    pub alarm_mode: Option<String>,
    pub alarm_text: Option<String>,
    pub alarm_link: Option<String>,
    pub alarm_count: usize,
    pub sound_suppressed: bool,
    pub watchdog_restarts: u64,
    pub muted_until: Option<DateTime<Utc>>,
    pub last_check: Option<DateTime<Utc>>,
    pub last_success: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub at: DateTime<Utc>,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub settings: Settings,
    pub status: RuntimeStatus,
    pub logs: Vec<LogEntry>,
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<StateInner>,
}

struct StateInner {
    app: AppHandle,
    http: Client,
    data: Mutex<AppData>,
    monitor_stop: Mutex<Option<Arc<AtomicBool>>>,
    monitor_thread: Mutex<Option<JoinHandle<()>>>,
    audio: Mutex<AudioRuntime>,
    sounds: SoundPaths,
}

struct AppData {
    settings: Settings,
    status: RuntimeStatus,
    logs: Vec<LogEntry>,
    dismissed_alarm: Option<AlarmFingerprint>,
}

#[derive(Clone)]
struct SoundPaths {
    alarm: PathBuf,
    gentle: PathBuf,
    notif: PathBuf,
}

#[derive(Default)]
struct AudioRuntime {
    mode: Option<String>,
    stop: Option<Arc<AtomicBool>>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone)]
struct AlarmDecision {
    active: bool,
    mode: Option<String>,
    text: Option<String>,
    link: Option<String>,
    count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AlarmFingerprint {
    mode: String,
    text: Option<String>,
    link: Option<String>,
}

impl AlarmDecision {
    fn fingerprint(&self) -> Option<AlarmFingerprint> {
        self.mode.as_ref().map(|mode| AlarmFingerprint {
            mode: mode.clone(),
            text: self.text.clone(),
            link: self.link.clone(),
        })
    }
}

impl AppState {
    fn new(app: AppHandle, settings: Settings, sounds: SoundPaths, http: Client) -> Self {
        let status = RuntimeStatus {
            sleep_guard_active: false,
            ..RuntimeStatus::default()
        };
        Self {
            inner: Arc::new(StateInner {
                app,
                http,
                data: Mutex::new(AppData {
                    settings,
                    status,
                    logs: Vec::new(),
                    dismissed_alarm: None,
                }),
                monitor_stop: Mutex::new(None),
                monitor_thread: Mutex::new(None),
                audio: Mutex::new(AudioRuntime::default()),
                sounds,
            }),
        }
    }
}

#[tauri::command]
fn load_snapshot(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    Ok(snapshot(&state.inner))
}

#[tauri::command]
fn save_settings(settings: Settings, state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    let mut next = normalize_settings(settings);
    if next.poll_interval_secs == 0 {
        next.poll_interval_secs = 1;
    }

    {
        let mut data = state.inner.data.lock().map_err(|_| "state lock poisoned")?;
        data.settings = next.clone();
    }

    write_settings(&state.inner.app, &next)?;
    set_sleep_guard_inner(&state.inner, next.sleep_guard_enabled)?;
    append_log(&state.inner, "info", "Settings saved");
    emit_snapshot(&state.inner);
    Ok(snapshot(&state.inner))
}

#[tauri::command]
fn start_monitor(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    start_monitor_inner(state.inner.clone())?;
    Ok(snapshot(&state.inner))
}

#[tauri::command]
fn stop_monitor(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    stop_monitor_inner(&state.inner)?;
    Ok(snapshot(&state.inner))
}

#[tauri::command]
fn stop_alarm(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    stop_audio(&state.inner)?;
    {
        let mut data = state.inner.data.lock().map_err(|_| "state lock poisoned")?;
        data.dismissed_alarm = status_fingerprint(&data.status);
        data.status.sound_suppressed = data.dismissed_alarm.is_some();
    }
    append_log(
        &state.inner,
        "info",
        "Alarm sound stopped until the current alarm changes or clears",
    );
    emit_snapshot(&state.inner);
    Ok(snapshot(&state.inner))
}

#[tauri::command]
fn mute_for_minutes(minutes: u64, state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    let until = Utc::now() + chrono::Duration::minutes(minutes.max(1) as i64);
    stop_audio(&state.inner)?;
    {
        let mut data = state.inner.data.lock().map_err(|_| "state lock poisoned")?;
        data.status.muted_until = Some(until);
    }
    append_log(
        &state.inner,
        "info",
        &format!(
            "Alarm muted until {}",
            until.format("%Y-%m-%d %H:%M:%S UTC")
        ),
    );
    emit_snapshot(&state.inner);
    Ok(snapshot(&state.inner))
}

#[tauri::command]
fn test_alarm(mode: String, state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    let mode = if mode.eq_ignore_ascii_case("gentle") {
        "gentle".to_string()
    } else {
        "alarm".to_string()
    };
    start_audio(&state.inner, &mode)?;
    {
        let mut data = state.inner.data.lock().map_err(|_| "state lock poisoned")?;
        data.status.alarm_active = true;
        data.status.alarm_mode = Some(mode.clone());
        data.status.alarm_text = Some("Test alarm".to_string());
        data.status.alarm_link = None;
        data.status.alarm_count = 1;
        data.status.sound_suppressed = false;
        data.dismissed_alarm = None;
    }
    append_log(
        &state.inner,
        "info",
        &format!("Test {} sound started", mode),
    );
    emit_snapshot(&state.inner);
    Ok(snapshot(&state.inner))
}

#[tauri::command]
fn set_sleep_guard(enabled: bool, state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    {
        let mut data = state.inner.data.lock().map_err(|_| "state lock poisoned")?;
        data.settings.sleep_guard_enabled = enabled;
        write_settings(&state.inner.app, &data.settings)?;
    }
    set_sleep_guard_inner(&state.inner, enabled)?;
    emit_snapshot(&state.inner);
    Ok(snapshot(&state.inner))
}

fn start_monitor_inner(inner: Arc<StateInner>) -> Result<(), String> {
    {
        let mut stop_slot = inner
            .monitor_stop
            .lock()
            .map_err(|_| "monitor lock poisoned")?;
        if stop_slot.is_some() {
            append_log(&inner, "info", "Monitor is already running");
            return Ok(());
        }

        let stop = Arc::new(AtomicBool::new(false));
        *stop_slot = Some(stop.clone());

        let thread_inner = inner.clone();
        let handle = match thread::Builder::new()
            .name("kds-watchdog".to_string())
            .spawn(move || monitor_supervisor_loop(thread_inner, stop))
        {
            Ok(handle) => handle,
            Err(error) => {
                *stop_slot = None;
                return Err(error.to_string());
            }
        };

        let mut thread_slot = inner
            .monitor_thread
            .lock()
            .map_err(|_| "monitor thread lock poisoned")?;
        *thread_slot = Some(handle);
    }

    {
        let mut data = inner.data.lock().map_err(|_| "state lock poisoned")?;
        data.status.running = true;
    }

    let guard_enabled = snapshot(&inner).settings.sleep_guard_enabled;
    set_sleep_guard_inner(&inner, guard_enabled)?;
    append_log(&inner, "info", "Monitor started");
    emit_snapshot(&inner);
    Ok(())
}

fn stop_monitor_inner(inner: &Arc<StateInner>) -> Result<(), String> {
    if let Some(stop) = inner
        .monitor_stop
        .lock()
        .map_err(|_| "monitor lock poisoned")?
        .take()
    {
        stop.store(true, Ordering::SeqCst);
    }

    if let Some(handle) = inner
        .monitor_thread
        .lock()
        .map_err(|_| "monitor thread lock poisoned")?
        .take()
    {
        let _ = handle.join();
    }

    stop_audio(inner)?;
    set_sleep_guard_inner(inner, false)?;
    {
        let mut data = inner.data.lock().map_err(|_| "state lock poisoned")?;
        data.status.running = false;
        data.status.alarm_active = false;
        data.status.alarm_mode = None;
        data.status.alarm_text = None;
        data.status.alarm_link = None;
        data.status.alarm_count = 0;
        data.status.sound_suppressed = false;
        data.dismissed_alarm = None;
    }
    append_log(inner, "info", "Monitor stopped");
    emit_snapshot(inner);
    Ok(())
}

fn monitor_supervisor_loop(inner: Arc<StateInner>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        let worker_inner = inner.clone();
        let worker_stop = stop.clone();
        let worker = thread::Builder::new()
            .name("kds-monitor".to_string())
            .spawn(move || monitor_loop(worker_inner, worker_stop));

        match worker {
            Ok(handle) => {
                if handle.join().is_ok() || stop.load(Ordering::SeqCst) {
                    break;
                }
                record_watchdog_restart(&inner, "Monitor worker panicked");
            }
            Err(error) => {
                record_watchdog_restart(
                    &inner,
                    &format!("Monitor worker could not start: {error}"),
                );
            }
        }

        for _ in 0..WATCHDOG_RESTART_DELAY_SECS {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            thread::sleep(Duration::from_secs(1));
        }
    }
}

fn record_watchdog_restart(inner: &Arc<StateInner>, reason: &str) {
    if let Ok(mut data) = inner.data.lock() {
        data.status.watchdog_restarts = data.status.watchdog_restarts.saturating_add(1);
        data.status.last_error = Some(reason.to_string());
    }
    append_log(
        inner,
        "error",
        &format!("{reason}; watchdog restarting monitor"),
    );
    emit_snapshot(inner);
}

fn monitor_loop(inner: Arc<StateInner>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        poll_once(&inner);

        let interval = snapshot(&inner).settings.poll_interval_secs.max(1);
        for _ in 0..interval {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            thread::sleep(Duration::from_secs(1));
        }
    }
}

fn poll_once(inner: &Arc<StateInner>) {
    let settings = snapshot(inner).settings;
    let now = Utc::now();

    {
        if let Ok(mut data) = inner.data.lock() {
            data.status.last_check = Some(now);
        }
    }

    let result = fetch_alarm_json(&inner.http, &settings).and_then(|json| {
        let decision = parse_alarm_decision(&json);
        apply_alarm_decision(inner, decision)?;
        Ok(())
    });

    match result {
        Ok(()) => {
            if let Ok(mut data) = inner.data.lock() {
                data.status.last_success = Some(Utc::now());
                data.status.last_error = None;
            }
        }
        Err(e) => {
            stop_audio(inner).ok();
            if let Ok(mut data) = inner.data.lock() {
                clear_alarm_status(&mut data.status);
                data.status.last_error = Some(e.clone());
            }
            append_log(inner, "error", &e);
        }
    }

    emit_snapshot(inner);
}

fn fetch_alarm_json(client: &Client, settings: &Settings) -> Result<Value, String> {
    let base = normalize_base_url(&settings.server_url)?;
    let url = format!("{}{}", base, ALARM_ENDPOINT);
    let mut req = client
        .get(&url)
        .header("Accept", "application/json")
        .header("User-Agent", "KDS-Guard/0.1 Windows");

    if !settings.cookie_header.trim().is_empty() {
        req = req.header("Cookie", settings.cookie_header.trim());
    }

    if !settings.basic_auth_user.trim().is_empty() || !settings.basic_auth_password.is_empty() {
        let raw = format!(
            "{}:{}",
            settings.basic_auth_user.trim(),
            settings.basic_auth_password
        );
        let encoded = general_purpose::STANDARD.encode(raw.as_bytes());
        req = req.header("Authorization", format!("Basic {}", encoded));
    }

    let response = req
        .send()
        .map_err(|e| format!("Poll request failed: {}", e))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|e| format!("Poll response read failed: {}", e))?;

    if !status.is_success() {
        return Err(format!("Poll HTTP {} from {}", status.as_u16(), url));
    }

    serde_json::from_str(&body).map_err(|e| format!("Alarm JSON parse failed: {}", e))
}

fn parse_alarm_decision(json: &Value) -> AlarmDecision {
    let alarms = json.get("alarms").and_then(Value::as_array);
    let count = alarms.map_or(0, Vec::len);
    let fallback_text = json
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("KDS")
        .to_string();

    let mut gentle: Option<(String, Option<String>)> = None;

    if let Some(items) = alarms {
        for item in items {
            let severity = item
                .get("severity")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let text = item
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or(&fallback_text)
                .to_string();
            let link = item
                .get("meta")
                .and_then(|meta| meta.get("link"))
                .and_then(Value::as_str)
                .map(ToString::to_string);

            if severity.eq_ignore_ascii_case("alarm") {
                return AlarmDecision {
                    active: true,
                    mode: Some("alarm".to_string()),
                    text: Some(text),
                    link,
                    count,
                };
            }

            if severity.eq_ignore_ascii_case("gentle") && gentle.is_none() {
                gentle = Some((text, link));
            }
        }
    }

    if let Some((text, link)) = gentle {
        return AlarmDecision {
            active: true,
            mode: Some("gentle".to_string()),
            text: Some(text),
            link,
            count,
        };
    }

    AlarmDecision {
        active: false,
        mode: None,
        text: None,
        link: None,
        count,
    }
}

fn apply_alarm_decision(inner: &Arc<StateInner>, decision: AlarmDecision) -> Result<(), String> {
    let suppress_sound = {
        let mut data = inner.data.lock().map_err(|_| "state lock poisoned")?;
        update_alarm_status(&mut data, &decision, Utc::now())
    };

    if let Some(mode) = decision.mode {
        if suppress_sound {
            stop_audio(inner)?;
        } else {
            start_audio(inner, &mode)?;
        }
    } else {
        stop_audio(inner)?;
    }

    Ok(())
}

fn update_alarm_status(data: &mut AppData, decision: &AlarmDecision, now: DateTime<Utc>) -> bool {
    if data
        .status
        .muted_until
        .map(|until| until <= now)
        .unwrap_or(false)
    {
        data.status.muted_until = None;
    }

    let fingerprint = decision.fingerprint();
    if !decision.active || data.dismissed_alarm.as_ref() != fingerprint.as_ref() {
        data.dismissed_alarm = None;
    }

    let manually_suppressed = fingerprint
        .as_ref()
        .zip(data.dismissed_alarm.as_ref())
        .map(|(current, dismissed)| current == dismissed)
        .unwrap_or(false);
    let muted = data
        .status
        .muted_until
        .map(|until| until > now)
        .unwrap_or(false);

    data.status.alarm_active = decision.active;
    data.status.alarm_mode = decision.mode.clone();
    data.status.alarm_text = decision.text.clone();
    data.status.alarm_link = decision.link.clone();
    data.status.alarm_count = decision.count;
    data.status.sound_suppressed = manually_suppressed;

    manually_suppressed || muted
}

fn status_fingerprint(status: &RuntimeStatus) -> Option<AlarmFingerprint> {
    if !status.alarm_active {
        return None;
    }
    status.alarm_mode.as_ref().map(|mode| AlarmFingerprint {
        mode: mode.clone(),
        text: status.alarm_text.clone(),
        link: status.alarm_link.clone(),
    })
}

fn clear_alarm_status(status: &mut RuntimeStatus) {
    status.alarm_active = false;
    status.alarm_mode = None;
    status.alarm_text = None;
    status.alarm_link = None;
    status.alarm_count = 0;
    status.sound_suppressed = false;
}

fn start_audio(inner: &Arc<StateInner>, mode: &str) -> Result<(), String> {
    let sound = match mode {
        "gentle" => inner.sounds.gentle.clone(),
        "notif" => inner.sounds.notif.clone(),
        _ => inner.sounds.alarm.clone(),
    };

    if !sound.exists() {
        return Err(format!("Sound file not found: {}", sound.display()));
    }

    let mut audio = inner.audio.lock().map_err(|_| "audio lock poisoned")?;
    if audio.mode.as_deref() == Some(mode) {
        return Ok(());
    }

    stop_audio_runtime(&mut audio);

    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let mode_name = mode.to_string();
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let handle = thread::Builder::new()
        .name(format!("kds-audio-{}", mode))
        .spawn(move || {
            let startup = (|| -> Result<(OutputStream, Sink), String> {
                let (stream, stream_handle) = OutputStream::try_default()
                    .map_err(|e| format!("Cannot open audio output: {e}"))?;
                let file = File::open(&sound)
                    .map_err(|e| format!("Cannot open sound {}: {e}", sound.display()))?;
                let source = Decoder::new(BufReader::new(file))
                    .map_err(|e| format!("Cannot decode sound {}: {e}", sound.display()))?;
                let sink = Sink::try_new(&stream_handle)
                    .map_err(|e| format!("Cannot create audio sink: {e}"))?;

                sink.append(source.repeat_infinite());
                sink.play();
                Ok((stream, sink))
            })();

            let (stream, sink) = match startup {
                Ok(runtime) => {
                    let _ = ready_tx.send(Ok(()));
                    runtime
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                    return;
                }
            };

            while !stop_thread.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(200));
            }
            sink.stop();
            drop(stream);
        })
        .map_err(|e| e.to_string())?;

    match ready_rx.recv_timeout(Duration::from_secs(AUDIO_START_TIMEOUT_SECS)) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            let _ = handle.join();
            return Err(error);
        }
        Err(error) => {
            stop.store(true, Ordering::SeqCst);
            let _ = handle.join();
            return Err(format!("Audio startup did not complete: {error}"));
        }
    }

    audio.mode = Some(mode_name);
    audio.stop = Some(stop);
    audio.thread = Some(handle);
    Ok(())
}

fn stop_audio(inner: &Arc<StateInner>) -> Result<(), String> {
    let mut audio = inner.audio.lock().map_err(|_| "audio lock poisoned")?;
    stop_audio_runtime(&mut audio);
    Ok(())
}

fn stop_audio_runtime(audio: &mut AudioRuntime) {
    if let Some(stop) = audio.stop.take() {
        stop.store(true, Ordering::SeqCst);
    }
    if let Some(handle) = audio.thread.take() {
        let _ = handle.join();
    }
    audio.mode = None;
}

fn set_sleep_guard_inner(inner: &Arc<StateInner>, enabled: bool) -> Result<(), String> {
    set_sleep_guard_os(enabled)?;
    {
        let mut data = inner.data.lock().map_err(|_| "state lock poisoned")?;
        data.status.sleep_guard_active = enabled;
    }
    append_log(
        inner,
        "info",
        if enabled {
            "Windows sleep guard enabled"
        } else {
            "Windows sleep guard disabled"
        },
    );
    Ok(())
}

#[cfg(windows)]
fn set_sleep_guard_os(enabled: bool) -> Result<(), String> {
    use windows::Win32::System::Power::{
        SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED, EXECUTION_STATE,
    };

    let flags = if enabled {
        EXECUTION_STATE(ES_CONTINUOUS.0 | ES_SYSTEM_REQUIRED.0)
    } else {
        ES_CONTINUOUS
    };

    let previous = unsafe { SetThreadExecutionState(flags) };
    if previous.0 == 0 {
        Err("SetThreadExecutionState failed".to_string())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn set_sleep_guard_os(_enabled: bool) -> Result<(), String> {
    Ok(())
}

fn snapshot(inner: &Arc<StateInner>) -> AppSnapshot {
    let data = inner.data.lock().expect("state lock poisoned");
    let logs = data
        .logs
        .iter()
        .rev()
        .take(200)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    AppSnapshot {
        settings: data.settings.clone(),
        status: data.status.clone(),
        logs,
    }
}

fn emit_snapshot(inner: &Arc<StateInner>) {
    let _ = inner.app.emit("kds-state", snapshot(inner));
}

fn append_log(inner: &Arc<StateInner>, level: &str, message: &str) {
    if let Ok(mut data) = inner.data.lock() {
        data.logs.push(LogEntry {
            at: Utc::now(),
            level: level.to_string(),
            message: message.to_string(),
        });
        if data.logs.len() > 500 {
            let overflow = data.logs.len() - 500;
            data.logs.drain(0..overflow);
        }
    }
}

fn normalize_settings(mut settings: Settings) -> Settings {
    settings.server_url = settings.server_url.trim().trim_end_matches('/').to_string();
    if settings.poll_interval_secs == 0 {
        settings.poll_interval_secs = 1;
    }
    settings.basic_auth_user = settings.basic_auth_user.trim().to_string();
    settings.cookie_header = settings.cookie_header.trim().to_string();
    settings
}

fn normalize_base_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("Server URL is empty".to_string());
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Ok(trimmed.to_string())
    } else {
        Ok(format!("https://{}", trimmed))
    }
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("Cannot resolve config dir: {}", e))?;
    fs::create_dir_all(&dir).map_err(|e| format!("Cannot create config dir: {}", e))?;
    Ok(dir.join(SETTINGS_FILE))
}

fn load_settings(app: &AppHandle) -> Result<Settings, String> {
    let path = settings_path(app)?;
    if !path.exists() {
        return Ok(Settings::default());
    }
    let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let settings = serde_json::from_str::<Settings>(&text).map_err(|e| e.to_string())?;
    Ok(normalize_settings(settings))
}

fn write_settings(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = settings_path(app)?;
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| e.to_string())
}

fn resolve_sound_paths(app: &AppHandle) -> SoundPaths {
    SoundPaths {
        alarm: resolve_resource(app, "resources/sounds/alarm.ogg"),
        gentle: resolve_resource(app, "resources/sounds/gentle.ogg"),
        notif: resolve_resource(app, "resources/sounds/notif.ogg"),
    }
}

fn resolve_resource(app: &AppHandle, rel: &str) -> PathBuf {
    app.path()
        .resolve(rel, BaseDirectory::Resource)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel))
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "tray-open", "Open KDS Guard", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray-quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;

    let mut builder = TrayIconBuilder::with_id("kds-guard")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("KDS Guard")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "tray-open" => show_main_window(app),
            "tray-quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let app_handle = app.handle().clone();
            let settings = load_settings(&app_handle).unwrap_or_default();
            let sounds = resolve_sound_paths(&app_handle);
            let http = Client::builder()
                .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
                .build()?;
            let state = AppState::new(app_handle, settings.clone(), sounds, http);
            app.manage(state.clone());
            setup_tray(app.handle())?;

            if settings.sleep_guard_enabled {
                let _ = set_sleep_guard_inner(&state.inner, true);
            }
            if settings.start_monitor_on_launch {
                let _ = start_monitor_inner(state.inner.clone());
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            load_snapshot,
            save_settings,
            start_monitor,
            stop_monitor,
            stop_alarm,
            mute_for_minutes,
            test_alarm,
            set_sleep_guard
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app_data() -> AppData {
        AppData {
            settings: Settings::default(),
            status: RuntimeStatus::default(),
            logs: Vec::new(),
            dismissed_alarm: None,
        }
    }

    fn alarm(text: &str) -> AlarmDecision {
        AlarmDecision {
            active: true,
            mode: Some("alarm".to_string()),
            text: Some(text.to_string()),
            link: Some("https://example.test/alarm".to_string()),
            count: 1,
        }
    }

    #[test]
    fn alarm_severity_has_priority_over_gentle() {
        let decision = parse_alarm_decision(&json!({
            "alarms": [
                { "severity": "gentle", "reason": "Heads up" },
                {
                    "severity": "alarm",
                    "reason": "Act now",
                    "meta": { "link": "https://example.test/urgent" }
                }
            ]
        }));

        assert!(decision.active);
        assert_eq!(decision.mode.as_deref(), Some("alarm"));
        assert_eq!(decision.text.as_deref(), Some("Act now"));
        assert_eq!(
            decision.link.as_deref(),
            Some("https://example.test/urgent")
        );
        assert_eq!(decision.count, 2);
    }

    #[test]
    fn gentle_and_empty_responses_are_parsed() {
        let gentle = parse_alarm_decision(&json!({
            "reason": "Fallback",
            "alarms": [{ "severity": "gentle" }]
        }));
        assert_eq!(gentle.mode.as_deref(), Some("gentle"));
        assert_eq!(gentle.text.as_deref(), Some("Fallback"));

        let empty = parse_alarm_decision(&json!({ "alarms": [] }));
        assert!(!empty.active);
        assert!(empty.mode.is_none());
        assert_eq!(empty.count, 0);
    }

    #[test]
    fn manually_stopped_alarm_stays_silent_until_it_changes() {
        let mut data = app_data();
        let first = alarm("First");

        assert!(!update_alarm_status(&mut data, &first, Utc::now()));
        data.dismissed_alarm = first.fingerprint();

        assert!(update_alarm_status(&mut data, &first, Utc::now()));
        assert!(data.status.alarm_active);
        assert!(data.status.sound_suppressed);

        let changed = alarm("Changed");
        assert!(!update_alarm_status(&mut data, &changed, Utc::now()));
        assert!(!data.status.sound_suppressed);
        assert!(data.dismissed_alarm.is_none());
    }

    #[test]
    fn clearing_alarm_resets_visible_state() {
        let mut data = app_data();
        update_alarm_status(&mut data, &alarm("First"), Utc::now());

        clear_alarm_status(&mut data.status);

        assert!(!data.status.alarm_active);
        assert!(data.status.alarm_mode.is_none());
        assert!(data.status.alarm_text.is_none());
        assert!(data.status.alarm_link.is_none());
        assert_eq!(data.status.alarm_count, 0);
        assert!(!data.status.sound_suppressed);
    }

    #[test]
    fn active_mute_suppresses_audio_and_expired_mute_does_not() {
        let now = Utc::now();
        let mut data = app_data();
        data.status.muted_until = Some(now + chrono::Duration::minutes(1));
        assert!(update_alarm_status(&mut data, &alarm("First"), now));

        data.status.muted_until = Some(now - chrono::Duration::seconds(1));
        assert!(!update_alarm_status(&mut data, &alarm("First"), now));
        assert!(data.status.muted_until.is_none());
    }
}
