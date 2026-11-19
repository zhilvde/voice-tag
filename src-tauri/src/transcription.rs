use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use tauri::{path::BaseDirectory, AppHandle, Manager};
use uuid::Uuid;

use crate::{
    speech_api::{self, Route, Transcript},
    store::{Preferences, Project, Segment, Speaker},
    wave::SpeechWave,
    AppState, Result, Store,
};

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Event {
    Stage {
        message: String,
    },
    Error {
        message: String,
    },
    Result {
        speakers: Vec<Speaker>,
        segments: Vec<Segment>,
    },
}

pub fn run(
    app: &AppHandle,
    project: &Project,
    prefs: &Preferences,
    language: &str,
    api_key: Option<&str>,
) -> Result<()> {
    if !Path::new(&project.audio_path).is_file() {
        return Err("找不到原始音频，请检查文件是否已移动。".into());
    }
    let store = &app.state::<AppState>().store;
    let mut polish = None;
    let (speakers, mut segments) = match prefs.method.as_str() {
        "api" => {
            let key = api_key.ok_or("缺少 API Key。")?;
            if speech_api::route(&prefs.api_model)? == Route::TextPolish {
                let transcript = local_model(app, store, project, language)?;
                store.set_transcription_stage(&project.id, "正在发送转写文字，等待模型校对")?;
                polish = Some(speech_api::polish(&transcript.1, prefs, key));
                transcript
            } else {
                store.set_transcription_stage(&project.id, "正在发送音频并等待服务转写")?;
                speech_api::transcribe(project, prefs, language, key, |stage, progress| {
                    store.set_transcription_progress(&project.id, stage, progress)
                })?
            }
        }
        "windows" => windows_speech(store, project, language)?,
        "local" => local_model(app, store, project, language)?,
        _ => return Err("请在设置中选择转写方式。".into()),
    };
    for segment in &mut segments {
        segment.id = Uuid::new_v4().to_string();
    }
    store.finish_transcription(
        &project.id,
        &speakers,
        &segments,
        &prefs.replacement_rules,
        polish,
    )
}

pub fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}

fn local_model(
    app: &AppHandle,
    store: &Store,
    project: &Project,
    language: &str,
) -> Result<Transcript> {
    store.set_transcription_stage(&project.id, "正在准备离线识别")?;
    let wav = SpeechWave::prepare(Path::new(&project.audio_path))?;
    let resource = app
        .path()
        .resolve("offline", BaseDirectory::Resource)
        .map_err(|e| e.to_string())?;
    let worker = resource.join("voicetag-offline.exe");
    let model = resource.join("ggml-base-q5_1.bin");
    if !worker.is_file() || !model.is_file() {
        return Err("声签的离线模型文件缺失，请重新安装声签。".into());
    }
    const STAGE: &str = "正在本机识别，音频留在此设备";
    store.set_transcription_progress(&project.id, STAGE, Some(0))?;
    let mut command = Command::new(worker);
    hide_console(&mut command);
    let mut process = command
        .arg(model)
        .arg(wav.path())
        .arg(language)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("无法启动内置离线识别：{e}"))?;
    let mut segments = Vec::new();
    let mut last_progress = 0;
    for line in BufReader::new(process.stdout.take().unwrap()).split(b'\n') {
        let bytes = match line {
            Ok(line) => line,
            Err(error) => {
                let _ = process.kill();
                let _ = process.wait();
                return Err(format!("无法读取本机转写结果：{error}"));
            }
        };
        let line = String::from_utf8_lossy(&bytes);
        let line = line.trim_end_matches('\r');
        if let Some(progress) = line.strip_prefix("progress\t") {
            if let Ok(progress) = progress.parse::<u8>() {
                let progress = progress.min(99);
                if progress > last_progress
                    && store
                        .set_transcription_progress(&project.id, STAGE, Some(progress))
                        .is_ok()
                {
                    last_progress = progress;
                }
            }
            continue;
        }
        let mut fields = line.splitn(3, '\t');
        let start = fields.next().and_then(|value| value.parse::<u64>().ok());
        let end = fields.next().and_then(|value| value.parse::<u64>().ok());
        let text = fields.next().map(str::trim);
        if let (Some(start), Some(end), Some(text)) = (start, end, text) {
            if !text.is_empty() {
                segments.push(Segment {
                    id: String::new(),
                    start: start as f64 / 1000.0,
                    end: end as f64 / 1000.0,
                    text: text.to_owned(),
                    speaker_id: "unknown".into(),
                    overlap: false,
                });
            }
        }
    }
    let status = process
        .wait()
        .map_err(|error| format!("无法读取本机转写状态：{error}"))?;
    if !status.success() {
        return Err(format!(
            "内置离线识别失败（{status}）。可回听音频或切换转写方式。"
        ));
    }
    if segments.is_empty() {
        return Err("内置离线识别未检测到可转写的文字。".into());
    }
    Ok((
        vec![Speaker {
            id: "unknown".into(),
            name: "待确认".into(),
        }],
        segments,
    ))
}

fn windows_speech(store: &Store, project: &Project, language: &str) -> Result<Transcript> {
    store.set_transcription_stage(&project.id, "正在准备 Windows 离线识别")?;
    let wav = SpeechWave::prepare(Path::new(&project.audio_path))?;
    let script: Vec<u8> = include_str!("../inference/windows-speech.ps1")
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
            &STANDARD.encode(script),
        ])
        .env("VOICETAG_SPEECH_WAV", wav.path())
        .env("VOICETAG_SPEECH_LANGUAGE", language);
    run_worker(command, store, project, "可以切换到声签的内置离线识别。")
}

fn run_worker(
    mut command: Command,
    store: &Store,
    project: &Project,
    help: &str,
) -> Result<Transcript> {
    hide_console(&mut command);
    let mut process = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("无法启动转写。{help}（{e}）"))?;
    let mut failure = None;
    let mut output = None;
    for line in BufReader::new(process.stdout.take().unwrap()).lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                let _ = process.kill();
                failure = Some(format!("无法读取转写结果：{error}"));
                break;
            }
        };
        match serde_json::from_str::<Event>(&line) {
            Ok(Event::Stage { message }) => {
                let _ = store.set_transcription_stage(&project.id, &message);
            }
            Ok(Event::Error { message }) => failure = Some(message),
            Ok(Event::Result { speakers, segments }) => output = Some((speakers, segments)),
            Err(_) => {}
        }
    }
    let status = process
        .wait()
        .map_err(|e| format!("无法读取转写进程状态：{e}"))?;
    if let Some(error) = failure {
        return Err(error);
    }
    if !status.success() {
        return Err(format!("转写进程异常退出（{status}）。{help}"));
    }
    output.ok_or_else(|| "转写进程未返回结果。".into())
}
