//! 開發工具：從終端機下載 STT 模型（走與 app 相同的 downloader，驗證續傳/進度）。
//! 用法：cargo run --example download_model [model-id]（預設 large-v3-turbo）

use claro_lib::models;
use claro_lib::stt::registry;

fn main() -> anyhow::Result<()> {
    let id = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "large-v3-turbo".into());
    let spec = registry::find(&id).ok_or_else(|| anyhow::anyhow!("未知的 STT 模型：{id}"))?;
    let dest = registry::model_path(spec);
    if models::model_file_is_verified(&dest, spec.sha256) {
        eprintln!("已存在且驗證通過：{}", dest.display());
        return Ok(());
    }
    eprintln!(
        "下載 {} / {:?}（約 {} MB）→ {}",
        spec.id,
        spec.family,
        spec.approx_bytes / 1_048_576,
        dest.display()
    );
    models::download(spec.url, &dest, spec.sha256, models::print_progress)?;
    eprintln!("完成：{}", dest.display());
    Ok(())
}
