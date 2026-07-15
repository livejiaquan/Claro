//! 測試工具：印出目前前景視窗的 AX 上下文與抽出的詞彙偏置（無聲驗證）。
//! 用法：cargo run --example show_context（跑在有輔助使用權限的終端機）

fn main() {
    match claro_lib::context::capture() {
        Some(ctx) => {
            println!("── 擷取到的上下文（{} 字） ──", ctx.chars().count());
            println!("{ctx}");
            let dict = vec!["PyTorch".to_string()];
            let terms = claro_lib::context::context_terms(&dict, &ctx, 15);
            println!("── 詞彙偏置（{}） ──", terms.len());
            println!("{}", terms.join("、"));
            println!("── initial_prompt ──");
            println!(
                "{}",
                claro_lib::stt::build_initial_prompt(&terms).unwrap_or_default()
            );
        }
        None => println!("擷取不到上下文（沒有輔助使用權限？）"),
    }
}
