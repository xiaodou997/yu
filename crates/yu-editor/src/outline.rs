//! 文档大纲：标题的层级视图。
//!
//! # 为什么不建在 `AccessibilitySemanticSnapshot` 上
//!
//! 那棵语义树里也有 Heading 节点与 label 区间，看上去大纲当它的第二个消费者
//! 就行。**不这么做**，理由是三者共享的「唯一实现」（D4）本来就不在语义树
//! 里，而在更下面一层：「哪些块是标题、几级、正文在哪」由
//! [`BlockKind::Heading`] 与 [`yu_markdown::heading_content_range`] 定义，
//! 语义树自己是它的第一个消费者，`yu-export` 是第二个，大纲是第三个。三份
//! 派生视图共用同一份定义，D4 要的唯一性已经满足；再把大纲叠在语义树上，
//! 唯一性一点没多，耦合多了一层。
//!
//! 而它们的定义域是**结构性**地不一样，不只是「语义树多了行内节点」：
//!
//! - **语义树是扁平的**——每个块节点的 parent 都是 Document(0)。大纲的全部
//!   内容恰恰是层级：`##` 挂在它上面最近的 `#` 下。让语义树长出标题嵌套会
//!   改掉 `parent` 字段对 VoiceOver 的含义，那是一次 ABI 变更，换来的只是
//!   大纲少写一个循环。
//! - **坐标不一样**。语义树的区间是 UTF-16、与 Revision 绑定，因为它服务的
//!   是 `NSTextInputClient` / AX 的 ABI。大纲是源码坐标的派生视图，UTF-16
//!   只在 FFI 边界上出现一次。
//! - **代价不一样**。`AccessibilitySemanticSnapshot::from_document` 会对每个
//!   非代码块跑一次 `parse_inline_with_definitions`。大纲只要标题，全文行内
//!   解析是纯浪费。
//!
//! 反过来说，把大纲建在语义树上，就要往语义树里塞进块索引与标题嵌套两样
//! 只有大纲需要的东西——把两个消费者的需求焊在一个类型上，正是这个项目
//! 反对的那件事。

use yu_core::{Revision, TextRange};
use yu_markdown::BlockKind;

use crate::EditorDocument;

/// 大纲里的一条标题。
///
/// `parent` 指向**上一级标题**在同一份快照里的序号，不是块索引；根级标题
/// 没有 parent。`block` 才是块索引，导航要用它。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutlineItem {
    index: usize,
    parent: Option<usize>,
    level: u8,
    block: usize,
    source_range: TextRange,
    label_range: TextRange,
}

impl OutlineItem {
    /// 这一条在 [`OutlineSnapshot::items`] 里的序号，也是 `parent` 的取值域。
    #[must_use]
    pub const fn index(self) -> usize {
        self.index
    }

    /// 上一级标题的 [`Self::index`]，根级标题为 `None`。
    #[must_use]
    pub const fn parent(self) -> Option<usize> {
        self.parent
    }

    /// 标题级别，1..=6。
    #[must_use]
    pub const fn level(self) -> u8 {
        self.level
    }

    /// 这条标题所在的块索引。导航按它走：把选区放到
    /// [`Self::label_range`] 的起点，再交给 viewport 那条路算滚动。
    #[must_use]
    pub const fn block(self) -> usize {
        self.block
    }

    /// 标题块的整段源码区间，含 `#` 前缀或 Setext 的下划线那一行。
    #[must_use]
    pub const fn source_range(self) -> TextRange {
        self.source_range
    }

    /// 标题正文的源码区间：大纲里显示的就是这一段。
    #[must_use]
    pub const fn label_range(self) -> TextRange {
        self.label_range
    }
}

/// 与一个 Revision 绑定的标题层级视图。
///
/// 只持有紧凑的元数据与源码区间，不复制正文（C4）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineSnapshot {
    revision: Revision,
    items: Vec<OutlineItem>,
}

impl OutlineSnapshot {
    /// 按文档顺序扫一遍块，把标题串成层级。
    ///
    /// 层级规则是「就近挂靠」：一条标题挂在它前面最近的、级别严格更小的那
    /// 条标题下。`###` 之后来一个 `#`，`#` 不会变成 `###` 的孩子；文档以
    /// `###` 开头时它就是根级。跳级（`#` 之后直接 `###`）保留原级别，不压
    /// 平成 `##`——大纲报的是文档写成什么样，不是它该写成什么样。
    #[must_use]
    pub fn from_document(document: &EditorDocument) -> Self {
        let markdown = document.markdown();
        let revision = markdown.revision();
        let mut items: Vec<OutlineItem> = Vec::new();
        // 祖先链：(序号, 级别)，级别自底向上严格递减。
        let mut ancestors: Vec<(usize, u8)> = Vec::new();

        for (block, markdown_block) in markdown.blocks().into_iter().enumerate() {
            let BlockKind::Heading { level } = markdown_block.kind() else {
                continue;
            };
            while ancestors
                .last()
                .is_some_and(|(_, ancestor_level)| *ancestor_level >= level)
            {
                ancestors.pop();
            }
            let index = items.len();
            items.push(OutlineItem {
                index,
                parent: ancestors.last().map(|(ancestor, _)| *ancestor),
                level,
                block,
                source_range: markdown_block.range(),
                // 「正文在哪」只有一个答案，由语法树给：ATX 的 `#` 前缀与收尾
                // 的 ` ##`、Setext 的下划线都不在里面。
                label_range: yu_markdown::heading_content_range(markdown, markdown_block),
            });
            ancestors.push((index, level));
        }

        Self { revision, items }
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub fn items(&self) -> &[OutlineItem] {
        &self.items
    }

    #[must_use]
    pub fn item(&self, index: usize) -> Option<OutlineItem> {
        self.items.get(index).copied()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }
}
