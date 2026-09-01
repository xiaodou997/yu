//! 后端无关的那一半：`RenderPlan` → 平铺绘制指令、damage 裁剪、surface 像素
//! 取整、Revision 闸门。
//!
//! 这些逻辑原来住在 `yu-render-macos` 里，但它们**一个原生指针都没有**：
//! 换成 Direct3D 之后每一行都成立。留在平台层的代价是第二端要抄第二遍，而
//! 抄错的表现是「滚动之后字形互相重叠」（`requires_full_clear` 少一个条件）
//! 与「damage 剔多了一块不刷新」——两样都不 panic、不报错。
//!
//! 平台那一侧剩下的是真正需要原生对象的部分：device / queue / pipeline /
//! drawable / texture，以及把这里产出的两个 `#[repr(C)]` 数组交给
//! Metal（或 D3D）桥的那一次调用。
//!
//! [`DrawCommand`] 与 [`RenderCommand`] 不是同一层：后者是 plan 里的枚举，
//! 带类型、带坐标空间；前者是**已经平铺成 ABI 数组**的一条指令，坐标相对
//! render plan 的 viewport，颜色已经归一化。翻译只有这一份实现。

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use yu_core::{Device, Document, Point, Rect, Revision, Scale};

use crate::{RenderCommand, RenderPlan};

/// 后端无关的那一半会产生的错误。
///
/// 平台的错误类型是它的超集（那一侧还有 device / drawable / encoder 之类的
/// 失败），各自用 `From` 把这几条映射进去。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendError {
    MissingAtlasPage(u32),
    InvalidRenderCommand(&'static str),
    InvalidDamageRect(&'static str),
    InvalidSurfaceConfig(&'static str),
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
}

impl fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingAtlasPage(page) => {
                write!(
                    formatter,
                    "render plan references missing atlas page {page}"
                )
            }
            Self::InvalidRenderCommand(message) => formatter.write_str(message),
            Self::InvalidDamageRect(message) => formatter.write_str(message),
            Self::InvalidSurfaceConfig(message) => formatter.write_str(message),
            Self::StaleRevision { expected, actual } => write!(
                formatter,
                "frame revision {actual:?} is stale for current {expected:?}"
            ),
        }
    }
}

impl Error for BackendError {}

pub const DRAW_FILL_RECT: u32 = 0;
pub const DRAW_GLYPH: u32 = 1;
pub const DRAW_IMAGE: u32 = 2;
pub const IMAGE_KIND_REGULAR: u32 = 0;
const IMAGE_KIND_EMBEDDED_SVG_BASE: u32 = 1;

pub const fn embedded_image_kind(kind: u8) -> u32 {
    IMAGE_KIND_EMBEDDED_SVG_BASE + kind as u32
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawCommand {
    pub kind: u32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
    pub page: u32,
    pub resource: u64,
    pub image_kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DamageRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Logical surface dimensions and their drawable pixel size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceConfig {
    logical_width: f64,
    logical_height: f64,
    scale: f64,
    pixel_width: u32,
    pixel_height: u32,
}

impl SurfaceConfig {
    pub fn new(width: f64, height: f64, scale: f64) -> Result<Self, BackendError> {
        if !width.is_finite() || width <= 0.0 {
            return Err(BackendError::InvalidSurfaceConfig(
                "surface width must be finite and positive",
            ));
        }
        if !height.is_finite() || height <= 0.0 {
            return Err(BackendError::InvalidSurfaceConfig(
                "surface height must be finite and positive",
            ));
        }
        if !scale.is_finite() || scale <= 0.0 {
            return Err(BackendError::InvalidSurfaceConfig(
                "surface scale must be finite and positive",
            ));
        }
        let pixel_width = pixels(width, scale)?;
        let pixel_height = pixels(height, scale)?;
        Ok(Self {
            logical_width: width,
            logical_height: height,
            scale,
            pixel_width,
            pixel_height,
        })
    }

    #[must_use]
    pub const fn logical_width(self) -> f64 {
        self.logical_width
    }

    #[must_use]
    pub const fn logical_height(self) -> f64 {
        self.logical_height
    }

    #[must_use]
    pub const fn scale(self) -> f64 {
        self.scale
    }

    #[must_use]
    pub const fn pixel_width(self) -> u32 {
        self.pixel_width
    }

    #[must_use]
    pub const fn pixel_height(self) -> u32 {
        self.pixel_height
    }
}

fn pixels(value: f64, scale: f64) -> Result<u32, BackendError> {
    let value = (value * scale).ceil();
    if !value.is_finite() || value <= 0.0 || value > f64::from(u32::MAX) {
        return Err(BackendError::InvalidSurfaceConfig(
            "surface pixel dimensions overflow u32",
        ));
    }
    Ok(value as u32)
}

/// 决定这一帧是否必须整体重绘，而不是只重绘 damage 区域。
///
/// 保持为纯函数，这样滚动/resize/surface 重建的组合可以在没有 Metal device
/// 的情况下测试——渲染路径本身的测试都需要真实 GPU，是 ignored 的。
///
/// 关键的一条是 `last_viewport != viewport`：damage 描述的是**内容**的变化，
/// 表达不了 viewport 自身的位移。滚动时每个 block 的内容都没变，damage 因此
/// 可能是空的，但屏幕上所有字形的位置都变了；沿用局部重绘会把旧字形留在
/// retained target 上，表现为滚动后字形互相重叠。
pub fn requires_full_clear(
    recreated_target: bool,
    needs_full_clear: bool,
    last_viewport: Option<yu_scene::Rect>,
    viewport: yu_scene::Rect,
    last_surface_generation: Option<u64>,
    surface_generation: u64,
) -> bool {
    recreated_target
        || needs_full_clear
        || last_viewport != Some(viewport)
        || last_surface_generation != Some(surface_generation)
}

/// 帧的 Revision 闸门：后端提交成功之后，不许一帧更旧的悄悄顶上去。
///
/// 它没有任何原生指针，也不认识 `yu-workspace` 的帧类型——调用方自己把帧的
/// Revision 取出来问。后端各持有一个实例。
#[derive(Clone, Debug, Default)]
pub struct FrameConsumer {
    last_revision: Option<Revision>,
}

impl FrameConsumer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn last_revision(&self) -> Option<Revision> {
        self.last_revision
    }

    pub fn validate_revision(
        &self,
        current_revision: Revision,
        frame_revision: Revision,
    ) -> Result<(), BackendError> {
        if frame_revision != current_revision {
            return Err(BackendError::StaleRevision {
                expected: current_revision,
                actual: frame_revision,
            });
        }
        if let Some(last_revision) = self.last_revision
            && last_revision > frame_revision
        {
            return Err(BackendError::StaleRevision {
                expected: last_revision,
                actual: frame_revision,
            });
        }
        Ok(())
    }

    pub fn commit_revision(
        &mut self,
        current_revision: Revision,
        frame_revision: Revision,
    ) -> Result<(), BackendError> {
        self.validate_revision(current_revision, frame_revision)?;
        self.last_revision = Some(frame_revision);
        Ok(())
    }
}

fn normalized_channel(channel: u8) -> f32 {
    f32::from(channel) / 255.0
}

pub fn build_draw_commands(
    plan: &RenderPlan,
    page_sizes: &BTreeMap<u32, (u32, u32)>,
    image_sizes: &BTreeMap<u64, (u32, u32)>,
    embedded_image_sizes: &BTreeMap<(u64, u32), (u32, u32)>,
) -> Result<Vec<DrawCommand>, BackendError> {
    let viewport = plan.viewport();
    // 栅格化缩放：逻辑坐标 → 物理像素。方向进类型，反方向只能走 `unscale`。
    let raster = Scale::<Document, Device>::new(plan.raster_scale()).map_err(|_| {
        BackendError::InvalidRenderCommand("render plan raster scale must be finite and positive")
    })?;
    let mut commands = Vec::with_capacity(plan.commands().len());
    for command in plan.commands() {
        match *command {
            RenderCommand::FillRect { bounds, color } => {
                if !bounds.x().is_finite()
                    || !bounds.y().is_finite()
                    || !bounds.width().is_finite()
                    || !bounds.height().is_finite()
                {
                    return Err(BackendError::InvalidRenderCommand(
                        "fill rectangle geometry is not finite",
                    ));
                }
                if bounds.width() == 0.0 || bounds.height() == 0.0 {
                    continue;
                }
                let x = bounds.x() - viewport.x();
                let y = bounds.y() - viewport.y();
                if !x.is_finite() || !y.is_finite() {
                    return Err(BackendError::InvalidRenderCommand(
                        "fill rectangle position is not finite",
                    ));
                }
                commands.push(DrawCommand {
                    kind: DRAW_FILL_RECT,
                    x,
                    y,
                    width: bounds.width(),
                    height: bounds.height(),
                    u0: 0.0,
                    v0: 0.0,
                    u1: 0.0,
                    v1: 0.0,
                    red: normalized_channel(color.red()),
                    green: normalized_channel(color.green()),
                    blue: normalized_channel(color.blue()),
                    alpha: normalized_channel(color.alpha()),
                    page: u32::MAX,
                    resource: 0,
                    image_kind: IMAGE_KIND_REGULAR,
                });
            }
            RenderCommand::Glyph {
                page,
                rect,
                origin,
                metrics,
                color,
            } => {
                let Some(page) = page else {
                    // Empty glyphs keep their advance in layout but have no
                    // coverage pixels to submit to Metal.
                    continue;
                };
                let Some(&(page_width, page_height)) = page_sizes.get(&page) else {
                    return Err(BackendError::MissingAtlasPage(page));
                };
                if page_width == 0 || page_height == 0 {
                    return Err(BackendError::InvalidRenderCommand(
                        "atlas page dimensions must be positive",
                    ));
                }
                let rect_right = u64::from(rect.x()) + u64::from(rect.width());
                let rect_bottom = u64::from(rect.y()) + u64::from(rect.height());
                if rect_right > u64::from(page_width) || rect_bottom > u64::from(page_height) {
                    return Err(BackendError::InvalidRenderCommand(
                        "glyph atlas rectangle exceeds its page",
                    ));
                }
                if rect.width() == 0 || rect.height() == 0 {
                    continue;
                }
                // atlas 矩形与 bearing 都是按 `font_size × raster_scale`
                // 栅格化出来的物理像素；除回逻辑坐标后，quad 才与 shader 使用
                // 的 document-space 单位一致，纹理也才能与 Retina 的物理像素
                // 1:1 对应而不被拉伸。
                //
                // 换算方向写在类型里而不只是写在这段注释里：`raster` 是
                // Document → Device，这里要的是反方向，所以是 `unscale`。
                // 乘错方向（e751f71 与 5fac1fe 那一类）现在编译不过。
                let device_quad = Rect::<Device>::new(
                    metrics.bearing_x(),
                    -metrics.bearing_y(),
                    rect.width() as f32,
                    rect.height() as f32,
                )
                .map_err(|_| {
                    BackendError::InvalidRenderCommand("glyph atlas quad is not finite")
                })?;
                let quad = Rect::<Document>::unscale(device_quad, raster)
                    .and_then(|logical| {
                        logical.translate_into(Point::<Document>::new(
                            origin.x() - viewport.x(),
                            origin.y() - viewport.y(),
                        ))
                    })
                    .map_err(|_| {
                        BackendError::InvalidRenderCommand("glyph origin is not finite")
                    })?;
                let (x, y, width, height) = (quad.x(), quad.y(), quad.width(), quad.height());
                commands.push(DrawCommand {
                    kind: DRAW_GLYPH,
                    x,
                    y,
                    width,
                    height,
                    u0: rect.x() as f32 / page_width as f32,
                    v0: rect.y() as f32 / page_height as f32,
                    u1: rect_right as f32 / page_width as f32,
                    v1: rect_bottom as f32 / page_height as f32,
                    red: normalized_channel(color.red()),
                    green: normalized_channel(color.green()),
                    blue: normalized_channel(color.blue()),
                    alpha: normalized_channel(color.alpha()),
                    page,
                    resource: 0,
                    image_kind: IMAGE_KIND_REGULAR,
                });
            }
            RenderCommand::Image {
                resource,
                bounds,
                fallback,
            } => {
                if !bounds.x().is_finite()
                    || !bounds.y().is_finite()
                    || !bounds.width().is_finite()
                    || !bounds.height().is_finite()
                {
                    return Err(BackendError::InvalidRenderCommand(
                        "image geometry is not finite",
                    ));
                }
                if bounds.width() == 0.0 || bounds.height() == 0.0 {
                    continue;
                }
                let x = bounds.x() - viewport.x();
                let y = bounds.y() - viewport.y();
                if !x.is_finite() || !y.is_finite() {
                    return Err(BackendError::InvalidRenderCommand(
                        "image position is not finite",
                    ));
                }
                if image_sizes.contains_key(&resource) {
                    commands.push(DrawCommand {
                        kind: DRAW_IMAGE,
                        x,
                        y,
                        width: bounds.width(),
                        height: bounds.height(),
                        u0: 0.0,
                        v0: 0.0,
                        u1: 1.0,
                        v1: 1.0,
                        red: 1.0,
                        green: 1.0,
                        blue: 1.0,
                        alpha: 1.0,
                        page: u32::MAX,
                        resource,
                        image_kind: IMAGE_KIND_REGULAR,
                    });
                } else {
                    commands.push(DrawCommand {
                        kind: DRAW_FILL_RECT,
                        x,
                        y,
                        width: bounds.width(),
                        height: bounds.height(),
                        u0: 0.0,
                        v0: 0.0,
                        u1: 0.0,
                        v1: 0.0,
                        red: normalized_channel(fallback.red()),
                        green: normalized_channel(fallback.green()),
                        blue: normalized_channel(fallback.blue()),
                        alpha: normalized_channel(fallback.alpha()),
                        page: u32::MAX,
                        resource: 0,
                        image_kind: IMAGE_KIND_REGULAR,
                    });
                }
            }
            RenderCommand::EmbeddedSvg {
                resource,
                kind,
                bounds,
                fallback,
                ..
            } => {
                if !bounds.x().is_finite()
                    || !bounds.y().is_finite()
                    || !bounds.width().is_finite()
                    || !bounds.height().is_finite()
                {
                    return Err(BackendError::InvalidRenderCommand(
                        "embedded SVG geometry is not finite",
                    ));
                }
                if bounds.width() == 0.0 || bounds.height() == 0.0 {
                    continue;
                }
                let x = bounds.x() - viewport.x();
                let y = bounds.y() - viewport.y();
                if !x.is_finite() || !y.is_finite() {
                    return Err(BackendError::InvalidRenderCommand(
                        "embedded SVG position is not finite",
                    ));
                }
                if embedded_image_sizes.contains_key(&(resource, u32::from(kind))) {
                    commands.push(DrawCommand {
                        kind: DRAW_IMAGE,
                        x,
                        y,
                        width: bounds.width(),
                        height: bounds.height(),
                        u0: 0.0,
                        v0: 0.0,
                        u1: 1.0,
                        v1: 1.0,
                        red: 1.0,
                        green: 1.0,
                        blue: 1.0,
                        alpha: 1.0,
                        page: u32::MAX,
                        resource,
                        image_kind: embedded_image_kind(kind),
                    });
                } else {
                    commands.push(DrawCommand {
                        kind: DRAW_FILL_RECT,
                        x,
                        y,
                        width: bounds.width(),
                        height: bounds.height(),
                        u0: 0.0,
                        v0: 0.0,
                        u1: 0.0,
                        v1: 0.0,
                        red: normalized_channel(fallback.red()),
                        green: normalized_channel(fallback.green()),
                        blue: normalized_channel(fallback.blue()),
                        alpha: normalized_channel(fallback.alpha()),
                        page: u32::MAX,
                        resource: 0,
                        image_kind: IMAGE_KIND_REGULAR,
                    });
                }
            }
        }
    }
    Ok(commands)
}

pub fn build_damage_rects(plan: &RenderPlan) -> Result<Vec<DamageRect>, BackendError> {
    let viewport = plan.viewport();
    if !viewport.x().is_finite()
        || !viewport.y().is_finite()
        || !viewport.width().is_finite()
        || !viewport.height().is_finite()
        || viewport.width() <= 0.0
        || viewport.height() <= 0.0
    {
        return Err(BackendError::InvalidDamageRect(
            "render plan viewport is not finite and positive",
        ));
    }

    let mut damage = Vec::with_capacity(plan.damage().len());
    for rect in plan.damage() {
        if !rect.x().is_finite()
            || !rect.y().is_finite()
            || !rect.width().is_finite()
            || !rect.height().is_finite()
            || rect.width() < 0.0
            || rect.height() < 0.0
        {
            return Err(BackendError::InvalidDamageRect(
                "damage rectangle must be finite and non-negative",
            ));
        }
        let x = rect.x() - viewport.x();
        let y = rect.y() - viewport.y();
        let right = x + rect.width();
        let bottom = y + rect.height();
        if !x.is_finite() || !y.is_finite() || !right.is_finite() || !bottom.is_finite() {
            return Err(BackendError::InvalidDamageRect(
                "damage rectangle overflowed viewport coordinates",
            ));
        }
        let left = x.max(0.0);
        let top = y.max(0.0);
        let right = right.min(viewport.width());
        let bottom = bottom.min(viewport.height());
        if right <= left || bottom <= top {
            continue;
        }
        damage.push(DamageRect {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        });
    }
    Ok(damage)
}

/// Keeps only commands whose covered bounds intersect at least one dirty
/// region. Coordinates are already relative to the render-plan viewport, the
/// same coordinate space used by `build_damage_rects` and the native scissor
/// ABI. Painter order is preserved, and a command is kept once even when it
/// intersects multiple damage regions.
pub fn cull_draw_commands(commands: Vec<DrawCommand>, damage: &[DamageRect]) -> Vec<DrawCommand> {
    commands
        .into_iter()
        .filter(|command| {
            damage
                .iter()
                .copied()
                .any(|region| command_intersects_damage(*command, region))
        })
        .collect()
}

fn command_intersects_damage(command: DrawCommand, damage: DamageRect) -> bool {
    let command_right = command.x + command.width;
    let command_bottom = command.y + command.height;
    let damage_right = damage.x + damage.width;
    let damage_bottom = damage.y + damage.height;
    command.x < damage_right
        && damage.x < command_right
        && command.y < damage_bottom
        && damage.y < command_bottom
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RenderPlanBuilder;
    use yu_scene::Rgba8;

    #[test]
    fn surface_config_rounds_logical_size_to_drawable_pixels() {
        let config = SurfaceConfig::new(640.0, 479.5, 2.0).expect("config");
        assert_eq!(config.pixel_width(), 1280);
        assert_eq!(config.pixel_height(), 959);
    }

    #[test]
    fn surface_config_rejects_invalid_dimensions() {
        assert!(SurfaceConfig::new(0.0, 10.0, 2.0).is_err());
        assert!(SurfaceConfig::new(10.0, f64::NAN, 2.0).is_err());
        assert!(SurfaceConfig::new(10.0, 10.0, 0.0).is_err());
    }

    #[test]
    fn frame_consumer_rejects_stale_frames_and_revision_rollback() {
        use yu_core::Revision;

        let revision_one = Revision::new(1);
        let revision_two = Revision::new(2);
        let mut consumer = FrameConsumer::new();

        assert_eq!(consumer.last_revision(), None);
        consumer
            .commit_revision(revision_one, revision_one)
            .expect("first frame");
        assert_eq!(consumer.last_revision(), Some(revision_one));

        assert_eq!(
            consumer.validate_revision(revision_two, revision_one),
            Err(BackendError::StaleRevision {
                expected: revision_two,
                actual: revision_one,
            })
        );
        assert_eq!(consumer.last_revision(), Some(revision_one));

        consumer
            .commit_revision(revision_two, revision_two)
            .expect("new frame");
        assert_eq!(consumer.last_revision(), Some(revision_two));

        assert_eq!(
            consumer.validate_revision(revision_one, revision_one),
            Err(BackendError::StaleRevision {
                expected: revision_two,
                actual: revision_one,
            })
        );
        assert_eq!(consumer.last_revision(), Some(revision_two));
    }

    #[test]
    fn draw_command_conversion_keeps_painter_order_and_atlas_uvs() {
        use std::collections::BTreeMap;

        use yu_core::Revision;
        use yu_font::{
            FontFaceId, GlyphAtlas, GlyphAtlasConfig, GlyphBitmap, GlyphId, GlyphMetrics,
            GlyphRasterKey, RasterizedGlyph,
        };
        use yu_scene::{GlyphPrimitive, Point, Rect, SceneBuilder};

        let key =
            GlyphRasterKey::new(FontFaceId::from_raw(3), GlyphId::from_raw(11), 14.0).expect("key");
        let bitmap = GlyphBitmap::new(4, 6, 4, vec![255; 24]).expect("bitmap");
        let metrics = GlyphMetrics::new(1.0, 7.0, 5.0).expect("metrics");
        let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::new(16, 16, 1).expect("config"));
        let entry = atlas
            .insert(RasterizedGlyph::new(key, metrics, bitmap))
            .expect("entry");
        let viewport = Rect::new(10.0, 20.0, 80.0, 40.0).expect("viewport");
        let mut scene = SceneBuilder::new(Revision::INITIAL, viewport).expect("scene");
        scene
            .fill_rect(
                Rect::new(10.0, 20.0, 20.0, 5.0).expect("rect"),
                Rgba8::new(10, 20, 30, 255),
            )
            .expect("fill");
        scene
            .glyph(
                GlyphPrimitive::new(entry, Point::new(14.0, 32.0), Rgba8::white())
                    .expect("glyph bounds"),
            )
            .expect("glyph");
        let plan = RenderPlanBuilder::new()
            .build(&scene.finish(), &atlas)
            .expect("plan");
        let mut page_sizes = BTreeMap::new();
        page_sizes.insert(0, (16, 16));

        let commands = build_draw_commands(&plan, &page_sizes, &BTreeMap::new(), &BTreeMap::new())
            .expect("native commands");
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].kind, DRAW_FILL_RECT);
        assert_eq!(commands[0].x, 0.0);
        assert_eq!(commands[0].y, 0.0);
        assert_eq!(commands[1].kind, DRAW_GLYPH);
        assert_eq!(commands[1].x, 5.0);
        assert_eq!(commands[1].y, 5.0);
        assert_eq!(commands[1].width, 4.0);
        assert_eq!(commands[1].height, 6.0);
        assert_eq!(commands[1].u0, entry.rect().x() as f32 / 16.0);
        assert_eq!(commands[1].v0, entry.rect().y() as f32 / 16.0);
    }

    #[test]
    fn image_draw_command_uses_placeholder_until_resource_is_ready() {
        use std::collections::BTreeMap;

        use yu_core::Revision;
        use yu_scene::{ImagePrimitive, Rect, SceneBuilder};

        let bounds = Rect::new(4.0, 6.0, 32.0, 24.0).expect("image bounds");
        let fallback = Rgba8::new(230, 232, 236, 255);
        let mut scene = SceneBuilder::new(
            Revision::INITIAL,
            Rect::new(0.0, 0.0, 120.0, 80.0).expect("viewport"),
        )
        .expect("scene");
        scene
            .image(ImagePrimitive::new(42, bounds, fallback))
            .expect("image");
        let atlas = yu_font::GlyphAtlas::new(
            yu_font::GlyphAtlasConfig::new(16, 16, 1).expect("atlas config"),
        );
        let plan = RenderPlanBuilder::new()
            .build(&scene.finish(), &atlas)
            .expect("plan");

        let placeholder =
            build_draw_commands(&plan, &BTreeMap::new(), &BTreeMap::new(), &BTreeMap::new())
                .expect("placeholder commands");
        assert_eq!(placeholder.len(), 1);
        assert_eq!(placeholder[0].kind, DRAW_FILL_RECT);
        assert_eq!(placeholder[0].resource, 0);
        assert_eq!(placeholder[0].red, normalized_channel(fallback.red()));

        let mut ready = BTreeMap::new();
        ready.insert(42, (2, 2));
        let image = build_draw_commands(&plan, &BTreeMap::new(), &ready, &BTreeMap::new())
            .expect("image command");
        assert_eq!(image.len(), 1);
        assert_eq!(image[0].kind, DRAW_IMAGE);
        assert_eq!(image[0].resource, 42);
        assert_eq!(image[0].u1, 1.0);
    }

    #[test]
    fn draw_command_conversion_rejects_missing_atlas_page() {
        use std::collections::BTreeMap;

        use yu_core::Revision;
        use yu_font::{
            FontFaceId, GlyphAtlas, GlyphAtlasConfig, GlyphBitmap, GlyphId, GlyphMetrics,
            GlyphRasterKey, RasterizedGlyph,
        };
        use yu_scene::{GlyphPrimitive, Point, Rect, SceneBuilder};

        let key =
            GlyphRasterKey::new(FontFaceId::from_raw(3), GlyphId::from_raw(11), 14.0).expect("key");
        let bitmap = GlyphBitmap::new(2, 2, 2, vec![255; 4]).expect("bitmap");
        let metrics = GlyphMetrics::new(0.0, 2.0, 2.0).expect("metrics");
        let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::new(8, 8, 1).expect("config"));
        let entry = atlas
            .insert(RasterizedGlyph::new(key, metrics, bitmap))
            .expect("entry");
        let viewport = Rect::new(0.0, 0.0, 20.0, 20.0).expect("viewport");
        let mut scene = SceneBuilder::new(Revision::INITIAL, viewport).expect("scene");
        scene
            .glyph(
                GlyphPrimitive::new(entry, Point::new(2.0, 4.0), Rgba8::white())
                    .expect("glyph bounds"),
            )
            .expect("glyph");
        let plan = RenderPlanBuilder::new()
            .build(&scene.finish(), &atlas)
            .expect("plan");

        assert_eq!(
            build_draw_commands(&plan, &BTreeMap::new(), &BTreeMap::new(), &BTreeMap::new(),)
                .expect_err("missing page"),
            BackendError::MissingAtlasPage(0)
        );
    }

    #[test]
    fn damage_conversion_clips_to_the_plan_viewport() {
        use yu_core::Revision;
        use yu_scene::{Rect, SceneBuilder};

        let viewport = Rect::new(10.0, 20.0, 40.0, 30.0).expect("viewport");
        let mut scene = SceneBuilder::new(Revision::INITIAL, viewport).expect("scene");
        scene
            .fill_rect(
                Rect::new(5.0, 10.0, 20.0, 20.0).expect("partially visible rect"),
                Rgba8::white(),
            )
            .expect("fill");
        let plan = RenderPlanBuilder::new()
            .build(
                &scene.finish(),
                &yu_font::GlyphAtlas::new(
                    yu_font::GlyphAtlasConfig::new(8, 8, 1).expect("atlas config"),
                ),
            )
            .expect("plan");

        let damage = build_damage_rects(&plan).expect("damage");
        assert_eq!(damage.len(), 1);
        assert_eq!(damage[0].x, 0.0);
        assert_eq!(damage[0].y, 0.0);
        assert_eq!(damage[0].width, 15.0);
        assert_eq!(damage[0].height, 10.0);
    }

    #[test]
    fn damage_culling_preserves_order_and_drops_disjoint_commands() {
        let commands = vec![
            DrawCommand {
                kind: DRAW_FILL_RECT,
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                u0: 0.0,
                v0: 0.0,
                u1: 0.0,
                v1: 0.0,
                red: 1.0,
                green: 0.0,
                blue: 0.0,
                alpha: 1.0,
                page: u32::MAX,
                resource: 0,
                image_kind: IMAGE_KIND_REGULAR,
            },
            DrawCommand {
                kind: DRAW_GLYPH,
                x: 30.0,
                y: 4.0,
                width: 6.0,
                height: 8.0,
                u0: 0.0,
                v0: 0.0,
                u1: 0.5,
                v1: 0.5,
                red: 1.0,
                green: 1.0,
                blue: 1.0,
                alpha: 1.0,
                page: 2,
                resource: 0,
                image_kind: IMAGE_KIND_REGULAR,
            },
            DrawCommand {
                kind: DRAW_FILL_RECT,
                x: 70.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                u0: 0.0,
                v0: 0.0,
                u1: 0.0,
                v1: 0.0,
                red: 0.0,
                green: 1.0,
                blue: 0.0,
                alpha: 1.0,
                page: u32::MAX,
                resource: 0,
                image_kind: IMAGE_KIND_REGULAR,
            },
        ];
        let damage = [DamageRect {
            x: 24.0,
            y: 0.0,
            width: 20.0,
            height: 20.0,
        }];

        let culled = cull_draw_commands(commands, &damage);
        assert_eq!(culled.len(), 1);
        assert_eq!(culled[0].kind, DRAW_GLYPH);
        assert_eq!(culled[0].x, 30.0);
    }

    #[test]
    fn damage_culling_keeps_commands_touching_overlapping_dirty_regions_once() {
        let command = DrawCommand {
            kind: DRAW_FILL_RECT,
            x: 8.0,
            y: 8.0,
            width: 12.0,
            height: 12.0,
            u0: 0.0,
            v0: 0.0,
            u1: 0.0,
            v1: 0.0,
            red: 1.0,
            green: 1.0,
            blue: 1.0,
            alpha: 1.0,
            page: u32::MAX,
            resource: 0,
            image_kind: IMAGE_KIND_REGULAR,
        };
        let damage = [
            DamageRect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            DamageRect {
                x: 15.0,
                y: 15.0,
                width: 10.0,
                height: 10.0,
            },
        ];

        let culled = cull_draw_commands(vec![command], &damage);
        assert_eq!(culled, vec![command]);
    }

    /// glyph quad 必须按 raster scale 除回逻辑坐标。
    ///
    /// 字形按 `font_size × raster_scale` 栅格化，atlas 矩形与 bearing 因此是
    /// 物理像素。若直接当逻辑坐标用，Retina 上字号会翻倍；若不提高取样倍率，
    /// 1x 纹理又会被拉伸到 2x 而发虚。两者必须成对：取样乘、绘制除。
    #[test]
    fn glyph_quad_is_divided_back_to_logical_coordinates() {
        use std::collections::BTreeMap;
        use yu_core::Revision;
        use yu_font::{
            FontFaceId, GlyphAtlas, GlyphAtlasConfig, GlyphBitmap, GlyphId, GlyphMetrics,
            GlyphRasterKey, RasterizedGlyph,
        };
        use yu_scene::{GlyphPrimitive, Point, Rect, SceneBuilder};

        // 2x 取样：24x12 物理像素的位图对应 12x6 逻辑点。
        let key =
            GlyphRasterKey::new(FontFaceId::from_raw(1), GlyphId::from_raw(7), 32.0).expect("key");
        let bitmap = GlyphBitmap::new(24, 12, 24, vec![255; 24 * 12]).expect("bitmap");
        let metrics = GlyphMetrics::new(2.0, 20.0, 30.0).expect("metrics");
        let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::new(64, 64, 1).expect("config"));
        let entry = atlas
            .insert(RasterizedGlyph::new(key, metrics, bitmap))
            .expect("entry");

        let viewport = Rect::new(0.0, 0.0, 200.0, 100.0).expect("viewport");
        let mut scene = SceneBuilder::new(Revision::INITIAL, viewport).expect("scene");
        scene
            .glyph(
                GlyphPrimitive::new(entry, Point::new(50.0, 60.0), Rgba8::white())
                    .expect("glyph bounds"),
            )
            .expect("glyph");
        let scene = scene.finish();

        let mut builder = RenderPlanBuilder::new();
        builder.set_raster_scale(2.0).expect("raster scale");
        let plan = builder.build(&scene, &atlas).expect("plan");
        assert_eq!(plan.raster_scale(), 2.0);

        let commands = build_draw_commands(
            &plan,
            &atlas_page_sizes(&plan),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("commands");
        let glyph = commands
            .iter()
            .find(|command| command.kind == DRAW_GLYPH)
            .expect("glyph command");

        // 24x12 物理像素 ÷ 2 = 12x6 逻辑点。
        assert_eq!(glyph.width, 12.0);
        assert_eq!(glyph.height, 6.0);
        // bearing 同样是物理像素：x = 50 + 2/2，y = 60 - 20/2。
        assert_eq!(glyph.x, 51.0);
        assert_eq!(glyph.y, 50.0);
    }

    /// 从 plan 的上传页推导页尺寸，供 build_draw_commands 校验 UV 范围。
    fn atlas_page_sizes(plan: &RenderPlan) -> std::collections::BTreeMap<u32, (u32, u32)> {
        plan.uploads()
            .iter()
            .map(|upload| (upload.page(), (upload.width(), upload.height())))
            .collect()
    }

    /// 滚动必须触发整帧重绘。
    ///
    /// damage 只描述内容变化：滚动时每个 block 的内容都没变，damage 可能是空
    /// 的，但所有字形的屏幕位置都变了。沿用局部重绘会把旧字形留在 retained
    /// target 上，表现为滚动后字形互相重叠——这是真实窗口里观察到的现象。
    #[test]
    fn viewport_movement_forces_a_full_clear() {
        let viewport = yu_scene::Rect::new(0.0, 0.0, 320.0, 240.0).expect("viewport");
        let scrolled = yu_scene::Rect::new(0.0, 40.0, 320.0, 240.0).expect("scrolled viewport");

        // 稳定状态：同一 viewport、同一 surface generation，允许局部重绘。
        assert!(!requires_full_clear(
            false,
            false,
            Some(viewport),
            viewport,
            Some(7),
            7
        ));

        // 滚动：viewport 位移，必须整帧重绘。
        assert!(requires_full_clear(
            false,
            false,
            Some(viewport),
            scrolled,
            Some(7),
            7
        ));

        // 首帧尚无记录的 viewport，同样必须整帧重绘。
        assert!(requires_full_clear(
            false,
            false,
            None,
            viewport,
            Some(7),
            7
        ));

        // 其余既有条件保持不变。
        assert!(requires_full_clear(
            true,
            false,
            Some(viewport),
            viewport,
            Some(7),
            7
        ));
        assert!(requires_full_clear(
            false,
            true,
            Some(viewport),
            viewport,
            Some(7),
            7
        ));
        assert!(requires_full_clear(
            false,
            false,
            Some(viewport),
            viewport,
            Some(7),
            8
        ));
    }

    #[test]
    fn embedded_draw_command_uses_image_quad_only_for_ready_texture() {
        use std::collections::BTreeMap;

        use yu_assets::{EmbeddedRenderPayload, EmbeddedRenderRequest, EmbeddedResourceKind};
        use yu_core::{ByteOffset, TextRange};
        use yu_scene::{EmbeddedSvgPrimitive, Rect, SceneBuilder};

        let revision = Revision::INITIAL;
        let source = TextRange::new(ByteOffset::ZERO, ByteOffset::new(2)).expect("source");
        let request =
            EmbeddedRenderRequest::new(revision, source, EmbeddedResourceKind::Math, "x^2")
                .expect("embedded request");
        let mut cache = yu_assets::EmbeddedResourceCache::new();
        let publication = cache
            .publish(
                request,
                revision,
                EmbeddedRenderPayload::svg(
                    32,
                    16,
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"32\" height=\"16\"/>",
                )
                .expect("svg"),
            )
            .expect("publication");
        let primitive = EmbeddedSvgPrimitive::new(
            publication.key().fingerprint(),
            publication.generation(),
            publication.kind().tag(),
            publication.source_range(),
            Rect::new(4.0, 6.0, 32.0, 16.0).expect("bounds"),
            32,
            16,
            Rgba8::new(230, 232, 236, 255),
        );
        let mut scene =
            SceneBuilder::new(revision, Rect::new(0.0, 0.0, 80.0, 40.0).expect("viewport"))
                .expect("scene");
        scene.embedded_svg(primitive).expect("primitive");
        let atlas = yu_font::GlyphAtlas::new(
            yu_font::GlyphAtlasConfig::new(8, 8, 1).expect("atlas config"),
        );
        let plan = RenderPlanBuilder::new()
            .build_with_embedded(&scene.finish(), &atlas, std::slice::from_ref(&publication))
            .expect("plan");

        let fallback =
            build_draw_commands(&plan, &BTreeMap::new(), &BTreeMap::new(), &BTreeMap::new())
                .expect("fallback command");
        assert_eq!(fallback[0].kind, DRAW_FILL_RECT);
        assert_eq!(fallback[0].image_kind, IMAGE_KIND_REGULAR);

        let mut embedded_sizes = BTreeMap::new();
        embedded_sizes.insert(
            (primitive.resource(), u32::from(primitive.kind())),
            (32, 16),
        );
        let ready = build_draw_commands(&plan, &BTreeMap::new(), &BTreeMap::new(), &embedded_sizes)
            .expect("ready command");
        assert_eq!(ready[0].kind, DRAW_IMAGE);
        assert_eq!(ready[0].image_kind, embedded_image_kind(primitive.kind()));
        assert_eq!(ready[0].resource, primitive.resource());
        assert_eq!(ready[0].u1, 1.0);
        assert_ne!(
            embedded_image_kind(EmbeddedResourceKind::Math.tag()),
            embedded_image_kind(EmbeddedResourceKind::Mermaid.tag())
        );
    }
}
