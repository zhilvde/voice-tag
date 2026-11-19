use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use cpal::{
    traits::{DeviceTrait, StreamTrait},
    FromSample, SampleFormat, SizedSample,
};
use serde::Serialize;

use crate::Result;

#[derive(Clone, Default, Serialize)]
pub struct RecordingStatus {
    pub paused: bool,
    pub duration: f64,
    pub level: f32,
    pub error: Option<String>,
}

#[derive(Default)]
struct Capture {
    stop: AtomicBool,
    paused: AtomicBool,
    status: Mutex<RecordingStatus>,
}

pub struct Recorder {
    pub id: String,
    capture: Arc<Capture>,
    thread: JoinHandle<()>,
}

impl Recorder {
    pub fn start(id: String, device: cpal::Device, path: PathBuf) -> Result<Self> {
        let capture = Arc::new(Capture::default());
        let shared = capture.clone();
        let (ready, started) = mpsc::sync_channel(1);
        let thread = thread::spawn(move || {
            if let Err(error) = record(device, &path, &shared, &ready) {
                shared.report(error.clone());
                let _ = ready.try_send(Err(error));
            }
        });
        match started.recv().map_err(|_| "录音线程未能启动。")? {
            Ok(()) => Ok(Self {
                id,
                capture,
                thread,
            }),
            Err(error) => {
                let _ = thread.join();
                Err(error)
            }
        }
    }

    pub fn status(&self) -> RecordingStatus {
        self.capture.status.lock().unwrap().clone()
    }

    pub fn pause(&self, paused: bool) {
        self.capture.paused.store(paused, Relaxed);
        let mut status = self.capture.status.lock().unwrap();
        status.paused = paused;
        status.level = 0.0;
    }

    pub fn finish(self) -> Result<RecordingStatus> {
        self.capture.stop.store(true, Relaxed);
        self.thread
            .join()
            .map_err(|_| "录音线程意外退出，重启后将尝试恢复音频。")?;
        Ok(self.capture.status.lock().unwrap().clone())
    }
}

impl Capture {
    fn report(&self, error: String) {
        let mut status = self.status.lock().unwrap();
        if status.error.is_none() {
            status.error = Some(error);
        }
        self.stop.store(true, Relaxed);
    }
}

fn record(
    device: cpal::Device,
    path: &Path,
    capture: &Arc<Capture>,
    ready: &SyncSender<Result<()>>,
) -> Result<()> {
    let supported = device
        .default_input_config()
        .map_err(|e| format!("无法读取麦克风设置，请检查 Windows 麦克风权限：{e}"))?;
    let config = supported.config();
    let sample_rate = config.sample_rate.0;
    let file = File::create(path).map_err(|e| format!("无法创建录音文件：{e}"))?;
    let disk = file.try_clone().map_err(|e| e.to_string())?;
    let mut writer = hound::WavWriter::new(
        std::io::BufWriter::new(file),
        hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .map_err(|e| format!("无法创建 WAV 文件：{e}"))?;
    writer
        .flush()
        .map_err(|e| format!("无法写入录音文件：{e}"))?;

    let (sender, data) = mpsc::sync_channel(64);
    let stream = match supported.sample_format() {
        SampleFormat::I8 => input::<i8>(&device, &config, sender, capture),
        SampleFormat::I16 => input::<i16>(&device, &config, sender, capture),
        SampleFormat::I24 => input::<cpal::I24>(&device, &config, sender, capture),
        SampleFormat::I32 => input::<i32>(&device, &config, sender, capture),
        SampleFormat::I64 => input::<i64>(&device, &config, sender, capture),
        SampleFormat::U8 => input::<u8>(&device, &config, sender, capture),
        SampleFormat::U16 => input::<u16>(&device, &config, sender, capture),
        SampleFormat::U32 => input::<u32>(&device, &config, sender, capture),
        SampleFormat::U64 => input::<u64>(&device, &config, sender, capture),
        SampleFormat::F32 => input::<f32>(&device, &config, sender, capture),
        SampleFormat::F64 => input::<f64>(&device, &config, sender, capture),
        _ => return Err("不支持此麦克风的采样格式。".into()),
    }
    .map_err(|e| format!("无法打开麦克风，请检查连接和权限：{e}"))?;
    stream.play().map_err(|e| format!("无法开始录音：{e}"))?;
    let _ = ready.send(Ok(()));
    let outcome = write_audio(&mut writer, &disk, &data, capture, sample_rate, stream);
    let finalized = writer.finalize().map_err(|e| format!("保存录音失败：{e}"));
    let synced = disk.sync_data().map_err(|e| format!("保存录音失败：{e}"));
    outcome.and(finalized).and(synced)
}

fn input<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sender: SyncSender<Vec<i16>>,
    capture: &Arc<Capture>,
) -> std::result::Result<cpal::Stream, cpal::BuildStreamError>
where
    T: SizedSample,
    i16: FromSample<T>,
{
    let channels = usize::from(config.channels);
    let state = capture.clone();
    let errors = capture.clone();
    device.build_input_stream(
        config,
        move |samples: &[T], _| {
            if state.paused.load(Relaxed) || state.stop.load(Relaxed) {
                return;
            }
            let mono = samples
                .chunks_exact(channels)
                .map(|frame| {
                    let sum: i32 = frame
                        .iter()
                        .map(|sample| i32::from((*sample).to_sample::<i16>()))
                        .sum();
                    (sum / channels as i32) as i16
                })
                .collect();
            if let Err(TrySendError::Full(_)) = sender.try_send(mono) {
                state.report("写入音频过慢，录音已停止。已保存部分可在历史记录中打开。".into());
            }
        },
        move |error| errors.report(format!("麦克风连接中断：{error}")),
        None,
    )
}

fn write_audio(
    writer: &mut hound::WavWriter<std::io::BufWriter<File>>,
    disk: &File,
    data: &Receiver<Vec<i16>>,
    capture: &Capture,
    sample_rate: u32,
    stream: cpal::Stream,
) -> Result<()> {
    let mut stream = Some(stream);
    let mut checkpoint = Instant::now();
    let mut sync = checkpoint;
    loop {
        let samples = if capture.stop.load(Relaxed) {
            drop(stream.take());
            match data.try_recv() {
                Ok(samples) => samples,
                Err(TryRecvError::Disconnected | TryRecvError::Empty) => break,
            }
        } else {
            match data.recv_timeout(Duration::from_millis(100)) {
                Ok(samples) => samples,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err("录音设备已断开。".into()),
            }
        };
        let mut peak = 0.0_f32;
        for sample in samples {
            writer
                .write_sample(sample)
                .map_err(|e| format!("写入录音失败：{e}"))?;
            peak = peak.max((f32::from(sample) / 32768.0).abs());
        }
        {
            let mut status = capture.status.lock().unwrap();
            status.duration = f64::from(writer.duration()) / f64::from(sample_rate);
            status.level = if status.paused {
                0.0
            } else {
                peak.max(status.level * 0.75)
            };
        }
        if checkpoint.elapsed() >= Duration::from_secs(1) {
            writer.flush().map_err(|e| format!("写入录音失败：{e}"))?;
            checkpoint = Instant::now();
        }
        if sync.elapsed() >= Duration::from_secs(5) {
            disk.sync_data().map_err(|e| format!("保存录音失败：{e}"))?;
            sync = Instant::now();
        }
    }
    Ok(())
}

pub fn recover(path: &Path) -> Result<f64> {
    fn repair(path: &Path) -> std::io::Result<f64> {
        let mut file = OpenOptions::new().read(true).write(true).open(path)?;
        let mut header = [0u8; 44];
        file.read_exact(&mut header)?;
        if &header[..4] != b"RIFF" || &header[36..40] != b"data" {
            return Err(std::io::Error::other("无法识别录音文件头"));
        }
        let bytes = (file.metadata()?.len() - 44) & !1;
        let bytes = u32::try_from(bytes).map_err(std::io::Error::other)?;
        let riff_size = bytes
            .checked_add(36)
            .ok_or_else(|| std::io::Error::other("WAV 文件超出大小限制"))?;
        let rate = u32::from_le_bytes(header[24..28].try_into().unwrap());
        if bytes == 0 || rate == 0 {
            return Err(std::io::Error::other("没有可恢复的音频"));
        }
        file.set_len(44 + u64::from(bytes))?;
        file.seek(SeekFrom::Start(4))?;
        file.write_all(&riff_size.to_le_bytes())?;
        file.seek(SeekFrom::Start(40))?;
        file.write_all(&bytes.to_le_bytes())?;
        file.sync_data()?;
        Ok(f64::from(bytes) / 2.0 / f64::from(rate))
    }
    repair(path).map_err(|e| e.to_string())
}
