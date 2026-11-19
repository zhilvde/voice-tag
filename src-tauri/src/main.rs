#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod recorder;
mod speech_api;
mod store;
mod transcription;
mod wave;

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};

use cpal::traits::HostTrait;
use recorder::{Recorder, RecordingStatus};
use store::{ApiProfile, ApiProfiles, Preferences, Project, Segment, Speaker, Store};
use symphonia::core::{
    formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};
use tauri::{Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow, WindowEvent};
use uuid::Uuid;
use winreg::{enums::HKEY_CURRENT_USER, RegKey};

type Result<T> = std::result::Result<T, String>;

struct AppState {
    store: Store,
    recording: Mutex<Option<Recorder>>,
    _instance_lock: File,
}

#[tauri::command]
fn list_projects(state: State<AppState>) -> Result<Vec<Project>> {
    state.store.list()
}

#[tauri::command]
fn get_project(state: State<AppState>, id: String) -> Result<Project> {
    state.store.get(&id)
}

#[tauri::command]
fn preferences(state: State<AppState>) -> Result<Preferences> {
    state.store.preferences()
}

#[tauri::command]
fn save_preferences(state: State<AppState>, preferences: Preferences) -> Result<()> {
    if !["auto", "zh", "en"].contains(&preferences.language.as_str()) {
        return Err("请选择自动识别、中文或英文作为默认识别语言。".into());
    }
    if !["windows", "api", "local"].contains(&preferences.method.as_str()) {
        return Err("请选择转写方式。".into());
    }
    if preferences
        .replacement_rules
        .iter()
        .any(|rule| rule.find.is_empty())
    {
        return Err("全局替换规则的查找文字不能为空。".into());
    }
    let directory = Path::new(preferences.audio_directory.trim());
    if !directory.is_absolute() {
        return Err("请选择录音保存目录的完整路径。".into());
    }
    fs::create_dir_all(directory).map_err(|e| format!("无法使用录音保存目录：{e}"))?;
    state.store.save_preferences(&preferences)
}

#[tauri::command]
fn api_profiles(state: State<AppState>) -> Result<ApiProfiles> {
    state.store.api_profiles()
}

#[tauri::command]
fn save_api_profile(
    state: State<AppState>,
    profile: ApiProfile,
    api_key: Option<String>,
    clear_key: bool,
) -> Result<()> {
    let key = match api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        Some(key) => Some(speech_api::protect_key(key)?),
        None if clear_key => Some(Vec::new()),
        None => None,
    };
    state.store.save_api_profile(&profile, key.as_deref())
}

#[tauri::command]
fn use_api_profile(state: State<AppState>, id: String) -> Result<()> {
    state.store.use_api_profile(&id)
}

#[tauri::command]
fn delete_api_profile(state: State<AppState>, id: String) -> Result<()> {
    state.store.delete_api_profile(&id)
}

#[tauri::command]
fn open_uninstaller(app: tauri::AppHandle) -> Result<()> {
    let state = app.state::<AppState>();
    if state.recording.lock().unwrap().is_some() {
        return Err("请先结束录音并保存，再卸载声签。".into());
    }
    if state.store.has_transcription_in_progress()? {
        return Err("请等待正在进行的转写结束，再卸载声签。".into());
    }
    let current = std::env::current_exe().map_err(|error| format!("无法定位当前程序：{error}"))?;
    let uninstaller = current.with_file_name("uninstall.exe");
    let entry = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\声签");
    let registered: Option<String> = entry
        .ok()
        .and_then(|entry| entry.get_value("UninstallString").ok());
    if !matches_uninstaller(registered.as_deref(), &uninstaller) || !uninstaller.is_file() {
        return Err("未找到当前程序对应的 Windows 卸载向导。请从已安装的声签中打开此入口。".into());
    }
    Command::new(&uninstaller)
        .spawn()
        .map_err(|error| format!("无法打开声签卸载向导：{error}"))?;
    app.exit(0);
    Ok(())
}

fn matches_uninstaller(registered: Option<&str>, expected: &Path) -> bool {
    registered
        .and_then(|path| path.strip_prefix('"')?.strip_suffix('"'))
        .is_some_and(|path| expected.as_os_str().eq_ignore_ascii_case(path))
}

#[cfg(test)]
#[test]
fn only_accepts_current_registered_uninstaller() {
    let expected = Path::new(r"D:\应用目录\声签\uninstall.exe");
    assert!(matches_uninstaller(
        Some(r#""d:\应用目录\声签\uninstall.exe""#),
        expected
    ));
    assert!(!matches_uninstaller(
        Some(r#""D:\其他目录\声签\uninstall.exe""#),
        expected
    ));
    assert!(!matches_uninstaller(
        Some(r#""D:\应用目录\声签\uninstall.exe" /S"#),
        expected
    ));
    assert!(!matches_uninstaller(None, expected));
}

#[tauri::command]
fn start_recording(state: State<AppState>) -> Result<()> {
    let mut active = state.recording.lock().unwrap();
    if active.is_some() {
        return Err("已有录音正在进行。".into());
    }
    let device = cpal::default_host()
        .default_input_device()
        .ok_or("找不到 Windows 默认麦克风，请检查连接与麦克风权限。")?;

    let path = new_audio_path(&state.store, "wav")?;
    let project = state
        .store
        .create("新录音", &path, 0.0, "microphone", "recording")?;
    match Recorder::start(project.id.clone(), device, path.clone()) {
        Ok(recorder) => {
            *active = Some(recorder);
            Ok(())
        }
        Err(error) => {
            state.store.delete(&project.id)?;
            if path.exists() {
                let _ = fs::remove_file(path);
            }
            Err(error)
        }
    }
}

#[tauri::command]
fn recording_status(state: State<AppState>) -> Option<RecordingStatus> {
    state
        .recording
        .lock()
        .unwrap()
        .as_ref()
        .map(Recorder::status)
}

#[tauri::command]
fn pause_recording(state: State<AppState>, paused: bool) -> Result<()> {
    let active = state.recording.lock().unwrap();
    let recorder = active.as_ref().ok_or("当前没有正在进行的录音。")?;
    recorder.pause(paused);
    Ok(())
}

fn finish_recording(state: &AppState) -> Result<Project> {
    let mut active = state.recording.lock().unwrap();
    let recorder = active.take().ok_or("当前没有正在进行的录音。")?;
    let id = recorder.id.clone();
    let status = recorder.finish()?;
    state
        .store
        .end_recording(&id, status.duration, status.error.as_deref())?;
    state.store.get(&id)
}

#[tauri::command]
async fn stop_recording(app: tauri::AppHandle) -> Result<Project> {
    tauri::async_runtime::spawn_blocking(move || finish_recording(&app.state::<AppState>()))
        .await
        .map_err(|e| e.to_string())?
}

fn new_audio_path(store: &Store, extension: &str) -> Result<PathBuf> {
    let directory = store.preferences()?.audio_directory;
    fs::create_dir_all(&directory).map_err(|e| format!("无法创建录音保存目录：{e}"))?;
    let directory = PathBuf::from(directory)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    Ok(directory.join(format!("声签-{}.{extension}", Uuid::new_v4())))
}

#[tauri::command]
async fn import_audio(app: tauri::AppHandle, path: PathBuf) -> Result<Project> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !["mp3", "wav", "m4a", "aac", "flac", "ogg"].contains(&extension.as_str()) {
            return Err("请选择 MP3、WAV、M4A、AAC、FLAC 或 OGG 音频。".into());
        }
        let file = File::open(&path).map_err(|e| format!("无法打开音频：{e}"))?;
        let mut hint = Hint::new();
        hint.with_extension(&extension);
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                MediaSourceStream::new(Box::new(file), Default::default()),
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|e| format!("无法读取音频格式：{e}"))?;
        let track = probed.format.default_track().ok_or("找不到可用的音轨。")?;
        let params = &track.codec_params;
        let duration = match (params.n_frames, params.time_base, params.sample_rate) {
            (Some(frames), Some(base), _) => {
                let time = base.calc_time(frames);
                time.seconds as f64 + time.frac
            }
            (Some(frames), None, Some(rate)) => frames as f64 / f64::from(rate),
            _ => 0.0,
        };
        let name = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("导入音频");
        let target = new_audio_path(&state.store, &extension)?;
        let result = (|| {
            fs::copy(&path, &target).map_err(|e| format!("无法保存导入的音频：{e}"))?;
            state
                .store
                .create(name, &target, duration, "import", "saved")
        })();
        if result.is_err() {
            let _ = fs::remove_file(target);
        }
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn audio_path(app: tauri::AppHandle, id: String) -> Result<String> {
    let project = app.state::<AppState>().store.get(&id)?;
    let path = PathBuf::from(project.audio_path);
    if !path.is_file() {
        return Err("找不到原始音频，文件可能已被移动。".into());
    }
    app.asset_protocol_scope()
        .allow_file(&path)
        .map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
fn update_duration(state: State<AppState>, id: String, duration: f64) -> Result<()> {
    if duration.is_finite() && duration > 0.0 {
        state.store.update_duration(&id, duration)?;
    }
    Ok(())
}

#[tauri::command]
fn rename_project(state: State<AppState>, id: String, name: String) -> Result<()> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err("请输入 1 至 120 个字符的录音名称。".into());
    }
    state.store.rename(&id, name)
}

#[tauri::command]
fn delete_project(state: State<AppState>, id: String, delete_audio: bool) -> Result<()> {
    state.store.delete_project(&id, delete_audio)
}

#[tauri::command]
fn save_transcript(
    state: State<AppState>,
    id: String,
    speakers: Vec<Speaker>,
    segments: Vec<Segment>,
) -> Result<()> {
    state.store.save_transcript(&id, &speakers, &segments)
}

#[tauri::command]
fn start_manual_transcript(state: State<AppState>, id: String) -> Result<Project> {
    state.store.begin_manual_transcript(&id)
}

#[tauri::command]
fn start_transcription(app: tauri::AppHandle, id: String, language: String) -> Result<()> {
    if !["auto", "zh", "en"].contains(&language.as_str()) {
        return Err("请选择自动识别、中文或英文。".into());
    }
    let state = app.state::<AppState>();
    let (preferences, protected_key) = state.store.transcription_preferences()?;
    let api_key = if preferences.method == "api" {
        Some(speech_api::read_key(&protected_key)?)
    } else {
        None
    };
    let project = state.store.begin_transcription(&id, &language)?;
    tauri::async_runtime::spawn_blocking(move || {
        let result =
            transcription::run(&app, &project, &preferences, &language, api_key.as_deref());
        let store = &app.state::<AppState>().store;
        if let Err(error) = result {
            let _ = store.fail_transcription(&project.id, &error);
        }
    });
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportOptions {
    path: PathBuf,
    format: String,
    timestamps: bool,
    speaker_labels: bool,
    merge: bool,
    selection: Option<Vec<String>>,
}

#[tauri::command]
async fn export_transcript(
    app: tauri::AppHandle,
    id: String,
    options: ExportOptions,
) -> Result<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let project = state.store.get(&id)?;
        if project.transcription_status != "complete" {
            return Err("完成转写后才能导出。".into());
        }
        let markdown = match options.format.as_str() {
            "txt" => false,
            "md" => true,
            _ => return Err("请选择 TXT 或 Markdown 格式。".into()),
        };
        if options
            .path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            != options.format
        {
            return Err("文件扩展名与导出格式不一致。".into());
        }
        let mut text = String::new();
        if markdown {
            text.push_str("# ");
        }
        text.push_str(&project.name);
        text.push('\n');
        let mut lines: Vec<(f64, &str, String)> = Vec::new();
        for segment in &project.segments {
            if segment.text.trim().is_empty() {
                continue;
            }
            if options
                .selection
                .as_ref()
                .is_some_and(|ids| !ids.contains(&segment.id))
            {
                continue;
            }
            if options.merge {
                if let Some((_, speaker, content)) = lines.last_mut() {
                    if *speaker == segment.speaker_id {
                        content.push(' ');
                        content.push_str(&segment.text);
                        continue;
                    }
                }
            }
            lines.push((segment.start, &segment.speaker_id, segment.text.clone()));
        }
        if lines.is_empty() {
            return Err("请选择至少一个有文字的片段。".into());
        }
        for (start, speaker, line) in lines {
            text.push('\n');
            if options.timestamps {
                let start = start as u64;
                if start >= 3600 {
                    text.push_str(&format!(
                        "[{:02}:{:02}:{:02}] ",
                        start / 3600,
                        start / 60 % 60,
                        start % 60
                    ));
                } else {
                    text.push_str(&format!("[{:02}:{:02}] ", start / 60, start % 60));
                }
            }
            if options.speaker_labels {
                if let Some(speaker) = project.speakers.iter().find(|entry| entry.id == speaker) {
                    if markdown {
                        text.push_str(&format!("**{}**：", speaker.name));
                    } else {
                        text.push_str(&format!("{}：", speaker.name));
                    }
                }
            }
            text.push_str(&line);
            text.push('\n');
        }
        let path = &options.path;
        let temp = path.with_file_name(format!(".voicetag-{}.tmp", Uuid::new_v4()));
        let write = (|| -> std::io::Result<()> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temp, path)
        })();
        if let Err(error) = write {
            let _ = fs::remove_file(temp);
            let message = format!("导出失败：{error}");
            let _ = state
                .store
                .log_export(&id, &options.format, path, "failed", Some(&message));
            return Err(message);
        }
        state
            .store
            .log_export(&id, &options.format, path, "complete", None)?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn fit_startup_window(window: &WebviewWindow) -> tauri::Result<()> {
    let Some(monitor) = window.current_monitor()?.or(window.primary_monitor()?) else {
        return Ok(());
    };
    let area = monitor.work_area();
    let requested = window.inner_size()?;
    let width = requested.width.min(area.size.width);
    let height = requested.height.min(area.size.height);
    if width < requested.width || height < requested.height {
        window.set_size(PhysicalSize::new(width, height))?;
    }
    window.set_position(PhysicalPosition::new(
        area.position.x + ((area.size.width - width) / 2) as i32,
        area.position.y + ((area.size.height - height) / 2) as i32,
    ))?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let audio = app.path().document_dir()?.join("声签");
            fs::create_dir_all(&data)?;
            let instance_lock = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(data.join("instance.lock"))?;
            instance_lock.try_lock().map_err(std::io::Error::other)?;
            let store = Store::open(&data, &audio).map_err(std::io::Error::other)?;
            app.manage(AppState {
                store,
                recording: Mutex::new(None),
                _instance_lock: instance_lock,
            });
            if let Some(window) = app.get_webview_window("main") {
                let _ = fit_startup_window(&window);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { .. } = event {
                let state = window.state::<AppState>();
                let _ = finish_recording(&state);
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_projects,
            get_project,
            preferences,
            save_preferences,
            api_profiles,
            save_api_profile,
            use_api_profile,
            delete_api_profile,
            open_uninstaller,
            start_recording,
            recording_status,
            pause_recording,
            stop_recording,
            import_audio,
            audio_path,
            update_duration,
            rename_project,
            delete_project,
            save_transcript,
            start_manual_transcript,
            start_transcription,
            export_transcript,
        ])
        .run(tauri::generate_context!())
        .expect("无法启动声签");
}
