//! 測試工具：列出 cpal 眼中的輸入裝置
fn main() {
    println!("default: {:?}", claro_lib::audio::default_input_name());
    for d in claro_lib::audio::list_input_devices() {
        println!("- {d}");
    }
}
