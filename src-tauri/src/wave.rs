use std::{
    fs::{self, File},
    io::ErrorKind,
    path::{Path, PathBuf},
};

use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error, formats::FormatOptions,
    io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};
use uuid::Uuid;

use crate::Result;

pub struct SpeechWave(PathBuf);

impl SpeechWave {
    pub fn prepare(path: &Path) -> Result<Self> {
        let wave = Self(std::env::temp_dir().join(format!("voicetag-{}.wav", Uuid::new_v4())));
        convert(path, &wave.0)?;
        Ok(wave)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for SpeechWave {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn convert(path: &Path, target: &Path) -> Result<()> {
    let error = |error: &dyn std::error::Error| format!("无法准备语音音频：{error}");
    let file = File::open(path).map_err(|e| error(&e))?;
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
        hint.with_extension(extension);
    }
    let mut format = symphonia::default::get_probe()
        .format(
            &hint,
            MediaSourceStream::new(Box::new(file), Default::default()),
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| error(&e))?
        .format;
    let track = format.default_track().ok_or("音频中没有可识别的音轨。")?;
    let id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| error(&e))?;
    let mut writer = hound::WavWriter::create(
        target,
        hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .map_err(|e| error(&e))?;
    let mut next_position = 0.0;
    let mut frame = 0_u64;
    let mut previous = 0.0_f32;
    let mut buffer: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(error(&e)),
        };
        if packet.track_id() != id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(error(&e)),
        };
        let spec = *decoded.spec();
        let channels = spec.channels.count();
        let step = f64::from(spec.rate) / 16000.0;
        if buffer
            .as_ref()
            .is_none_or(|buf| buf.capacity() < decoded.capacity() * channels)
        {
            buffer = Some(SampleBuffer::new(decoded.capacity() as u64, spec));
        }
        let samples = buffer.as_mut().unwrap();
        samples.copy_interleaved_ref(decoded);
        for values in samples.samples().chunks_exact(channels) {
            let sample = values.iter().sum::<f32>() / channels as f32;
            while next_position <= frame as f64 {
                let fraction = (next_position - (frame as f64 - 1.0)) as f32;
                let mixed = previous + (sample - previous) * fraction;
                let pcm = (mixed * 32768.0).clamp(-32768.0, 32767.0) as i16;
                writer.write_sample(pcm).map_err(|e| error(&e))?;
                next_position += step;
            }
            previous = sample;
            frame += 1;
        }
    }
    if writer.duration() == 0 {
        return Err("音频没有可识别的内容。".into());
    }
    writer.finalize().map_err(|e| error(&e))
}
