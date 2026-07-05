//! 開發工具：把 WAV 檔丟進完整 STT 管線（轉錄→清理→繁化→字典），驗證端到端。
//! 用法：cargo run --example transcribe_file <path.wav> [model-id]

use claro_lib::stt::{registry, whisper::WhisperEngine, SttEngine, SttRequest};
use claro_lib::{audio, textproc};

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("用法：transcribe_file <path.wav> [model-id]");
    let model_id = std::env::args().nth(2).unwrap_or_else(|| "large-v3-turbo".into());

    let mut reader = hound::WavReader::open(&path)?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader.samples::<i32>().map(|s| s.unwrap() as f32 / max).collect()
        }
        hound::SampleFormat::Float => reader.samples::<f32>().map(Result::unwrap).collect(),
    };
    // downmix + 重採樣走 app 同一條路
    let mono: Vec<f32> = if spec.channels > 1 {
        samples
            .chunks(spec.channels as usize)
            .map(|f| f.iter().sum::<f32>() / spec.channels as f32)
            .collect()
    } else {
        samples
    };
    let mono = audio::resample_to_16k(&mono, spec.sample_rate)?;
    let mono = audio::normalize(mono);
    eprintln!("音訊 {:.1}s @16k，RMS {:.4}", mono.len() as f64 / 16000.0, audio::rms(&mono));

    let model = registry::resolve(&model_id);
    let mut engine = WhisperEngine::new(model.id, registry::model_path(model));
    let t0 = std::time::Instant::now();
    engine.load()?;
    eprintln!("模型載入 {:.1}s", t0.elapsed().as_secs_f32());

    let t1 = std::time::Instant::now();
    let raw = engine.transcribe(&SttRequest {
        audio: &mono,
        language: Some("zh"),
        initial_prompt: Some(claro_lib::stt::build_initial_prompt(&[])),
    })?;
    let stt_s = t1.elapsed().as_secs_f32();

    let cleaned = textproc::apply_dict(
        &textproc::to_traditional(&textproc::clean_transcript(&raw)),
        &textproc::default_dict(),
    );
    println!("raw:     {raw}");
    println!("cleaned: {cleaned}");
    eprintln!("轉錄耗時 {stt_s:.2}s");
    Ok(())
}
