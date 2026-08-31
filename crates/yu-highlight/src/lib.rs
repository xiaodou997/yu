#![forbid(unsafe_code)]

//! 围栏代码块内部的语法着色。
//!
//! # 这个 crate 存在的理由：把 tree-sitter 关在一个盒子里
//!
//! 它是全仓唯一认识 tree-sitter 的地方。对外只有一句话——
//! **「(语言名, 代码文本) → 一串带角色的区间」**——所以 `yu-markdown` 拿到的
//! 是一个纯函数，不是一个解析器。三条理由：
//!
//! 1. **`yu-markdown` 仍然一个外部依赖都没有。** 已登记的 F3 欠账（引用标签
//!    的 case folding）要动就必须往 `yu-markdown` 自己的比较逻辑里接一个外部
//!    crate，那件事这里挡不住；而着色能整个装进一个 crate。**两件事看着都是
//!    「接受一个外部依赖」，形状不同。**
//! 2. **overview-v2 第 6.2 节的结论落在这里**：tree-sitter 只用于 fenced code
//!    内部，「那里高亮错误无害」。这个 crate 的公开面把这条边界变成类型上的
//!    事实——它拿不到 Markdown，也产不出隐藏区间。
//! 3. 语法树、query 与它们的编译代价住在一处。
//!
//! # 第二棵树活不过一次调用
//!
//! [`Highlighter::spans`] 里建树、遍历、丢掉。**跨调用留下来的只有结果**，见
//! [`Highlighter`] 上那段关于 memo 的文档。所以「两棵树怎么共存」这个问题在
//! 这一层没有内容：`yu-syntax` 的树跟着 `MarkdownDocument` 走，这一棵谁也不
//! 跟，因为它不存在到下一次。
//!
//! tree-sitter 自己的增量解析**没有用上**，这是有意的：两套增量要两份「什么
//! 变了」的答案，而块级那一份已经在 `yu-editor::DecorationCache` 里了。

use std::sync::{Mutex, OnceLock};

use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent};
use yu_core::TextRole;

/// 一段代码里被着色的区间。
///
/// `start` / `end` 是**传进来的那个 `&str` 的局部字节偏移**，不是源码坐标。
/// 这里不用 `yu_core::TextRange`：那是文档坐标的类型，拿它装局部偏移正是
/// `ShapingProvider::shape` 那条已登记的糊涂账的形状（它的 range 参数是零基
/// 局部空间，看类型看不出来）。调用方加上代码正文的起点就得到源码区间。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoleSpan {
    /// 局部起点，字节。
    pub start: usize,
    /// 局部终点，字节。
    pub end: usize,
    pub role: TextRole,
}

/// 认得的语言。
///
/// 加一种语言是**三处**：这里一个变体、[`Language::from_info`] 一行别名、
/// [`Language::config`] 一条 grammar。`tests/languages.rs` 会要求每一个变体
/// 都真的着上色，所以漏掉后两处的任何一处都会红。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    Bash,
    JavaScript,
    Json,
    Python,
    Rust,
}

impl Language {
    /// 全部认得的语言，给用例遍历。
    pub const ALL: &'static [Self] = &[
        Self::Bash,
        Self::JavaScript,
        Self::Json,
        Self::Python,
        Self::Rust,
    ];

    /// 围栏上的 info string 认到哪一种语言。
    ///
    /// 只看第一个词：CommonMark 的 info string 可以带参数
    /// （```` ```rust,ignore ````、```` ```js title=a.js ````），语言名是空白
    /// 或逗号之前那一段。大小写不敏感——`Rust` 与 `rust` 是同一件事，而这里
    /// 只在 ASCII 上折叠：语言名都是 ASCII，用不着第二份 case folding（已登记
    /// 的 F3 欠账正是那件事）。
    #[must_use]
    pub fn from_info(info: &str) -> Option<Self> {
        let word = info
            .trim_start()
            .split([' ', '\t', ',', '{', ':'])
            .next()
            .unwrap_or("");
        if word.is_empty() {
            return None;
        }
        let lower = word.to_ascii_lowercase();
        Some(match lower.as_str() {
            "bash" | "sh" | "shell" | "zsh" | "console" => Self::Bash,
            "javascript" | "js" | "jsx" | "mjs" | "cjs" => Self::JavaScript,
            "json" | "jsonc" => Self::Json,
            "python" | "py" | "python3" => Self::Python,
            "rust" | "rs" => Self::Rust,
            _ => return None,
        })
    }

    /// 这一种语言的名字，给诊断与用例用。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::JavaScript => "javascript",
            Self::Json => "json",
            Self::Python => "python",
            Self::Rust => "rust",
        }
    }

    /// 编译好的 query 配置，一个进程一份。
    ///
    /// query 编译实测 **19–28 ms/语言**，而着色一个百行代码块只要 0.4 ms。
    /// 放进调用里就是把这条路的代价整个颠倒过来，所以它必须只做一次。
    fn config(self) -> Option<&'static HighlightConfiguration> {
        macro_rules! cell {
            ($name:ident, $grammar:path, $highlights:expr, $injections:expr) => {{
                static $name: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
                $name
                    .get_or_init(|| build(self, $grammar.into(), $highlights, $injections))
                    .as_ref()
            }};
        }
        match self {
            Self::Bash => cell!(
                BASH,
                tree_sitter_bash::LANGUAGE,
                tree_sitter_bash::HIGHLIGHT_QUERY,
                ""
            ),
            Self::JavaScript => cell!(
                JAVASCRIPT,
                tree_sitter_javascript::LANGUAGE,
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::INJECTIONS_QUERY
            ),
            Self::Json => cell!(
                JSON,
                tree_sitter_json::LANGUAGE,
                tree_sitter_json::HIGHLIGHTS_QUERY,
                ""
            ),
            Self::Python => cell!(
                PYTHON,
                tree_sitter_python::LANGUAGE,
                tree_sitter_python::HIGHLIGHTS_QUERY,
                ""
            ),
            Self::Rust => cell!(
                RUST,
                tree_sitter_rust::LANGUAGE,
                tree_sitter_rust::HIGHLIGHTS_QUERY,
                tree_sitter_rust::INJECTIONS_QUERY
            ),
        }
    }
}

/// capture 名到 [`TextRole`] 的对照表。
///
/// 下标就是 `tree_sitter_highlight::Highlight` 里的那个数，所以两列必须一一
/// 对齐——[`ROLES`] 与这张表长度不等会在 [`role_of`] 里被断言拦住。
///
/// `configure` 按**最长的点分前缀**匹配，所以 `@keyword.function` 会落到
/// `keyword` 上；点后面那一级不是各家 grammar 的公共词汇，收进来只会让语言
/// 之间不一致。
const CAPTURES: &[&str] = &[
    "comment",
    "string",
    "character",
    "number",
    "keyword",
    "function",
    "type",
    "constructor",
    "constant",
    "variable",
    "property",
    "operator",
    "punctuation",
    "tag",
    "attribute",
    "label",
    "escape",
];

const ROLES: &[TextRole] = &[
    TextRole::Comment,
    TextRole::Literal,
    TextRole::Literal,
    TextRole::Number,
    TextRole::Keyword,
    TextRole::Function,
    TextRole::Type,
    TextRole::Type,
    TextRole::Constant,
    TextRole::Variable,
    TextRole::Variable,
    TextRole::Operator,
    TextRole::Punctuation,
    TextRole::Type,
    TextRole::Variable,
    TextRole::Constant,
    TextRole::Literal,
];

fn role_of(index: usize) -> TextRole {
    debug_assert_eq!(CAPTURES.len(), ROLES.len(), "两列必须一一对齐");
    ROLES.get(index).copied().unwrap_or(TextRole::Plain)
}

fn build(
    language: Language,
    grammar: tree_sitter::Language,
    highlights: &str,
    injections: &str,
) -> Option<HighlightConfiguration> {
    let mut config =
        HighlightConfiguration::new(grammar, language.name(), highlights, injections, "").ok()?;
    config.configure(CAPTURES);
    Some(config)
}

/// 上一次问过的那一份。
struct Memo {
    language: Language,
    code: String,
    spans: Vec<RoleSpan>,
}

struct Inner {
    engine: tree_sitter_highlight::Highlighter,
    memo: Option<Memo>,
}

/// 一份着色器。一个文档一个。
///
/// # 为什么带一条 memo，以及为什么只有一条
///
/// 一帧里**焦点块的装饰要重产好几次**：`EditorDocument::block_layout_for_visual_state`
/// 每个可见块都问一次 `selection_reveal_block_index()`，而它走的是未缓存的
/// `DecorationCache::decorate`。实测（5 个可见块、光标停在代码块里）稳态
/// **一帧 5 次**，真实窗口二三十个可见块就是二三十次。而一个百行代码块着色
/// 一次 0.4 ms——不留一条 memo，光标停在代码块里就是每帧十几毫秒。
///
/// **它不是 `DecorationCache` 的第三道失效门。** 那两道（Revision + range/kind、
/// 引用表指纹）是**正确性**的门：漏掉一次就画出一份对不上源码的东西。这一条
/// 是**代价**的门，键就是内容本身，所以陈旧条目在定义上不存在——文本不同就
/// 是另一个键，没有任何「什么时候该清」要回答。
///
/// 只有一条，是因为一帧里被反复问的只有**焦点块**那一个：其余块走
/// `DecorationCache` 自己的条目，各问一次。第二条要等到「一帧里有两个块反复
/// 被问」真的出现——那需要先有第二个未缓存的装饰路径。
pub struct Highlighter {
    /// `Mutex` 而不是 `RefCell`：`yu_markdown::Extension` 要求 `Send + Sync`，
    /// 而着色器是一个 extension 的字段。未争用的一次加解锁是几十纳秒，而它
    /// 挡住的一次 parse 是几十**微**秒——这笔账不用算。
    inner: Mutex<Inner>,
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Highlighter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Highlighter")
            .finish_non_exhaustive()
    }
}

impl Highlighter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                engine: tree_sitter_highlight::Highlighter::new(),
                memo: None,
            }),
        }
    }

    /// 一段代码里被着色的那些区间，按局部字节偏移升序、互不重叠。
    ///
    /// `TextRole::Plain` 的段**不回报**：它们用这一帧的正文颜色，产一条装饰
    /// 出来只会多占一个 `StyleId` 与一次 shaping。
    ///
    /// 认不出语言、grammar 装不起来、或者解析失败，都回报**空**而不是错误。
    /// 调用方对这三种情况能做的事情完全相同（照常画一段等宽文字），而把它们
    /// 变成 `Result` 只会让装饰这条路上多一种没人能处理的失败。这条降级是
    /// 安全的，理由在 overview-v2 第 6.2 节：tree-sitter 只用在 fenced code
    /// 内部，**那里高亮错误无害**——它不藏任何 source、不动任何几何。
    #[must_use]
    pub fn spans(&self, language: Language, code: &str) -> Vec<RoleSpan> {
        // 锁被一次 panic 毒化之后照常用。里面只有一条 memo 与一个 parser，
        // 最差的后果是颜色不对——而这一层本来就允许什么都不做。把它变成一次
        // panic 或一条错误，换来的是装饰这条路上多一种没人能处理的失败。
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(memo) = inner.memo.as_ref()
            && memo.language == language
            && memo.code == code
        {
            return memo.spans.clone();
        }
        let spans = compute(&mut inner.engine, language, code);
        inner.memo = Some(Memo {
            language,
            code: code.to_owned(),
            spans: spans.clone(),
        });
        spans
    }
}

fn compute(
    engine: &mut tree_sitter_highlight::Highlighter,
    language: Language,
    code: &str,
) -> Vec<RoleSpan> {
    let Some(config) = language.config() else {
        return Vec::new();
    };
    // 内嵌语言（Rust 文档注释里的 ```rust、JS 模板串里的 SQL）不下钻：回调
    // 一律给 `None`。少的是内嵌那一段的颜色，不是正确性。
    let Ok(events) = engine.highlight(config, code.as_bytes(), None, |_| None) else {
        return Vec::new();
    };
    let mut spans: Vec<RoleSpan> = Vec::new();
    // `HighlightEvent` 是一个**栈**：栈顶就是最里面那一层 capture，也就是该
    // 用的那个角色。自己按 query 里的先后再判一次优先级是第二份实现，而
    // `tree-sitter-highlight` 正是这件事的参照实现。
    let mut stack: Vec<usize> = Vec::new();
    for event in events {
        let Ok(event) = event else {
            return Vec::new();
        };
        match event {
            HighlightEvent::HighlightStart(highlight) => stack.push(highlight.0),
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                if start >= end {
                    continue;
                }
                let role = stack.last().copied().map_or(TextRole::Plain, role_of);
                if role == TextRole::Plain {
                    continue;
                }
                // 相邻同角色的两段并成一段：`Source` 事件会被 capture 的
                // 起止切碎，不并的话一个标识符可能产出好几条一模一样的装饰。
                match spans.last_mut() {
                    Some(last) if last.role == role && last.end == start => last.end = end,
                    _ => spans.push(RoleSpan { start, end, role }),
                }
            }
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::{CAPTURES, Language, ROLES};

    #[test]
    fn capture_names_and_roles_stay_aligned() {
        assert_eq!(CAPTURES.len(), ROLES.len());
    }

    #[test]
    fn info_string_takes_the_first_word_only() {
        assert_eq!(Language::from_info("rust"), Some(Language::Rust));
        assert_eq!(Language::from_info("RUST"), Some(Language::Rust));
        assert_eq!(Language::from_info("rust,ignore"), Some(Language::Rust));
        assert_eq!(Language::from_info("rust no_run"), Some(Language::Rust));
        assert_eq!(
            Language::from_info("  js title=a.js"),
            Some(Language::JavaScript)
        );
        assert_eq!(Language::from_info("js{1,3}"), Some(Language::JavaScript));
        assert_eq!(Language::from_info(""), None);
        assert_eq!(Language::from_info("   "), None);
        assert_eq!(Language::from_info("brainfuck"), None);
        // 语言名不是前缀匹配：`rustic` 不是 Rust。
        assert_eq!(Language::from_info("rustic"), None);
    }
}
