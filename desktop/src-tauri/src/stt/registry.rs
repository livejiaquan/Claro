//! 模型目錄（SPEC §6）。transcribe.cpp GGUF 家族；URL 固定到 upstream commit，
//! 下載完成後強制校驗 sha256。
//! 沿用 prototype config 的 `whisper_model` 值，舊值（*-mlx）自動映射。

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFamily {
    Whisper,
    Qwen3Asr,
}

pub struct ModelSpec {
    /// config 中的值（正規化後）
    pub id: &'static str,
    pub filename: &'static str,
    pub url: &'static str,
    /// 近似大小（bytes），供 UI 顯示與下載前提示
    pub approx_bytes: u64,
    /// 固定 upstream artifact 的 sha256。
    pub sha256: &'static str,
    pub family: ModelFamily,
    pub label: &'static str,
    pub desc: &'static str,
    pub recommended: bool,
    /// 尚未通過 Claro 長音訊／真人台灣語料 release gate 的候選模型仍可供
    /// 離線 eval 使用，但正式 UI 不得下載或啟用。
    pub preview: bool,
    pub available: bool,
}

const WHISPER_REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";
const QWEN_1_7B_REVISION: &str = "92282af1610a2db19d66f2bef1e260f5deca782d";
const QWEN_0_6B_REVISION: &str = "e4e16599b900eb0cb36e524514756bb92eb092b7";

macro_rules! whisper_model {
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
            family: ModelFamily::Whisper,
            label: $label,
            desc: $desc,
            recommended: $rec,
            preview: false,
            available: true,
        }
    };
}

pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: "qwen3-asr-1.7b-q8_0",
        filename: "Qwen3-ASR-1.7B-Q8_0.gguf",
        url: "https://huggingface.co/handy-computer/Qwen3-ASR-1.7B-gguf/resolve/92282af1610a2db19d66f2bef1e260f5deca782d/Qwen3-ASR-1.7B-Q8_0.gguf",
        approx_bytes: 2_185_030_624,
        sha256: "9a0d81792dfea2d5f278b8a63deb3ea6e02139ce42c2301f32ea19c4f77526b7",
        family: ModelFamily::Qwen3Asr,
        label: "Qwen3-ASR 1.7B（預覽）",
        desc: "新一代短篇多語辨識；長篇分段與台灣真人語料仍在驗證",
        recommended: false,
        preview: true,
        available: false,
    },
    ModelSpec {
        id: "qwen3-asr-0.6b-q8_0",
        filename: "Qwen3-ASR-0.6B-Q8_0.gguf",
        url: "https://huggingface.co/handy-computer/Qwen3-ASR-0.6B-gguf/resolve/e4e16599b900eb0cb36e524514756bb92eb092b7/Qwen3-ASR-0.6B-Q8_0.gguf",
        approx_bytes: 850_423_456,
        sha256: "f081b2d5e23bd669d92cc331d722a8a0681943b8e6f34b48996fd5c319b5acd8",
        family: ModelFamily::Qwen3Asr,
        label: "Qwen3-ASR 0.6B（預覽）",
        desc: "輕量短篇多語辨識；長篇分段與台灣真人語料仍在驗證",
        recommended: false,
        preview: true,
        available: false,
    },
    whisper_model!(
        "large-v3-turbo",
        "ggml-large-v3-turbo.bin",
        1_624_555_275,
        "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        "Large v3 Turbo",
        "Whisper 速度與品質的平衡選擇，支援詞彙提示",
        false
    ),
    whisper_model!(
        "large-v3-turbo-q5_0",
        "ggml-large-v3-turbo-q5_0.bin",
        574_041_195,
        "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        "Large v3 Turbo（量化）",
        "體積只有 1/3、更省記憶體，品質略降",
        false
    ),
    whisper_model!(
        "large-v3",
        "ggml-large-v3.bin",
        3_095_033_483,
        "64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2",
        "Large v3",
        "Whisper 最高辨識品質，支援專有名詞提示，較慢且較吃記憶體",
        true
    ),
    whisper_model!(
        "large-v3-q5_0",
        "ggml-large-v3-q5_0.bin",
        1_081_140_203,
        "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1",
        "Large v3（量化）",
        "接近最高品質的省空間選擇",
        false
    ),
    whisper_model!(
        "medium",
        "ggml-medium.bin",
        1_533_763_059,
        "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
        "Medium",
        "中型模型，品質尚可",
        false
    ),
    whisper_model!(
        "small",
        "ggml-small.bin",
        487_601_967,
        "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        "Small",
        "輕量快速，舊機友善（中文品質有限）",
        false
    ),
];

const _: () = {
    assert!(!WHISPER_REVISION.is_empty());
    assert!(!QWEN_1_7B_REVISION.is_empty());
    assert!(!QWEN_0_6B_REVISION.is_empty());
};

/// config 值 → 模型 spec。prototype 舊值（*-mlx，MLX repo 名）一律映射到
/// large-v3-turbo（避免逼既有使用者多下載 3.1GB 的 large-v3——
/// 想用 large-v3 的人明確設 "large-v3" 即可）。未知值 → 相容性預設 Turbo；
/// 新安裝的硬體推薦由 settings/hardware 明確寫入，不走這條錯誤復原路徑。
pub fn resolve(config_value: &str) -> &'static ModelSpec {
    let id = match config_value {
        "large-v3-mlx" | "large-v3-turbo-mlx" => "large-v3-turbo",
        other => other,
    };
    MODELS
        .iter()
        .find(|m| m.id == id && m.available)
        .unwrap_or_else(|| {
            tracing::warn!(
            "unavailable or unknown whisper_model '{config_value}', falling back to large-v3-turbo"
        );
            find("large-v3-turbo").expect("registry always contains compatibility fallback")
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
    fn resolve_unknown_falls_back_to_compatibility_default() {
        assert_eq!(resolve("no-such-model").id, "large-v3-turbo");
        assert_eq!(
            resolve("qwen3-asr-1.7b-q8_0").id,
            "large-v3-turbo",
            "preview model config must not bypass the release gate"
        );
    }

    #[test]
    fn registry_has_unique_ids_and_valid_urls() {
        let mut seen = std::collections::HashSet::new();
        for m in MODELS {
            assert!(seen.insert(m.id), "duplicate id {}", m.id);
            assert!(m.url.starts_with("https://huggingface.co/"));
            assert!(m.url.ends_with(m.filename));
            assert!(!m.url.contains("/resolve/main/"));
            match m.family {
                ModelFamily::Whisper => assert!(m.url.contains(WHISPER_REVISION)),
                ModelFamily::Qwen3Asr if m.id.contains("1.7b") => {
                    assert!(m.url.contains(QWEN_1_7B_REVISION))
                }
                ModelFamily::Qwen3Asr => assert!(m.url.contains(QWEN_0_6B_REVISION)),
            }
            assert_eq!(m.sha256.len(), 64);
            assert!(m.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
            assert!(m.approx_bytes > 100_000_000);
            assert_eq!(m.preview, !m.available, "preview gate drift for {}", m.id);
        }
        assert_eq!(MODELS.iter().filter(|m| m.recommended).count(), 1);
    }

    #[test]
    fn qwen_models_have_pinned_artifacts_and_family() {
        let large = find("qwen3-asr-1.7b-q8_0").unwrap();
        assert_eq!(large.family, ModelFamily::Qwen3Asr);
        assert!(large.preview);
        assert!(!large.available);
        assert_eq!(large.approx_bytes, 2_185_030_624);
        assert_eq!(
            large.url,
            "https://huggingface.co/handy-computer/Qwen3-ASR-1.7B-gguf/resolve/92282af1610a2db19d66f2bef1e260f5deca782d/Qwen3-ASR-1.7B-Q8_0.gguf"
        );
        assert_eq!(
            large.sha256,
            "9a0d81792dfea2d5f278b8a63deb3ea6e02139ce42c2301f32ea19c4f77526b7"
        );

        let compact = find("qwen3-asr-0.6b-q8_0").unwrap();
        assert_eq!(compact.family, ModelFamily::Qwen3Asr);
        assert_eq!(compact.approx_bytes, 850_423_456);
        assert_eq!(
            compact.url,
            "https://huggingface.co/handy-computer/Qwen3-ASR-0.6B-gguf/resolve/e4e16599b900eb0cb36e524514756bb92eb092b7/Qwen3-ASR-0.6B-Q8_0.gguf"
        );
        assert_eq!(
            compact.sha256,
            "f081b2d5e23bd669d92cc331d722a8a0681943b8e6f34b48996fd5c319b5acd8"
        );
    }
}
