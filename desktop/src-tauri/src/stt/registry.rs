//! 模型目錄（SPEC §6）。whisper.cpp GGUF 家族；URL 固定到 upstream commit，
//! 下載完成後強制校驗 sha256。
//! 沿用 prototype config 的 `whisper_model` 值，舊值（*-mlx）自動映射。

use std::path::PathBuf;

pub struct ModelSpec {
    /// config 中的值（正規化後）
    pub id: &'static str,
    pub filename: &'static str,
    pub url: &'static str,
    /// 近似大小（bytes），供 UI 顯示與下載前提示
    pub approx_bytes: u64,
    /// 固定 upstream artifact 的 sha256。
    pub sha256: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
    pub recommended: bool,
}

const WHISPER_REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";

macro_rules! model {
    ($id:literal, $file:literal, $bytes:expr, $sha256:literal, $label:literal, $desc:literal, $rec:expr) => {
        ModelSpec {
            id: $id,
            filename: $file,
            url: concat!(
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/",
                $file
            ),
            approx_bytes: $bytes,
            sha256: $sha256,
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
        "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        "Large v3 Turbo",
        "速度與品質的最佳平衡，中英混合表現好",
        true
    ),
    model!(
        "large-v3-turbo-q5_0",
        "ggml-large-v3-turbo-q5_0.bin",
        574_041_195,
        "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        "Large v3 Turbo（量化）",
        "體積只有 1/3、更省記憶體，品質略降",
        false
    ),
    model!(
        "large-v3",
        "ggml-large-v3.bin",
        3_095_033_483,
        "64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2",
        "Large v3",
        "最高辨識品質，較慢、較吃記憶體",
        false
    ),
    model!(
        "large-v3-q5_0",
        "ggml-large-v3-q5_0.bin",
        1_081_140_203,
        "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1",
        "Large v3（量化）",
        "接近最高品質的省空間選擇",
        false
    ),
    model!(
        "medium",
        "ggml-medium.bin",
        1_533_763_059,
        "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
        "Medium",
        "中型模型，品質尚可",
        false
    ),
    model!(
        "small",
        "ggml-small.bin",
        487_601_967,
        "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        "Small",
        "輕量快速，舊機友善（中文品質有限）",
        false
    ),
];

const _: () = assert!(!WHISPER_REVISION.is_empty());

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

/// 只有內容通過 registry 內固定的 SHA-256 才能視為已下載。
pub fn model_is_verified(spec: &ModelSpec) -> bool {
    crate::models::model_file_is_verified(&model_path(spec), spec.sha256)
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
            assert!(m.url.contains(WHISPER_REVISION));
            assert!(!m.url.contains("/resolve/main/"));
            assert_eq!(m.sha256.len(), 64);
            assert!(m.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
            assert!(m.approx_bytes > 100_000_000);
        }
        assert_eq!(MODELS.iter().filter(|m| m.recommended).count(), 1);
    }
}
