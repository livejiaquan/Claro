//! macOS 硬體分級與資源策略。
//!
//! 決策必須是純函數，讓 8/16/32 GB 與 Intel 路徑可以在 CI 模擬；
//! 實機只負責讀取實體記憶體與目前執行架構。

use serde::Serialize;

const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HardwareProfile {
    pub architecture: String,
    pub memory_gb: u64,
    pub tier: &'static str,
    pub low_memory_mode: bool,
    pub keep_models_warm: bool,
    pub recommended_stt: &'static str,
    pub recommended_llm_provider: &'static str,
    pub recommended_llm_model: &'static str,
    pub reason: String,
}

pub fn profile(apple_status: i32) -> HardwareProfile {
    let architecture = std::env::consts::ARCH.to_string();
    let memory_bytes = physical_memory_bytes();
    let memory_gb = ((memory_bytes + GIB - 1) / GIB).max(1);
    policy(memory_gb, &architecture, apple_status)
}

pub fn low_memory_mode() -> bool {
    profile(crate::polish::apple_status()).low_memory_mode
}

pub fn policy(memory_gb: u64, architecture: &str, apple_status: i32) -> HardwareProfile {
    let apple_silicon = matches!(architecture, "aarch64" | "arm64");
    let low_memory_mode = memory_gb <= 12 || !apple_silicon;
    let (tier, recommended_stt) = if low_memory_mode {
        ("compact", "large-v3-turbo-q5_0")
    } else if memory_gb >= 24 {
        ("performance", "large-v3-turbo")
    } else {
        ("balanced", "large-v3-turbo")
    };
    let recommended_llm_provider = if apple_silicon && apple_status == 0 {
        "apple"
    } else {
        "builtin"
    };
    let reason = if recommended_llm_provider == "apple" {
        format!("{memory_gb} GB Apple Silicon：語音模型採 {tier} 配置，文字整理交由 macOS 端上模型")
    } else if low_memory_mode {
        format!(
            "{memory_gb} GB {}：使用量化語音模型，轉錄後先釋放 STT 再載入 Claro 內建整理模型",
            if apple_silicon {
                "Apple Silicon"
            } else {
                "Intel"
            }
        )
    } else {
        format!("{memory_gb} GB Apple Silicon：使用平衡語音模型與 Claro 內建整理模型")
    };

    HardwareProfile {
        architecture: architecture.to_string(),
        memory_gb,
        tier,
        low_memory_mode,
        keep_models_warm: !low_memory_mode,
        recommended_stt,
        recommended_llm_provider,
        recommended_llm_model: "qwen3-4b-instruct-2507",
        reason,
    }
}

#[cfg(target_os = "macos")]
fn physical_memory_bytes() -> u64 {
    use std::ffi::CString;

    let Ok(name) = CString::new("hw.memsize") else {
        return 8 * GIB;
    };
    let mut value: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            &mut value as *mut u64 as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if status == 0 && size == std::mem::size_of::<u64>() && value > 0 {
        value
    } else {
        8 * GIB
    }
}

#[cfg(not(target_os = "macos"))]
fn physical_memory_bytes() -> u64 {
    8 * GIB
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_gb_uses_quantized_stt_and_sequential_models() {
        let p = policy(8, "aarch64", 2);
        assert_eq!(p.recommended_stt, "large-v3-turbo-q5_0");
        assert_eq!(p.recommended_llm_provider, "builtin");
        assert!(p.low_memory_mode);
        assert!(!p.keep_models_warm);
    }

    #[test]
    fn apple_intelligence_is_preferred_when_available() {
        let p = policy(16, "aarch64", 0);
        assert_eq!(p.recommended_stt, "large-v3-turbo");
        assert_eq!(p.recommended_llm_provider, "apple");
        assert!(!p.low_memory_mode);
    }

    #[test]
    fn intel_stays_on_the_compact_resource_policy() {
        let p = policy(32, "x86_64", 1);
        assert_eq!(p.recommended_stt, "large-v3-turbo-q5_0");
        assert_eq!(p.recommended_llm_provider, "builtin");
        assert!(p.low_memory_mode);
    }
}
