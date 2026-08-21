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
//! # 与 `yu-projection` 的关系
//!
//! `yu-projection` 是 v1 的实现，S4 结束时删除。在那之前两者并存，由
//! `tests/projection_differential.rs` 在真实文档上逐点比对——一个已经在
//! 产品里跑着的实现是比自证性质更强的 oracle，S3 的解析器就没有这个条件。

mod decoration;
mod hidden;
mod set;

pub use decoration::{Decoration, DecorationRange, LineStyleId, StyleId, WidgetId, WidgetSide};
pub use hidden::Bias;
pub use set::{DecorationSet, MapError};
