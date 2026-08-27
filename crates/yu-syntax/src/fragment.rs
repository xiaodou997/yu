//! 增量复用：把上一棵树里没被编辑碰到的部分整段搬进新树。
//!
//! 移植自 `@lezer/common` 的 `TreeFragment` 与 `@lezer/markdown` 的
//! `FragmentCursor`。
//!
//! # 为什么复用只发生在块边界
//!
//! 块解析器是单遍不回溯的，它在任意时刻的全部状态就是「容器栈」。因此只要
//! 满足两个条件，一棵旧子树就可以原样搬过来：
//!
//! 1. 它覆盖的字节没有被这次编辑改动（由 [`TreeFragment::apply_changes`] 保证）；
//! 2. 它当初所处的容器上下文与现在相同（由 context hash 比较保证）。
//!
//! 只比第 1 条是不够的。`foo` 在文档顶层是 Paragraph，在 `> ` 里面是
//! Blockquote 的孩子——字节一样，树不一样。这正是「静默地做错事」的典型：
//! 复用一个上下文不符的子树不会 panic，只会让某个块的类型悄悄错掉。
//!
//! # 与上游的分歧之二：`open_end` 是沿用的，不是每次重写的
//!
//! （分歧之一是上游 `FragmentCursor` 压根不读 `open_start`，说明写在
//! `FragmentCursor::move_to` 里。）
//!
//! 上游 `@lezer/common` 切 fragment 时无条件写 `openStart: cI > 0` 与
//! `openEnd: !!nextC`——**只看这一次的改动，把传进来的 fragment 已经带着的
//! 标记丢掉**。
//!
//! 每次编辑之后都重新 `from_tree` 时这不要紧：那时标记本来就都是 `false`。
//! 但连着编辑两次而中间没有重新解析（批量替换、连续粘贴，或者编辑器那边没
//! 人要过树）时，第二次 [`TreeFragment::apply_changes`] 会把**上一次编辑留下
//! 的那个洞**的边界说成「文档末尾」，于是紧挨着洞的那个块被原样复用。
//!
//! 复现见 `tests/incremental.rs` 的 `chained_edits_keep_earlier_holes_open`：
//! 先在一个空行处插入 `X`，让上一个段落把这一行吃成延续行；再在它前面某处插
//! 一个字符，中途不解析。第二次解析会把那个段落从洞的位置一刀两断，得到两个
//! 段落而不是一个——不 panic、不报错，正是不变量 C3 要防的那种静默出错。
//!
//! ## 为什么只有 `open_end` 要沿用
//!
//! 起点那一端照上游写就是对的，**沿用与否不可能有区别**：
//!
//! - 上游写出 `open_start == false` 只有一处：`cI == 0` 那一轮。而那一轮
//!   `pos == 0`、`offset == 0`，于是切出来的 `from == candidate.from`。
//!   （另一条路是整段原样保留，`from` 与标记一起继承，同样不破坏下面这条。）
//! - 于是有不变量：**`open_start == false` 的 fragment 必然 `from == 0`**，
//!   基例是 [`TreeFragment::from_tree`] 造的那一个。
//! - 沿用与不沿用只在 `pos < candidate.from` 且 `candidate.open_start == false`
//!   时给出不同答案，而按上一条那要求 `pos < 0`。不存在。
//!
//! 终点那一端没有这个保护，因为右边的哨兵是 `u32::MAX`——没有 fragment 的
//! `to` 能等于它，所以「终点来自 fragment 自己」与「终点来自一次改动」是两种
//! 都会发生的情形。不对称就是从这里来的。
//!
//! 这不是推理出来就算数的：`open_start` 沿用与否的两种写法，
//! `chained_fragments_match_full_parse_through_random_edits` 的 1,000 步随机
//! 链式编辑分不出来——与上面的证明一致。所以那一端**不改**。

use crate::block::BlockContext;
use crate::input::Input;
use crate::tree::{Tree, TreeCursor};

/// 上一次解析结果中，本次编辑没有触及的一段。
#[derive(Clone, Debug)]
pub struct TreeFragment {
    /// 未变化区间在**新文档**中的起点。
    from: u32,
    /// 未变化区间在新文档中的终点。
    to: u32,
    /// 这一段所属的旧树（整棵，不是切片）。
    tree: Tree,
    /// 旧位置 = 新位置 + `offset`。
    offset: i64,
    /// 起点是不是一次改动的右边界（而非解析的起点）。
    ///
    /// 这个字段决定复用能不能从 fragment 的第一个字节开始，见
    /// [`FragmentCursor::move_to`]。上游 `@lezer/common` 定义了它，
    /// 但 `@lezer/markdown` 的 `FragmentCursor` 从不读它——那是个真 bug，
    /// 说明见 `move_to`。
    open_start: bool,
    /// 终点是不是一次改动的左边界（而非文档末尾）。
    open_end: bool,
}

/// 一次编辑在旧/新两套坐标下的区间。
#[derive(Clone, Copy, Debug)]
pub struct FragmentChange {
    pub from_old: u32,
    pub to_old: u32,
    pub from_new: u32,
    pub to_new: u32,
}

/// 复用区间小于这个字节数时不值得保留：定位与上下文校验本身有成本，
/// 而省下的重扫描还不够付。与上游默认值一致。
const MIN_GAP: u32 = 128;

impl TreeFragment {
    /// 从一棵刚解析出的完整树建立初始 fragment 集合。
    #[must_use]
    pub fn from_tree(tree: &Tree) -> Vec<Self> {
        vec![Self {
            from: 0,
            to: tree.len_bytes(),
            tree: tree.clone(),
            offset: 0,
            open_start: false,
            open_end: false,
        }]
    }

    /// 把一组编辑应用到 fragment 集合上：切掉被改动的部分，平移其余部分。
    #[must_use]
    pub fn apply_changes(fragments: &[Self], changes: &[FragmentChange]) -> Vec<Self> {
        if changes.is_empty() {
            return fragments.to_vec();
        }
        let mut result: Vec<Self> = Vec::new();
        let mut fragment_index = 1_usize;
        let mut next = fragments.first().cloned();
        let mut pos = 0_u32;
        let mut offset = 0_i64;

        for change_index in 0..=changes.len() {
            let change = changes.get(change_index).copied();
            // 末尾那一轮没有 change，用一个哨兵位置把剩余 fragment 收进来。
            let next_pos = change.map_or(u32::MAX, |change| change.from_old);
            if next_pos.saturating_sub(pos) >= MIN_GAP {
                while let Some(candidate) = next.clone() {
                    if candidate.from >= next_pos {
                        break;
                    }
                    let mut cut = Some(candidate.clone());
                    if pos >= candidate.from || next_pos <= candidate.to || offset != 0 {
                        let from = i64::from(candidate.from.max(pos)) - offset;
                        let to = i64::from(candidate.to.min(next_pos)) - offset;
                        cut = (from < to).then(|| Self {
                            from: u32::try_from(from.max(0)).unwrap_or(0),
                            to: u32::try_from(to.max(0)).unwrap_or(0),
                            tree: candidate.tree.clone(),
                            offset: candidate.offset + offset,
                            // `open_start` 照上游写：`change_index > 0`。
                            //
                            // `open_end` **要沿用传进来的标记**——终点还是
                            // fragment 自己的终点时，它开不开由上一轮决定，
                            // 不由这一轮有没有改动决定。两端为什么不对称，
                            // 以及不沿用会错成什么样，见模块文档。
                            open_start: change_index > 0,
                            open_end: (next_pos <= candidate.to && change.is_some())
                                || (candidate.to <= next_pos && candidate.open_end),
                        });
                    }
                    if let Some(cut) = cut {
                        result.push(cut);
                    }
                    if candidate.to > next_pos {
                        break;
                    }
                    next = fragments.get(fragment_index).cloned();
                    fragment_index += 1;
                }
            }
            let Some(change) = change else { break };
            pos = change.to_old;
            offset = i64::from(change.to_old) - i64::from(change.to_new);
        }
        result
    }
}

/// 在 fragment 集合上定位并搬运可复用节点。
pub(crate) struct FragmentCursor<'a> {
    fragments: &'a [TreeFragment],
    /// 下一个待启用的 fragment 下标。
    next_index: usize,
    current: Option<&'a TreeFragment>,
    /// 当前 fragment 里最后一个完整行的结束位置（旧文档坐标），
    /// 复用不能越过它——半行没有块边界的含义。
    fragment_end: Option<u32>,
    cursor: Option<TreeCursor<'a>>,
}

impl<'a> FragmentCursor<'a> {
    pub(crate) fn new(fragments: &'a [TreeFragment]) -> Self {
        let current = fragments.first();
        Self {
            fragments,
            next_index: 1,
            current,
            fragment_end: None,
            cursor: None,
        }
    }

    fn next_fragment(&mut self) {
        self.current = self.fragments.get(self.next_index);
        self.next_index += 1;
        self.cursor = None;
        self.fragment_end = None;
    }

    /// 尝试在 `pos` 处复用。成功时返回消耗掉的字节数。
    pub(crate) fn try_take<I: Input + ?Sized>(
        &mut self,
        cx: &mut BlockContext<'a, I>,
        pos: u32,
        line_start: u32,
    ) -> Option<u32> {
        if !self.move_to(cx, pos, line_start) {
            return None;
        }
        let cursor = self.cursor.as_ref()?;
        if cursor.tree().context_hash() != cx.block_hash() {
            return None;
        }
        let taken = self.take_nodes(cx, line_start);
        (taken > 0).then_some(taken)
    }

    fn move_to<I: Input + ?Sized>(
        &mut self,
        cx: &BlockContext<'a, I>,
        pos: u32,
        line_start: u32,
    ) -> bool {
        while self.current.is_some_and(|fragment| fragment.to <= pos) {
            self.next_fragment();
        }
        let Some(fragment) = self.current else {
            return false;
        };
        if fragment.from > pos.saturating_sub(1) {
            return false;
        }
        // **复用必须从一个在新旧两份文档里都成立的行首开始。**
        //
        // fragment 内部新旧字节逐一对应（差一个 `offset`），所以 fragment
        // **内部**的换行在两边都是换行。但 fragment 的第一个字节不在这个保证
        // 里：如果它是一次改动的右边界（`open_start`），那么新文档里的行首
        // 映射回旧文档可能落在半行上，那里的解析状态根本不是「行首」。
        //
        // 上游 `@lezer/markdown` 只判 `fragment.from <= lineStart`，漏了这一
        // 条。规范用例 253 加一次插入就能触发：
        //
        // ```text
        // 1.  段落
        //
        //         indented code      ← 8 空格，是列表项里的缩进代码块
        // ```
        //
        // 在第一个空格后插一个换行，这一行只剩 7 个空格，不再是代码块；
        // 但它的字节没变，旧树里的 CodeBlock 会被原样复用。不 panic、
        // 不报错，只是块类型悄悄错了。
        if fragment.open_start && line_start <= fragment.from {
            return false;
        }
        if self.fragment_end.is_none() {
            // 往回找到 fragment 内最后一个换行符。
            let mut end = fragment.to;
            while end > 0 && cx.input_byte_at(end - 1) != Some(b'\n') {
                end -= 1;
            }
            self.fragment_end = Some(end.saturating_sub(1));
        }

        if self.cursor.is_none() {
            let mut cursor = fragment.tree.cursor(0);
            if !cursor.first_child() {
                return false;
            }
            self.cursor = Some(cursor);
        }
        let cursor = self.cursor.as_mut().expect("刚刚建好");

        // 新文档位置换算成旧树位置。
        let Ok(relative) = u32::try_from(i64::from(pos) + fragment.offset) else {
            return false;
        };
        while cursor.to() <= relative {
            if !cursor.parent() {
                return false;
            }
        }
        loop {
            if cursor.from() >= relative {
                return fragment.from <= line_start;
            }
            if !cursor.child_ending_after(relative) {
                return false;
            }
        }
    }

    /// 把游标处开始的一串兄弟节点搬进 `cx`，返回消耗的字节数。
    fn take_nodes<I: Input + ?Sized>(
        &mut self,
        cx: &mut BlockContext<'a, I>,
        line_start: u32,
    ) -> u32 {
        let Some(fragment) = self.current else {
            return 0;
        };
        let Some(cursor) = self.cursor.as_mut() else {
            return 0;
        };
        let offset = fragment.offset;
        let fragment_end = self
            .fragment_end
            .unwrap_or(0)
            .saturating_sub(u32::from(fragment.open_end));

        let start = line_start;
        let mut end = start;
        let mut block_len = cx.block_children_len();
        let mut previous_end = end;
        let mut previous_len = block_len;

        loop {
            let node_to = i64::from(cursor.to()) - offset;
            if node_to > i64::from(fragment_end) {
                break;
            }
            let Ok(node_from) = u32::try_from(i64::from(cursor.from()) - offset) else {
                break;
            };
            cx.reuse_tree(cursor.tree().clone(), node_from);

            if cursor.kind().is_block() {
                let node_to = u32::try_from(node_to).unwrap_or(0);
                if cursor.kind().spans_blank_lines() {
                    // 能跨空行的块只有在后一个兄弟也被复用时才算数：紧随其后的
                    // 一个空行会改变它的边界，而它自己的字节没有变化。
                    end = previous_end;
                    block_len = previous_len;
                    previous_end = node_to;
                    previous_len = cx.block_children_len();
                } else {
                    end = node_to;
                    block_len = cx.block_children_len();
                }
            }
            if !cursor.next_sibling() {
                break;
            }
        }
        // 回退到最后一个可以充当边界的块。
        cx.truncate_block_children(block_len);
        end.saturating_sub(start)
    }
}

/// 与 `yu-text` 的 `ChangeSet` 之间的桥。
///
/// 放在这里而不是让调用方自己转：`TextChange` 的两个 range 分别属于旧、新两套
/// 坐标，弄反不会 panic，只会让复用落在错误的位置上——而复用错的后果是树
/// 悄悄不对，正是不变量 C3 要防的东西。转换只写一次。
impl TreeFragment {
    /// 把一次 Transaction 的改动应用到 fragment 集合。
    #[must_use]
    pub fn apply_change_set(fragments: &[Self], changes: &yu_text::ChangeSet) -> Vec<Self> {
        let converted: Vec<FragmentChange> = changes
            .changes()
            .iter()
            .map(|change| FragmentChange {
                from_old: clamp_u32(change.old_range().start().get()),
                to_old: clamp_u32(change.old_range().end().get()),
                from_new: clamp_u32(change.new_range().start().get()),
                to_new: clamp_u32(change.new_range().end().get()),
            })
            .collect();
        Self::apply_changes(fragments, &converted)
    }
}

fn clamp_u32(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
