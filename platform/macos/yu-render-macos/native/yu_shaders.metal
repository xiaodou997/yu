#include <metal_stdlib>

using namespace metal;

struct YuVertexIn {
    float2 position [[attribute(0)]];
    float2 uv [[attribute(1)]];
};

struct YuFrameUniforms {
    float2 viewport;
    float scale;
};

struct YuVertexOut {
    float4 position [[position]];
    float2 uv;
};

struct YuPrimitiveUniforms {
    float4 color;
};

// 圆角矩形（DRAW_ROUNDED_FILL_RECT）的 fragment 参数。一个外扩后的 quad
// 同时承载填充与阴影：quad_size 是 quad 自身尺寸，rect_offset / rect_size
// 把几何矩形在 quad 内的位置与大小告诉 shader，uv × quad_size 即可还原
// 几何坐标系。坐标一律是 document-space 逻辑像素，与顶点一致。
struct YuRoundedUniforms {
    float2 quad_size;
    float2 rect_offset;
    float2 rect_size;
    float radius;
    float shadow_blur;
    float2 shadow_offset;
    float4 shadow_color;
    float4 fill_color;
};

// 图片（DRAW_IMAGE）的 fragment 参数：颜色乘子之外带上 quad 尺寸与裁剪
// 圆角。radius <= 0 时走无 SDF 的旧路径，输出与 M2 之前逐位一致。
struct YuImageUniforms {
    float4 color;
    float2 size;
    float radius;
    float padding;
};

// 标准圆角矩形 SDF：p 相对矩形中心，half_size 是半宽高，radius 角半径。
// 返回带符号距离（负值在形状内）。
static float yu_sd_rounded_rect(float2 p, float2 half_size, float radius) {
    float2 q = abs(p) - half_size + float2(radius);
    return length(max(q, float2(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

vertex YuVertexOut yu_vertex(
    YuVertexIn input [[stage_in]],
    constant YuFrameUniforms& frame [[buffer(1)]]
) {
    YuVertexOut output;
    // position 与 viewport 同为 document-space 逻辑坐标，而 NDC 是归一化的、
    // 与物理像素无关，因此这里**不能**再乘 backing scale：那会让内容整体放大
    // scale 倍，并使按逻辑宽度算好的换行位置溢出可视区域。
    // scale 只用于 damage scissor（那里确实需要物理像素）。
    float2 ndc = float2(
        (input.position.x / frame.viewport.x) * 2.0 - 1.0,
        1.0 - (input.position.y / frame.viewport.y) * 2.0
    );
    output.position = float4(ndc, 0.0, 1.0);
    output.uv = input.uv;
    return output;
}

fragment float4 yu_solid_fragment(
    constant YuPrimitiveUniforms& primitive [[buffer(0)]]
) {
    return primitive.color;
}

fragment float4 yu_glyph_fragment(
    YuVertexOut input [[stage_in]],
    texture2d<float, access::sample> atlas [[texture(0)]],
    sampler atlas_sampler [[sampler(0)]],
    constant YuPrimitiveUniforms& primitive [[buffer(0)]]
) {
    float coverage = atlas.sample(atlas_sampler, input.uv).r;
    return float4(primitive.color.rgb, primitive.color.a * coverage);
}

fragment float4 yu_image_fragment(
    YuVertexOut input [[stage_in]],
    texture2d<float, access::sample> image [[texture(0)]],
    sampler image_sampler [[sampler(0)]],
    constant YuImageUniforms& primitive [[buffer(0)]]
) {
    float4 sampled = image.sample(image_sampler, input.uv);
    float alpha = sampled.a * primitive.color.a;
    // 圆角裁剪：uv 铺满整个 quad，乘回逻辑像素尺寸后算圆角矩形 SDF。
    // radius <= 0 跳过本分支，采样输出与直角图片（M2 之前的行为）逐位一致。
    float radius = min(primitive.radius, 0.5 * min(primitive.size.x, primitive.size.y));
    if (radius > 0.0) {
        float2 center = primitive.size * 0.5;
        float sd = yu_sd_rounded_rect(input.uv * primitive.size - center, center, radius);
        float coverage = clamp(0.5 - sd / max(fwidth(sd), 1e-4), 0.0, 1.0);
        alpha *= coverage;
    }
    return float4(sampled.rgb * primitive.color.rgb, alpha);
}

fragment float4 yu_rounded_fragment(
    YuVertexOut input [[stage_in]],
    constant YuRoundedUniforms& primitive [[buffer(0)]]
) {
    // 还原几何坐标系中的片元位置：uv 铺满外扩后的 quad。
    float2 p = input.uv * primitive.quad_size;
    float2 half_size = primitive.rect_size * 0.5;
    float2 center = primitive.rect_offset + half_size;
    // 半径在 shader 侧也夹一次：调用方的值已经过校验，但 min(w,h)/2 的
    // 上限是不变量，不依赖调用方记得夹。
    float radius = max(min(primitive.radius, min(half_size.x, half_size.y)), 0.0);
    float sd = yu_sd_rounded_rect(p - center, half_size, radius);
    // fwidth 抗锯齿：边缘一个像素内从 0 过渡到 1。
    float fill_coverage = clamp(0.5 - sd / max(fwidth(sd), 1e-4), 0.0, 1.0);
    float4 fill = float4(primitive.fill_color.rgb, primitive.fill_color.a * fill_coverage);

    // 阴影：同一 SDF 平移 shadow_offset 后取形状外的距离，按 σ = shadow_blur
    // 的钟形衰减 exp(-(d/σ)²)。quad 外扩量（scene damage 与顶点共用
    // 3σ + offset 的公式）保证 quad 边缘处衰减到 e⁻⁹，截断硬边不可见。
    // shadow_blur <= 0 或颜色 alpha 为 0 即无阴影。
    float4 shadow = float4(0.0);
    if (primitive.shadow_blur > 0.0 && primitive.shadow_color.a > 0.0) {
        float2 sp = p - primitive.shadow_offset;
        float sd_shadow = yu_sd_rounded_rect(sp - center, half_size, radius);
        float d = max(sd_shadow, 0.0);
        float sigma = primitive.shadow_blur;
        float falloff = exp(-(d * d) / (sigma * sigma));
        shadow = float4(primitive.shadow_color.rgb, primitive.shadow_color.a * falloff);
    }

    // 阴影在下、填充在上的一次 source-over 合成；结果仍是直出 alpha，
    // 交给管线的标准混合再叠到 retained target 上。
    float alpha = fill.a + shadow.a * (1.0 - fill.a);
    float3 premultiplied = fill.rgb * fill.a + shadow.rgb * shadow.a * (1.0 - fill.a);
    float3 rgb = alpha > 0.0 ? premultiplied / alpha : float3(0.0);
    return float4(rgb, alpha);
}
