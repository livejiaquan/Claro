//! 開發工具：把 WAV 檔丟進完整 STT 管線（轉錄→清理→繁化→字典），驗證端到端。
//! 用法：cargo run --example transcribe_file <path.wav> [model-id] [auto|zh|en]

use claro_lib::stt::{registry, transcribe::TranscribeEngine, SttEngine, SttRequest};
use claro_lib::{audio, textproc};

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .expect("用法：transcribe_file <path.wav> [model-id] [auto|zh|en]");
    let model_id = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "large-v3-turbo".into());
    let language_arg = std::env::args().nth(3).unwrap_or_else(|| "zh".into());
    let language = match language_arg.as_str() {
        "auto" => None,
        "zh" => Some("zh"),
        "en" => Some("en"),
        other => anyhow::bail!("未知語言 '{other}'，請用 auto、zh 或 en"),
    };

    let mut reader = hound::WavReader::open(&path)?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.unwrap() as f32 / max)
                .collect()
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
    audio::validate(&mono, audio::TARGET_RATE)
        .map_err(|guard| anyhow::anyhow!("音訊未通過正式管線防呆：{guard:?}"))?;
    let input_rms = audio::rms(&mono);
    let mono = audio::normalize(mono);
    eprintln!(
        "音訊 {:.1}s @16k，RMS {:.4}",
        mono.len() as f64 / 16000.0,
        input_rms
    );

    let model =
        registry::find(&model_id).ok_or_else(|| anyhow::anyhow!("未知的 STT 模型：{model_id}"))?;
    eprintln!(
        "STT 模型 {} ({:?})，language={}",
        model.id, model.family, language_arg
    );
    let mut engine = TranscribeEngine::new(model, registry::model_path(model));
    let t0 = std::time::Instant::now();
    engine.load()?;
    eprintln!("模型載入 {:.1}s", t0.elapsed().as_secs_f32());

    let t1 = std::time::Instant::now();
    let raw = engine.transcribe(&SttRequest {
        audio: &mono,
        language,
        initial_prompt: claro_lib::stt::build_initial_prompt(&[]),
    })?;
    let stt_s = t1.elapsed().as_secs_f32();

    let base =
        textproc::normalize_cjk_punct(&textproc::to_traditional(&textproc::clean_transcript(&raw)));
    let cleaned = textproc::apply_dict(&base, &textproc::default_dict());
    println!("raw:     {raw}");
    println!("cleaned: {cleaned}");
    eprintln!("轉錄耗時 {stt_s:.2}s");
    Ok(())
}
