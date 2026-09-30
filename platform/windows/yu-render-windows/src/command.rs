use yu_render::DrawCommand;

#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct GpuCommand {
    pub(crate) rect: [f32; 4],
    pub(crate) uv: [f32; 4],
    pub(crate) color: [f32; 4],
    pub(crate) extra0: [f32; 4],
    pub(crate) extra1: [f32; 4],
    pub(crate) meta: [u32; 4],
    pub(crate) viewport: [f32; 4],
}

impl GpuCommand {
    #[must_use]
    pub(crate) fn from_draw(command: DrawCommand, width: f32, height: f32, scale: f32) -> Self {
        Self {
            rect: [command.x, command.y, command.width, command.height],
            uv: [command.u0, command.v0, command.u1, command.v1],
            color: [command.red, command.green, command.blue, command.alpha],
            extra0: [
                command.radius,
                command.rect_offset_x,
                command.rect_offset_y,
                command.rect_width,
            ],
            extra1: [
                command.rect_height,
                command.shadow_offset_x,
                command.shadow_offset_y,
                command.shadow_blur,
            ],
            meta: [command.kind, command.image_kind, command.shadow_color, 0],
            viewport: [width, height, scale, 0.0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d3d_constant_buffer_preserves_the_shared_draw_command_exactly() {
        let draw = DrawCommand {
            kind: 4,
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
            u0: 0.1,
            v0: 0.2,
            u1: 0.8,
            v1: 0.9,
            red: 0.25,
            green: 0.5,
            blue: 0.75,
            alpha: 0.6,
            page: 17,
            resource: 23,
            image_kind: 5,
            radius: 6.0,
            rect_offset_x: 7.0,
            rect_offset_y: 8.0,
            rect_width: 9.0,
            rect_height: 10.0,
            shadow_offset_x: 11.0,
            shadow_offset_y: 12.0,
            shadow_blur: 13.0,
            shadow_color: 0x1122_3344,
        };

        let gpu = GpuCommand::from_draw(draw, 640.0, 480.0, 1.5);

        assert_eq!(gpu.rect, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(gpu.uv, [0.1, 0.2, 0.8, 0.9]);
        assert_eq!(gpu.color, [0.25, 0.5, 0.75, 0.6]);
        assert_eq!(gpu.extra0, [6.0, 7.0, 8.0, 9.0]);
        assert_eq!(gpu.extra1, [10.0, 11.0, 12.0, 13.0]);
        assert_eq!(gpu.meta, [4, 5, 0x1122_3344, 0]);
        assert_eq!(gpu.viewport, [640.0, 480.0, 1.5, 0.0]);
    }

    #[test]
    fn gpu_command_remains_constant_buffer_aligned() {
        assert_eq!(std::mem::align_of::<GpuCommand>(), 16);
        assert_eq!(std::mem::size_of::<GpuCommand>() % 16, 0);
    }
}
