//! 模型下載器（SPEC §6，規格照 Handy 實證）：
//! `.partial` 暫存 + HTTP Range 續傳 + 進度回報 + 強制 sha256 校驗。
//! 絕不自動下載——呼叫端（UI/CLI）必須先取得使用者同意。

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub struct Progress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

/// 模型檔 fingerprint。除規格要求的 size/mtime 外，Unix 再納入 inode、device
/// 與 ctime；原路徑替換檔案或試圖把 mtime 改回去也會讓 marker 失效。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FileFingerprint {
    len: u64,
    modified_secs: i64,
    modified_nanos: i64,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    changed_secs: i64,
    #[cfg(unix)]
    changed_nanos: i64,
}

const VERIFICATION_MARKER_SCHEMA: u32 = 1;
const VERIFIER_ID: &str = concat!(env!("CARGO_PKG_VERSION"), ":sha256-v1");

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VerificationMarker {
    schema: u32,
    verifier_id: String,
    expected_sha256: String,
    fingerprint: FileFingerprint,
}

#[derive(Debug, Clone)]
struct VerificationCacheEntry {
    fingerprint: FileFingerprint,
    expected_sha256: String,
    actual_sha256: String,
}

static VERIFICATION_CACHE: OnceLock<Mutex<HashMap<PathBuf, VerificationCacheEntry>>> =
    OnceLock::new();
/// 每個檔案一把 gate（review M3）：只序列化「同一檔案」的併發 hash，
/// 不同模型檔互不阻塞——否則背景預校驗 hash 數 GB LLM 時，
/// whisper 預載與設定頁的 verify 會全部排在同一把全域鎖後面。
static VERIFICATION_GATES: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();

fn verification_gate(path: &Path) -> Arc<Mutex<()>> {
    VERIFICATION_GATES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .entry(path.to_path_buf())
        .or_default()
        .clone()
}

fn verification_cache() -> &'static Mutex<HashMap<PathBuf, VerificationCacheEntry>> {
    VERIFICATION_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn file_fingerprint(path: &Path) -> Result<FileFingerprint> {
    let metadata =
        fs::metadata(path).with_context(|| format!("read model metadata {}", path.display()))?;
    if !metadata.is_file() {
        bail!("model path is not a regular file: {}", path.display());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(FileFingerprint {
            len: metadata.len(),
            modified_secs: metadata.mtime(),
            modified_nanos: metadata.mtime_nsec(),
            device: metadata.dev(),
            inode: metadata.ino(),
            changed_secs: metadata.ctime(),
            changed_nanos: metadata.ctime_nsec(),
        })
    }
    #[cfg(not(unix))]
    {
        use std::time::UNIX_EPOCH;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok());
        Ok(FileFingerprint {
            len: metadata.len(),
            modified_secs: modified
                .as_ref()
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or_default(),
            modified_nanos: modified
                .map(|duration| duration.subsec_nanos() as i64)
                .unwrap_or_default(),
        })
    }
}

fn verification_marker_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".claro-verified.json");
    path.with_file_name(name)
}

fn marker_is_reusable(
    marker: &VerificationMarker,
    expected_sha256: &str,
    fingerprint: &FileFingerprint,
) -> bool {
    marker.schema == VERIFICATION_MARKER_SCHEMA
        && marker.verifier_id == VERIFIER_ID
        && marker.expected_sha256.eq_ignore_ascii_case(expected_sha256)
        && marker.fingerprint == *fingerprint
}

fn reusable_persistent_marker(
    path: &Path,
    expected_sha256: &str,
    fingerprint: &FileFingerprint,
) -> bool {
    let marker_path = verification_marker_path(path);
    let Ok(marker_metadata) = fs::symlink_metadata(&marker_path) else {
        return false;
    };
    if !marker_metadata.is_file() || marker_metadata.file_type().is_symlink() {
        return false;
    }
    // marker 很小；拒絕異常大檔，避免把不可信 sidecar 整個讀進記憶體。
    if marker_metadata.len() > 4096 {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let Ok(model_metadata) = fs::metadata(path) else {
            return false;
        };
        if marker_metadata.permissions().mode() & 0o777 != 0o600
            || marker_metadata.uid() != model_metadata.uid()
            || marker_metadata.nlink() != 1
        {
            return false;
        }
    }

    let Ok(bytes) = fs::read(&marker_path) else {
        return false;
    };
    let Ok(marker) = serde_json::from_slice::<VerificationMarker>(&bytes) else {
        return false;
    };
    marker_is_reusable(&marker, expected_sha256, fingerprint)
}

fn write_persistent_marker(
    path: &Path,
    expected_sha256: &str,
    fingerprint: &FileFingerprint,
) -> Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let marker = VerificationMarker {
        schema: VERIFICATION_MARKER_SCHEMA,
        verifier_id: VERIFIER_ID.to_string(),
        expected_sha256: expected_sha256.to_ascii_lowercase(),
        fingerprint: fingerprint.clone(),
    };
    let bytes = serde_json::to_vec(&marker)?;
    let marker_path = verification_marker_path(path);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut temp_name = marker_path.file_name().unwrap_or_default().to_os_string();
    temp_name.push(format!(".tmp-{}-{nonce}-{sequence}", std::process::id()));
    let temp_path = marker_path.with_file_name(temp_name);

    let result = (|| -> Result<()> {
        let mut options = fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp_path)
            .with_context(|| format!("create verification marker {}", temp_path.display()))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp_path, &marker_path)
            .with_context(|| format!("atomically replace marker {}", marker_path.display()))?;
        sync_parent_directory(&marker_path)?;
        Ok(())
    })();
    if result.is_err() {
        fs::remove_file(&temp_path).ok();
    }
    result
}

fn remove_persistent_marker(path: &Path) {
    let marker_path = verification_marker_path(path);
    match fs::remove_file(&marker_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!(
            "could not remove stale model verification marker {}: {error}",
            marker_path.display()
        ),
    }
}

fn cache_verification(
    path: &Path,
    fingerprint: FileFingerprint,
    expected_sha256: &str,
    actual_sha256: &str,
) {
    verification_cache().lock().unwrap().insert(
        path.to_path_buf(),
        VerificationCacheEntry {
            fingerprint,
            expected_sha256: expected_sha256.to_ascii_lowercase(),
            actual_sha256: actual_sha256.to_ascii_lowercase(),
        },
    );
}

fn cached_digest(
    path: &Path,
    fingerprint: &FileFingerprint,
    expected_sha256: &str,
) -> Option<String> {
    verification_cache()
        .lock()
        .unwrap()
        .get(path)
        .filter(|entry| {
            entry.fingerprint == *fingerprint
                && entry.expected_sha256.eq_ignore_ascii_case(expected_sha256)
        })
        .map(|entry| entry.actual_sha256.clone())
}

fn forget_verification(path: &Path) {
    verification_cache().lock().unwrap().remove(path);
}

/// 驗證模型內容，並以檔案 identity/metadata 做雙層快取。
///
/// process cache 記住正確與錯誤的 digest；0600 persistent marker 只記錄成功
/// 驗證，並綁定 app version/verifier schema。升級後第一次、marker 權限異常，
/// 或檔案 fingerprint 改變都會重新 hash；UI polling 只需 stat。
///
/// 已知限制（review M4）：verify 通過與 loader 開檔之間存在 TOCTOU——
/// whisper/llama 的 loader 只吃路徑、吃不了已驗證的 fd。前提是同使用者的
/// 本地程序在這個窗口替換檔案，屬防禦縱深而非信任邊界；fingerprint
/// （inode/mtime/ctime）會讓下一次 verify 察覺替換。
pub fn verify_model_file(path: &Path, expected_sha256: &str) -> Result<()> {
    validate_expected_sha256(expected_sha256)?;
    let before = file_fingerprint(path)?;
    if let Some(actual) = cached_digest(path, &before, expected_sha256) {
        return require_matching_digest(path, expected_sha256, &actual);
    }

    // get_status、list_models 與背景 preload 可能同時查同一檔。第一個 miss
    // hash 時序列化，拿到 gate 後再查一次，避免並發重讀數 GB。
    let gate = verification_gate(path);
    let _gate = gate.lock().unwrap();
    let current = file_fingerprint(path)?;
    if let Some(actual) = cached_digest(path, &current, expected_sha256) {
        return require_matching_digest(path, expected_sha256, &actual);
    }
    if reusable_persistent_marker(path, expected_sha256, &current) {
        cache_verification(path, current, expected_sha256, expected_sha256);
        return Ok(());
    }

    let (after, actual) = sha256_stable_file(path)?;
    cache_verification(path, after.clone(), expected_sha256, &actual);
    if actual.eq_ignore_ascii_case(expected_sha256) {
        if let Err(error) = write_persistent_marker(path, expected_sha256, &after) {
            // marker 是效能快取，不應把已完整驗過的模型判成失敗；本 process
            // 仍有可信 cache，下次啟動會保守地重新 hash。
            tracing::warn!(
                "could not persist verification marker for {}: {error}",
                path.display()
            );
        }
    } else {
        remove_persistent_marker(path);
    }
    require_matching_digest(path, expected_sha256, &actual)
}

fn require_matching_digest(path: &Path, expected_sha256: &str, actual: &str) -> Result<()> {
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        bail!(
            "sha256 mismatch for {}: expected {expected_sha256}, got {actual}",
            path.display()
        );
    }
    Ok(())
}

/// UI/status 用：只有內容已通過 digest 驗證才算 downloaded。
pub fn model_file_is_verified(path: &Path, expected_sha256: &str) -> bool {
    verify_model_file(path, expected_sha256).is_ok()
}

/// 呼叫端已在同一 inode 上算過 digest 時，將該結果放入可信的 process cache。
fn remember_computed_digest(path: &Path, expected_sha256: &str, actual_sha256: &str) -> Result<()> {
    let fingerprint = file_fingerprint(path)?;
    cache_verification(path, fingerprint.clone(), expected_sha256, actual_sha256);
    if actual_sha256.eq_ignore_ascii_case(expected_sha256) {
        if let Err(error) = write_persistent_marker(path, expected_sha256, &fingerprint) {
            tracing::warn!(
                "could not persist verification marker for {}: {error}",
                path.display()
            );
        }
    } else {
        remove_persistent_marker(path);
    }
    Ok(())
}

fn partial_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".partial");
    dest.with_file_name(name)
}

fn remaining_download_bytes(dest: &Path, expected_total: u64) -> u64 {
    let partial = partial_path(dest);
    expected_total.saturating_sub(
        partial
            .metadata()
            .map(|metadata| metadata.len())
            .unwrap_or(0),
    )
}

/// 下載前先確認空間；需要剩餘內容再加 10%（至少 128 MB）給暫存與檔案系統。
/// 這在任何 HTTP request 前執行，避免慢網路下載到一半才發現磁碟已滿。
pub fn ensure_download_capacity(dest: &Path, expected_total: u64) -> Result<()> {
    let remaining = remaining_download_bytes(dest, expected_total);
    let margin = (remaining / 10).max(128 * 1024 * 1024);
    let required = remaining.saturating_add(margin);
    let Some(available) = available_space_near(dest) else {
        // 無法可靠讀取時不假裝磁碟不足；實際 write error 仍會被回報且 partial 可續傳。
        return Ok(());
    };
    if available < required {
        bail!(
            "磁碟空間不足：還需要約 {} MB（目前可用 {} MB）；清出空間後可繼續下載",
            required / 1_048_576,
            available / 1_048_576
        );
    }
    Ok(())
}

#[cfg(unix)]
fn available_space_near(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let mut existing = path.parent().unwrap_or_else(|| Path::new("/"));
    while !existing.exists() {
        existing = existing.parent()?;
    }
    let c_path = CString::new(existing.as_os_str().as_bytes()).ok()?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    let result = unsafe { libc::statvfs(c_path.as_ptr(), stats.as_mut_ptr()) };
    if result != 0 {
        return None;
    }
    let stats = unsafe { stats.assume_init() };
    Some((stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64))
}

#[cfg(not(unix))]
fn available_space_near(_path: &Path) -> Option<u64> {
    None
}

/// 每個目的地一把下載互斥（review 發現）：兩個下載同開同一個 `.partial`
/// 時，A 驗證＋rename 後 B 仍握著同 inode 的 FD 繼續寫，A 之後取 fingerprint
/// 會把損壞內容綁進 process cache 與 persistent marker。UI 的單槽 guard 只
/// 擋得住 app 內按鈕，examples／未來的多入口一律在這裡兜底。
static DOWNLOAD_GATES: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();

fn download_gate(dest: &Path) -> Arc<Mutex<()>> {
    DOWNLOAD_GATES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .entry(dest.to_path_buf())
        .or_default()
        .clone()
}

/// 下載 url 到 dest。支援中斷續傳，完成後強制校驗 sha256。
/// progress 每 ~1MB 回呼一次。
/// 使用者主動取消下載時的錯誤訊息。`.partial` 會保留，下次可續傳。
pub const DOWNLOAD_CANCELLED: &str = "下載已取消";

pub fn download(
    url: &str,
    dest: &Path,
    expected_sha256: &str,
    cancel: &std::sync::atomic::AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Result<()> {
    let gate = download_gate(dest);
    let _gate = gate.lock().unwrap();
    validate_expected_sha256(expected_sha256)?;
    if dest.exists() {
        let (_, actual) = sha256_stable_file(dest)?;
        remember_computed_digest(dest, expected_sha256, &actual)?;
        if actual.eq_ignore_ascii_case(expected_sha256) {
            return Ok(());
        }
        // download() 只會由使用者明確點擊後進入。既有檔 hash 不符代表
        // 它不可載入，先移除才能以驗過的內容修復。
        tracing::warn!(
            "existing model {} failed sha256 verification; replacing it",
            dest.display()
        );
        fs::remove_file(dest).context("remove corrupt model file")?;
        forget_verification(dest);
        remove_persistent_marker(dest);
    }
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir)?;
    }
    let partial = partial_path(dest);
    // 上次程序可能剛好在 hash 通過、rename 前退出。先驗證完整 partial，
    // 成功就直接完成，不再做一次沒有意義的 Range request。
    if partial.exists() {
        let (_, actual) = sha256_stable_file(&partial)?;
        if actual.eq_ignore_ascii_case(expected_sha256) {
            finalize_verified_partial(&partial, dest)?;
            remember_computed_digest(dest, expected_sha256, &actual)?;
            return Ok(());
        }
    }
    let mut resume_from = partial.metadata().map(|m| m.len()).unwrap_or(0);

    // 沒有讀取逾時的話，伺服器建立連線後停住不送 body 會讓 read() 永久阻塞，
    // 取消旗標永遠沒機會被檢查。給一個上限讓 read 返回錯誤，使用者按取消才有效。
    let agent = ureq::AgentBuilder::new()
        .timeout_read(std::time::Duration::from_secs(30))
        .build();
    let mut req = agent.get(url);
    if resume_from > 0 {
        req = req.set("Range", &format!("bytes={resume_from}-"));
    }
    let resp = match req.call() {
        Ok(resp) => resp,
        // 416（review M5）：.partial 比遠端 pinned artifact 還長（revision 換過
        // 或檔案損壞）——這個 offset 永遠續傳不完，每次重試都卡在同一個 416。
        // 砍掉 partial 從頭來，一次就好。
        Err(ureq::Error::Status(416, _)) if resume_from > 0 => {
            tracing::warn!("stale .partial larger than remote artifact — restarting download");
            fs::remove_file(&partial).ok();
            resume_from = 0;
            agent.get(url).call().context("model download restart after 416")?
        }
        Err(e) => return Err(e).context("model download request"),
    };

    let (mut offset, total) = match resp.status() {
        // 206：伺服器接受續傳
        206 => {
            let content_range = resp
                .header("Content-Range")
                .context("206 response missing Content-Range")?;
            let range = parse_content_range(content_range)
                .with_context(|| format!("invalid Content-Range '{content_range}'"))?;
            if range.start != resume_from {
                bail!(
                    "resume offset mismatch: requested {resume_from}, server started at {}",
                    range.start
                );
            }
            if let Some(length) = resp
                .header("Content-Length")
                .and_then(|l| l.parse::<u64>().ok())
            {
                let range_len = range.end - range.start + 1;
                if length != range_len {
                    bail!(
                        "Content-Length {length} does not match Content-Range length {range_len}"
                    );
                }
            }
            (resume_from, Some(range.total))
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
        // 取消點放在讀取之前：`.partial` 只保留已完整落盤的位元組，
        // 下次以 Range 續傳接得上，不會留下半截的 chunk。
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            file.flush().ok();
            bail!("{DOWNLOAD_CANCELLED}");
        }
        let n = reader.read(&mut buf).context("read download stream")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        offset += n as u64;
        if total.is_some_and(|total| offset > total) {
            drop(file);
            fs::remove_file(&partial).ok();
            bail!("download exceeded declared total size");
        }
        since_report += n as u64;
        if since_report >= 1024 * 1024 {
            since_report = 0;
            progress(Progress {
                downloaded: offset,
                total,
            });
        }
    }
    file.flush()?;
    file.sync_all()?;
    drop(file);
    progress(Progress {
        downloaded: offset,
        total,
    });

    if let Some(total) = total {
        if offset != total {
            bail!("incomplete download: {offset}/{total} bytes (再跑一次會續傳)");
        }
    }

    // EOF 之後、整檔 sha256 之前再檢查一次：這段對數 GB 檔案要跑數秒，
    // 若在此期間按取消卻照樣 finalize，UI 會顯示「下載完成」而無視使用者操作。
    if cancel.load(std::sync::atomic::Ordering::SeqCst) {
        bail!("{DOWNLOAD_CANCELLED}");
    }

    let (_, actual) = sha256_stable_file(&partial)?;
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        fs::remove_file(&partial).ok();
        bail!("sha256 mismatch: expected {expected_sha256}, got {actual}");
    }

    finalize_verified_partial(&partial, dest)?;
    remember_computed_digest(dest, expected_sha256, &actual)
}

/// partial 與目標檔位於同一目錄，rename 在同一 filesystem 內是 atomic；
/// rename 後再同步父目錄，避免斷電時只落下檔案內容、沒落下目錄項目。
fn finalize_verified_partial(partial: &Path, dest: &Path) -> Result<()> {
    fs::rename(partial, dest).context("atomically finalize model file")?;
    sync_parent_directory(dest).context("sync model directory")
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .context("model destination has no parent directory")?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct ContentRange {
    start: u64,
    end: u64,
    total: u64,
}

/// 只接受 `bytes START-END/TOTAL`；`*` 或數值不一致都不能續傳。
fn parse_content_range(value: &str) -> Result<ContentRange> {
    let rest = value.strip_prefix("bytes ").context("unit must be bytes")?;
    let (range, total) = rest.split_once('/').context("missing total size")?;
    let (start, end) = range.split_once('-').context("missing byte range")?;
    let start = start.parse::<u64>().context("invalid range start")?;
    let end = end.parse::<u64>().context("invalid range end")?;
    let total = total.parse::<u64>().context("invalid total size")?;
    if end < start {
        bail!("range end precedes start");
    }
    if total == 0 || end >= total {
        bail!("range end is outside total size");
    }
    Ok(ContentRange { start, end, total })
}

fn validate_expected_sha256(expected: &str) -> Result<()> {
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("expected_sha256 must be exactly 64 hexadecimal characters");
    }
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

fn sha256_stable_file(path: &Path) -> Result<(FileFingerprint, String)> {
    let before = file_fingerprint(path)?;
    let actual = sha256_file(path)?;
    let after = file_fingerprint(path)?;
    if before != after {
        forget_verification(path);
        bail!("model file changed while verifying: {}", path.display());
    }
    Ok((after, actual))
}

/// 給 CLI/測試用的簡單進度輸出
pub fn print_progress(p: Progress) {
    match p.total {
        Some(t) if t > 0 => {
            let pct = p.downloaded as f64 / t as f64 * 100.0;
            eprint!(
                "\r  下載中 {:.1}% ({}/{} MB)",
                pct,
                p.downloaded / 1_048_576,
                t / 1_048_576
            );
        }
        _ => eprint!("\r  下載中 {} MB", p.downloaded / 1_048_576),
    }
    if p.total == Some(p.downloaded) {
        eprintln!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_strict_content_range() {
        assert_eq!(
            parse_content_range("bytes 3-5/6").unwrap(),
            ContentRange {
                start: 3,
                end: 5,
                total: 6
            }
        );
        assert!(parse_content_range("bytes */6").is_err());
        assert!(parse_content_range("bytes 5-3/6").is_err());
        assert!(parse_content_range("bytes 3-6/6").is_err());
        assert!(parse_content_range("items 3-5/6").is_err());
    }

    #[test]
    fn remaining_space_accounts_for_a_resumable_partial() {
        let dir = tempdir();
        fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("model.bin");
        fs::write(partial_path(&dest), vec![0_u8; 400]).unwrap();
        assert_eq!(remaining_download_bytes(&dest, 1_000), 600);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn accepts_an_existing_file_only_when_hash_matches() {
        let dir = tempdir();
        let dest = dir.join("model.bin");
        fs::write(&dest, b"already verified").unwrap();
        let expected = digest(b"already verified");
        download("not a URL", &dest, &expected, &std::sync::atomic::AtomicBool::new(false), |_| {}).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"already verified");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn verification_cache_is_reused_until_file_identity_or_metadata_changes() {
        let dir = tempdir();
        let dest = dir.join("model.bin");
        let original = b"trusted-model";
        let replacement = b"altered-model";
        assert_eq!(original.len(), replacement.len());
        fs::write(&dest, original).unwrap();
        let expected = digest(original);

        forget_verification(&dest);
        verify_model_file(&dest, &expected).unwrap();
        let original_fingerprint = file_fingerprint(&dest).unwrap();
        assert_eq!(
            cached_digest(&dest, &original_fingerprint, &expected).as_deref(),
            Some(expected.as_str())
        );

        // 即使大小相同，改寫也會改變 mtime/ctime，舊 cache 不得沿用。
        fs::write(&dest, replacement).unwrap();
        let changed_fingerprint = file_fingerprint(&dest).unwrap();
        assert_ne!(original_fingerprint, changed_fingerprint);
        assert!(cached_digest(&dest, &changed_fingerprint, &expected).is_none());
        assert!(verify_model_file(&dest, &expected).is_err());
        assert!(!model_file_is_verified(&dest, &expected));

        // 修復同一路徑後，negative cache 也必須因 metadata 改變而失效。
        fs::write(&dest, original).unwrap();
        verify_model_file(&dest, &expected).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn persistent_marker_is_private_reusable_and_version_bound() {
        let dir = tempdir();
        let dest = dir.join("model.bin");
        let bytes = b"persistently verified";
        let expected = digest(bytes);
        fs::write(&dest, bytes).unwrap();

        forget_verification(&dest);
        verify_model_file(&dest, &expected).unwrap();
        let marker_path = verification_marker_path(&dest);
        let marker_metadata = fs::symlink_metadata(&marker_path).unwrap();
        assert!(marker_metadata.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(marker_metadata.permissions().mode() & 0o777, 0o600);
        }
        let fingerprint = file_fingerprint(&dest).unwrap();
        assert!(reusable_persistent_marker(&dest, &expected, &fingerprint));

        // 模擬 app 升級／verifier schema 變動：舊 marker 絕不能沿用。
        let mut marker: VerificationMarker =
            serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
        marker.verifier_id = "older-app:sha256-v1".into();
        fs::write(&marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();
        forget_verification(&dest);
        assert!(!reusable_persistent_marker(&dest, &expected, &fingerprint));
        verify_model_file(&dest, &expected).unwrap();
        let refreshed: VerificationMarker =
            serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
        assert_eq!(refreshed.verifier_id, VERIFIER_ID);

        let temp_marker_count = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
            .count();
        assert_eq!(temp_marker_count, 0);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn persistent_marker_is_invalidated_by_expected_hash_or_file_change() {
        let dir = tempdir();
        let dest = dir.join("model.bin");
        let original = b"trusted-model";
        let replacement = b"altered-model";
        let expected = digest(original);
        fs::write(&dest, original).unwrap();
        verify_model_file(&dest, &expected).unwrap();

        let fingerprint = file_fingerprint(&dest).unwrap();
        let marker_path = verification_marker_path(&dest);
        let marker: VerificationMarker =
            serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
        assert!(!marker_is_reusable(
            &marker,
            &digest(b"different expected artifact"),
            &fingerprint
        ));

        assert_eq!(original.len(), replacement.len());
        fs::write(&dest, replacement).unwrap();
        let changed_fingerprint = file_fingerprint(&dest).unwrap();
        assert!(!reusable_persistent_marker(
            &dest,
            &expected,
            &changed_fingerprint
        ));
        assert!(verify_model_file(&dest, &expected).is_err());
        assert!(!marker_path.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_or_mismatched_file_is_never_reported_as_verified() {
        let dir = tempdir();
        let dest = dir.join("model.bin");
        let expected = digest(b"expected");
        assert!(!model_file_is_verified(&dest, &expected));

        fs::write(&dest, b"unexpected").unwrap();
        assert!(!model_file_is_verified(&dest, &expected));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replaces_a_corrupt_existing_file_with_verified_download() {
        let dir = tempdir();
        let dest = dir.join("model.bin");
        fs::write(&dest, b"corrupt").unwrap();
        let body = b"verified replacement";
        let url = serve_once(format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            String::from_utf8_lossy(body)
        ));
        download(&url, &dest, &digest(body), &std::sync::atomic::AtomicBool::new(false), |_| {}).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), body);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn resumes_only_from_the_exact_content_range_start() {
        let dir = tempdir();
        let dest = dir.join("model.bin");
        fs::write(partial_path(&dest), b"abc").unwrap();
        let url = serve_once(
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 3-5/6\r\nContent-Length: 3\r\nConnection: close\r\n\r\ndef"
                .to_string(),
        );
        download(&url, &dest, &digest(b"abcdef"), &std::sync::atomic::AtomicBool::new(false), |_| {}).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"abcdef");
        fs::remove_dir_all(dir).unwrap();
    }

    /// 下載到一半才取消時，`.partial` 必須保留**已下載的位元組**，否則使用者
    /// 取消一次就得從零重下（網路慢時很痛）。刻意讓進度回呼在跑過至少一個
    /// 回報間隔後才設旗標，才會走到「已寫入資料後取消」這條路徑——一開始就
    /// 取消只會得到空的 `.partial`，證明不了續傳契約。
    #[test]
    fn cancelled_download_keeps_downloaded_bytes_for_resume() {
        let dir = tempdir();
        let dest = dir.join("model.bin");
        let body = vec![b'x'; 3 * 1024 * 1024];
        let mut response =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())
                .into_bytes();
        response.extend_from_slice(&body);
        let url = serve_once_bytes(response);

        let cancel = std::sync::atomic::AtomicBool::new(false);
        let error = download(&url, &dest, &digest(&body), &cancel, |p| {
            // 進度每 1MB 回報一次；收到第一次回報代表資料已落盤。
            assert!(p.downloaded > 0);
            cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        })
        .unwrap_err();

        assert!(
            error.to_string().contains(DOWNLOAD_CANCELLED),
            "取消要回報可辨識的原因，UI 才能跟一般失敗區分：{error}"
        );
        assert!(!dest.exists(), "取消不可產生成品檔");
        let kept = fs::metadata(partial_path(&dest)).unwrap().len();
        assert!(
            kept > 0 && kept < body.len() as u64,
            "`.partial` 要保留已下載的位元組供續傳，實得 {kept} bytes"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_a_mismatched_content_range_without_touching_partial() {
        let dir = tempdir();
        let dest = dir.join("model.bin");
        let partial = partial_path(&dest);
        fs::write(&partial, b"abc").unwrap();
        let url = serve_once(
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 4-5/6\r\nContent-Length: 2\r\nConnection: close\r\n\r\nef"
                .to_string(),
        );
        let error = download(&url, &dest, &digest(b"abcdef"), &std::sync::atomic::AtomicBool::new(false), |_| {}).unwrap_err();
        assert!(error.to_string().contains("resume offset mismatch"));
        assert_eq!(fs::read(&partial).unwrap(), b"abc");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_a_206_without_content_range_without_touching_partial() {
        let dir = tempdir();
        let dest = dir.join("model.bin");
        let partial = partial_path(&dest);
        fs::write(&partial, b"abc").unwrap();
        let url = serve_once(
            "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nConnection: close\r\n\r\ndef"
                .to_string(),
        );
        let error = download(&url, &dest, &digest(b"abcdef"), &std::sync::atomic::AtomicBool::new(false), |_| {}).unwrap_err();
        assert!(error.to_string().contains("missing Content-Range"));
        assert_eq!(fs::read(&partial).unwrap(), b"abc");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn finalization_atomically_renames_and_syncs_the_parent() {
        let dir = tempdir();
        let dest = dir.join("model.bin");
        let partial = partial_path(&dest);
        fs::write(&partial, b"verified bytes").unwrap();

        finalize_verified_partial(&partial, &dest).unwrap();

        assert!(!partial.exists());
        assert_eq!(fs::read(&dest).unwrap(), b"verified bytes");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn already_complete_partial_is_finalized_without_network() {
        let dir = tempdir();
        let dest = dir.join("model.bin");
        let partial = partial_path(&dest);
        let bytes = b"completed before rename";
        fs::write(&partial, bytes).unwrap();

        download("not a URL", &dest, &digest(bytes), &std::sync::atomic::AtomicBool::new(false), |_| {}).unwrap();

        assert!(!partial.exists());
        assert_eq!(fs::read(&dest).unwrap(), bytes);
        fs::remove_dir_all(dir).unwrap();
    }

    fn digest(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    }

    fn tempdir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};

        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "claro-model-test-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn serve_once(response: String) -> String {
        serve_once_bytes(response.into_bytes())
    }

    /// 二進位版本：取消測試需要送幾 MB 的 body 才會觸發進度回報。
    /// 用戶端可能中途斷線（取消），因此寫入失敗不 panic。
    fn serve_once_bytes(response: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(&response);
        });
        format!("http://{address}/model")
    }
}
