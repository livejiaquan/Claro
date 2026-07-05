//! 音訊擷取（SPEC §3）：cpal 以裝置原生取樣率擷取、downmix mono，
//! 停止時用 rubato 重採樣到 16kHz——不強迫硬體改率（Handy 實證作法）。
//! 防呆與正規化語意自 prototype/main.py 移植（<0.3s、RMS<0.01、peak normalize 0.95）。

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{bounded, Sender};

pub const TARGET_RATE: u32 = 16_000;
pub const MIN_DURATION_S: f64 = 0.3;
pub const SILENCE_RMS: f32 = 0.01;
pub const MAX_RECORDING_S: f64 = 300.0;

// ─── 純函數（可測） ───────────────────────────────────────────────────────────

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// peak normalize 到 0.95（peak 太小就原樣返回，避免放大底噪）
pub fn normalize(mut samples: Vec<f32>) -> Vec<f32> {
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak > 0.001 {
        let gain = 0.95 / peak;
        for s in &mut samples {
            *s *= gain;
        }
    }
    samples
}

#[derive(Debug, PartialEq, Eq)]
pub enum AudioGuard {
    TooShort,
    Silent,
}

/// 錄音品質防呆：太短或靜音時回 Err（對應 prototype `_collect_audio`）
pub fn validate(samples: &[f32], rate: u32) -> Result<(), AudioGuard> {
    let dur = samples.len() as f64 / rate as f64;
    if dur < MIN_DURATION_S {
        return Err(AudioGuard::TooShort);
    }
    if rms(samples) < SILENCE_RMS {
        return Err(AudioGuard::Silent);
    }
    Ok(())
}

/// 整段重採樣到 16kHz mono。from_rate == 16k 時原樣返回。
pub fn resample_to_16k(samples: &[f32], from_rate: u32) -> Result<Vec<f32>> {
    if from_rate == TARGET_RATE {
        return Ok(samples.to_vec());
    }
    use rubato::{FftFixedIn, Resampler};

    const CHUNK: usize = 1024;
    let mut resampler = FftFixedIn::<f32>::new(from_rate as usize, TARGET_RATE as usize, CHUNK, 2, 1)
        .context("create resampler")?;
    let mut out: Vec<f32> = Vec::with_capacity(
        (samples.len() as f64 * TARGET_RATE as f64 / from_rate as f64) as usize + CHUNK,
    );

    let mut pos = 0;
    while pos + CHUNK <= samples.len() {
        let chunk = [&samples[pos..pos + CHUNK]];
        let res = resampler.process(&chunk, None).context("resample chunk")?;
        out.extend_from_slice(&res[0]);
        pos += CHUNK;
    }
    // 尾段 + flush
    if pos < samples.len() {
        let tail = [&samples[pos..]];
        let res = resampler
            .process_partial(Some(&tail), None)
            .context("resample tail")?;
        out.extend_from_slice(&res[0]);
    }
    let res = resampler
        .process_partial::<&[f32]>(None, None)
        .context("resampler flush")?;
    out.extend_from_slice(&res[0]);
    Ok(out)
}

// ─── 擷取（cpal Stream 不是 Send，整個生命週期關在專屬執行緒） ─────────────────

/// f32 的原子存取（bit cast 進 AtomicU32）
#[derive(Default)]
pub struct AtomicLevel(AtomicU32);

impl AtomicLevel {
    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
}

pub struct CaptureHandle {
    stop_tx: Sender<()>,
    join: JoinHandle<Result<Vec<f32>>>,
    level: Arc<AtomicLevel>,
    started: Instant,
}

impl CaptureHandle {
    /// 平滑後的即時 RMS（給 overlay 波形）
    pub fn level(&self) -> f32 {
        self.level.get()
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// 停止並取回 16kHz mono 音訊（未 normalize、未 validate——由 pipeline 決定）
    pub fn stop(self) -> Result<Vec<f32>> {
        let _ = self.stop_tx.send(());
        self.join
            .join()
            .map_err(|_| anyhow!("audio thread panicked"))?
    }
}

/// 開始錄音：專屬執行緒持有 cpal stream，收 mono 原生率樣本；
/// stop() 後重採樣到 16k 返回。
pub fn start_capture() -> Result<CaptureHandle> {
    let (stop_tx, stop_rx) = bounded::<()>(1);
    let (ready_tx, ready_rx) = bounded::<Result<()>>(1);
    let level = Arc::new(AtomicLevel::default());
    let level_thread = level.clone();

    let join = std::thread::spawn(move || -> Result<Vec<f32>> {
        let host = cpal::default_host();
        let device = match host.default_input_device() {
            Some(d) => d,
            None => {
                let _ = ready_tx.send(Err(anyhow!("no default input device")));
                return Err(anyhow!("no default input device"));
            }
        };
        let config = match device.default_input_config() {
            Ok(c) => c,
            Err(e) => {
                let _ = ready_tx.send(Err(anyhow!("default_input_config: {e}")));
                return Err(anyhow!("default_input_config: {e}"));
            }
        };
        let rate = config.sample_rate().0;
        let channels = config.channels() as usize;

        let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        let samples_cb = samples.clone();
        let level_cb = level_thread.clone();

        let err_fn = |e| tracing::warn!("[audio] stream error: {e}");
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config.into(),
                move |data: &[f32], _| {
                    push_mono(data, channels, &samples_cb, &level_cb);
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config.into(),
                move |data: &[i16], _| {
                    let f: Vec<f32> = data.iter().map(|s| *s as f32 / 32768.0).collect();
                    push_mono(&f, channels, &samples_cb, &level_cb);
                },
                err_fn,
                None,
            ),
            fmt => {
                let _ = ready_tx.send(Err(anyhow!("unsupported sample format {fmt:?}")));
                return Err(anyhow!("unsupported sample format {fmt:?}"));
            }
        }
        .context("build input stream")?;

        stream.play().context("start stream")?;
        let _ = ready_tx.send(Ok(()));

        // 等 stop 訊號（錄音上限由 pipeline 的 watchdog 送 force_stop 事件處理）
        let _ = stop_rx.recv();
        drop(stream);

        let native: Vec<f32> = std::mem::take(&mut *samples.lock().unwrap());
        resample_to_16k(&native, rate)
    });

    // 等 stream 真的開起來（或失敗），失敗要立刻讓 caller 知道
    match ready_rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(())) => Ok(CaptureHandle { stop_tx, join, level, started: Instant::now() }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(anyhow!("audio thread did not start in time")),
    }
}

fn push_mono(data: &[f32], channels: usize, samples: &Mutex<Vec<f32>>, level: &AtomicLevel) {
    let mono: Vec<f32> = if channels <= 1 {
        data.to_vec()
    } else {
        data.chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    };
    // 平滑：level*0.6 + rms*0.4（prototype 語意）
    let r = rms(&mono);
    level.set(level.get() * 0.6 + r * 0.4);
    samples.lock().unwrap().extend_from_slice(&mono);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_of_silence_is_zero_and_of_ones_is_one() {
        assert_eq!(rms(&[0.0; 100]), 0.0);
        assert!((rms(&[1.0; 100]) - 1.0).abs() < 1e-6);
        assert_eq!(rms(&[]), 0.0);
    }

    #[test]
    fn normalize_scales_peak_to_095() {
        let out = normalize(vec![0.1, -0.5, 0.25]);
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((peak - 0.95).abs() < 1e-6);
    }

    #[test]
    fn normalize_leaves_near_silence_untouched() {
        let out = normalize(vec![0.0005, -0.0002]);
        assert_eq!(out, vec![0.0005, -0.0002]);
    }

    #[test]
    fn validate_rejects_short_and_silent() {
        // 0.2s @16k → 太短
        let short = vec![0.5f32; (TARGET_RATE as f64 * 0.2) as usize];
        assert_eq!(validate(&short, TARGET_RATE), Err(AudioGuard::TooShort));
        // 1s 靜音
        let silent = vec![0.001f32; TARGET_RATE as usize];
        assert_eq!(validate(&silent, TARGET_RATE), Err(AudioGuard::Silent));
        // 1s 有聲
        let ok = vec![0.5f32; TARGET_RATE as usize];
        assert_eq!(validate(&ok, TARGET_RATE), Ok(()));
    }

    #[test]
    fn resample_48k_to_16k_produces_expected_length() {
        // 1 秒 440Hz 正弦 @48k
        let src: Vec<f32> = (0..48_000)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin())
            .collect();
        let out = resample_to_16k(&src, 48_000).unwrap();
        let expected = 16_000f64;
        let got = out.len() as f64;
        assert!(
            (got - expected).abs() / expected < 0.02,
            "expected ~16000 samples, got {got}"
        );
        // 能量沒有消失
        assert!(rms(&out) > 0.5);
    }

    #[test]
    fn resample_16k_is_identity() {
        let src = vec![0.1f32; 1600];
        let out = resample_to_16k(&src, 16_000).unwrap();
        assert_eq!(out, src);
    }
}
