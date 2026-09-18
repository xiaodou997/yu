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

/// Geometry that changes the contents of a rendered frame rather than merely
/// moving the camera over it.  Scroll position intentionally does not belong
/// here: a retained publication can be presented at another scroll offset as
/// long as that offset stays inside its coverage range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameBuildGeometry {
    size_bits: u32,
    max_width_bits: u32,
    viewport_height_bits: u32,
    surface_width_bits: u64,
    surface_height_bits: u64,
    scale_bits: u64,
}

impl FrameGeometry {
    #[must_use]
    pub const fn build_geometry(self) -> FrameBuildGeometry {
        FrameBuildGeometry {
            size_bits: self.size_bits,
            max_width_bits: self.max_width_bits,
            viewport_height_bits: self.viewport_height_bits,
            surface_width_bits: self.surface_width_bits,
            surface_height_bits: self.surface_height_bits,
            scale_bits: self.scale_bits,
        }
    }

    #[must_use]
    pub fn scroll_y(self) -> f32 {
        f32::from_bits(self.scroll_y_bits)
    }

    #[must_use]
    pub fn viewport_height(self) -> f32 {
        f32::from_bits(self.viewport_height_bits)
    }
}

/// The camera/presentation part of a retained frame.  It is separate from
/// [`FrameBuildGeometry`] so a scroll gesture can reuse an already shaped and
/// rasterized publication without rebuilding Markdown layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramePresentationState {
    scroll_y_bits: u32,
    coverage_top_bits: u32,
    coverage_bottom_bits: u32,
}

impl FramePresentationState {
    #[must_use]
    pub fn new(scroll_y: f32, coverage_top: f32, coverage_bottom: f32) -> Option<Self> {
        if !scroll_y.is_finite()
            || scroll_y < 0.0
            || !coverage_top.is_finite()
            || coverage_top < 0.0
            || !coverage_bottom.is_finite()
            || coverage_bottom < coverage_top
            || scroll_y < coverage_top
            || scroll_y > coverage_bottom
        {
            return None;
        }
        Some(Self {
            scroll_y_bits: scroll_y.to_bits(),
            coverage_top_bits: coverage_top.to_bits(),
            coverage_bottom_bits: coverage_bottom.to_bits(),
        })
    }

    #[must_use]
    pub fn scroll_y(self) -> f32 {
        f32::from_bits(self.scroll_y_bits)
    }

    #[must_use]
    pub fn coverage_top(self) -> f32 {
        f32::from_bits(self.coverage_top_bits)
    }

    #[must_use]
    pub fn coverage_bottom(self) -> f32 {
        f32::from_bits(self.coverage_bottom_bits)
    }

    #[must_use]
    pub fn covers(self, scroll_y: f32) -> bool {
        scroll_y.is_finite()
            && scroll_y >= self.coverage_top()
            && scroll_y <= self.coverage_bottom()
    }
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

/// 一帧的完整身份：内容构建身份 + 平台提供的呈现几何。
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
/// `FrameBuildKey` 已经把不影响内容的 scroll origin 从构建身份中分离出来；
/// `FrameKey` 暂时保留完整的 presentation 比较，以兼容当前严格 host 提交路径。
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
pub struct FrameBuildKey {
    revision: u64,
    source_mode: bool,
    composition_generation: u64,
    selections: Vec<EditorSelection>,
    table_columns: Option<usize>,
    table_width_generation: u64,
    search_generation: u64,
    table_resize: Option<FrameTableResize>,
    appearance: Appearance,
    geometry: FrameBuildGeometry,
}

/// Ticket carried by a background frame-build job. The key is immutable and
/// the generation is owned by the platform scheduler; both must match before
/// a completed plan may be published.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameBuildRequest {
    key: FrameBuildKey,
    generation: u64,
}

impl FrameBuildRequest {
    #[must_use]
    pub fn new(key: FrameBuildKey, generation: u64) -> Self {
        Self { key, generation }
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub fn key(&self) -> &FrameBuildKey {
        &self.key
    }

    /// Returns true only when the scheduler still wants this exact result.
    #[must_use]
    pub fn accepts(&self, current_key: &FrameBuildKey, current_generation: u64) -> bool {
        self.generation == current_generation && &self.key == current_key
    }
}

impl FrameBuildKey {
    /// 组装一份需要重新准备内容的身份。滚动位置不在其中。
    ///
    /// 这份 key 供 retained frame builder 使用；当前 `FrameKey` 仍把
    /// presentation 也带上，用于严格判断屏幕上的帧是否已经跟随滚动。
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
            source_mode: false,
            composition_generation,
            selections,
            table_columns: None,
            table_width_generation: 0,
            search_generation,
            table_resize,
            appearance,
            geometry: geometry.build_geometry(),
        }
    }

    #[must_use]
    pub const fn geometry(&self) -> FrameBuildGeometry {
        self.geometry
    }
}

/// 一帧的完整身份：内容构建身份 + 屏幕呈现身份。
#[derive(Clone, Debug, PartialEq)]
pub struct FrameKey {
    build: FrameBuildKey,
    presentation: FramePresentationState,
}

impl FrameKey {
    #[must_use]
    pub fn with_source_mode(mut self, enabled: bool) -> Self {
        self.build.source_mode = enabled;
        self
    }
    /// Confirmed widths alter geometry without changing Markdown revision.
    #[must_use]
    pub fn with_table_width_generation(mut self, generation: u64) -> Self {
        self.build.table_width_generation = generation;
        self
    }

    #[must_use]
    pub fn with_table_columns(mut self, columns: Option<usize>) -> Self {
        self.build.table_columns = columns;
        self
    }

    /// 组装一帧的身份。
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
        let scroll_y = geometry.scroll_y();
        let viewport_height = geometry.viewport_height();
        let presentation =
            FramePresentationState::new(scroll_y, scroll_y, scroll_y + viewport_height)
                .expect("validated frame geometry must produce a presentation state");
        Self {
            build: FrameBuildKey::new(
                revision,
                composition_generation,
                selections,
                search_generation,
                table_resize,
                appearance,
                geometry,
            ),
            presentation,
        }
    }

    #[must_use]
    pub const fn build(&self) -> &FrameBuildKey {
        &self.build
    }

    #[must_use]
    pub const fn presentation(&self) -> FramePresentationState {
        self.presentation
    }

    /// 这一帧建立在哪个 Revision 上。
    ///
    /// 存在的理由是用例要能说「这两帧不同**不是**因为 Revision 变了」——那正是
    /// 这个类型的全部意义：四项可视状态都不推进 Revision。
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.build.revision
    }

    /// 平台送来的那一份几何。同上：用例要能说「变的不是几何」。
    #[must_use]
    pub const fn geometry(&self) -> FrameGeometry {
        // `FrameKey` is retained for the strict host equality path.  The
        // original full geometry is reconstructed only for callers that still
        // need it; new retained-frame code should use `build()` and
        // `presentation()` instead.
        FrameGeometry {
            size_bits: self.build.geometry.size_bits,
            max_width_bits: self.build.geometry.max_width_bits,
            scroll_y_bits: self.presentation.scroll_y_bits,
            viewport_height_bits: self.build.geometry.viewport_height_bits,
            surface_width_bits: self.build.geometry.surface_width_bits,
            surface_height_bits: self.build.geometry.surface_height_bits,
            scale_bits: self.build.geometry.scale_bits,
        }
    }

    /// 这一帧的全部选区。同上：用例要能说「变的不是选区」。
    #[must_use]
    pub fn selections(&self) -> &[EditorSelection] {
        &self.build.selections
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
    fn cell_selection_identity_invalidates_an_equal_text_selection_frame() {
        let text = FrameKey::new(7, 3, Vec::new(), 2, None, Appearance::Light, geometry());
        let cells = text.clone().with_table_columns(Some(2));
        assert_ne!(text, cells);
        assert_ne!(text.build(), cells.build());
        assert_eq!(text.presentation(), cells.presentation());
    }

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

    #[test]
    fn build_key_can_be_reused_while_presentation_moves_inside_coverage() {
        let top =
            FrameGeometry::new(16.0, 720.0, 0.0, 480.0, 1440.0, 960.0, 2.0).expect("top geometry");
        let scrolled = FrameGeometry::new(16.0, 720.0, 120.0, 480.0, 1440.0, 960.0, 2.0)
            .expect("scrolled geometry");
        let first = FrameKey::new(7, 3, Vec::new(), 2, None, Appearance::Light, top);
        let second = FrameKey::new(7, 3, Vec::new(), 2, None, Appearance::Light, scrolled);

        assert_eq!(first.build(), second.build());
        assert_ne!(first.presentation(), second.presentation());

        let coverage = FramePresentationState::new(240.0, 0.0, 720.0).expect("coverage");
        assert!(coverage.covers(0.0));
        assert!(coverage.covers(720.0));
        assert!(!coverage.covers(721.0));

        let request = FrameBuildRequest::new(first.build().clone(), 9);
        assert!(request.accepts(second.build(), 9));
        assert!(!request.accepts(second.build(), 10));
    }

    #[test]
    fn confirmed_width_change_rejects_old_background_result_without_source_edit() {
        let old = FrameKey::new(7, 0, Vec::new(), 0, None, Appearance::Light, geometry());
        let new = old.clone().with_table_width_generation(1);
        assert_ne!(old, new);
        assert_ne!(old.build(), new.build());
        let request = FrameBuildRequest::new(old.build().clone(), 4);
        assert!(!request.accepts(new.build(), 4));
        assert_eq!(old.presentation(), new.presentation());
    }

    #[test]
    fn presentation_state_rejects_offsets_outside_coverage() {
        assert!(FramePresentationState::new(1.0, 2.0, 10.0).is_none());
        assert!(FramePresentationState::new(11.0, 0.0, 10.0).is_none());
        assert!(FramePresentationState::new(f32::NAN, 0.0, 10.0).is_none());
    }
}
