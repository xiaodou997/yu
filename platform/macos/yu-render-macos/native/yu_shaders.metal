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

vertex YuVertexOut yu_vertex(
    YuVertexIn input [[stage_in]],
    constant YuFrameUniforms& frame [[buffer(1)]]
) {
    YuVertexOut output;
    float2 pixel = input.position * frame.scale;
    float2 ndc = float2(
        (pixel.x / frame.viewport.x) * 2.0 - 1.0,
        1.0 - (pixel.y / frame.viewport.y) * 2.0
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
    constant YuPrimitiveUniforms& primitive [[buffer(0)]]
) {
    float4 sampled = image.sample(image_sampler, input.uv);
    return float4(sampled.rgb * primitive.color.rgb, sampled.a * primitive.color.a);
}
