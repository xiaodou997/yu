//! 排版样式的标识与词汇。
//!
//! # 为什么这三个 id 住在 `yu-core`
//!
//! `StyleId` / `LineStyleId` / `WidgetId` 是**装饰层与布局层之间的共用词汇**：
//! `yu-decoration` 产出它们，`yu-layout` 消费它们，而这两个 crate 互不依赖
//! （不变量 E2）。共用词汇只能住在两者的共同下游，也就是这里。
//!
//! # 颜色也在这条路上（S7 第五刀）
//!
//! [`TextAttrs`] 多了一个 [`TextRole`]。它**不是颜色**——具体的 RGBA 由
//! `yu-workspace` 那一层给（那里已经写着「产品选色住在这一层，不住在场景
//! 层」）。这一层与 `yu-layout` 拿到的仍然只有「等宽、1.0 倍、Keyword 角
//! 色」，拿不到「这是 Rust 的 `fn`」。
//!
//! 为什么颜色必须与字型挤在**同一个** [`StyleId`] 上：`yu-editor::marks` 的
//! `winner_over` 是「最窄的 Mark 赢，而且只赢一个」，Mark 不叠加。代码块整段
//! 有一条 `Code` 的 Mark，token 的 Mark 更窄会把它整个盖掉——token 那份属性
//! 不自带 `Code` 的话，高亮的字会掉出等宽字体，不报错。
//!
//! 它们最初定义在 `yu-decoration`。挪过来的理由与 S3 的 `VisualOffset`、
//! S4 的 `SourceCaretPosition` 相同：纯类型归 `yu-core`，逻辑留在原处。
//! `yu-decoration` 原样再导出，它的公开面不变。
//!
//! # 谁解释这些 id
//!
//! 这一层**不解释**。`StyleId(3)` 在这里没有含义，只有「两个相同的 id 该被
//! 同样对待」。解释权归产出装饰的那个 extension：`yu-markdown` 知道
//! `StyleId(3)` 是强调，于是由它提供 [`StyleId`] → [`TextAttrs`] 的表。
//! [`TextAttrs`] 是一套**排版**词汇（字型、字号倍率），不是 Markdown 词汇——
//! 这正是不变量 E1 要求的边界：`yu-layout` 拿到的是「斜体、1.0 倍字号」，
//! 不是「这是强调」。

/// 样式表里的一项。具体内容由上层解释，这一层只搬运标识。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StyleId(pub u32);

/// 一个视觉物件的标识。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WidgetId(pub u32);

/// 整行/整块样式的标识。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineStyleId(pub u32);

/// 空 range 上的 widget 落在光标的哪一侧。
///
/// 非空 range 的 widget 用不到它——那种情况下 widget 覆盖并隐藏了一段 source，
/// 位置没有歧义。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WidgetSide {
    Before,
    After,
}

/// 一段文本的字型请求。
///
/// 这个枚举此前叫 `VisualRunStyle`，定义在 `yu-projection` 里，于是 `yu-font`
/// 为了实现 `ClusterMetrics` 不得不反向依赖投影层。它描述的是「这段文本按什么
/// 字型排」，与 Markdown 无关：斜体、粗体、等宽是排版概念，任何来源的文本都
/// 可能带上它们。
///
/// 它是 shaping 后端的输入（见 [`crate::ClusterMetrics`] 与
/// [`crate::ShapingProvider`]），所以留在这里而不是并进 [`StyleId`]：
/// `yu-font` 只依赖 `yu-core`。[`TextAttrs`] 是它加上字号倍率之后的完整形态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextStyle {
    #[default]
    Plain,
    Emphasis,
    Strong,
    Code,
}

/// 一段文字在配色里扮演的角色。
///
/// 与 [`TextStyle`] 分开是因为两者答的不是一个问题：`TextStyle` 决定**按哪
/// 个字面排**（shaper 的输入，影响几何），`TextRole` 决定**上什么颜色**（不
/// 影响任何几何）。代码块里的关键字两样都要：`Code` 的字面加 `Keyword` 的
/// 颜色。
///
/// 这一层不解释成 RGBA。角色名取自 tree-sitter 的 capture 命名约定
/// （`@keyword` / `@string` / …），那套名字是各家 grammar 的公共词汇，
/// 不是某一种语言的概念。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextRole {
    /// 不着色，用这一帧的正文颜色。
    #[default]
    Plain,
    Keyword,
    /// 字符串与字符字面量。
    Literal,
    Number,
    Comment,
    /// 函数名与宏名。
    Function,
    /// 类型名。
    Type,
    /// 常量与内置值（`true` / `None` / `nil`）。
    Constant,
    /// 变量、字段、参数。
    Variable,
    Operator,
    /// 括号、分号、逗号。
    Punctuation,
}

/// 一个 [`StyleId`] 解释之后的排版属性。
///
/// `size_scale` 是相对 [`crate::ClusterMetrics`] 基准字号的倍率。标题靠它变大，
/// 而 `yu-layout` 只看见「1.6 倍」，看不见「这是二级标题」。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextAttrs {
    style: TextStyle,
    size_scale: f32,
    role: TextRole,
}

impl TextAttrs {
    /// 基准字号下的一种字型。
    #[must_use]
    pub const fn new(style: TextStyle) -> Self {
        Self {
            style,
            size_scale: 1.0,
            role: TextRole::Plain,
        }
    }

    /// 带字号倍率。非有限或非正的倍率被拒绝——布局里一个 NaN 宽度会一路传播
    /// 成不 panic 的错画面，这是本项目最危险的失败模式。
    #[must_use]
    pub fn with_size_scale(mut self, size_scale: f32) -> Option<Self> {
        if !size_scale.is_finite() || size_scale <= 0.0 {
            return None;
        }
        self.size_scale = size_scale;
        Some(self)
    }

    /// 带配色角色。
    ///
    /// 与 [`Self::with_size_scale`] 不同，这里没有可以拒绝的值——角色是一个
    /// 枚举，不是一个可能是 NaN 的数。
    #[must_use]
    pub const fn with_role(mut self, role: TextRole) -> Self {
        self.role = role;
        self
    }

    #[must_use]
    pub const fn style(self) -> TextStyle {
        self.style
    }

    #[must_use]
    pub const fn role(self) -> TextRole {
        self.role
    }

    #[must_use]
    pub const fn size_scale(self) -> f32 {
        self.size_scale
    }
}

impl Default for TextAttrs {
    fn default() -> Self {
        Self::new(TextStyle::Plain)
    }
}

#[cfg(test)]
mod tests {
    use super::{TextAttrs, TextRole, TextStyle};

    #[test]
    fn size_scale_rejects_non_finite_and_non_positive() {
        assert!(TextAttrs::default().with_size_scale(f32::NAN).is_none());
        assert!(TextAttrs::default().with_size_scale(0.0).is_none());
        assert!(TextAttrs::default().with_size_scale(-1.0).is_none());
        assert!(
            TextAttrs::default()
                .with_size_scale(f32::INFINITY)
                .is_none()
        );
        let scaled = TextAttrs::new(TextStyle::Strong)
            .with_size_scale(2.0)
            .expect("2.0 有效");
        assert_eq!(scaled.style(), TextStyle::Strong);
        assert_eq!(scaled.size_scale(), 2.0);
    }

    /// 角色是**加**在字型上的，不是替掉字型。
    ///
    /// 这条断言压的是代码高亮那一刀最容易犯的错：`winner_over` 只让最窄的
    /// 那条 Mark 赢，token 那份属性丢掉 `Code` 就意味着高亮的字掉出等宽字体。
    #[test]
    fn role_is_added_on_top_of_the_typeface_not_instead_of_it() {
        let plain = TextAttrs::new(TextStyle::Code);
        assert_eq!(plain.role(), TextRole::Plain);
        let keyword = plain.with_role(TextRole::Keyword);
        assert_eq!(keyword.style(), TextStyle::Code);
        assert_eq!(keyword.size_scale(), 1.0);
        assert_eq!(keyword.role(), TextRole::Keyword);
        // 角色不同的两份属性不相等——样式表按相等去重，合并了就是同一个号。
        assert_ne!(plain, keyword);
    }
}
