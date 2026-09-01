//! 一帧的身份：这一帧的内容取决于什么。
//!
//! 这份定义原来叫 `MacosFrameKey`，住在 `yu-storage-ffi` 的一个
//! `#[cfg(target_os = "macos")]` 块里。六个字段没有一个是 macOS 概念——它叫
//! `Macos` 只因为它住在那个 cfg 底下。而它的文档写着「新增一种不推进
//! Revision 的可视状态时必须同时加进来，否则静默跳过」，那句话住在一个
//! Windows 上根本不编译的块里：**第二端加了一种可视状态，第一端不会红**。
//!
//! 它还有第二个消费者在等着：TSF 是推模型，壳必须自己算出该发
//! `OnTextChange` / `OnSelectionChange` / `OnLayoutChange` 中的哪一条——那正是
//! 这个键已经在做的事。所以它是工作区的定义，不是某个 C ABI 的私事。

use yu_editor::{EditorSelection, TableResizeCommit, TableResizeTarget};

use crate::Appearance;

/// 平台提供的一帧几何。
///
/// 这些值只有平台知道（view bounds、滚动位置、backing scale），因此必须由
/// 平台传入；其余判断全部留在 Rust。用位模式比较而不是浮点相等，避免 NaN
/// 让「相同几何」永远判为不同而每帧重画。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameGeometry {
    size_bits: u32,
    max_width_bits: u32,
    scroll_y_bits: u32,
    viewport_height_bits: u32,
    surface_width_bits: u64,
    surface_height_bits: u64,
    scale_bits: u64,
}

impl FrameGeometry {
    /// 校验并记下一帧的几何。任何一项不是有限值（或该为正却不为正）就拒绝。
    ///
    /// 字号、换行宽度与视口高度必须为正；滚动位置可以是 0。
    #[must_use]
    pub fn new(
        size: f32,
        max_width: f32,
        scroll_y: f32,
        viewport_height: f32,
        surface_width: f64,
        surface_height: f64,
        scale: f64,
    ) -> Option<Self> {
        let finite32 = |value: f32, positive: bool| {
            value.is_finite() && (if positive { value > 0.0 } else { value >= 0.0 })
        };
        let finite64 = |value: f64| value.is_finite() && value > 0.0;
        if !finite32(size, true)
            || !finite32(max_width, true)
            || !finite32(scroll_y, false)
            || !finite32(viewport_height, true)
            || !finite64(surface_width)
            || !finite64(surface_height)
            || !finite64(scale)
        {
            return None;
        }
        Some(Self {
            size_bits: size.to_bits(),
            max_width_bits: max_width.to_bits(),
            scroll_y_bits: scroll_y.to_bits(),
            viewport_height_bits: viewport_height.to_bits(),
            surface_width_bits: surface_width.to_bits(),
            surface_height_bits: surface_height.to_bits(),
            scale_bits: scale.to_bits(),
        })
    }
}

/// 表格 resize 的有效覆盖，作为帧身份的一部分。
///
/// 拖动分隔线既不推进 Revision 也不改变几何，但整张表的列宽都会变。少了这一项，
/// 一次拖动会被判为「与屏幕上的帧等价」而整段被跳过。
///
/// 与几何同理用位模式比较：`TableResizeCommit` 携带 f32，直接用 `PartialEq`
/// 会让任何 NaN 与自身不等，从而每帧重画。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameTableResize {
    revision: u64,
    block_index: usize,
    target: TableResizeTarget,
    initial_position_bits: u32,
    final_position_bits: u32,
}

impl FrameTableResize {
    #[must_use]
    pub fn capture(commit: TableResizeCommit) -> Self {
        Self {
            revision: commit.revision().get(),
            block_index: commit.block_index(),
            target: commit.target(),
            initial_position_bits: commit.initial_position().to_bits(),
            final_position_bits: commit.final_position().to_bits(),
        }
    }
}

/// 一帧的完整身份：Rust 拥有的可视状态 + 平台提供的几何。
///
/// 全部一起比较，任何一项变化都要重画：
///
/// - `revision`：源码改变。
/// - `composition_generation`：marked text 更新——它不推进 Revision。
/// - `selections`：光标与选区装饰改变——它同样不推进 Revision。**条数变化也在
///   内**：从一根光标变成三根既不推进 Revision 也不改几何，少了它的表现是
///   「按下全部选中，画面一动不动」。
/// - `search_generation`：换了查询——同样不推进 Revision、不改几何、不改选区。
///   「当前命中」换一个不用单列一项：那是从 `selection` 推出来的。
/// - `table_resize`：拖动中的列宽覆盖——既不推进 Revision 也不改变几何。
/// - `appearance`：系统外观。切深浅**既不推进 Revision 也不改几何**，少了它
///   的表现是「切到深色，侧栏面板变深了而文档区一动不动」——面板走 AppKit 的
///   语义色自动跟，文档区由这一帧画，而这一帧被判成了「与屏幕上那一帧等价」。
/// - `geometry`：字号、换行宽度、滚动、surface 尺寸与 backing scale。
///
/// 这个列表就是「帧内容取决于什么」的完整定义。新增一种不推进 Revision 的
/// 可视状态时必须同时加进来，否则它的变化会被静默跳过——本项目最危险的失败
/// 模式正是这种不报错的漏画。
///
/// # 为什么是整组比较，不是一个摘要
///
/// 把 N 条选区哈希成一个 u64 会让 `FrameKey` 继续是 `Copy` 的，代价是碰撞
/// ——而碰撞的表现正是这个类型的文档明令要防的那件事：**静默跳过一帧**。
/// 一次 `Vec` 分配（N 是光标数）换掉一个不报错的漏画，这笔账不用算。
#[derive(Clone, Debug, PartialEq)]
pub struct FrameKey {
    revision: u64,
    composition_generation: u64,
    selections: Vec<EditorSelection>,
    search_generation: u64,
    table_resize: Option<FrameTableResize>,
    appearance: Appearance,
    geometry: FrameGeometry,
}

impl FrameKey {
    /// 组装一帧的身份。
    ///
    /// 提交路径与「这一帧是不是当前帧」共用这一个构造函数。两边各写一份是这个
    /// 判断最容易出错的地方：只要有一项不对称，就会出现「明明变了却判为等价」
    /// 或「明明没变却每帧重画」。
    #[must_use]
    pub fn new(
        revision: u64,
        composition_generation: u64,
        selections: Vec<EditorSelection>,
        search_generation: u64,
        table_resize: Option<FrameTableResize>,
        appearance: Appearance,
        geometry: FrameGeometry,
    ) -> Self {
        Self {
            revision,
            composition_generation,
            selections,
            search_generation,
            table_resize,
            appearance,
            geometry,
        }
    }

    /// 这一帧建立在哪个 Revision 上。
    ///
    /// 存在的理由是用例要能说「这两帧不同**不是**因为 Revision 变了」——那正是
    /// 这个类型的全部意义：四项可视状态都不推进 Revision。
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// 平台送来的那一份几何。同上：用例要能说「变的不是几何」。
    #[must_use]
    pub const fn geometry(&self) -> FrameGeometry {
        self.geometry
    }

    /// 这一帧的全部选区。同上：用例要能说「变的不是选区」。
    #[must_use]
    pub fn selections(&self) -> &[EditorSelection] {
        &self.selections
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry() -> FrameGeometry {
        FrameGeometry::new(16.0, 720.0, 0.0, 480.0, 1440.0, 960.0, 2.0).expect("geometry")
    }

    /// 几何必须是有限值，而且该为正的要为正。
    ///
    /// 滚动位置是唯一允许为 0 的一项——文档顶端就是 0。
    #[test]
    fn frame_geometry_rejects_values_that_cannot_describe_a_frame() {
        assert!(FrameGeometry::new(f32::NAN, 720.0, 0.0, 480.0, 1440.0, 960.0, 2.0).is_none());
        assert!(FrameGeometry::new(0.0, 720.0, 0.0, 480.0, 1440.0, 960.0, 2.0).is_none());
        assert!(FrameGeometry::new(16.0, 720.0, -1.0, 480.0, 1440.0, 960.0, 2.0).is_none());
        assert!(FrameGeometry::new(16.0, 720.0, 0.0, 0.0, 1440.0, 960.0, 2.0).is_none());
        assert!(FrameGeometry::new(16.0, 720.0, 0.0, 480.0, 1440.0, 960.0, 0.0).is_none());
        assert!(FrameGeometry::new(16.0, 720.0, 0.0, 480.0, 1440.0, 960.0, 2.0).is_some());
    }

    /// 同一份状态两次组装出来的身份必须相等，否则每帧重画。
    ///
    /// 这就是几何按**位模式**存的理由：直接留着 f32 比较的话，任何一个 NaN
    /// 都与自身不等，于是「没变」永远判不出来。而 `FrameGeometry::new` 已经
    /// 拒了 NaN，所以这条压的是另一半——位模式没有把相等判坏。
    #[test]
    fn the_same_state_captures_an_equal_key_twice() {
        let selections = Vec::new();
        let first = FrameKey::new(
            7,
            3,
            selections.clone(),
            2,
            None,
            Appearance::Light,
            geometry(),
        );
        let second = FrameKey::new(7, 3, selections, 2, None, Appearance::Light, geometry());
        assert_eq!(first, second);
    }

    /// 四项不推进 Revision 的可视状态，任何一项变了都必须判为「不是同一帧」。
    ///
    /// 只比 Revision 的表现是：preedit 更新、加一根光标、换查询、拖列宽全部
    /// 静默跳过——画面一动不动，而且不报错。
    #[test]
    fn each_visual_state_that_does_not_advance_the_revision_still_changes_the_key() {
        let base = FrameKey::new(7, 3, Vec::new(), 2, None, Appearance::Light, geometry());

        assert_ne!(
            base,
            FrameKey::new(7, 4, Vec::new(), 2, None, Appearance::Light, geometry()),
            "composition generation"
        );
        assert_ne!(
            base,
            FrameKey::new(7, 3, Vec::new(), 5, None, Appearance::Light, geometry()),
            "search generation"
        );
        assert_ne!(
            base,
            FrameKey::new(
                7,
                3,
                Vec::new(),
                2,
                None,
                Appearance::Light,
                FrameGeometry::new(16.0, 720.0, 40.0, 480.0, 1440.0, 960.0, 2.0)
                    .expect("scrolled geometry"),
            ),
            "geometry"
        );
        assert_ne!(
            base,
            FrameKey::new(7, 3, Vec::new(), 2, None, Appearance::Dark, geometry()),
            "外观：切深浅既不推进 Revision 也不改几何，少了它文档区不会重画"
        );
    }
}
