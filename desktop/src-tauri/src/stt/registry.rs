//! 模型目錄（SPEC §6）。M1：whisper.cpp GGUF 家族；sha256 於 M2 補齊目錄後強制。
//! 沿用 prototype config 的 `whisper_model` 值，舊值（*-mlx）自動映射。

use std::path::PathBuf;

pub struct ModelSpec {
    /// config 中的值（正規化後）
    pub id: &'static str,
    pub filename: &'static str,
    pub url: &'static str,
    /// 近似大小（bytes），供 UI 顯示與下載前提示
    pub approx_bytes: u64,
    /// 已知 sha256（M2 補齊；None = 下載後不校驗）
    pub sha256: Option<&'static str>,
}

const HF: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: "large-v3-turbo",
        filename: "ggml-large-v3-turbo.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin",
        approx_bytes: 1_624_555_275,
        sha256: None,
    },
    ModelSpec {
        id: "large-v3",
        filename: "ggml-large-v3.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin",
        approx_bytes: 3_095_033_483,
        sha256: None,
    },
    ModelSpec {
        id: "medium",
        filename: "ggml-medium.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin",
        approx_bytes: 1_533_763_059,
        sha256: None,
    },
    ModelSpec {
        id: "small",
        filename: "ggml-small.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
        approx_bytes: 487_601_967,
        sha256: None,
    },
];

const _: () = assert!(!HF.is_empty()); // HF 常數保留給 M2 目錄擴充

/// config 值 → 模型 spec。prototype 舊值映射：
/// "large-v3-mlx"（MLX repo 名）→ large-v3；未知值 → 預設 large-v3-turbo。
pub fn resolve(config_value: &str) -> &'static ModelSpec {
    let id = match config_value {
        "large-v3-mlx" => "large-v3",
        "large-v3-turbo-mlx" => "large-v3-turbo",
        other => other,
    };
    MODELS
        .iter()
        .find(|m| m.id == id)
        .unwrap_or_else(|| {
            tracing::warn!("unknown whisper_model '{config_value}', falling back to large-v3-turbo");
            &MODELS[0]
        })
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
        assert_eq!(resolve("large-v3-mlx").id, "large-v3");
        assert_eq!(resolve("large-v3-turbo").id, "large-v3-turbo");
    }

    #[test]
    fn resolve_unknown_falls_back_to_turbo() {
        assert_eq!(resolve("no-such-model").id, "large-v3-turbo");
    }
}
