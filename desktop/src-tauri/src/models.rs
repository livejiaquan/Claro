//! 模型下載器（SPEC §6，規格照 Handy 實證）：
//! `.partial` 暫存 + HTTP Range 續傳 + 進度回報 + sha256 校驗（目錄有值才驗）。
//! 絕不自動下載——呼叫端（UI/CLI）必須先取得使用者同意。

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

pub struct Progress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

fn partial_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".partial");
    dest.with_file_name(name)
}

/// 下載 url 到 dest。支援中斷續傳；expected_sha256 有值時完成後校驗。
/// progress 每 ~1MB 回呼一次。
pub fn download(
    url: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
    mut progress: impl FnMut(Progress),
) -> Result<()> {
    if dest.exists() {
        return Ok(());
    }
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir)?;
    }
    let partial = partial_path(dest);
    let resume_from = partial.metadata().map(|m| m.len()).unwrap_or(0);

    let mut req = ureq::get(url);
    if resume_from > 0 {
        req = req.set("Range", &format!("bytes={resume_from}-"));
    }
    let resp = req.call().context("model download request")?;

    let (mut offset, total) = match resp.status() {
        // 206：伺服器接受續傳
        206 => {
            let total = resp
                .header("Content-Range")
                .and_then(|cr| cr.rsplit('/').next())
                .and_then(|t| t.parse::<u64>().ok());
            (resume_from, total)
        }
        // 200：伺服器不理 Range（或本來就從頭），重新開始
        200 => {
            let total = resp
                .header("Content-Length")
                .and_then(|l| l.parse::<u64>().ok());
            if resume_from > 0 {
                fs::remove_file(&partial).ok();
            }
            (0, total)
        }
        s => bail!("unexpected status {s} downloading {url}"),
    };

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&partial)
        .context("open partial file")?;
    if offset == 0 {
        file.set_len(0)?;
    }

    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut since_report: u64 = 0;
    loop {
        let n = reader.read(&mut buf).context("read download stream")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        offset += n as u64;
        since_report += n as u64;
        if since_report >= 1024 * 1024 {
            since_report = 0;
            progress(Progress { downloaded: offset, total });
        }
    }
    file.flush()?;
    drop(file);
    progress(Progress { downloaded: offset, total });

    if let Some(total) = total {
        if offset != total {
            bail!("incomplete download: {offset}/{total} bytes (再跑一次會續傳)");
        }
    }

    if let Some(expected) = expected_sha256 {
        let actual = sha256_file(&partial)?;
        if !actual.eq_ignore_ascii_case(expected) {
            fs::remove_file(&partial).ok();
            bail!("sha256 mismatch: expected {expected}, got {actual}");
        }
    }

    fs::rename(&partial, dest).context("finalize model file")?;
    Ok(())
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// 給 CLI/測試用的簡單進度輸出
pub fn print_progress(p: Progress) {
    match p.total {
        Some(t) if t > 0 => {
            let pct = p.downloaded as f64 / t as f64 * 100.0;
            eprint!("\r  下載中 {:.1}% ({}/{} MB)", pct, p.downloaded / 1_048_576, t / 1_048_576);
        }
        _ => eprint!("\r  下載中 {} MB", p.downloaded / 1_048_576),
    }
    if p.total == Some(p.downloaded) {
        eprintln!();
    }
}
