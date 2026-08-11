#import <Metal/Metal.h>
#import <QuartzCore/CAMetalLayer.h>

#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>

typedef struct {
    uint32_t kind;
    float x;
    float y;
    float width;
    float height;
    float u0;
    float v0;
    float u1;
    float v1;
    float red;
    float green;
    float blue;
    float alpha;
    uint32_t page;
} YuMetalDrawCommand;

typedef struct {
    uint32_t page;
    void *texture;
} YuMetalTextureBinding;

typedef struct {
    id<MTLRenderPipelineState> solid_pipeline;
    id<MTLRenderPipelineState> glyph_pipeline;
    id<MTLSamplerState> sampler;
} YuMetalPipeline;

int yu_metal_create_device(void **out_device, uint64_t *out_registry_id) {
    if (out_device == NULL || out_registry_id == NULL) {
        return 0;
    }
    id<MTLDevice> device = MTLCreateSystemDefaultDevice();
    if (device == nil) {
        return 0;
    }
    *out_device = (void *)device;
    *out_registry_id = device.registryID;
    return 1;
}

int yu_metal_create_layer(
    void *device_ptr,
    double pixel_width,
    double pixel_height,
    double scale,
    void **out_layer
) {
    if (device_ptr == NULL || out_layer == NULL || pixel_width <= 0.0 || pixel_height <= 0.0 || scale <= 0.0) {
        return 0;
    }
    CAMetalLayer *layer = [CAMetalLayer layer];
    if (layer == nil) {
        return 0;
    }
    [layer retain];
    layer.device = (id<MTLDevice>)device_ptr;
    layer.pixelFormat = MTLPixelFormatBGRA8Unorm;
    layer.framebufferOnly = YES;
    layer.contentsScale = scale;
    layer.drawableSize = CGSizeMake(pixel_width, pixel_height);
    *out_layer = (void *)layer;
    return 1;
}

int yu_metal_resize_layer(
    void *layer_ptr,
    double pixel_width,
    double pixel_height,
    double scale
) {
    if (layer_ptr == NULL || pixel_width <= 0.0 || pixel_height <= 0.0 || scale <= 0.0) {
        return 0;
    }
    CAMetalLayer *layer = (CAMetalLayer *)layer_ptr;
    layer.contentsScale = scale;
    layer.drawableSize = CGSizeMake(pixel_width, pixel_height);
    return 1;
}

int yu_metal_upload_alpha_texture(
    void *device_ptr,
    uint32_t width,
    uint32_t height,
    const uint8_t *pixels,
    size_t pixel_length,
    void **out_texture
) {
    if (device_ptr == NULL || pixels == NULL || out_texture == NULL || width == 0 || height == 0) {
        return 0;
    }
    size_t expected = (size_t)width * (size_t)height;
    if (expected != pixel_length) {
        return 0;
    }
    id<MTLDevice> device = (id<MTLDevice>)device_ptr;
    MTLTextureDescriptor *descriptor = [MTLTextureDescriptor
        texture2DDescriptorWithPixelFormat:MTLPixelFormatR8Unorm
                                      width:width
                                     height:height
                                  mipmapped:NO];
    if (descriptor == nil) {
        return 0;
    }
    descriptor.storageMode = MTLStorageModeShared;
    descriptor.usage = MTLTextureUsageShaderRead;
    id<MTLTexture> texture = [device newTextureWithDescriptor:descriptor];
    if (texture == nil) {
        return 0;
    }
    MTLRegion region = MTLRegionMake2D(0, 0, width, height);
    [texture replaceRegion:region mipmapLevel:0 withBytes:pixels bytesPerRow:width];
    *out_texture = (void *)texture;
    return 1;
}

int yu_metal_create_command_queue(void *device_ptr, void **out_queue) {
    if (device_ptr == NULL || out_queue == NULL) {
        return 0;
    }
    id<MTLCommandQueue> queue = [(id<MTLDevice>)device_ptr newCommandQueue];
    if (queue == nil) {
        return 0;
    }
    *out_queue = (void *)queue;
    return 1;
}

int yu_metal_clear_and_present(
    void *queue_ptr,
    void *layer_ptr,
    float red,
    float green,
    float blue,
    float alpha
) {
    if (queue_ptr == NULL || layer_ptr == NULL) {
        return 0;
    }
    id<CAMetalDrawable> drawable = [(CAMetalLayer *)layer_ptr nextDrawable];
    if (drawable == nil) {
        return 2;
    }
    id<MTLCommandBuffer> command_buffer = [(id<MTLCommandQueue>)queue_ptr commandBuffer];
    if (command_buffer == nil) {
        return 3;
    }
    MTLRenderPassDescriptor *pass = [MTLRenderPassDescriptor renderPassDescriptor];
    if (pass == nil) {
        return 4;
    }
    MTLRenderPassColorAttachmentDescriptor *color = pass.colorAttachments[0];
    color.texture = drawable.texture;
    color.loadAction = MTLLoadActionClear;
    color.storeAction = MTLStoreActionStore;
    color.clearColor = MTLClearColorMake(red, green, blue, alpha);
    id<MTLRenderCommandEncoder> encoder =
        [command_buffer renderCommandEncoderWithDescriptor:pass];
    if (encoder == nil) {
        return 4;
    }
    [encoder endEncoding];
    [command_buffer presentDrawable:drawable];
    [command_buffer commit];
    return 1;
}

int yu_metal_create_pipeline(
    void *device_ptr,
    const char *source,
    size_t source_length,
    void **out_pipeline
) {
    if (device_ptr == NULL || source == NULL || source_length == 0 || out_pipeline == NULL) {
        return 0;
    }

    id<MTLDevice> device = (id<MTLDevice>)device_ptr;
    NSString *shader_source = [[NSString alloc]
        initWithBytes:source
               length:source_length
             encoding:NSUTF8StringEncoding];
    if (shader_source == nil) {
        return 0;
    }

    NSError *library_error = nil;
    id<MTLLibrary> library = [device newLibraryWithSource:shader_source options:nil error:&library_error];
    [shader_source release];
    if (library == nil) {
        return 0;
    }

    id<MTLFunction> vertex = [library newFunctionWithName:@"yu_vertex"];
    id<MTLFunction> solid = [library newFunctionWithName:@"yu_solid_fragment"];
    id<MTLFunction> glyph = [library newFunctionWithName:@"yu_glyph_fragment"];
    if (vertex == nil || solid == nil || glyph == nil) {
        [vertex release];
        [solid release];
        [glyph release];
        [library release];
        return 0;
    }

    MTLVertexDescriptor *vertex_descriptor = [[MTLVertexDescriptor alloc] init];
    vertex_descriptor.attributes[0].format = MTLVertexFormatFloat2;
    vertex_descriptor.attributes[0].offset = 0;
    vertex_descriptor.attributes[0].bufferIndex = 0;
    vertex_descriptor.attributes[1].format = MTLVertexFormatFloat2;
    vertex_descriptor.attributes[1].offset = sizeof(float) * 2;
    vertex_descriptor.attributes[1].bufferIndex = 0;
    vertex_descriptor.layouts[0].stride = sizeof(float) * 4;
    vertex_descriptor.layouts[0].stepFunction = MTLVertexStepFunctionPerVertex;

    MTLRenderPipelineDescriptor *solid_descriptor = [[MTLRenderPipelineDescriptor alloc] init];
    solid_descriptor.vertexFunction = vertex;
    solid_descriptor.fragmentFunction = solid;
    solid_descriptor.vertexDescriptor = vertex_descriptor;
    solid_descriptor.colorAttachments[0].pixelFormat = MTLPixelFormatBGRA8Unorm;
    solid_descriptor.colorAttachments[0].blendingEnabled = YES;
    solid_descriptor.colorAttachments[0].rgbBlendOperation = MTLBlendOperationAdd;
    solid_descriptor.colorAttachments[0].alphaBlendOperation = MTLBlendOperationAdd;
    solid_descriptor.colorAttachments[0].sourceRGBBlendFactor = MTLBlendFactorSourceAlpha;
    solid_descriptor.colorAttachments[0].destinationRGBBlendFactor = MTLBlendFactorOneMinusSourceAlpha;
    solid_descriptor.colorAttachments[0].sourceAlphaBlendFactor = MTLBlendFactorOne;
    solid_descriptor.colorAttachments[0].destinationAlphaBlendFactor = MTLBlendFactorOneMinusSourceAlpha;

    MTLRenderPipelineDescriptor *glyph_descriptor = [solid_descriptor copy];
    glyph_descriptor.fragmentFunction = glyph;

    NSError *pipeline_error = nil;
    id<MTLRenderPipelineState> solid_pipeline =
        [device newRenderPipelineStateWithDescriptor:solid_descriptor error:&pipeline_error];
    id<MTLRenderPipelineState> glyph_pipeline =
        [device newRenderPipelineStateWithDescriptor:glyph_descriptor error:&pipeline_error];

    MTLSamplerDescriptor *sampler_descriptor = [[MTLSamplerDescriptor alloc] init];
    sampler_descriptor.minFilter = MTLSamplerMinMagFilterLinear;
    sampler_descriptor.magFilter = MTLSamplerMinMagFilterLinear;
    sampler_descriptor.sAddressMode = MTLSamplerAddressModeClampToEdge;
    sampler_descriptor.tAddressMode = MTLSamplerAddressModeClampToEdge;
    id<MTLSamplerState> sampler = [device newSamplerStateWithDescriptor:sampler_descriptor];

    [sampler_descriptor release];
    [glyph_descriptor release];
    [solid_descriptor release];
    [vertex_descriptor release];
    [vertex release];
    [solid release];
    [glyph release];
    [library release];

    if (solid_pipeline == nil || glyph_pipeline == nil || sampler == nil) {
        [solid_pipeline release];
        [glyph_pipeline release];
        [sampler release];
        return 0;
    }

    YuMetalPipeline *pipeline = calloc(1, sizeof(YuMetalPipeline));
    if (pipeline == NULL) {
        [solid_pipeline release];
        [glyph_pipeline release];
        [sampler release];
        return 0;
    }
    pipeline->solid_pipeline = solid_pipeline;
    pipeline->glyph_pipeline = glyph_pipeline;
    pipeline->sampler = sampler;
    *out_pipeline = (void *)pipeline;
    return 1;
}

int yu_metal_render_plan(
    void *queue_ptr,
    void *layer_ptr,
    void *pipeline_ptr,
    float viewport_width,
    float viewport_height,
    float scale,
    const YuMetalDrawCommand *commands,
    size_t command_count,
    const YuMetalTextureBinding *textures,
    size_t texture_count
) {
    if (queue_ptr == NULL || layer_ptr == NULL || pipeline_ptr == NULL
        || viewport_width <= 0.0f || viewport_height <= 0.0f || scale <= 0.0f
        || (command_count > 0 && commands == NULL)
        || (texture_count > 0 && textures == NULL)) {
        return 0;
    }

    YuMetalPipeline *pipeline = (YuMetalPipeline *)pipeline_ptr;
    id<CAMetalDrawable> drawable = [(CAMetalLayer *)layer_ptr nextDrawable];
    if (drawable == nil) {
        return 2;
    }
    id<MTLCommandBuffer> command_buffer = [(id<MTLCommandQueue>)queue_ptr commandBuffer];
    if (command_buffer == nil) {
        return 3;
    }

    MTLRenderPassDescriptor *pass = [MTLRenderPassDescriptor renderPassDescriptor];
    if (pass == nil) {
        return 4;
    }
    MTLRenderPassColorAttachmentDescriptor *color = pass.colorAttachments[0];
    color.texture = drawable.texture;
    color.loadAction = MTLLoadActionClear;
    color.storeAction = MTLStoreActionStore;
    color.clearColor = MTLClearColorMake(0.0, 0.0, 0.0, 0.0);
    id<MTLRenderCommandEncoder> encoder =
        [command_buffer renderCommandEncoderWithDescriptor:pass];
    if (encoder == nil) {
        return 4;
    }

    typedef struct {
        float x;
        float y;
        float u;
        float v;
    } YuMetalVertex;
    typedef struct {
        float viewport_width;
        float viewport_height;
        float scale;
        float padding;
    } YuMetalFrameUniforms;
    typedef struct {
        float red;
        float green;
        float blue;
        float alpha;
    } YuMetalPrimitiveUniforms;

    YuMetalFrameUniforms frame = {
        viewport_width,
        viewport_height,
        scale,
        0.0f,
    };
    [encoder setVertexBytes:&frame length:sizeof(frame) atIndex:1];

    for (size_t index = 0; index < command_count; index += 1) {
        YuMetalDrawCommand command = commands[index];
        YuMetalVertex vertices[6] = {
            {command.x, command.y, command.u0, command.v0},
            {command.x + command.width, command.y, command.u1, command.v0},
            {command.x, command.y + command.height, command.u0, command.v1},
            {command.x + command.width, command.y, command.u1, command.v0},
            {command.x + command.width, command.y + command.height, command.u1, command.v1},
            {command.x, command.y + command.height, command.u0, command.v1},
        };
        YuMetalPrimitiveUniforms primitive = {
            command.red,
            command.green,
            command.blue,
            command.alpha,
        };

        if (command.kind == 0) {
            [encoder setRenderPipelineState:pipeline->solid_pipeline];
        } else if (command.kind == 1) {
            void *texture_ptr = NULL;
            for (size_t texture_index = 0; texture_index < texture_count; texture_index += 1) {
                if (textures[texture_index].page == command.page) {
                    texture_ptr = textures[texture_index].texture;
                    break;
                }
            }
            if (texture_ptr == NULL) {
                [encoder endEncoding];
                return 5;
            }
            [encoder setRenderPipelineState:pipeline->glyph_pipeline];
            [encoder setFragmentTexture:(id<MTLTexture>)texture_ptr atIndex:0];
            [encoder setFragmentSamplerState:pipeline->sampler atIndex:0];
        } else {
            [encoder endEncoding];
            return 5;
        }

        [encoder setVertexBytes:vertices length:sizeof(vertices) atIndex:0];
        [encoder setFragmentBytes:&primitive length:sizeof(primitive) atIndex:0];
        [encoder drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:6];
    }

    [encoder endEncoding];
    [command_buffer presentDrawable:drawable];
    [command_buffer commit];
    return 1;
}

void yu_metal_release_pipeline(void *pipeline_ptr) {
    if (pipeline_ptr == NULL) {
        return;
    }
    YuMetalPipeline *pipeline = (YuMetalPipeline *)pipeline_ptr;
    [pipeline->solid_pipeline release];
    [pipeline->glyph_pipeline release];
    [pipeline->sampler release];
    free(pipeline);
}

void yu_metal_release(void *object) {
    if (object != NULL) {
        [(id)object release];
    }
}
