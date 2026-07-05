//! 模型目錄（SPEC §6）。whisper.cpp GGUF 家族；sha256 之後補齊目錄後強制。
//! 沿用 prototype config 的 `whisper_model` 值，舊值（*-mlx）自動映射。

use std::path::PathBuf;

pub struct ModelSpec {
    /// config 中的值（正規化後）
    pub id: &'static str,
    pub filename: &'static str,
    pub url: &'static str,
    /// 近似大小（bytes），供 UI 顯示與下載前提示
    pub approx_bytes: u64,
    /// 已知 sha256（None = 下載後不校驗）
    pub sha256: Option<&'static str>,
    pub label: &'static str,
    pub desc: &'static str,
    pub recommended: bool,
}

const HF: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

macro_rules! model {
    ($id:literal, $file:literal, $bytes:expr, $label:literal, $desc:literal, $rec:expr) => {
        ModelSpec {
            id: $id,
            filename: $file,
            url: concat!(
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/",
                $file
            ),
            approx_bytes: $bytes,
            sha256: None,
            label: $label,
            desc: $desc,
            recommended: $rec,
        }
    };
}

pub const MODELS: &[ModelSpec] = &[
    model!(
        "large-v3-turbo",
        "ggml-large-v3-turbo.bin",
        1_624_555_275,
        "Large v3 Turbo",
        "速度與品質的最佳平衡，中英混合表現好",
        true
    ),
    model!(
        "large-v3-turbo-q5_0",
        "ggml-large-v3-turbo-q5_0.bin",
        574_041_195,
        "Large v3 Turbo（量化）",
        "體積只有 1/3、更省記憶體，品質略降",
        false
    ),
    model!(
        "large-v3",
        "ggml-large-v3.bin",
        3_095_033_483,
        "Large v3",
        "最高辨識品質，較慢、較吃記憶體",
        false
    ),
    model!(
        "large-v3-q5_0",
        "ggml-large-v3-q5_0.bin",
        1_081_140_203,
        "Large v3（量化）",
        "接近最高品質的省空間選擇",
        false
    ),
    model!(
        "medium",
        "ggml-medium.bin",
        1_533_763_059,
        "Medium",
        "中型模型，品質尚可",
        false
    ),
    model!(
        "small",
        "ggml-small.bin",
        487_601_967,
        "Small",
        "輕量快速，舊機友善（中文品質有限）",
        false
    ),
];

const _: () = assert!(!HF.is_empty()); // 保留給之後目錄擴充（SenseVoice 等）

/// config 值 → 模型 spec。prototype 舊值（*-mlx，MLX repo 名）一律映射到
/// large-v3-turbo（SPEC 建議預設；避免逼使用者多下載 3.1GB 的 large-v3——
/// 想用 large-v3 的人明確設 "large-v3" 即可）。未知值 → 預設 large-v3-turbo。
pub fn resolve(config_value: &str) -> &'static ModelSpec {
    let id = match config_value {
        "large-v3-mlx" | "large-v3-turbo-mlx" => "large-v3-turbo",
        other => other,
    };
    MODELS.iter().find(|m| m.id == id).unwrap_or_else(|| {
        tracing::warn!("unknown whisper_model '{config_value}', falling back to large-v3-turbo");
        &MODELS[0]
    })
}

pub fn find(id: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|m| m.id == id)
}

/// 模型存放目錄：~/Library/Application Support/Claro/models（SPEC §6）
pub fn models_dir() -> PathBuf {
    dirs::data_dir()
        .expect("no data dir")
        .join("Claro")
        .join("models")
}

pub fn model_path(spec: &ModelSpec) -> PathBuf {
    models_dir().join(spec.filename)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_maps_legacy_mlx_values() {
        assert_eq!(resolve("large-v3-mlx").id, "large-v3-turbo");
        assert_eq!(resolve("large-v3-turbo").id, "large-v3-turbo");
        assert_eq!(resolve("large-v3").id, "large-v3");
    }

    #[test]
    fn resolve_unknown_falls_back_to_turbo() {
        assert_eq!(resolve("no-such-model").id, "large-v3-turbo");
    }

    #[test]
    fn registry_has_unique_ids_and_valid_urls() {
        let mut seen = std::collections::HashSet::new();
        for m in MODELS {
            assert!(seen.insert(m.id), "duplicate id {}", m.id);
            assert!(m.url.starts_with("https://huggingface.co/"));
            assert!(m.url.ends_with(m.filename));
            assert!(m.approx_bytes > 100_000_000);
        }
        assert_eq!(MODELS.iter().filter(|m| m.recommended).count(), 1);
    }
}
