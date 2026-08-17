#![forbid(unsafe_code)]

//! Revision-bound retained drawing data for Yu Editor.
//!
//! A scene contains only owned geometry, colors and glyph-atlas placements. It
//! deliberately does not contain source text, layout caches, pixels, native
//! window objects or GPU handles. A renderer can therefore consume a scene on
//! its own thread and discard it when its source revision is stale.

use std::error::Error;
use std::fmt;

use yu_core::Revision;
use yu_font::{AtlasEntry, GlyphRasterKey};
use yu_layout::LayoutSnapshot;

mod viewport;

pub use viewport::{ViewportBlockGeometry, ViewportSceneInput};

/// A finite point in scene coordinates. The coordinate system is chosen by
/// the platform shell; Yu's current layout convention is x-right/y-down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    x: f32,
    y: f32,
}

impl Point {
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }

    fn validate(self) -> Result<(), SceneError> {
        if self.x.is_finite() && self.y.is_finite() {
            Ok(())
        } else {
            Err(SceneError::InvalidGeometry("point must be finite"))
        }
    }
}

/// A finite, non-negative rectangle in scene coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Result<Self, SceneError> {
        let rect = Self {
            x,
            y,
            width,
            height,
        };
        rect.validate()?;
        Ok(rect)
    }

    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }

    #[must_use]
    pub const fn width(self) -> f32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> f32 {
        self.height
    }

    #[must_use]
    pub fn right(self) -> f32 {
        self.x + self.width
    }

    #[must_use]
    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.width == 0.0 || self.height == 0.0
    }

    #[must_use]
    pub fn union(self, other: Self) -> Self {
        let left = self.x.min(other.x);
        let top = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }
    }

    #[must_use]
    pub fn intersects_or_touches(self, other: Self) -> bool {
        self.x <= other.right()
            && other.x <= self.right()
            && self.y <= other.bottom()
            && other.y <= self.bottom()
    }

    fn validate(self) -> Result<(), SceneError> {
        if !self.x.is_finite()
            || !self.y.is_finite()
            || !self.width.is_finite()
            || !self.height.is_finite()
            || self.width < 0.0
            || self.height < 0.0
        {
            return Err(SceneError::InvalidGeometry(
                "rectangle must be finite and non-negative",
            ));
        }
        if !self.right().is_finite() || !self.bottom().is_finite() {
            return Err(SceneError::InvalidGeometry(
                "rectangle bounds must be finite",
            ));
        }
        Ok(())
    }
}

/// Packed non-premultiplied sRGB color used by scene primitives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba8(u32);

impl Rgba8 {
    #[must_use]
    pub const fn new(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self(u32::from_be_bytes([red, green, blue, alpha]))
    }

    #[must_use]
    pub const fn packed(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn red(self) -> u8 {
        self.0.to_be_bytes()[0]
    }

    #[must_use]
    pub const fn green(self) -> u8 {
        self.0.to_be_bytes()[1]
    }

    #[must_use]
    pub const fn blue(self) -> u8 {
        self.0.to_be_bytes()[2]
    }

    #[must_use]
    pub const fn alpha(self) -> u8 {
        self.0.to_be_bytes()[3]
    }

    #[must_use]
    pub const fn white() -> Self {
        Self::new(255, 255, 255, 255)
    }

    #[must_use]
    pub const fn black() -> Self {
        Self::new(0, 0, 0, 255)
    }
}

impl Default for Rgba8 {
    fn default() -> Self {
        Self::black()
    }
}

/// A glyph draw operation referencing an entry in a separate CPU/GPU atlas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphPrimitive {
    atlas: AtlasEntry,
    origin: Point,
    color: Rgba8,
}

impl GlyphPrimitive {
    #[must_use]
    pub const fn new(atlas: AtlasEntry, origin: Point, color: Rgba8) -> Self {
        Self {
            atlas,
            origin,
            color,
        }
    }

    #[must_use]
    pub const fn atlas(self) -> AtlasEntry {
        self.atlas
    }

    #[must_use]
    pub const fn key(self) -> GlyphRasterKey {
        self.atlas.key()
    }

    #[must_use]
    pub const fn origin(self) -> Point {
        self.origin
    }

    #[must_use]
    pub const fn color(self) -> Rgba8 {
        self.color
    }

    /// Returns the visual bounds in baseline-oriented scene coordinates.
    #[must_use]
    pub fn bounds(self) -> Rect {
        let rect = self.atlas.rect();
        Rect {
            x: self.origin.x() + self.atlas.metrics().bearing_x(),
            y: self.origin.y() - self.atlas.metrics().bearing_y(),
            width: rect.width() as f32,
            height: rect.height() as f32,
        }
    }
}

/// A source-independent image draw operation.
///
/// `resource` is a stable `yu-assets::ImageKey::fingerprint()` supplied by
/// the host. The scene carries only that scalar identity and a fallback
/// color; decoded pixels and GPU textures remain owned by the platform
/// backend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImagePrimitive {
    resource: u64,
    bounds: Rect,
    fallback: Rgba8,
}

impl ImagePrimitive {
    #[must_use]
    pub const fn new(resource: u64, bounds: Rect, fallback: Rgba8) -> Self {
        Self {
            resource,
            bounds,
            fallback,
        }
    }

    #[must_use]
    pub const fn resource(self) -> u64 {
        self.resource
    }

    #[must_use]
    pub const fn bounds(self) -> Rect {
        self.bounds
    }

    #[must_use]
    pub const fn fallback(self) -> Rgba8 {
        self.fallback
    }
}

/// Semantic role for a source-backed table decoration. The renderer may map
/// all roles to solid fills today, while native selection/accessibility layers
/// can still distinguish header, selection and grid geometry without parsing
/// Markdown again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TablePrimitiveRole {
    HeaderFill,
    SelectionFill,
    Border,
}

/// One source-backed table decoration. `source` identifies the cell for a
/// header/selection fill and the complete table range for a border.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TablePrimitive {
    source: yu_core::TextRange,
    bounds: Rect,
    color: Rgba8,
    role: TablePrimitiveRole,
}

impl TablePrimitive {
    #[must_use]
    pub const fn new(
        source: yu_core::TextRange,
        bounds: Rect,
        color: Rgba8,
        role: TablePrimitiveRole,
    ) -> Self {
        Self {
            source,
            bounds,
            color,
            role,
        }
    }

    #[must_use]
    pub const fn source(self) -> yu_core::TextRange {
        self.source
    }

    #[must_use]
    pub const fn bounds(self) -> Rect {
        self.bounds
    }

    #[must_use]
    pub const fn color(self) -> Rgba8 {
        self.color
    }

    #[must_use]
    pub const fn role(self) -> TablePrimitiveRole {
        self.role
    }
}

/// Colors and border width used when projecting a table layout into scene
/// decorations. `None` disables the corresponding fill while a zero border
/// width disables grid lines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableSceneStyle {
    border_width: f32,
    border_color: Rgba8,
    header_fill: Option<Rgba8>,
    selection_fill: Option<Rgba8>,
}

impl TableSceneStyle {
    #[must_use]
    pub const fn new(
        border_width: f32,
        border_color: Rgba8,
        header_fill: Option<Rgba8>,
        selection_fill: Option<Rgba8>,
    ) -> Self {
        Self {
            border_width,
            border_color,
            header_fill,
            selection_fill,
        }
    }

    #[must_use]
    pub const fn border_width(self) -> f32 {
        self.border_width
    }

    #[must_use]
    pub const fn border_color(self) -> Rgba8 {
        self.border_color
    }

    #[must_use]
    pub const fn header_fill(self) -> Option<Rgba8> {
        self.header_fill
    }

    #[must_use]
    pub const fn selection_fill(self) -> Option<Rgba8> {
        self.selection_fill
    }
}

/// One retained scene primitive. Insertion order is the painter's order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Primitive {
    FillRect { bounds: Rect, color: Rgba8 },
    Glyph(GlyphPrimitive),
    Image(ImagePrimitive),
    Table(TablePrimitive),
}

impl Primitive {
    #[must_use]
    pub fn bounds(self) -> Rect {
        match self {
            Self::FillRect { bounds, .. } => bounds,
            Self::Glyph(glyph) => glyph.bounds(),
            Self::Image(image) => image.bounds(),
            Self::Table(table) => table.bounds(),
        }
    }
}

/// Damage rectangles for one scene build. Adjacent or overlapping rectangles
/// are merged; when the budget is exceeded, all damage is collapsed to bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct DamageSet {
    max_rects: usize,
    rects: Vec<Rect>,
}

impl DamageSet {
    pub fn new(max_rects: usize) -> Result<Self, SceneError> {
        if max_rects == 0 {
            return Err(SceneError::InvalidDamageBudget);
        }
        Ok(Self {
            max_rects,
            rects: Vec::new(),
        })
    }

    #[must_use]
    pub const fn max_rects(&self) -> usize {
        self.max_rects
    }

    #[must_use]
    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    #[must_use]
    pub fn bounds(&self) -> Option<Rect> {
        self.rects.iter().copied().reduce(Rect::union)
    }

    pub fn add(&mut self, rect: Rect) -> Result<(), SceneError> {
        rect.validate()?;
        if rect.is_empty() {
            return Ok(());
        }
        let mut merged = rect;
        let mut index = 0;
        while index < self.rects.len() {
            if self.rects[index].intersects_or_touches(merged) {
                merged = merged.union(self.rects.remove(index));
            } else {
                index += 1;
            }
        }
        self.rects.push(merged);
        if self.rects.len() > self.max_rects {
            let bounds = self.bounds().ok_or(SceneError::InvalidDamageBudget)?;
            self.rects.clear();
            self.rects.push(bounds);
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.rects.clear();
    }
}

impl Default for DamageSet {
    fn default() -> Self {
        Self {
            max_rects: 64,
            rects: Vec::new(),
        }
    }
}

/// Errors raised while constructing a retained scene.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneError {
    InvalidGeometry(&'static str),
    InvalidViewportInput(&'static str),
    InvalidDamageBudget,
    PrimitiveLimitExceeded,
    InvalidTableStyle(u32),
    RevisionMismatch {
        scene: Revision,
        layout: Revision,
    },
    ViewportRevisionMismatch {
        expected: Revision,
        actual: Revision,
    },
    ViewportSourceMismatch,
    InvalidFontSize(u32),
    MissingGlyphAtlas(GlyphRasterKey),
}

impl fmt::Display for SceneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGeometry(message) => formatter.write_str(message),
            Self::InvalidViewportInput(message) => formatter.write_str(message),
            Self::InvalidDamageBudget => formatter.write_str("damage budget must be positive"),
            Self::PrimitiveLimitExceeded => formatter.write_str("scene primitive limit exceeded"),
            Self::InvalidTableStyle(width) => write!(
                formatter,
                "invalid table border width {}",
                f32::from_bits(*width)
            ),
            Self::RevisionMismatch { scene, layout } => write!(
                formatter,
                "scene revision {scene:?} does not match layout revision {layout:?}"
            ),
            Self::ViewportRevisionMismatch { expected, actual } => write!(
                formatter,
                "viewport revision {actual:?} does not match expected {expected:?}"
            ),
            Self::ViewportSourceMismatch => {
                formatter.write_str("viewport block source range does not match layout")
            }
            Self::InvalidFontSize(size) => {
                write!(
                    formatter,
                    "invalid scene font size {}",
                    f32::from_bits(*size)
                )
            }
            Self::MissingGlyphAtlas(key) => write!(
                formatter,
                "layout references missing atlas glyph {}",
                key.glyph().get()
            ),
        }
    }
}

impl Error for SceneError {}

/// A retained, source-revision-bound drawing scene.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    revision: Revision,
    viewport: Rect,
    primitives: Vec<Primitive>,
    damage: DamageSet,
}

impl Scene {
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn viewport(&self) -> Rect {
        self.viewport
    }

    #[must_use]
    pub fn primitives(&self) -> &[Primitive] {
        &self.primitives
    }

    #[must_use]
    pub fn damage(&self) -> &DamageSet {
        &self.damage
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.primitives.is_empty()
    }
}

fn translate_rect(rect: Rect, origin: Point) -> Result<Rect, SceneError> {
    Rect::new(
        rect.x() + origin.x(),
        rect.y() + origin.y(),
        rect.width(),
        rect.height(),
    )
}

fn translate_layout_rect(rect: yu_layout::LayoutRect, origin: Point) -> Result<Rect, SceneError> {
    Rect::new(
        rect.x() + origin.x(),
        rect.y() + origin.y(),
        rect.width(),
        rect.height(),
    )
}

fn ranges_intersect_or_caret(selection: yu_core::TextRange, cell: yu_core::TextRange) -> bool {
    if selection.is_empty() {
        if cell.is_empty() {
            return selection.start() == cell.start();
        }
        return cell.contains(selection.start());
    }
    selection.start() < cell.end() && cell.start() < selection.end()
}

/// Builds a scene while keeping primitive and damage order deterministic.
#[derive(Clone, Debug)]
pub struct SceneBuilder {
    revision: Revision,
    viewport: Rect,
    primitives: Vec<Primitive>,
    damage: DamageSet,
    max_primitives: usize,
}

impl SceneBuilder {
    pub fn new(revision: Revision, viewport: Rect) -> Result<Self, SceneError> {
        viewport.validate()?;
        Ok(Self {
            revision,
            viewport,
            primitives: Vec::new(),
            damage: DamageSet::default(),
            max_primitives: 1_000_000,
        })
    }

    pub fn with_damage_budget(mut self, max_rects: usize) -> Result<Self, SceneError> {
        self.damage = DamageSet::new(max_rects)?;
        Ok(self)
    }

    #[must_use]
    pub const fn with_primitive_limit(mut self, max_primitives: usize) -> Self {
        self.max_primitives = max_primitives;
        self
    }

    pub fn push(&mut self, primitive: Primitive) -> Result<u32, SceneError> {
        if self.primitives.len() >= self.max_primitives {
            return Err(SceneError::PrimitiveLimitExceeded);
        }
        if let Primitive::Glyph(glyph) = primitive {
            glyph.origin.validate()?;
        }
        primitive.bounds().validate()?;
        let index =
            u32::try_from(self.primitives.len()).map_err(|_| SceneError::PrimitiveLimitExceeded)?;
        self.primitives.push(primitive);
        self.damage.add(primitive.bounds())?;
        Ok(index)
    }

    pub fn fill_rect(&mut self, bounds: Rect, color: Rgba8) -> Result<u32, SceneError> {
        self.push(Primitive::FillRect { bounds, color })
    }

    pub fn glyph(&mut self, glyph: GlyphPrimitive) -> Result<u32, SceneError> {
        self.push(Primitive::Glyph(glyph))
    }

    pub fn image(&mut self, image: ImagePrimitive) -> Result<u32, SceneError> {
        self.push(Primitive::Image(image))
    }

    /// Appends source-backed table header, selection and grid decorations.
    /// The table layout remains block-local; `origin` moves its geometry into
    /// document/scene coordinates without copying cell text.
    pub fn append_table(
        &mut self,
        layout: &yu_layout::TableLayoutSnapshot,
        origin: Point,
        style: TableSceneStyle,
    ) -> Result<usize, SceneError> {
        self.append_table_with_selection(layout, origin, style, None)
    }

    /// Appends table decorations and highlights every visible cell whose
    /// source range intersects `selection`. A collapsed selection highlights
    /// the cell containing its source caret; an unrelated selection produces
    /// no selection primitive.
    pub fn append_table_with_selection(
        &mut self,
        layout: &yu_layout::TableLayoutSnapshot,
        origin: Point,
        style: TableSceneStyle,
        selection: Option<yu_core::TextRange>,
    ) -> Result<usize, SceneError> {
        let primitives = self.collect_table_primitives(layout, origin, style, selection)?;
        self.commit_primitives(primitives)
    }

    fn collect_table_primitives(
        &self,
        layout: &yu_layout::TableLayoutSnapshot,
        origin: Point,
        style: TableSceneStyle,
        selection: Option<yu_core::TextRange>,
    ) -> Result<Vec<Primitive>, SceneError> {
        if self.revision != layout.revision() {
            return Err(SceneError::RevisionMismatch {
                scene: self.revision,
                layout: layout.revision(),
            });
        }
        origin.validate()?;
        if !style.border_width().is_finite() || style.border_width() < 0.0 {
            return Err(SceneError::InvalidTableStyle(
                style.border_width().to_bits(),
            ));
        }

        let table_source = layout.source_range();
        let mut primitives = Vec::new();
        if let Some(color) = style.header_fill() {
            for cell in layout
                .cells()
                .iter()
                .copied()
                .filter(|cell| cell.row() == 0)
            {
                primitives.push(Primitive::Table(TablePrimitive::new(
                    cell.source(),
                    translate_layout_rect(cell.bounds(), origin)?,
                    color,
                    TablePrimitiveRole::HeaderFill,
                )));
            }
        }
        if let Some(color) = style.selection_fill() {
            for cell in layout.cells().iter().copied() {
                if selection.is_some_and(|range| ranges_intersect_or_caret(range, cell.source())) {
                    primitives.push(Primitive::Table(TablePrimitive::new(
                        cell.source(),
                        translate_layout_rect(cell.bounds(), origin)?,
                        color,
                        TablePrimitiveRole::SelectionFill,
                    )));
                }
            }
        }

        let border_width = style.border_width();
        let bounds = layout.bounds();
        let thickness_x = border_width.min(bounds.width());
        let thickness_y = border_width.min(bounds.height());
        if thickness_x > 0.0 && thickness_y > 0.0 {
            let border_color = style.border_color();
            let total_width = bounds.width();
            let total_height = bounds.height();
            let mut x = 0.0;
            for column_width in layout.column_widths() {
                primitives.push(Primitive::Table(TablePrimitive::new(
                    table_source,
                    translate_rect(Rect::new(x, 0.0, thickness_x, total_height)?, origin)?,
                    border_color,
                    TablePrimitiveRole::Border,
                )));
                x += *column_width;
            }
            primitives.push(Primitive::Table(TablePrimitive::new(
                table_source,
                translate_rect(
                    Rect::new(
                        (total_width - thickness_x).max(0.0),
                        0.0,
                        thickness_x,
                        total_height,
                    )?,
                    origin,
                )?,
                border_color,
                TablePrimitiveRole::Border,
            )));

            let mut y = 0.0;
            let row_count = layout
                .cells()
                .iter()
                .map(|cell| cell.row())
                .max()
                .map_or(0, |row| row.saturating_add(1));
            for _ in 0..row_count {
                primitives.push(Primitive::Table(TablePrimitive::new(
                    table_source,
                    translate_rect(Rect::new(0.0, y, total_width, thickness_y)?, origin)?,
                    border_color,
                    TablePrimitiveRole::Border,
                )));
                y += layout.row_height();
            }
            primitives.push(Primitive::Table(TablePrimitive::new(
                table_source,
                translate_rect(
                    Rect::new(
                        0.0,
                        (total_height - thickness_y).max(0.0),
                        total_width,
                        thickness_y,
                    )?,
                    origin,
                )?,
                border_color,
                TablePrimitiveRole::Border,
            )));
        }
        Ok(primitives)
    }

    /// Appends all shaped glyphs from a layout using entries already present
    /// in the CPU atlas. The operation is revision-bound and resolves every
    /// atlas entry before mutating the scene, so a failed lookup cannot leave
    /// a partially appended layout.
    pub fn append_layout(
        &mut self,
        layout: &LayoutSnapshot,
        atlas: &yu_font::GlyphAtlas,
        font_size: f32,
        color: Rgba8,
    ) -> Result<usize, SceneError> {
        self.append_layout_at(layout, atlas, font_size, color, Point::new(0.0, 0.0))
    }

    /// Appends all shaped glyphs from a block layout at a document-space
    /// origin. Layout coordinates remain block-local; only the scene origin
    /// translates them, so the viewport height index remains the sole source
    /// of block positioning.
    pub fn append_layout_at(
        &mut self,
        layout: &LayoutSnapshot,
        atlas: &yu_font::GlyphAtlas,
        font_size: f32,
        color: Rgba8,
        origin: Point,
    ) -> Result<usize, SceneError> {
        let primitives =
            self.collect_layout_primitives_at(layout, atlas, font_size, color, origin)?;
        self.commit_glyphs(primitives)
    }

    /// Appends every visible block in one preflighted scene transaction.
    ///
    /// The layouts are block-local and must be in the same order as
    /// `input.blocks()`. Every revision, source range, atlas lookup, geometry
    /// and primitive-budget check completes before the scene is mutated, so a
    /// stale or partially materialized viewport cannot publish a prefix of its
    /// primitives.
    pub fn append_viewport(
        &mut self,
        input: &ViewportSceneInput,
        layouts: &[&LayoutSnapshot],
        atlas: &yu_font::GlyphAtlas,
        font_size: f32,
        color: Rgba8,
    ) -> Result<usize, SceneError> {
        self.append_viewport_with_fills(input, layouts, atlas, font_size, color, &[])
    }

    /// Appends every visible block and optional background fills in one
    /// preflighted scene transaction. `fills` is ordered like
    /// `input.blocks()`; a `None` entry keeps the block glyph-only. The scene
    /// layer stays Markdown-agnostic: callers choose a color from their own
    /// block-kind/style policy, while this method only validates geometry and
    /// preserves fill-before-glyph painter order.
    pub fn append_viewport_with_fills(
        &mut self,
        input: &ViewportSceneInput,
        layouts: &[&LayoutSnapshot],
        atlas: &yu_font::GlyphAtlas,
        font_size: f32,
        color: Rgba8,
        fills: &[Option<Rgba8>],
    ) -> Result<usize, SceneError> {
        self.append_viewport_with_fills_and_images(
            input,
            layouts,
            atlas,
            font_size,
            color,
            fills,
            &[],
        )
    }

    /// Appends visible blocks with optional backgrounds and source-backed
    /// image overlays. Images are ordered after the block's glyphs so an
    /// opaque ready texture or fallback placeholder covers the projected alt
    /// label without changing canonical source text.
    #[allow(clippy::too_many_arguments)]
    pub fn append_viewport_with_fills_and_images(
        &mut self,
        input: &ViewportSceneInput,
        layouts: &[&LayoutSnapshot],
        atlas: &yu_font::GlyphAtlas,
        font_size: f32,
        color: Rgba8,
        fills: &[Option<Rgba8>],
        images: &[Vec<ImagePrimitive>],
    ) -> Result<usize, SceneError> {
        self.append_viewport_with_fills_and_images_and_tables(
            input, layouts, atlas, font_size, color, fills, images, None, None,
        )
    }

    /// Appends visible blocks with optional table decorations. Table fills and
    /// borders are emitted before the block glyphs, so a cell overlay cannot
    /// cover its source-backed text. The optional selection is source-based
    /// and is applied only to table layouts in the same revision.
    #[allow(clippy::too_many_arguments)]
    pub fn append_viewport_with_fills_and_images_and_tables(
        &mut self,
        input: &ViewportSceneInput,
        layouts: &[&LayoutSnapshot],
        atlas: &yu_font::GlyphAtlas,
        font_size: f32,
        color: Rgba8,
        fills: &[Option<Rgba8>],
        images: &[Vec<ImagePrimitive>],
        table_style: Option<TableSceneStyle>,
        selection: Option<yu_core::TextRange>,
    ) -> Result<usize, SceneError> {
        if input.revision() != self.revision {
            return Err(SceneError::ViewportRevisionMismatch {
                expected: self.revision,
                actual: input.revision(),
            });
        }
        if layouts.len() != input.blocks().len() {
            return Err(SceneError::InvalidViewportInput(
                "viewport layout count must match input blocks",
            ));
        }
        if !fills.is_empty() && fills.len() != input.blocks().len() {
            return Err(SceneError::InvalidViewportInput(
                "viewport fill count must match input blocks",
            ));
        }
        if !images.is_empty() && images.len() != input.blocks().len() {
            return Err(SceneError::InvalidViewportInput(
                "viewport image count must match input blocks",
            ));
        }

        let mut primitives = Vec::new();
        for (offset, (geometry, layout)) in input
            .blocks()
            .iter()
            .copied()
            .zip(layouts.iter().copied())
            .enumerate()
        {
            if geometry.revision() != self.revision {
                return Err(SceneError::ViewportRevisionMismatch {
                    expected: self.revision,
                    actual: geometry.revision(),
                });
            }
            if layout.revision() != geometry.revision() {
                return Err(SceneError::RevisionMismatch {
                    scene: geometry.revision(),
                    layout: layout.revision(),
                });
            }
            if layout.source_range() != geometry.source() {
                return Err(SceneError::ViewportSourceMismatch);
            }
            if let Some(fill) = fills.get(offset).copied().flatten() {
                let bounds = Rect::new(
                    self.viewport.x(),
                    geometry.y(),
                    self.viewport.width(),
                    geometry.height(),
                )?;
                primitives.push(Primitive::FillRect {
                    bounds,
                    color: fill,
                });
            }
            if let (Some(style), Some(table)) = (table_style, layout.table()) {
                primitives.extend(self.collect_table_primitives(
                    table,
                    Point::new(0.0, geometry.y()),
                    style,
                    selection,
                )?);
            }
            primitives.extend(
                self.collect_layout_primitives_at(
                    layout,
                    atlas,
                    font_size,
                    color,
                    Point::new(0.0, geometry.y()),
                )?
                .into_iter()
                .map(Primitive::Glyph),
            );
            if let Some(block_images) = images.get(offset) {
                primitives.extend(block_images.iter().copied().map(Primitive::Image));
            }
        }
        self.commit_primitives(primitives)
    }

    fn collect_layout_primitives_at(
        &self,
        layout: &LayoutSnapshot,
        atlas: &yu_font::GlyphAtlas,
        font_size: f32,
        color: Rgba8,
        origin: Point,
    ) -> Result<Vec<GlyphPrimitive>, SceneError> {
        if self.revision != layout.revision() {
            return Err(SceneError::RevisionMismatch {
                scene: self.revision,
                layout: layout.revision(),
            });
        }
        origin.validate()?;
        if !font_size.is_finite() || font_size <= 0.0 {
            return Err(SceneError::InvalidFontSize(font_size.to_bits()));
        }

        let mut primitives = Vec::with_capacity(layout.glyphs().len());
        for placement in layout.glyphs() {
            let key = GlyphRasterKey::new(placement.face(), placement.glyph(), font_size)
                .map_err(|_| SceneError::InvalidFontSize(font_size.to_bits()))?;
            let entry = atlas.entry(key).ok_or(SceneError::MissingGlyphAtlas(key))?;
            let glyph = GlyphPrimitive::new(
                entry,
                Point::new(origin.x() + placement.x(), origin.y() + placement.y()),
                color,
            );
            glyph.origin.validate()?;
            glyph.bounds().validate()?;
            primitives.push(glyph);
        }

        Ok(primitives)
    }

    fn commit_glyphs(&mut self, glyphs: Vec<GlyphPrimitive>) -> Result<usize, SceneError> {
        self.commit_primitives(glyphs.into_iter().map(Primitive::Glyph).collect())
    }

    fn commit_primitives(&mut self, primitives: Vec<Primitive>) -> Result<usize, SceneError> {
        if primitives.len() > self.max_primitives.saturating_sub(self.primitives.len()) {
            return Err(SceneError::PrimitiveLimitExceeded);
        }
        let new_len = self
            .primitives
            .len()
            .checked_add(primitives.len())
            .ok_or(SceneError::PrimitiveLimitExceeded)?;
        if !primitives.is_empty() && u32::try_from(new_len.saturating_sub(1)).is_err() {
            return Err(SceneError::PrimitiveLimitExceeded);
        }

        let mut damage = self.damage.clone();
        for primitive in &primitives {
            if let Primitive::Glyph(glyph) = primitive {
                glyph.origin.validate()?;
            }
            let bounds = primitive.bounds();
            bounds.validate()?;
            damage.add(bounds)?;
        }
        let count = primitives.len();
        self.primitives.extend(primitives);
        self.damage = damage;
        Ok(count)
    }

    /// Appends one visible block layout using the document-space origin from
    /// a validated viewport input. The source range and revision checks keep
    /// a stale/local layout from being painted at another block's origin.
    pub fn append_layout_at_block(
        &mut self,
        geometry: ViewportBlockGeometry,
        layout: &LayoutSnapshot,
        atlas: &yu_font::GlyphAtlas,
        font_size: f32,
        color: Rgba8,
    ) -> Result<usize, SceneError> {
        if geometry.revision() != self.revision {
            return Err(SceneError::ViewportRevisionMismatch {
                expected: self.revision,
                actual: geometry.revision(),
            });
        }
        if layout.revision() != geometry.revision() {
            return Err(SceneError::RevisionMismatch {
                scene: geometry.revision(),
                layout: layout.revision(),
            });
        }
        if layout.source_range() != geometry.source() {
            return Err(SceneError::ViewportSourceMismatch);
        }
        self.append_layout_at(
            layout,
            atlas,
            font_size,
            color,
            Point::new(0.0, geometry.y()),
        )
    }

    #[must_use]
    pub fn finish(self) -> Scene {
        Scene {
            revision: self.revision,
            viewport: self.viewport,
            primitives: self.primitives,
            damage: self.damage,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::{ByteOffset, TextRange};
    use yu_font::{
        GlyphAtlas, GlyphAtlasConfig, GlyphBitmap, GlyphMetrics, GlyphRasterKey, RasterizedGlyph,
    };
    use yu_projection::Projection;
    use yu_text::TextBuffer;

    fn atlas_entry(glyph: u32) -> AtlasEntry {
        let key = GlyphRasterKey::new(
            yu_font::FontFaceId::from_raw(1),
            yu_font::GlyphId::from_raw(glyph),
            12.0,
        )
        .expect("key");
        let bitmap = GlyphBitmap::new(2, 3, 2, vec![255; 6]).expect("bitmap");
        let metrics = GlyphMetrics::new(1.0, 9.0, 8.0).expect("metrics");
        let glyph = RasterizedGlyph::new(key, metrics, bitmap);
        let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::new(16, 16, 1).expect("config"));
        atlas.insert(glyph).expect("atlas entry")
    }

    #[test]
    fn damage_merges_touching_rectangles_and_collapses_over_budget() {
        let mut damage = DamageSet::new(2).expect("budget");
        damage
            .add(Rect::new(0.0, 0.0, 10.0, 10.0).expect("rect"))
            .expect("damage");
        damage
            .add(Rect::new(10.0, 0.0, 4.0, 4.0).expect("touching rect"))
            .expect("damage");
        assert_eq!(damage.rects().len(), 1);
        assert_eq!(damage.bounds().expect("bounds").width(), 14.0);

        damage
            .add(Rect::new(100.0, 100.0, 2.0, 2.0).expect("rect"))
            .expect("damage");
        damage
            .add(Rect::new(200.0, 200.0, 2.0, 2.0).expect("rect"))
            .expect("damage");
        assert_eq!(damage.rects().len(), 1);
        assert_eq!(damage.bounds().expect("bounds").right(), 202.0);
    }

    #[test]
    fn scene_keeps_revision_order_and_glyph_bounds() {
        let viewport = Rect::new(0.0, 0.0, 640.0, 480.0).expect("viewport");
        let mut builder = SceneBuilder::new(Revision::new(7), viewport)
            .expect("builder")
            .with_damage_budget(8)
            .expect("damage budget");
        let glyph = GlyphPrimitive::new(atlas_entry(1), Point::new(10.0, 20.0), Rgba8::white());
        builder
            .fill_rect(Rect::new(0.0, 0.0, 4.0, 4.0).expect("rect"), Rgba8::black())
            .expect("rect primitive");
        builder.glyph(glyph).expect("glyph primitive");
        let scene = builder.finish();
        assert_eq!(scene.revision(), Revision::new(7));
        assert_eq!(scene.primitives().len(), 2);
        assert_eq!(scene.primitives()[1], Primitive::Glyph(glyph));
        let bounds = glyph.bounds();
        assert_eq!(bounds.x(), 11.0);
        assert_eq!(bounds.y(), 11.0);
        assert_eq!(bounds.width(), 2.0);
        assert_eq!(bounds.height(), 3.0);
    }

    #[test]
    fn invalid_geometry_is_rejected_before_scene_publication() {
        assert!(Rect::new(0.0, 0.0, f32::NAN, 1.0).is_err());
        assert!(Point::new(f32::INFINITY, 0.0).validate().is_err());
        assert_eq!(DamageSet::new(0), Err(SceneError::InvalidDamageBudget));
    }

    #[test]
    fn table_scene_primitives_are_source_backed_and_painter_ordered() {
        let source = "| A | B |\n| --- | :---: |\n| 1 | 2 |\n";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let markdown = yu_markdown::parse(&snapshot);
        let block = markdown.blocks().get(0).expect("table block");
        let projection = yu_projection::BlockProjection::from_block_with_definitions(
            &snapshot,
            block,
            markdown.reference_definitions(),
        )
        .expect("table projection");
        let layout = LayoutSnapshot::from_block_projection(
            &projection,
            yu_layout::LayoutConfig::new(20.0, 2.0),
        )
        .expect("table layout");
        let table = layout.table().expect("table layout");
        let selected = table.cells()[3].source();
        let style = TableSceneStyle::new(
            1.0,
            Rgba8::new(150, 155, 165, 255),
            Some(Rgba8::new(235, 238, 244, 255)),
            Some(Rgba8::new(210, 225, 255, 255)),
        );
        let mut builder = SceneBuilder::new(
            layout.revision(),
            Rect::new(0.0, 0.0, 40.0, 20.0).expect("viewport"),
        )
        .expect("builder");
        let count = builder
            .append_table_with_selection(table, Point::new(10.0, 20.0), style, Some(selected))
            .expect("table scene");
        assert_eq!(count, 9);
        let scene = builder.finish();
        assert_eq!(scene.primitives().len(), 9);
        assert!(matches!(
            scene.primitives()[0],
            Primitive::Table(TablePrimitive {
                role: TablePrimitiveRole::HeaderFill,
                ..
            })
        ));
        assert!(matches!(
            scene.primitives()[2],
            Primitive::Table(TablePrimitive {
                role: TablePrimitiveRole::SelectionFill,
                ..
            })
        ));
        assert!(scene.primitives()[2].bounds().x() >= 10.0);
        assert!(scene.primitives()[2].bounds().y() >= 20.0);
        assert!(scene.primitives()[3..].iter().all(|primitive| matches!(
            primitive,
            Primitive::Table(TablePrimitive {
                role: TablePrimitiveRole::Border,
                source,
                ..
            }) if *source == table.source_range()
        )));
        assert_eq!(scene.revision(), layout.revision());
    }

    #[test]
    fn table_scene_rejects_stale_layout_and_invalid_border_width() {
        let source = "| A | B |\n| --- | --- |\n| 1 | 2 |\n";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let markdown = yu_markdown::parse(&snapshot);
        let block = markdown.blocks().get(0).expect("table block");
        let projection =
            yu_projection::BlockProjection::from_block(&snapshot, block).expect("table projection");
        let layout = LayoutSnapshot::from_block_projection(
            &projection,
            yu_layout::LayoutConfig::new(20.0, 2.0),
        )
        .expect("table layout");
        let style = TableSceneStyle::new(1.0, Rgba8::black(), None, None);
        let mut stale = SceneBuilder::new(
            Revision::new(1),
            Rect::new(0.0, 0.0, 40.0, 20.0).expect("viewport"),
        )
        .expect("builder");
        assert!(matches!(
            stale.append_table(layout.table().expect("table"), Point::new(0.0, 0.0), style),
            Err(SceneError::RevisionMismatch { .. })
        ));
        let mut invalid = SceneBuilder::new(
            layout.revision(),
            Rect::new(0.0, 0.0, 40.0, 20.0).expect("viewport"),
        )
        .expect("builder");
        let invalid_style = TableSceneStyle::new(f32::NAN, Rgba8::black(), None, None);
        assert!(matches!(
            invalid.append_table(
                layout.table().expect("table"),
                Point::new(0.0, 0.0),
                invalid_style
            ),
            Err(SceneError::InvalidTableStyle(_))
        ));
        assert!(invalid.finish().primitives().is_empty());
    }

    #[test]
    fn viewport_images_are_appended_after_block_content() {
        let source = TextBuffer::new("").snapshot();
        let source_range = TextRange::new(ByteOffset::ZERO, ByteOffset::ZERO).expect("range");
        let projection = Projection::inline(&source, source_range).expect("projection");
        let layout =
            LayoutSnapshot::from_projection(&projection, yu_layout::LayoutConfig::new(80.0, 10.0))
                .expect("layout");
        let revision = layout.revision();
        let geometry = ViewportBlockGeometry::new(revision, 0, source_range, 0.0, 10.0, true, 0)
            .expect("geometry");
        let input = ViewportSceneInput::new(revision, 0..1, 10.0, vec![geometry]).expect("input");
        let bounds = Rect::new(4.0, 0.0, 32.0, 10.0).expect("image bounds");
        let image = ImagePrimitive::new(42, bounds, Rgba8::new(232, 234, 238, 255));
        let atlas = GlyphAtlas::new(GlyphAtlasConfig::default());
        let mut builder =
            SceneBuilder::new(revision, Rect::new(0.0, 0.0, 80.0, 10.0).expect("viewport"))
                .expect("builder");
        let count = builder
            .append_viewport_with_fills_and_images(
                &input,
                &[&layout],
                &atlas,
                12.0,
                Rgba8::black(),
                &[],
                &[vec![image]],
            )
            .expect("append image");
        assert_eq!(count, 1);
        assert_eq!(builder.finish().primitives(), &[Primitive::Image(image)]);
    }
}
