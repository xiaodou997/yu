#![forbid(unsafe_code)]

//! 增量 Markdown 语法树。
//!
//! 本 crate 是 `@lezer/markdown` 1.5.0（MIT，Marijn Haverbeke）算法的 Rust
//! 移植。选它的理由见 `docs/architecture/overview-v2.md` 第 6.2 / 6.3 节：
//! tree-sitter-markdown 的作者明说它「不建议用在正确性重要的地方」，而 Yu
//! 解析错等于投影错、隐藏错、编辑落到错误的 source range。
//!
//! # 这一层不知道什么
//!
//! 不变量 E1 把 Markdown 语义限制在 `yu-markdown`。**本 crate 在那条线的下方
//! 一层**：它认识 Markdown 语法，但只到「语法树」为止——不产出 decoration、
//! 不知道样式、不知道任何视觉概念（第 4.3 节给 `yu-syntax` 的禁止项）。
//!
//! # 与上游的差异
//!
//! 移植不是照抄。以下差异都是有意的，并且都不改变解析结果：
//!
//! - **不支持多 range 解析。** lezer 的 `ranges` / `injectGaps` / `toRelative`
//!   是给 `parseMixed` 混合语言用的（在一份输入里只解析若干不连续片段）。Yu
//!   的 fenced code 高亮走 tree-sitter 旁路，不经过这里。
//! - **树的表示不同**（`Arc` 持久树、无 `TreeBuffer`、无 balance）。
//! - **不移植 `PartialParse` 的分步 `advance()`。** 那是为了在浏览器主线程上
//!   切片解析；Yu 的解析跑在不可变 Snapshot 上，不占主线程。
//! - **不移植 `configure` 扩展机制。** 它服务的是 `yu-markdown` 的 extension，
//!   在 S6 落地时再建；现在建只会得到一份没有使用者也没有测试的抽象。
//! - **引用链接的成立与否不在这里判定**（不变量 C6）。parser 只产出候选
//!   `LinkReference` / `LinkLabel` 节点，是否成立由同 Revision 的 reference
//!   table 在装饰阶段决定。这一条**修正**了 lezer 自己声明的偏差。

mod block;
mod element;
mod fragment;
mod inline;
mod input;
mod node;
mod tree;

pub use fragment::{FragmentChange, TreeFragment};
pub use input::Input;
pub use node::NodeKind;
pub use tree::{Tree, TreeCursor};

/// 解析失败的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// 源码超过 `u32::MAX` 字节。
    ///
    /// 树里的位置是 32 位的（一份文档有几十万个节点，每个多 4 字节不是小数）。
    /// 4 GiB 的上限与 `docs/architecture/overview-v2.md` 第 6.4 节
    /// 「百万行以内无问题，GB 级文件另立方案」一致，超出时明确拒绝而不是
    /// 悄悄截断。
    SourceTooLarge,
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SourceTooLarge => write!(formatter, "源码超过 4 GiB，超出语法树的位置宽度"),
        }
    }
}

impl core::error::Error for ParseError {}

/// 一次解析的结果。
#[derive(Clone, Debug)]
pub struct Parse {
    tree: Tree,
    reparsed_bytes: u32,
}

impl Parse {
    /// 语法树，根节点是 `Document`。
    #[must_use]
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    #[must_use]
    pub fn into_tree(self) -> Tree {
        self.tree
    }

    /// 本次解析实际重新扫描过的源码字节数。
    ///
    /// 这是不变量 J1「编辑只重解析受影响范围」的**可断言量**。选它而不是
    /// 耗时，是因为它对同样的输入永远给同样的答案：耗时会随机器和负载浮动，
    /// 拿它当门禁只会得到一条时不时变红的检查，然后被调松到失去意义。
    #[must_use]
    pub fn reparsed_bytes(&self) -> u32 {
        self.reparsed_bytes
    }
}

/// 全量解析。
///
/// # Errors
///
/// 源码超过 `u32::MAX` 字节时返回 [`ParseError::SourceTooLarge`]。
pub fn parse<I: Input + ?Sized>(input: &I) -> Result<Parse, ParseError> {
    parse_with_fragments(input, &[])
}

/// 带 fragment 复用的解析。
///
/// `fragments` 由 [`TreeFragment::from_tree`] 与 [`TreeFragment::apply_changes`]
/// 产出。传空切片等价于 [`parse`]。
///
/// 不变量 C3 要求 `parse_with_fragments(new, fragments) == parse(new)`，
/// 这一条由差分测试守护，不由人工推理保证。
///
/// # Errors
///
/// 源码超过 `u32::MAX` 字节时返回 [`ParseError::SourceTooLarge`]。
pub fn parse_with_fragments<I: Input + ?Sized>(
    input: &I,
    fragments: &[TreeFragment],
) -> Result<Parse, ParseError> {
    if input.len_bytes() == u32::MAX {
        return Err(ParseError::SourceTooLarge);
    }
    let cursor = (!fragments.is_empty()).then(|| fragment::FragmentCursor::new(fragments));
    let (tree, reparsed_bytes) = block::BlockContext::new(input, cursor).parse();
    Ok(Parse {
        tree,
        reparsed_bytes,
    })
}
