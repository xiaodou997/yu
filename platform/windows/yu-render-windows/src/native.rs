#![cfg(target_os = "windows")]

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::mem::size_of;
use std::slice;

use windows::Win32::Foundation::{BOOL, HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0,
    D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::*;
use windows::core::{Interface, PCSTR};
use yu_assets::DecodedImage;
use yu_core::Revision;
use yu_font::GlyphAtlas;
use yu_render::{
    DRAW_GLYPH, DRAW_IMAGE, DrawCommand, FrameConsumer, IMAGE_KIND_REGULAR, RenderPlan,
    SurfaceConfig, build_draw_commands,
};
use yu_scene::Rgba8;
use yu_workspace::ViewportRenderFrame;

use crate::{D3DRenderError, command::GpuCommand};

const SHADER: &str = r#"
cbuffer CommandBuffer : register(b0)
{
    float4 cmdRect;
    float4 cmdUv;
    float4 cmdColor;
    float4 cmdExtra0; // radius, rect_offset_x, rect_offset_y, rect_width
    float4 cmdExtra1; // rect_height, shadow_offset_x, shadow_offset_y, shadow_blur
    uint4 cmdMeta;    // kind, image_kind, shadow_color, unused
    float4 viewport;  // logical width, logical height, scale, unused
};

Texture2D tex0 : register(t0);
SamplerState samp0 : register(s0);

struct VSOut {
    float4 position : SV_Position;
    float2 uv : TEXCOORD0;
    float2 local : TEXCOORD1;
};

VSOut vs_main(uint id : SV_VertexID)
{
    static const float2 corners[6] = {
        float2(0,0), float2(1,0), float2(0,1),
        float2(1,0), float2(1,1), float2(0,1)
    };
    float2 corner = corners[id];
    float2 p = cmdRect.xy + corner * cmdRect.zw;
    VSOut output;
    output.position = float4(
        p.x / viewport.x * 2.0 - 1.0,
        1.0 - p.y / viewport.y * 2.0,
        0.0,
        1.0
    );
    output.uv = lerp(cmdUv.xy, cmdUv.zw, corner);
    output.local = corner * cmdRect.zw;
    return output;
}

float sdRoundRect(float2 p, float2 size, float radius)
{
    radius = min(radius, min(size.x, size.y) * 0.5);
    float2 q = abs(p - size * 0.5) - (size * 0.5 - radius);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}

float segmentDistance(float2 p, float2 a, float2 b)
{
    float2 ab = b - a;
    float denom = max(dot(ab, ab), 1e-6);
    float t = saturate(dot(p - a, ab) / denom);
    return length(p - (a + t * ab));
}

float4 unpackColor(uint packed)
{
    float4 bytes = float4(
        (packed >> 24) & 255,
        (packed >> 16) & 255,
        (packed >> 8) & 255,
        packed & 255
    ) / 255.0;
    return float4(bytes.rgb * bytes.a, bytes.a);
}

float4 premultiply(float4 color)
{
    return float4(color.rgb * color.a, color.a);
}

float4 ps_main(VSOut input) : SV_Target
{
    uint kind = cmdMeta.x;

    if (kind == 0) {
        return premultiply(cmdColor);
    }

    if (kind == 1) {
        float4 sample = tex0.Sample(samp0, input.uv);
        // Atlas pixels are premultiplied. Monochrome coverage is stored as
        // [coverage; 4], so the same multiplication also tints ordinary text.
        return float4(
            sample.rgb * cmdColor.rgb * cmdColor.a,
            sample.a * cmdColor.a
        );
    }

    if (kind == 2) {
        float4 sample = tex0.Sample(samp0, input.uv);
        float mask = 1.0;
        if (cmdExtra0.x > 0.0) {
            float d = sdRoundRect(input.local, cmdRect.zw, cmdExtra0.x);
            mask = saturate(0.5 - d);
        }
        sample.a *= mask * cmdColor.a;
        return float4(sample.rgb * sample.a, sample.a);
    }

    if (kind == 3) {
        float radius = cmdExtra0.x;
        float2 offset = cmdExtra0.yz;
        float2 size = float2(cmdExtra0.w, cmdExtra1.x);
        float2 p = input.local - offset;
        float d = sdRoundRect(p, size, radius);
        float fill = saturate(0.5 - d);
        float4 fillColor = premultiply(cmdColor);
        fillColor *= fill;

        float4 shadow = unpackColor(cmdMeta.z);
        float blur = cmdExtra1.w;
        if (shadow.a > 0.0 && blur > 0.0) {
            float2 shadowP = p - cmdExtra1.yz;
            float shadowD = max(sdRoundRect(shadowP, size, radius), 0.0);
            float shadowMask = exp(-0.5 * (shadowD * shadowD) / max(blur * blur, 1e-4));
            shadow *= shadowMask * (1.0 - fill);
        } else {
            shadow = 0.0;
        }
        return fillColor + shadow * (1.0 - fillColor.a);
    }

    if (kind == 4) {
        float2 a = cmdExtra0.yz;
        float2 b = float2(cmdExtra0.w, cmdExtra1.x);
        float2 c = cmdExtra1.yz;
        float halfWidth = cmdExtra0.x * 0.5;
        float d = min(segmentDistance(input.local, a, b), segmentDistance(input.local, b, c));
        float alpha = saturate(halfWidth + 0.5 - d);
        return premultiply(cmdColor) * alpha;
    }

    return float4(0, 0, 0, 0);
}
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct D3DTextureIdentity {
    pub width: u32,
    pub height: u32,
    pub generation: u64,
}

struct NativeTexture {
    srv: ID3D11ShaderResourceView,
    width: u32,
    height: u32,
    identity: D3DTextureIdentity,
}

pub struct D3DRenderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    swap_chain: Option<IDXGISwapChain1>,
    target: Option<ID3D11RenderTargetView>,
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    command_buffer: ID3D11Buffer,
    sampler: ID3D11SamplerState,
    blend: ID3D11BlendState,
    surface: SurfaceConfig,
    generation: u64,
    atlas: BTreeMap<u32, NativeTexture>,
    images: BTreeMap<(u64, u32), NativeTexture>,
    frame_consumer: FrameConsumer,
    capture_requested: bool,
    captured_frame: Option<DecodedImage>,
}

impl std::fmt::Debug for D3DRenderer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("D3DRenderer")
            .field("surface", &self.surface)
            .field("generation", &self.generation)
            .field("atlas_pages", &self.atlas.len())
            .field("images", &self.images.len())
            .finish_non_exhaustive()
    }
}

fn compile_shader(entry: &'static [u8], target: &'static [u8]) -> Result<Vec<u8>, D3DRenderError> {
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    let result = unsafe {
        D3DCompile(
            SHADER.as_ptr().cast(),
            SHADER.len(),
            PCSTR(c"yu-render-windows".as_ptr().cast()),
            None,
            None::<&windows::Win32::Graphics::Direct3D::ID3DInclude>,
            PCSTR(entry.as_ptr()),
            PCSTR(target.as_ptr()),
            0,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    if let Err(error) = result {
        if let Some(blob) = errors {
            let message = unsafe {
                let bytes = slice::from_raw_parts(
                    blob.GetBufferPointer().cast::<u8>(),
                    blob.GetBufferSize(),
                );
                String::from_utf8_lossy(bytes).into_owned()
            };
            return Err(D3DRenderError::Native(message));
        }
        return Err(error.into());
    }
    let blob = code.ok_or(D3DRenderError::InvalidResource(
        "D3D compiler returned no shader bytecode",
    ))?;
    let bytes = unsafe {
        slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize()).to_vec()
    };
    Ok(bytes)
}

fn create_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    pixels: &[u8],
    generation: u64,
) -> Result<NativeTexture, D3DRenderError> {
    if width == 0 || height == 0 {
        return Err(D3DRenderError::InvalidResource(
            "D3D texture dimensions must be positive",
        ));
    }
    let expected = usize::try_from(width)
        .ok()
        .and_then(|w| usize::try_from(height).ok()?.checked_mul(w)?.checked_mul(4))
        .ok_or(D3DRenderError::InvalidResource(
            "D3D texture dimensions overflow usize",
        ))?;
    if pixels.len() != expected {
        return Err(D3DRenderError::InvalidResource(
            "D3D texture pixel length does not match dimensions",
        ));
    }

    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: pixels.as_ptr().cast::<c_void>(),
        SysMemPitch: width
            .checked_mul(4)
            .ok_or(D3DRenderError::InvalidResource("D3D row pitch overflow"))?,
        SysMemSlicePitch: 0,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&desc, Some(&data), Some(&mut texture))? };
    let texture = texture.ok_or(D3DRenderError::InvalidResource(
        "D3D did not return a texture",
    ))?;
    let mut srv = None;
    unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut srv))? };
    let srv = srv.ok_or(D3DRenderError::InvalidResource(
        "D3D did not return a shader resource view",
    ))?;
    Ok(NativeTexture {
        srv,
        width,
        height,
        identity: D3DTextureIdentity {
            width,
            height,
            generation,
        },
    })
}

impl D3DRenderer {
    pub fn new(hwnd: HWND, surface: SurfaceConfig) -> Result<Self, D3DRenderError> {
        let feature_levels = [D3D_FEATURE_LEVEL_11_0];
        let mut device = None;
        let mut context = None;
        let mut level = D3D_FEATURE_LEVEL::default();
        unsafe {
            D3D11CreateDevice(
                None::<&IDXGIAdapter>,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&feature_levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                Some(&mut level),
                Some(&mut context),
            )?;
        }
        if level.0 < D3D_FEATURE_LEVEL_11_0.0 {
            return Err(D3DRenderError::InvalidResource(
                "Yu requires Direct3D feature level 11_0",
            ));
        }
        let device = device.ok_or(D3DRenderError::InvalidResource(
            "D3D11 did not return a device",
        ))?;
        let context = context.ok_or(D3DRenderError::InvalidResource(
            "D3D11 did not return an immediate context",
        ))?;

        let dxgi_device: IDXGIDevice = device.cast()?;
        let adapter = unsafe { dxgi_device.GetAdapter()? };
        let factory: IDXGIFactory2 = unsafe { adapter.GetParent()? };
        let swap_desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: surface.pixel_width(),
            Height: surface.pixel_height(),
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            Stereo: BOOL(0),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: 0,
        };
        let swap_chain = unsafe {
            factory.CreateSwapChainForHwnd(&device, hwnd, &swap_desc, None, None::<&IDXGIOutput>)?
        };
        let _ = unsafe { factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER) };

        let vs_bytes = compile_shader(b"vs_main\0", b"vs_5_0\0")?;
        let ps_bytes = compile_shader(b"ps_main\0", b"ps_5_0\0")?;
        let mut vertex_shader = None;
        let mut pixel_shader = None;
        unsafe {
            device.CreateVertexShader(
                &vs_bytes,
                None::<&ID3D11ClassLinkage>,
                Some(&mut vertex_shader),
            )?;
            device.CreatePixelShader(
                &ps_bytes,
                None::<&ID3D11ClassLinkage>,
                Some(&mut pixel_shader),
            )?;
        }
        let vertex_shader = vertex_shader.ok_or(D3DRenderError::InvalidResource(
            "D3D did not return a vertex shader",
        ))?;
        let pixel_shader = pixel_shader.ok_or(D3DRenderError::InvalidResource(
            "D3D did not return a pixel shader",
        ))?;

        let buffer_desc = D3D11_BUFFER_DESC {
            ByteWidth: u32::try_from(size_of::<GpuCommand>())
                .map_err(|_| D3DRenderError::InvalidResource("D3D command buffer is too large"))?,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let mut command_buffer = None;
        unsafe { device.CreateBuffer(&buffer_desc, None, Some(&mut command_buffer))? };
        let command_buffer = command_buffer.ok_or(D3DRenderError::InvalidResource(
            "D3D did not return a command buffer",
        ))?;

        let sampler_desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
            MaxAnisotropy: 1,
            MaxLOD: f32::MAX,
            ..Default::default()
        };
        let mut sampler = None;
        unsafe { device.CreateSamplerState(&sampler_desc, Some(&mut sampler))? };
        let sampler = sampler.ok_or(D3DRenderError::InvalidResource(
            "D3D did not return a sampler",
        ))?;

        let mut blend_desc = D3D11_BLEND_DESC::default();
        blend_desc.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: BOOL(1),
            SrcBlend: D3D11_BLEND_ONE,
            DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ONE,
            DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
        };
        let mut blend = None;
        unsafe { device.CreateBlendState(&blend_desc, Some(&mut blend))? };
        let blend = blend.ok_or(D3DRenderError::InvalidResource(
            "D3D did not return a blend state",
        ))?;

        let mut renderer = Self {
            device,
            context,
            swap_chain: Some(swap_chain),
            target: None,
            vertex_shader,
            pixel_shader,
            command_buffer,
            sampler,
            blend,
            surface,
            generation: 1,
            atlas: BTreeMap::new(),
            images: BTreeMap::new(),
            frame_consumer: FrameConsumer::new(),
            capture_requested: false,
            captured_frame: None,
        };
        renderer.recreate_target()?;
        Ok(renderer)
    }

    #[must_use]
    pub const fn surface(&self) -> SurfaceConfig {
        self.surface
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Revisions are monotonic within one document, not across newly opened
    /// sessions. A replacement document starts a new revision ownership scope.
    pub fn reset_document(&mut self) {
        self.frame_consumer = FrameConsumer::new();
    }

    fn recreate_target(&mut self) -> Result<(), D3DRenderError> {
        let swap_chain = self
            .swap_chain
            .as_ref()
            .ok_or(D3DRenderError::InvalidResource("no swapchain"))?;
        let back_buffer: ID3D11Texture2D = unsafe { swap_chain.GetBuffer(0)? };
        let mut target = None;
        unsafe {
            self.device
                .CreateRenderTargetView(&back_buffer, None, Some(&mut target))?;
        }
        self.target = Some(target.ok_or(D3DRenderError::InvalidResource(
            "D3D did not return a render target view",
        ))?);
        Ok(())
    }

    pub fn resize(&mut self, surface: SurfaceConfig) -> Result<bool, D3DRenderError> {
        if self.surface.pixel_width() == surface.pixel_width()
            && self.surface.pixel_height() == surface.pixel_height()
            && self.surface.scale() == surface.scale()
        {
            self.surface = surface;
            return Ok(false);
        }
        unsafe {
            self.context
                .OMSetRenderTargets(None, None::<&ID3D11DepthStencilView>);
        }
        self.target = None;
        unsafe {
            self.swap_chain
                .as_ref()
                .ok_or(D3DRenderError::InvalidResource("no swapchain"))?
                .ResizeBuffers(
                    2,
                    surface.pixel_width(),
                    surface.pixel_height(),
                    DXGI_FORMAT_R8G8B8A8_UNORM,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )?;
        }
        self.surface = surface;
        self.generation = self.generation.wrapping_add(1).max(1);
        self.recreate_target()?;
        Ok(true)
    }

    /// A HWND can own only one live flip swapchain. Release its views and
    /// deferred context references before constructing the replacement.
    pub fn recreate(&mut self, hwnd: HWND) -> Result<(), D3DRenderError> {
        unsafe {
            self.context.ClearState();
        }
        self.target = None;
        self.swap_chain = None;
        self.atlas.clear();
        self.images.clear();
        unsafe {
            self.context.Flush();
        }
        let generation = self.generation.wrapping_add(1).max(1);
        let mut replacement = Self::new(hwnd, self.surface)?;
        replacement.generation = generation;
        *self = replacement;
        Ok(())
    }

    pub fn sync_glyph_atlas(&mut self, atlas: &GlyphAtlas) -> Result<usize, D3DRenderError> {
        let config = atlas.config();
        let mut uploaded = 0;
        for page_index in 0..atlas.page_count() {
            let page = u32::try_from(page_index)
                .map_err(|_| D3DRenderError::InvalidResource("atlas page index overflow"))?;
            let fingerprint = atlas
                .page_fingerprint(page)
                .map_err(|_| D3DRenderError::MissingAtlasPage(page))?;
            let identity = D3DTextureIdentity {
                width: config.page_width(),
                height: config.page_height(),
                generation: fingerprint,
            };
            if self
                .atlas
                .get(&page)
                .is_some_and(|texture| texture.identity == identity)
            {
                continue;
            }
            let texture = create_texture(
                &self.device,
                config.page_width(),
                config.page_height(),
                atlas
                    .page_pixels(page)
                    .map_err(|_| D3DRenderError::MissingAtlasPage(page))?,
                fingerprint,
            )?;
            self.atlas.insert(page, texture);
            uploaded += 1;
        }
        let count = u32::try_from(atlas.page_count()).unwrap_or(u32::MAX);
        self.atlas.retain(|page, _| *page < count);
        Ok(uploaded)
    }

    pub fn upload_image(
        &mut self,
        resource: u64,
        image_kind: u32,
        image: &DecodedImage,
        generation: u64,
    ) -> Result<bool, D3DRenderError> {
        let identity = D3DTextureIdentity {
            width: image.width(),
            height: image.height(),
            generation,
        };
        let key = (resource, image_kind);
        if self
            .images
            .get(&key)
            .is_some_and(|texture| texture.identity == identity)
        {
            return Ok(false);
        }
        let texture = create_texture(
            &self.device,
            image.width(),
            image.height(),
            image.pixels(),
            generation,
        )?;
        self.images.insert(key, texture);
        Ok(true)
    }

    pub fn retain_images(&mut self, keys: &[(u64, u32)]) -> usize {
        let before = self.images.len();
        self.images.retain(|key, _| keys.contains(key));
        before.saturating_sub(self.images.len())
    }

    /// Opt into readback before the next Present. Flip-discard contents are
    /// undefined after Present, so reading the buffer afterwards is invalid.
    pub fn request_frame_capture(&mut self) {
        self.capture_requested = true;
    }

    pub fn take_captured_frame(&mut self) -> Option<DecodedImage> {
        self.captured_frame.take()
    }

    fn capture_frame(&self) -> Result<DecodedImage, D3DRenderError> {
        let target = self
            .target
            .as_ref()
            .ok_or(D3DRenderError::InvalidResource("no render target"))?;
        let texture: ID3D11Texture2D = unsafe { target.GetResource()? }.cast()?;
        let mut description = D3D11_TEXTURE2D_DESC::default();
        unsafe {
            texture.GetDesc(&mut description);
        }
        let byte_count = (description.Width as usize)
            .checked_mul(description.Height as usize)
            .and_then(|count| count.checked_mul(4))
            .filter(|count| *count <= 128 * 1024 * 1024)
            .ok_or(D3DRenderError::InvalidResource(
                "readback dimensions exceed budget",
            ))?;
        description.Usage = D3D11_USAGE_STAGING;
        description.BindFlags = 0;
        description.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        description.MiscFlags = 0;
        let mut staging = None;
        unsafe {
            self.device
                .CreateTexture2D(&description, None, Some(&mut staging))?;
        }
        let staging = staging.ok_or(D3DRenderError::InvalidResource("no readback texture"))?;
        unsafe {
            self.context.CopyResource(&staging, &texture);
        }
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        }
        let mut pixels = vec![0; byte_count];
        let stride = description.Width as usize * 4;
        for y in 0..description.Height as usize {
            let row = unsafe {
                slice::from_raw_parts(
                    mapped.pData.cast::<u8>().add(y * mapped.RowPitch as usize),
                    stride,
                )
            };
            pixels[y * stride..(y + 1) * stride].copy_from_slice(row);
        }
        unsafe {
            self.context.Unmap(&staging, 0);
        }
        DecodedImage::new(description.Width, description.Height, pixels)
            .map_err(|_| D3DRenderError::InvalidResource("invalid readback pixels"))
    }

    fn page_sizes(&self) -> BTreeMap<u32, (u32, u32)> {
        self.atlas
            .iter()
            .map(|(page, texture)| (*page, (texture.width, texture.height)))
            .collect()
    }

    fn image_sizes(&self) -> BTreeMap<u64, (u32, u32)> {
        self.images
            .iter()
            .filter_map(|(&(resource, kind), texture)| {
                (kind == IMAGE_KIND_REGULAR).then_some((resource, (texture.width, texture.height)))
            })
            .collect()
    }

    fn embedded_sizes(&self) -> BTreeMap<(u64, u32), (u32, u32)> {
        self.images
            .iter()
            .filter(|(key, _)| key.1 != IMAGE_KIND_REGULAR)
            .map(|(&(resource, kind), texture)| {
                ((resource, kind - 1), (texture.width, texture.height))
            })
            .collect()
    }

    fn texture_for(
        &self,
        command: DrawCommand,
    ) -> Result<Option<&ID3D11ShaderResourceView>, D3DRenderError> {
        match command.kind {
            DRAW_GLYPH => self
                .atlas
                .get(&command.page)
                .map(|texture| Some(&texture.srv))
                .ok_or(D3DRenderError::MissingAtlasPage(command.page)),
            DRAW_IMAGE => self
                .images
                .get(&(command.resource, command.image_kind))
                .map(|texture| Some(&texture.srv))
                .ok_or(D3DRenderError::MissingTexture {
                    resource: command.resource,
                    image_kind: command.image_kind,
                }),
            _ => Ok(None),
        }
    }

    fn submit_commands(
        &mut self,
        commands: &[DrawCommand],
        logical_width: f32,
        logical_height: f32,
        clear: Rgba8,
    ) -> Result<(), D3DRenderError> {
        let target = self.target.as_ref().ok_or(D3DRenderError::InvalidResource(
            "D3D render target is unavailable",
        ))?;
        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: self.surface.pixel_width() as f32,
            Height: self.surface.pixel_height() as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let clear = [
            f32::from(clear.red()) / 255.0,
            f32::from(clear.green()) / 255.0,
            f32::from(clear.blue()) / 255.0,
            f32::from(clear.alpha()) / 255.0,
        ];
        unsafe {
            self.context.OMSetRenderTargets(
                Some(&[Some(target.clone())]),
                None::<&ID3D11DepthStencilView>,
            );
            self.context.ClearRenderTargetView(target, &clear);
            self.context.RSSetViewports(Some(&[viewport]));
            self.context
                .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.context.VSSetShader(&self.vertex_shader, None);
            self.context.PSSetShader(&self.pixel_shader, None);
            self.context
                .VSSetConstantBuffers(0, Some(&[Some(self.command_buffer.clone())]));
            self.context
                .PSSetConstantBuffers(0, Some(&[Some(self.command_buffer.clone())]));
            self.context
                .PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            self.context.OMSetBlendState(&self.blend, None, u32::MAX);
        }

        let scale = self.surface.scale() as f32;
        for &command in commands {
            let gpu = GpuCommand::from_draw(command, logical_width, logical_height, scale);
            let texture = self.texture_for(command)?.cloned();
            unsafe {
                self.context.UpdateSubresource(
                    &self.command_buffer,
                    0,
                    None,
                    (&gpu as *const GpuCommand).cast(),
                    0,
                    0,
                );
                self.context.PSSetShaderResources(0, Some(&[texture]));
                self.context.Draw(6, 0);
            }
        }
        unsafe {
            self.context.PSSetShaderResources(0, Some(&[None]));
        }
        if self.capture_requested {
            self.captured_frame = Some(self.capture_frame()?);
            self.capture_requested = false;
        }
        let swap_chain = self
            .swap_chain
            .as_ref()
            .ok_or(D3DRenderError::InvalidResource("no swapchain"))?;
        let present = unsafe { swap_chain.Present(1, DXGI_PRESENT(0)) };
        if present == DXGI_ERROR_DEVICE_REMOVED || present == DXGI_ERROR_DEVICE_RESET {
            return Err(D3DRenderError::DeviceLost);
        }
        present.ok()?;
        Ok(())
    }

    pub fn render_plan(
        &mut self,
        plan: &RenderPlan,
        atlas: &GlyphAtlas,
        clear: Rgba8,
    ) -> Result<(), D3DRenderError> {
        self.sync_glyph_atlas(atlas)?;
        let commands = build_draw_commands(
            plan,
            &self.page_sizes(),
            &self.image_sizes(),
            &self.embedded_sizes(),
        )?;
        self.submit_commands(
            &commands,
            plan.viewport().width(),
            plan.viewport().height(),
            clear,
        )
    }

    pub fn render_viewport_frame(
        &mut self,
        current_revision: Revision,
        frame: &ViewportRenderFrame,
        atlas: &GlyphAtlas,
    ) -> Result<(), D3DRenderError> {
        self.frame_consumer
            .validate_revision(current_revision, frame.revision())?;
        self.render_plan(frame.plan(), atlas, frame.scene().background())?;
        self.frame_consumer
            .commit_revision(current_revision, frame.revision())?;
        Ok(())
    }
}
