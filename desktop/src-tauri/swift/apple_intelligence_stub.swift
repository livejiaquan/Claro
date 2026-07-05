// Stub：SDK 沒有 FoundationModels（macOS 26 SDK 之前）時由 build.rs 選用，
// 讓同一套 Rust 程式碼在舊環境照常編譯；runtime 一律回報「系統過舊」。

import Foundation

private func dupCString(_ text: String) -> UnsafeMutablePointer<CChar>? {
    text.withCString { strdup($0) }
}

@_cdecl("claro_apple_ai_status")
public func claroAppleAiStatus() -> Int32 {
    return 4 // 系統過舊
}

@_cdecl("claro_apple_ai_polish")
public func claroAppleAiPolish(
    _ instructions: UnsafePointer<CChar>,
    _ prompt: UnsafePointer<CChar>
) -> UnsafeMutablePointer<CChar>? {
    _ = instructions
    _ = prompt
    return dupCString(#"{"err":"需要 macOS 26 以上"}"#)
}

@_cdecl("claro_apple_ai_free")
public func claroAppleAiFree(_ ptr: UnsafeMutablePointer<CChar>?) {
    if let ptr { free(ptr) }
}
