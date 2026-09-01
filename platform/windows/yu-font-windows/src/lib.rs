#![forbid(unsafe_op_in_unsafe_fn)]

//! Windows DirectWrite 字体后端：`ShapingProvider` + `GlyphRasterizer`。
//!
//! 与 `yu-font-macos` 同层同形：各自实现同一组 trait，互相不认识。共享的
//! 契约写在 `yu_core::ShapingProvider` 上（不变量 E7），可执行的那一份是
//! `yu_core::shaping_conformance`。
//!
//! # 这个 crate 的判据分两半，只有一半在开发机上关得掉
//!
//! 本仓的开发机是 macOS，**交叉编译不了、也没有 DirectWrite**。所以：
//!
//! - **[`cluster`] 那一半在哪儿都能验**：`clusterMap` 反转是纯函数，合成
//!   输入随便造。这是 S7 第七刀 b 的教训在第二端的直接应用——真实后端造不出
//!   让几种取法分开的输入，判据必须落在纯函数上。
//! - **DirectWrite 调用那一半只有 Windows CI 算数**：本机能做到的上限是
//!   `cargo check --target x86_64-pc-windows-msvc`（类型与签名），跑不了。
//!
//! 因此这个 crate 里凡是标着「实测」的都来自 [`cluster`] 的用例；凡是关于
//! DirectWrite 运行时行为的都**未在本仓库核实**，不要当已验证事实用。
//!
//! # CTLine 顺手做的三件事，DirectWrite 不做
//!
//! script 分段、bidi、字体 fallback 分段——CoreText 在 `CTLine` 那一层就做完
//! 了，`shape_with_core_text` 拿到的是**已经分好**的 `CTRun`。DirectWrite 的
//! `IDWriteTextAnalyzer` 只给分析结果，分段要自己拼。E5（断行归共享 Rust）
//! 在这里帮了大忙：Yu 不用 `IDWriteTextLayout`，两端职责一样。

pub mod cluster;
pub mod run;

pub use cluster::{ClusterMapError, GlyphSpan, glyph_spans};
pub use run::{RunAssemblyError, ShapedArrays, assemble_run};
