#![forbid(unsafe_code)]

//! 编辑状态：历史、选区、caret 绑定、输入法组合。
//!
//! # 这个 crate 收的是什么
//!
//! `docs/architecture/overview-v2.md` 第 8 节给 S4 的任务是「`yu-state` 收敛
//! EditorState / Transaction / Facet / History」。实际收进来的与那句话有两处
//! 出入，都在第 8 节里写明了理由：
//!
//! - **`Transaction` 不在这里，它留在 `yu-text`。** 它是文本编辑的原语，
//!   不是编辑器状态——`yu-text` 的 `TextBuffer::apply` 就以它为输入。往上搬
//!   会让 `yu-text` 反过来依赖 `yu-state`。
//! - **`Facet` 没有建。** 它零消费者，而 S4 的两条验收标准都不涉及它。
//!   真实的配置聚合需求要等 S6 的 extension 化才出现。S3 就是为同样的理由
//!   没有移植 lezer 的 `configure`。
//!
//! # 这里为什么没有布局
//!
//! `yu-editor` 里那 4,266 行 `document.rs` 的公开方法大量是 `block_layout_*`
//! 的组合，那是布局入口，属于 S5。搬进来的四个模块的依赖只有 `yu-core` 与
//! `yu-text`，一个布局或投影类型都没有——这是搬迁前逐个文件核对过的，也是
//! 这条边界画在这里的依据。

mod caret;
mod composition;
mod history;
mod selection;

pub use caret::{CaretPositionError, CaretPositionMap};
pub use composition::{CompositionError, CompositionOverlay};
pub use history::{EditorHistory, HistoryEntry, HistoryGroup, HistoryStats};
pub use selection::{EditorSelection, SelectionError};
