//! Claro STT 合成語音 regression runner。
//!
//! 用法：
//! `cargo run --example eval_stt -- <audio-dir> [model-id] [auto|zh|en] [prompt|no-prompt]`
//!
//! `<audio-dir>` 必須包含 corpus 每一案的 `<id>.wav`；缺檔會在模型載入前失敗。
//! stdout 只輸出 pretty JSON，進度與診斷走 stderr。這個 corpus 只能比較版本與
//! 解碼設定，不能代替真人台灣口音驗收。為了公平比較不同模型家族，prompt
//! profile 預設為 `no-prompt`，需明確傳入 `prompt` 才啟用詞彙提示。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use claro_lib::stt::registry::{self, ModelFamily};
use claro_lib::stt::{transcribe::TranscribeEngine, SttEngine, SttRequest};
use claro_lib::{audio, textproc};
use serde::{Deserialize, Serialize};

const DEFAULT_MODEL_ID: &str = "large-v3-turbo";
const DEFAULT_PROMPT_PROFILE: &str = "no-prompt";
const LIMITATION: &str = "這是 macOS TTS 合成語音 regression，僅可比較版本與解碼設定；不代表麥克風聲學、自發口語、說話者多樣性或真人台灣口音準確率。";

#[derive(Debug, Deserialize)]
struct Corpus {
    version: u32,
    description: String,
    cases: Vec<CorpusCase>,
}

#[derive(Debug, Deserialize)]
struct CorpusCase {
    id: String,
    reference: String,
    #[serde(default)]
    accepted_references: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    terms: Vec<String>,
    #[serde(default)]
    anchors: Vec<String>,
}

#[derive(Debug, Serialize)]
struct Report {
    schema_version: u32,
    limitation: &'static str,
    corpus: CorpusReport,
    model: ModelReport,
    decoding: DecodingReport,
    metric_profile: MetricProfile,
    model_load_ms: f64,
    aggregate: AggregateReport,
    cases: Vec<CaseReport>,
}

#[derive(Debug, Serialize)]
struct CorpusReport {
    version: u32,
    description: String,
    case_count: usize,
}

#[derive(Debug, Serialize)]
struct ModelReport {
    id: String,
    family: &'static str,
    sha256: String,
    hash_status: &'static str,
}

#[derive(Debug, Serialize)]
struct DecodingReport {
    requested_language: String,
    effective_language: String,
    requested_prompt_profile: &'static str,
    effective_prompt_profile: &'static str,
}

#[derive(Debug, Serialize)]
struct MetricProfile {
    cer: &'static str,
    term_recall: &'static str,
    anchor_accuracy: &'static str,
    aggregate: &'static str,
}

#[derive(Debug, Serialize)]
struct AggregateReport {
    normalized_char_edits: usize,
    normalized_reference_chars: usize,
    cer: f64,
    terms_matched: usize,
    terms_total: usize,
    term_recall: Option<f64>,
    anchors_matched: usize,
    anchors_total: usize,
    anchor_accuracy: Option<f64>,
    transcribe_latency_ms_p50: f64,
    transcribe_latency_ms_p95: f64,
}

#[derive(Debug, Serialize)]
struct CaseReport {
    id: String,
    tags: Vec<String>,
    reference: String,
    accepted_references: Vec<String>,
    matched_reference: String,
    transcript_raw: String,
    transcript_processed: String,
    normalized_reference: String,
    normalized_hypothesis: String,
    audio_duration_s: f64,
    source_sample_rate: u32,
    source_channels: u16,
    initial_prompt: Option<String>,
    transcribe_latency_ms: f64,
    normalized_char_edits: usize,
    normalized_reference_chars: usize,
    cer: f64,
    term_matches: Vec<LexicalMatch>,
    term_recall: Option<f64>,
    anchor_matches: Vec<AnchorMatch>,
    anchor_accuracy: Option<f64>,
}

#[derive(Debug, Serialize)]
struct LexicalMatch {
    expected: String,
    matched: bool,
}

#[derive(Debug, Serialize)]
struct AnchorMatch {
    expression: String,
    alternatives: Vec<String>,
    matched_alternative: Option<String>,
}

struct WavAudio {
    samples: Vec<f32>,
    source_sample_rate: u32,
    source_channels: u16,
}

struct ReferenceMatch {
    text: String,
    normalized: String,
    edits: usize,
    reference_chars: usize,
    cer: f64,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.len() > 4 {
        bail!("用法：eval_stt <audio-dir> [model-id] [auto|zh|en] [prompt|no-prompt]");
    }

    let audio_dir = PathBuf::from(&args[0]);
    let model_id = args.get(1).map(String::as_str).unwrap_or(DEFAULT_MODEL_ID);
    let requested_language = args.get(2).map(String::as_str).unwrap_or("auto");
    let language = parse_language(requested_language)?;
    let prompt_enabled = parse_prompt(
        args.get(3)
            .map(String::as_str)
            .unwrap_or(DEFAULT_PROMPT_PROFILE),
    )?;

    let corpus_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/eval/stt_accuracy.json");
    let corpus: Corpus = serde_json::from_slice(
        &std::fs::read(&corpus_path)
            .with_context(|| format!("讀取 corpus {}", corpus_path.display()))?,
    )
    .with_context(|| format!("解析 corpus {}", corpus_path.display()))?;
    validate_corpus_and_audio(&corpus, &audio_dir)?;

    let spec = registry::find(model_id).with_context(|| {
        let ids = registry::MODELS
            .iter()
            .map(|model| model.id)
            .collect::<Vec<_>>()
            .join(", ");
        format!("未知模型 '{model_id}'；可用模型：{ids}")
    })?;
    let family = family_name(spec.family);
    let effective_language = requested_language;
    let effective_prompt_profile = match (prompt_enabled, spec.family) {
        (false, _) => "none",
        (true, ModelFamily::Whisper) => "case_terms_when_present",
        (true, ModelFamily::Qwen3Asr) => "ignored_by_model",
    };

    eprintln!("載入模型 {} ({family}, sha256 {})", spec.id, spec.sha256);
    let mut engine = TranscribeEngine::new(spec, registry::model_path(spec));
    let load_started = Instant::now();
    engine.load()?;
    let model_load_ms = elapsed_ms(load_started);
    eprintln!("模型載入完成：{model_load_ms:.1} ms");

    let mut case_reports = Vec::with_capacity(corpus.cases.len());
    let mut total_edits = 0usize;
    let mut total_reference_chars = 0usize;
    let mut total_terms = 0usize;
    let mut matched_terms = 0usize;
    let mut total_anchors = 0usize;
    let mut matched_anchors = 0usize;
    let mut latencies_ms = Vec::with_capacity(corpus.cases.len());

    for (index, case) in corpus.cases.iter().enumerate() {
        eprintln!("[{}/{}] {}", index + 1, corpus.cases.len(), case.id);
        let wav_path = audio_dir.join(format!("{}.wav", case.id));
        let wav = read_production_audio(&wav_path)
            .with_context(|| format!("處理 {}", wav_path.display()))?;
        let initial_prompt = prompt_enabled
            .then(|| claro_lib::stt::build_initial_prompt(&case.terms))
            .flatten();

        let started = Instant::now();
        let raw = engine
            .transcribe(&SttRequest {
                audio: &wav.samples,
                language,
                initial_prompt: initial_prompt.clone(),
            })
            .with_context(|| format!("轉錄 case {}", case.id))?;
        let latency_ms = elapsed_ms(started);
        latencies_ms.push(latency_ms);

        // 與正式管線一致的 base deterministic pass；刻意不套 default dictionary，
        // 避免把模型錯字藏在回歸分數裡。
        let processed = base_process(&raw);
        let normalized_hypothesis = normalize_for_cer(&processed);
        let reference_match = best_reference(
            &case.reference,
            &case.accepted_references,
            &normalized_hypothesis,
        );

        let term_matches = case
            .terms
            .iter()
            .map(|term| LexicalMatch {
                expected: term.clone(),
                matched: lexical_contains(&processed, term),
            })
            .collect::<Vec<_>>();
        let case_matched_terms = term_matches.iter().filter(|item| item.matched).count();

        let anchor_matches = case
            .anchors
            .iter()
            .map(|anchor| match_anchor(&processed, anchor))
            .collect::<Result<Vec<_>>>()
            .with_context(|| format!("case {} anchor 格式錯誤", case.id))?;
        let case_matched_anchors = anchor_matches
            .iter()
            .filter(|item| item.matched_alternative.is_some())
            .count();

        total_edits += reference_match.edits;
        total_reference_chars += reference_match.reference_chars;
        total_terms += term_matches.len();
        matched_terms += case_matched_terms;
        total_anchors += anchor_matches.len();
        matched_anchors += case_matched_anchors;

        eprintln!(
            "  CER {:.3}，terms {}/{}，anchors {}/{}，{latency_ms:.1} ms",
            reference_match.cer,
            case_matched_terms,
            term_matches.len(),
            case_matched_anchors,
            anchor_matches.len(),
        );

        case_reports.push(CaseReport {
            id: case.id.clone(),
            tags: case.tags.clone(),
            reference: case.reference.clone(),
            accepted_references: case.accepted_references.clone(),
            matched_reference: reference_match.text,
            transcript_raw: raw,
            transcript_processed: processed,
            normalized_reference: reference_match.normalized,
            normalized_hypothesis,
            audio_duration_s: wav.samples.len() as f64 / audio::TARGET_RATE as f64,
            source_sample_rate: wav.source_sample_rate,
            source_channels: wav.source_channels,
            initial_prompt,
            transcribe_latency_ms: latency_ms,
            normalized_char_edits: reference_match.edits,
            normalized_reference_chars: reference_match.reference_chars,
            cer: reference_match.cer,
            term_matches,
            term_recall: ratio(case_matched_terms, case.terms.len()),
            anchor_matches,
            anchor_accuracy: ratio(case_matched_anchors, case.anchors.len()),
        });
    }

    let report = Report {
        schema_version: 1,
        limitation: LIMITATION,
        corpus: CorpusReport {
            version: corpus.version,
            description: corpus.description,
            case_count: corpus.cases.len(),
        },
        model: ModelReport {
            id: spec.id.to_string(),
            family,
            sha256: spec.sha256.to_string(),
            hash_status: "registry-pinned SHA-256 verified by model load",
        },
        decoding: DecodingReport {
            requested_language: requested_language.to_string(),
            effective_language: effective_language.to_string(),
            requested_prompt_profile: if prompt_enabled {
                "case_terms_when_present"
            } else {
                "none"
            },
            effective_prompt_profile,
        },
        metric_profile: MetricProfile {
            cer: "Minimum CER across reference and accepted_references; Unicode alphanumeric characters only; whitespace and punctuation ignored; Latin is lowercased",
            term_recall: "case-insensitive substring after removing whitespace; punctuation remains significant",
            anchor_accuracy: "same lexical matching as term recall; '|' separates accepted alternatives",
            aggregate: "micro-average for CER/recall/accuracy; nearest-rank percentile for latency",
        },
        model_load_ms,
        aggregate: AggregateReport {
            normalized_char_edits: total_edits,
            normalized_reference_chars: total_reference_chars,
            cer: cer(total_edits, total_reference_chars),
            terms_matched: matched_terms,
            terms_total: total_terms,
            term_recall: ratio(matched_terms, total_terms),
            anchors_matched: matched_anchors,
            anchors_total: total_anchors,
            anchor_accuracy: ratio(matched_anchors, total_anchors),
            transcribe_latency_ms_p50: percentile(&latencies_ms, 0.50),
            transcribe_latency_ms_p95: percentile(&latencies_ms, 0.95),
        },
        cases: case_reports,
    };

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn parse_language(value: &str) -> Result<Option<&str>> {
    match value {
        "auto" => Ok(None),
        "zh" => Ok(Some("zh")),
        "en" => Ok(Some("en")),
        other => bail!("未知語言 '{other}'，請用 auto、zh 或 en"),
    }
}

fn parse_prompt(value: &str) -> Result<bool> {
    match value {
        "prompt" => Ok(true),
        "no-prompt" => Ok(false),
        other => bail!("未知 prompt profile '{other}'，請用 prompt 或 no-prompt"),
    }
}

fn family_name(family: ModelFamily) -> &'static str {
    match family {
        ModelFamily::Whisper => "whisper",
        ModelFamily::Qwen3Asr => "qwen3-asr",
    }
}

fn validate_corpus_and_audio(corpus: &Corpus, audio_dir: &Path) -> Result<()> {
    if corpus.cases.is_empty() {
        bail!("corpus 沒有任何 cases");
    }
    if !audio_dir.is_dir() {
        bail!("audio-dir 不存在或不是目錄：{}", audio_dir.display());
    }

    let mut ids = HashSet::new();
    let mut missing = Vec::new();
    for case in &corpus.cases {
        if case.id.trim().is_empty() {
            bail!("corpus 含空白 case id");
        }
        if !ids.insert(case.id.as_str()) {
            bail!("corpus case id 重複：{}", case.id);
        }
        if case.reference.trim().is_empty() {
            bail!("case {} 的 reference 為空", case.id);
        }
        if case
            .accepted_references
            .iter()
            .any(|reference| reference.trim().is_empty())
        {
            bail!("case {} 的 accepted_references 含空字串", case.id);
        }
        let wav = audio_dir.join(format!("{}.wav", case.id));
        if !wav.is_file() {
            missing.push(wav);
        }
    }
    if !missing.is_empty() {
        let list = missing
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n  ");
        bail!(
            "缺少 {} 個 corpus 音檔（fail closed）：\n  {list}",
            missing.len()
        );
    }
    Ok(())
}

fn read_production_audio(path: &Path) -> Result<WavAudio> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    if spec.channels == 0 {
        bail!("WAV channels 不可為 0");
    }

    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            if !(1..=32).contains(&spec.bits_per_sample) {
                bail!("不支援的 integer WAV bit depth：{}", spec.bits_per_sample);
            }
            let full_scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|sample| sample.map(|value| value as f32 / full_scale))
                .collect::<std::result::Result<_, _>>()?
        }
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<std::result::Result<_, _>>()?,
    };

    if interleaved.len() % spec.channels as usize != 0 {
        bail!(
            "WAV sample 數 {} 無法整除 channels {}",
            interleaved.len(),
            spec.channels
        );
    }
    let mono = if spec.channels > 1 {
        interleaved
            .chunks_exact(spec.channels as usize)
            .map(|frame| frame.iter().sum::<f32>() / spec.channels as f32)
            .collect::<Vec<_>>()
    } else {
        interleaved
    };
    let mono = audio::resample_to_16k(&mono, spec.sample_rate)?;
    audio::validate(&mono, audio::TARGET_RATE)
        .map_err(|guard| anyhow::anyhow!("音訊未通過正式 guard：{guard:?}"))?;
    let samples = audio::normalize(mono);

    Ok(WavAudio {
        samples,
        source_sample_rate: spec.sample_rate,
        source_channels: spec.channels,
    })
}

fn base_process(text: &str) -> String {
    textproc::normalize_cjk_punct(&textproc::to_traditional(&textproc::clean_transcript(text)))
}

/// 逐一計算主要 reference 與明列的合理等價寫法，使用 CER 最低者計分。
/// 同分時保留 manifest 中較前的寫法（主要 reference 永遠排第一），讓報告穩定。
fn best_reference(
    reference: &str,
    accepted_references: &[String],
    normalized_hypothesis: &str,
) -> ReferenceMatch {
    let hypothesis_chars = normalized_hypothesis.chars().collect::<Vec<_>>();
    std::iter::once(reference)
        .chain(accepted_references.iter().map(String::as_str))
        .map(|candidate| {
            let normalized = normalize_for_cer(&base_process(candidate));
            let reference_chars = normalized.chars().count();
            let reference_vec = normalized.chars().collect::<Vec<_>>();
            let edits = char_edit_distance(&reference_vec, &hypothesis_chars);
            ReferenceMatch {
                text: candidate.to_string(),
                normalized,
                edits,
                reference_chars,
                cer: cer(edits, reference_chars),
            }
        })
        .reduce(|best, candidate| {
            if candidate.cer.total_cmp(&best.cer).is_lt() {
                candidate
            } else {
                best
            }
        })
        .expect("primary reference always exists")
}

/// CER 專用正規化：只保留 Unicode 英數字，因此空白與標點完全不計；
/// `to_lowercase` 也涵蓋非 ASCII 拉丁字母，但不改 CJK。
fn normalize_for_cer(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// 詞彙／anchor 匹配比 CER 更嚴格：只忽略空白與大小寫，保留 @、連字號、
/// 小數點等有語意的符號。accepted alternatives 應在 corpus 明確列出。
fn normalize_for_lexical_match(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

fn lexical_contains(hypothesis: &str, expected: &str) -> bool {
    let needle = normalize_for_lexical_match(expected);
    !needle.is_empty() && normalize_for_lexical_match(hypothesis).contains(&needle)
}

fn match_anchor(hypothesis: &str, expression: &str) -> Result<AnchorMatch> {
    let alternatives = expression
        .split('|')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if alternatives.is_empty() {
        bail!("anchor '{expression}' 沒有有效 alternative");
    }
    let matched_alternative = alternatives
        .iter()
        .find(|alternative| lexical_contains(hypothesis, alternative))
        .cloned();
    Ok(AnchorMatch {
        expression: expression.to_string(),
        alternatives,
        matched_alternative,
    })
}

fn char_edit_distance(reference: &[char], hypothesis: &[char]) -> usize {
    if reference.is_empty() {
        return hypothesis.len();
    }
    if hypothesis.is_empty() {
        return reference.len();
    }

    // O(min(n, m)) 記憶體；corpus 是短句，但這也避免未來長句配置二維矩陣。
    let (rows, columns) = if hypothesis.len() <= reference.len() {
        (reference, hypothesis)
    } else {
        (hypothesis, reference)
    };
    let mut previous: Vec<usize> = (0..=columns.len()).collect();
    let mut current = vec![0usize; columns.len() + 1];

    for (row_index, row_char) in rows.iter().enumerate() {
        current[0] = row_index + 1;
        for (column_index, column_char) in columns.iter().enumerate() {
            let substitution = previous[column_index] + usize::from(row_char != column_char);
            let deletion = previous[column_index + 1] + 1;
            let insertion = current[column_index] + 1;
            current[column_index + 1] = substitution.min(deletion).min(insertion);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[columns.len()]
}

fn cer(edits: usize, reference_chars: usize) -> f64 {
    if reference_chars == 0 {
        if edits == 0 {
            0.0
        } else {
            1.0
        }
    } else {
        edits as f64 / reference_chars as f64
    }
}

fn ratio(matched: usize, total: usize) -> Option<f64> {
    (total > 0).then(|| matched as f64 / total as f64)
}

fn percentile(values: &[f64], quantile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = (quantile * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cer_normalization_ignores_space_punctuation_and_latin_case() {
        assert_eq!(normalize_for_cer("Qwen3-ASR， 你好！"), "qwen3asr你好");
        assert_eq!(normalize_for_cer("PyTorch"), normalize_for_cer("py torch!"));
    }

    #[test]
    fn edit_distance_handles_cjk_and_empty_input() {
        let reference: Vec<char> = "今天開會".chars().collect();
        let hypothesis: Vec<char> = "今天會".chars().collect();
        assert_eq!(char_edit_distance(&reference, &hypothesis), 1);
        assert_eq!(char_edit_distance(&[], &hypothesis), 3);
        assert_eq!(char_edit_distance(&reference, &[]), 4);
    }

    #[test]
    fn lexical_matching_ignores_case_and_space_but_keeps_symbols() {
        assert!(lexical_contains("請用 PY TORCH 跑", "PyTorch"));
        assert!(lexical_contains("寄到 claro@example.com", "@"));
        assert!(!lexical_contains("Qwen3 ASR", "Qwen3-ASR"));
    }

    #[test]
    fn anchor_accepts_pipe_separated_alternatives() {
        let matched = match_anchor("金額是 125800 元", "十二萬五千八百|125800").unwrap();
        assert_eq!(matched.matched_alternative.as_deref(), Some("125800"));
        assert!(match_anchor("anything", " | ").is_err());
    }

    #[test]
    fn best_reference_uses_lowest_cer_and_records_the_variant() {
        let accepted = vec!["版本是2.7.1，連接埠是8080。".to_string()];
        let hypothesis = normalize_for_cer(&base_process("版本是 2.7.1，連接埠是 8080。"));
        let matched = best_reference(
            "版本是二點七點一，連接埠是八千零八十。",
            &accepted,
            &hypothesis,
        );
        assert_eq!(matched.text, accepted[0]);
        assert_eq!(matched.edits, 0);
        assert_eq!(matched.cer, 0.0);
    }

    #[test]
    fn default_prompt_profile_is_fair_cross_family_baseline() {
        assert_eq!(DEFAULT_PROMPT_PROFILE, "no-prompt");
        assert!(!parse_prompt(DEFAULT_PROMPT_PROFILE).unwrap());
    }

    #[test]
    fn percentile_uses_nearest_rank() {
        let values = [50.0, 10.0, 40.0, 20.0, 30.0];
        assert_eq!(percentile(&values, 0.50), 30.0);
        assert_eq!(percentile(&values, 0.95), 50.0);
        assert_eq!(percentile(&[], 0.50), 0.0);
    }
}
