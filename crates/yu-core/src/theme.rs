//! Resolved product theme and reading-column geometry, shared by native shells
//! and document layout. Values are logical points; colors are sRGB RGBA bytes.

/// Named native theme. It participates in paragraph and height cache keys.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ThemeId {
    #[default]
    Github,
    Night,
    YuLight,
    YuDark,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskCheckboxStyle {
    pub radius: f32,
    pub border_width: f32,
    pub border: u32,
    pub checked_fill: u32,
    pub fill: u32,
    pub mark: u32,
    pub mark_width: f32,
    pub mark_points: [(f32, f32); 3],
}

impl ThemeId {
    pub const fn task_checkbox_style(self) -> TaskCheckboxStyle {
        match self {
            Self::Github | Self::YuLight | Self::YuDark => TaskCheckboxStyle {
                radius: 3.0,
                border_width: 1.5,
                border: 0x808080ff,
                checked_fill: 0x007affff,
                fill: 0xffffffff,
                mark: 0xffffffff,
                mark_width: 1.4,
                mark_points: [(0.25, 0.52), (0.43, 0.72), (0.77, 0.27)],
            },
            Self::Night => TaskCheckboxStyle {
                radius: 0.0,
                border_width: 1.0,
                border: 0xb8bfc6ff,
                checked_fill: 0xb8bfc6ff,
                fill: 0x363b40ff,
                mark: 0xdededeff,
                mark_width: 0.8,
                mark_points: [(0.32, 0.55), (0.49, 0.81), (0.69, 0.15)],
            },
        }
    }

    pub const fn task_checkbox_size(self) -> f32 {
        match self {
            Self::Github | Self::YuLight | Self::YuDark => 12.0,
            Self::Night => 14.0,
        }
    }

    /// Border width and color below headings, before zoom.
    pub const fn heading_border(self, level: u8) -> (f32, u32) {
        match (self, level) {
            (Self::Github, 1 | 2) => (1.0, 0xeeeeeeff),
            _ => (0.0, 0x00000000),
        }
    }

    pub const fn inline_padding(self) -> (f32, f32) {
        match self {
            Self::Github | Self::YuLight | Self::YuDark => (2.0, 0.0),
            Self::Night => (5.0, 2.0),
        }
    }
    pub const fn inline_border(self) -> f32 {
        match self {
            Self::Github | Self::YuLight | Self::YuDark => 1.0,
            Self::Night => 0.0,
        }
    }

    /// Code roles: keyword, string, number, comment, definition, type, atom, operator.
    pub const fn code_colors(self) -> [u32; 8] {
        match self {
            Self::Github | Self::YuLight => [
                0x770088ff, 0xaa1111ff, 0x116644ff, 0xaa5500ff, 0x0000ffff, 0x008855ff, 0x221199ff,
                0x981a1aff,
            ],
            Self::Night | Self::YuDark => [
                0xc88fd0ff, 0xd26b6bff, 0x64ab8fff, 0xda924aff, 0x8d8df0ff, 0x1cc685ff, 0x84b6cbff,
                0xb8bfc6ff,
            ],
        }
    }

    /// Github overrides heading code to inherit its heading size; Night keeps
    /// the code's relative font-size within headings as well as body text.
    pub const fn inline_code_size_ratio(self, in_heading: bool) -> f32 {
        match (self, in_heading) {
            (Self::Github | Self::YuLight | Self::YuDark, true) => 1.0,
            _ => self.spec().code_size_ratio,
        }
    }

    /// Github's relative inline-code box may slightly enlarge the body strut;
    /// Night uses an inherited absolute line height for each declared font.
    pub const fn body_font_struts(self) -> crate::FontStrutMode {
        match self {
            Self::Github | Self::YuLight | Self::YuDark => crate::FontStrutMode::RelativeFonts,
            Self::Night => crate::FontStrutMode::AllFonts,
        }
    }

    pub const fn code_font_struts(self) -> crate::FontStrutMode {
        match self {
            Self::Github | Self::YuLight | Self::YuDark => crate::FontStrutMode::CodeFont,
            Self::Night => crate::FontStrutMode::RunMetrics,
        }
    }

    pub const fn heading_color(self, level: u8) -> u32 {
        match (self, level) {
            (Self::Github, 6) => 0x777777ff,
            (Self::Github | Self::YuLight | Self::YuDark, _) => self.spec().text,
            (Self::Night, 4 | 6) => 0xffffffff,
            (Self::Night, _) => 0xdededeff,
        }
    }

    pub const fn quote_text_color(self) -> u32 {
        match self {
            Self::Github | Self::YuLight => 0x777777ff,
            Self::Night | Self::YuDark => 0x9da2a6ff,
        }
    }

    pub const fn unordered_marker(self, depth: u8) -> &'static str {
        match (self, depth) {
            (Self::Night, _) => "▪",
            (Self::Github | Self::YuLight | Self::YuDark, 0 | 1) => "•",
            (Self::Github | Self::YuLight | Self::YuDark, 2) => "◦",
            (Self::Github | Self::YuLight | Self::YuDark, _) => "▪",
        }
    }

    pub const fn spec(self) -> ThemeSpec {
        match self {
            Self::Github => ThemeSpec::GITHUB,
            Self::YuLight => ThemeSpec::YU_LIGHT,
            Self::YuDark => ThemeSpec::YU_DARK,
            Self::Night => ThemeSpec::NIGHT,
        }
    }
}

/// Font identities used by the bundled themes; platform adapters resolve these
/// to actual native faces and preserve the resolved face through rasterization.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ThemeFont {
    #[default]
    Inherit,
    OpenSans,
    HelveticaNeue,
    LucidaGrande,
    Menlo,
    Monaco,
    Courier,
    SystemUi,
    SystemMono,
}
impl ThemeFont {
    pub const fn family(self) -> Option<&'static str> {
        match self {
            Self::Inherit | Self::SystemUi | Self::SystemMono => None,
            Self::OpenSans => Some("Open Sans"),
            Self::HelveticaNeue => Some("Helvetica Neue"),
            Self::LucidaGrande => Some("Lucida Grande"),
            Self::Menlo => Some("Menlo"),
            Self::Monaco => Some("Monaco"),
            Self::Courier => Some("Courier"),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeSpec {
    pub body_font: ThemeFont,
    pub heading_font: ThemeFont,
    pub code_font: ThemeFont,
    pub heading_bold: [u32; 6],
    pub heading_letter_spacing: [f32; 6],
    pub heading_margin_before: [f32; 6],
    pub heading_margin_after: [f32; 6],
    pub paragraph_margin_before: f32,
    pub body_size: f32,
    pub body_line_ratio: f32,
    pub heading_sizes: [f32; 6],
    pub heading_lines: [f32; 6],
    pub paragraph_margin: f32,
    pub code_size_ratio: f32,
    /// Block-code wrapper size, independent of inline `code` element size.
    pub code_block_size_ratio: f32,
    pub code_line_ratio: f32,
    pub code_padding_left: f32,
    pub code_padding_right: f32,
    pub code_padding_top: f32,
    pub code_padding_bottom: f32,
    pub code_border_width: f32,
    pub code_border_color: u32,
    /// Code block margins in unscaled logical points.
    pub code_margin_before: f32,
    pub code_margin_after: f32,
    pub table_padding_x: f32,
    pub table_padding_y: f32,
    pub quote_padding: f32,
    pub quote_padding_right: f32,
    pub quote_margin_left: f32,
    pub quote_margin_before: f32,
    pub quote_margin_after: f32,
    pub quote_border_color: u32,
    pub quote_border_width: f32,
    pub list_indent: f32,
    pub gutter: f32,
    pub top: f32,
    pub bottom: f32,
    pub column_width: f32,
    pub sidebar_width: f32,
    pub block_radius: f32,
    pub inline_radius: f32,
    pub background: u32,
    pub text: u32,
    /// Explicit RGBA emphasis color; zero inherits the enclosing semantic color.
    pub strong_text: u32,
    pub sidebar: u32,
    pub code_background: u32,
    pub inline_background: u32,
    pub highlight_background: u32,
    pub border: u32,
    pub table_border: u32,
    pub table_header: u32,
    pub table_stripe: u32,
    pub link: u32,
}

impl ThemeSpec {
    pub const SCRIPT_SIZE_RATIO: f32 = 0.75;
    pub const SUPERSCRIPT_RISE_EM: f32 = 0.4;
    pub const SUBSCRIPT_RISE_EM: f32 = -0.2;

    /// Both reference themes position task inputs 1.3em before the list text.
    pub const TASK_MARKER_ADVANCE_EM: f32 = 1.3;

    pub const TABLE_BORDER_WIDTH: f32 = 1.0;
    /// Native table figure margin in the fixed Typora reference themes.
    pub const TABLE_MARGIN_EM: f32 = 1.2;
    /// Both bundled themes inherit the base li paragraph margin, in rem.
    pub const LIST_PARAGRAPH_MARGIN: f32 = 0.5;

    pub const GITHUB: Self = Self {
        body_font: ThemeFont::OpenSans,
        heading_font: ThemeFont::OpenSans,
        code_font: ThemeFont::Courier,
        heading_bold: [1; 6],
        heading_letter_spacing: [0.0; 6],
        heading_margin_before: [1.0; 6],
        heading_margin_after: [1.0; 6],
        paragraph_margin_before: 0.8,
        body_size: 16.0,
        body_line_ratio: 1.6,
        heading_sizes: [2.25, 1.75, 1.5, 1.25, 1.0, 1.0],
        heading_lines: [1.2, 1.225, 1.43, 1.4, 1.4, 1.4],
        paragraph_margin: 0.8,
        code_size_ratio: 0.9,
        code_block_size_ratio: 0.9,
        code_line_ratio: 1.6,
        // Fence padding (4) + line wrapper (4) + code line padding (4).
        code_padding_left: 12.0,
        code_padding_right: 8.0,
        code_padding_top: 8.0,
        code_padding_bottom: 6.0,
        code_border_width: 1.0,
        code_border_color: 0xe7eaedff,
        code_margin_before: 15.0,
        code_margin_after: 15.0,
        table_padding_x: 13.0,
        table_padding_y: 6.0,
        quote_padding: 15.0,
        quote_padding_right: 15.0,
        quote_margin_left: 0.0,
        quote_margin_before: 12.8,
        quote_margin_after: 12.8,
        quote_border_color: 0xdfe2e5ff,
        quote_border_width: 4.0,
        list_indent: 30.0,
        gutter: 30.0,
        top: 30.0,
        bottom: 100.0,
        column_width: 860.0,
        sidebar_width: 240.0,
        block_radius: 3.0,
        inline_radius: 3.0,
        background: 0xffffffff,
        text: 0x333333ff,
        strong_text: 0,
        sidebar: 0xfafafaff,
        code_background: 0xf8f8f8ff,
        inline_background: 0xf3f4f4ff,
        highlight_background: 0xffe780ff,
        border: 0xe7eaedff,
        table_border: 0xdfe2e5ff,
        table_header: 0xf8f8f8ff,
        table_stripe: 0xf8f8f8ff,
        link: 0x4183c4ff,
    };
    pub const YU_LIGHT: Self = Self {
        body_font: ThemeFont::SystemUi,
        heading_font: ThemeFont::SystemUi,
        code_font: ThemeFont::SystemMono,
        body_line_ratio: 1.625,
        heading_sizes: [2.0, 1.625, 1.375, 1.125, 1.0, 1.0],
        gutter: 24.0,
        column_width: 808.0,
        top: 32.0,
        bottom: 96.0,
        block_radius: 6.0,
        text: 0x242426ff,
        link: 0x0066ccff,
        ..Self::GITHUB
    };
    pub const YU_DARK: Self = Self {
        background: 0x202022ff,
        text: 0xe6e6e8ff,
        code_background: 0x2a2a2dff,
        inline_background: 0x303034ff,
        highlight_background: 0x665522ff,
        code_border_color: 0x424247ff,
        border: 0x424247ff,
        table_border: 0x424247ff,
        table_header: 0x2a2a2dff,
        table_stripe: 0x252528ff,
        quote_border_color: 0x646469ff,
        link: 0x64b5ffff,
        ..Self::YU_LIGHT
    };
    pub const NIGHT: Self = Self {
        strong_text: 0xdededeff,
        quote_padding: 30.0,
        quote_padding_right: 0.0,
        quote_margin_left: 30.0,
        quote_margin_before: 35.0,
        quote_margin_after: 30.0,
        quote_border_width: 2.0,
        quote_border_color: 0x474d54ff,
        // Fence padding plus the code line's own 4pt horizontal padding.
        code_padding_left: 34.0,
        code_padding_right: 14.0,
        // Night inherits the absolute 26pt body line-height at 16pt zoom.
        code_line_ratio: 1.625 / 0.9,
        block_radius: 0.0,
        code_padding_top: 10.0,
        code_padding_bottom: 10.0,
        code_border_width: 0.0,
        code_border_color: 0,
        code_margin_before: 0.0,
        code_margin_after: 20.0,
        top: 36.0,
        // Night keeps the editor's 70pt bottom padding; Github overrides it to 100pt.
        bottom: 70.0,
        table_padding_x: 10.0,
        table_padding_y: 5.0,
        table_border: 0x474d54ff,
        table_header: 0x363b40ff,
        table_stripe: 0x00000000,
        body_font: ThemeFont::HelveticaNeue,
        heading_font: ThemeFont::LucidaGrande,
        code_font: ThemeFont::Monaco,
        heading_bold: [0, 1, 1, 0, 1, 0],
        heading_letter_spacing: [-1.5, -1.0, -1.0, 0.0, 0.0, 0.0],
        heading_margin_before: [5.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        heading_margin_after: [1.5, 1.5, 1.5, 1.5, 1.5, 0.75],
        paragraph_margin_before: 0.0,
        body_line_ratio: 1.625,
        heading_sizes: [2.5, 1.63, 1.17, 1.12, 0.97, 0.93],
        heading_lines: [
            2.75 / 2.5,
            1.875 / 1.63,
            1.5 / 1.17,
            1.375 / 1.12,
            1.25 / 0.97,
            1.0 / 0.93,
        ],
        paragraph_margin: 1.5,
        code_size_ratio: 0.875,
        column_width: 914.0,
        background: 0x363b40ff,
        text: 0xb8bfc6ff,
        sidebar: 0x2e3033ff,
        code_background: 0x333333ff,
        inline_background: 0x0000000d,
        highlight_background: 0x665522ff,
        inline_radius: 0.0,
        border: 0x52575cff,
        link: 0xe0e0e0ff,
        ..Self::GITHUB
    };

    #[must_use]
    pub fn reading_geometry(self, width: f32, window_width: f32, scroll_y: f32) -> ReadingGeometry {
        self.reading_geometry_with_column(width, window_width, scroll_y, None)
    }

    /// A user-selected text width overrides theme breakpoints while retaining
    /// the theme gutters and the same geometry for rendering and native input.
    #[must_use]
    pub fn reading_geometry_with_column(
        self,
        width: f32,
        window_width: f32,
        scroll_y: f32,
        column: Option<f32>,
    ) -> ReadingGeometry {
        let width = width.max(1.0);
        let outer_limit =
            if let Some(column) = column.filter(|value| value.is_finite() && *value > 0.0) {
                column + self.gutter * 2.0
            } else if self.body_font == ThemeFont::SystemUi {
                self.column_width
            } else if window_width >= 1800.0 {
                1200.0
            } else if window_width >= 1400.0 {
                1024.0
            } else {
                self.column_width
            };
        // The theme's max-width includes the two #write paddings (border-box).
        let outer_width = width.min(outer_limit);
        let origin_x = ((width - outer_width) * 0.5 + self.gutter).min((width - 1.0) * 0.5);
        ReadingGeometry {
            origin_x,
            origin_y: self.top,
            content_width: (width - origin_x * 2.0).max(1.0),
            camera_x: -origin_x,
            camera_y: scroll_y.max(0.0) - self.top,
            bottom: self.bottom,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReadingGeometry {
    pub origin_x: f32,
    pub origin_y: f32,
    pub content_width: f32,
    pub camera_x: f32,
    pub camera_y: f32,
    pub bottom: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_reading_column_uses_shared_geometry_and_narrow_window_gutters() {
        for theme in [ThemeSpec::YU_LIGHT, ThemeSpec::GITHUB, ThemeSpec::NIGHT] {
            let wide = theme.reading_geometry_with_column(1600.0, 1800.0, 125.0, Some(600.0));
            assert_eq!(wide.content_width, 600.0);
            assert_eq!(wide.origin_x, 500.0);
            assert_eq!(wide.camera_x, -wide.origin_x);
            assert_eq!(wide.camera_y, 125.0 - theme.top);
            let narrow = theme.reading_geometry_with_column(400.0, 1800.0, 0.0, Some(600.0));
            assert_eq!(narrow.content_width, 400.0 - theme.gutter * 2.0);
            assert_eq!(narrow.origin_x, theme.gutter);
            assert_eq!(
                theme.reading_geometry_with_column(1600.0, 1800.0, 0.0, None),
                theme.reading_geometry(1600.0, 1800.0, 0.0)
            );
        }
    }

    #[test]
    fn yu_theme_keeps_one_reading_geometry_across_appearance_and_wide_windows() {
        for width in [100.0, 620.0, 900.0, 1600.0, 2400.0] {
            let light = ThemeSpec::YU_LIGHT.reading_geometry(width, width, 125.0);
            let dark = ThemeSpec::YU_DARK.reading_geometry(width, width, 125.0);
            assert_eq!(light, dark);
            assert!(light.content_width <= 760.0);
            assert!(light.origin_x >= 24.0);
            assert_eq!(light.content_width + 2.0 * light.origin_x, width);
        }
        assert_eq!(
            ThemeSpec::YU_LIGHT.body_size * ThemeSpec::YU_LIGHT.body_line_ratio,
            26.0
        );
        assert_ne!(
            ThemeSpec::YU_LIGHT.background,
            ThemeSpec::YU_DARK.background
        );
    }

    #[test]
    fn sidebar_does_not_change_window_breakpoint() {
        for theme in [ThemeSpec::GITHUB, ThemeSpec::NIGHT] {
            let wide = theme.reading_geometry(1330.0, 1600.0, 125.0);
            assert_eq!(wide.content_width, 964.0);
            assert_eq!(wide.origin_x, 183.0);
            assert_eq!(wide.camera_x, -183.0);
            let full = theme.reading_geometry(1600.0, 1600.0, 125.0);
            assert_eq!(full.content_width, wide.content_width);
            let narrow = theme.reading_geometry(930.0, 1200.0, 0.0);
            assert_eq!(narrow.content_width, theme.column_width - 60.0);
            let huge = theme.reading_geometry(1530.0, 1800.0, 0.0);
            assert_eq!(huge.content_width, 1140.0);
            let constrained = theme.reading_geometry(500.0, 1600.0, 0.0);
            assert_eq!(constrained.content_width, 440.0);
        }
    }

    #[test]
    fn column_padding_and_camera_use_the_same_origin() {
        for width in [100.0, 660.0, 900.0, 1200.0, 1600.0, 1900.0] {
            let g = ThemeSpec::GITHUB.reading_geometry(width, width, 125.0);
            assert_eq!(g.content_width + 2.0 * g.origin_x, width);
            let point = (46.0_f32, 240.0_f32);
            let viewport = (point.0 - g.camera_x, point.1 - g.camera_y);
            assert_eq!(
                (viewport.0 - g.origin_x, viewport.1 + 125.0 - g.origin_y),
                point
            );
        }
    }
}
