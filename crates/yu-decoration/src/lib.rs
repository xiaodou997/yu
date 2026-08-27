#![forbid(unsafe_code)]

//! 装饰集合：视觉表现的唯一来源。
//!
//! 不变量 D1 规定「隐藏语法字符」「替换为控件」「改变样式」都必须表达成
//! 一条 [`Decoration`]，不得在 layout 或 scene 里开特殊分支。这个 crate
//! 提供承载它们的不可变集合，以及 source ↔ visual 的双向映射。
//!
//! # 这一层不知道 Markdown
//!
//! 第 4.3 节给 `yu-decoration` 的禁止项是「知道 Markdown」。这里的
//! `StyleId(3)` 不是「加粗」，只是一个上层会解释的标识；`Replace` 不知道
//! 自己盖住的是 `##` 还是别的什么。
//!
//! # 映射曾经有一个 oracle
//!
//! v1 的 `yu-projection` 是一份已经在产品里跑着的 source↔visual 实现。
//! S4 建这个 crate 时拿它当 oracle 逐点比对过（`projection_differential.rs`,
//! 76 份真实 Markdown × 每个偏移 × 两种 bias），S6 换完消费者之后它连同那条
//! 差分一起删掉了。
//!
//! **留下的是什么。** 「O(log n) 的树下降」由 `hidden.rs` 里的线性参照实现
//! 逐点压着——两份独立的推理互相校验，比 round-trip 那种自证性质强。
//! 「哪些字节该被隐藏」现在由 `yu-markdown` 的 extension 回答，它的 oracle
//! 是 CommonMark 的官方用例（不变量 C7）。

mod decoration;
mod hidden;
mod set;

pub use decoration::{Decoration, DecorationRange, LineStyleId, StyleId, WidgetId, WidgetSide};
pub use hidden::Bias;
pub use set::{DecorationSet, MapError, MergeError};
