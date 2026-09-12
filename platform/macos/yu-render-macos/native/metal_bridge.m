#import <Metal/Metal.h>
#import <AppKit/AppKit.h>
#import <QuartzCore/CAMetalLayer.h>
#import <QuartzCore/QuartzCore.h>
#include <stdio.h>

#include <dispatch/dispatch.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <math.h>
#include <stdatomic.h>

static BOOL yu_render_timing_enabled(void) {
    static dispatch_once_t once;
    static BOOL enabled;
    dispatch_once(&once, ^{ enabled = getenv("YU_RENDER_TIMING") != NULL; });
    return enabled;
}

static void yu_render_metric(const char *event, void *surface, double duration_ms) {
    if (!yu_render_timing_enabled()) return;
    fprintf(stdout, "yu-render-metric event=%s surface=%p time_s=%.9f duration_ms=%.6f\n",
        event, surface, CACurrentMediaTime(), duration_ms);
    fflush(stdout);
}

void yu_metal_notify_resource_completion(void) {
    static atomic_bool pending = false;
    if (atomic_exchange(&pending, true)) return;
    dispatch_async(dispatch_get_main_queue(), ^{
        atomic_store(&pending, false);
        [[NSNotificationCenter defaultCenter]
            postNotificationName:@"YuRenderResourceCompleted" object:nil];
    });
}

// Only the acquisition worker may wait in CAMetalLayer. The main thread
// exchanges owned drawables under a short lock that never encloses Metal calls.
@interface YuMetalLayer : CAMetalLayer {
    id<CAMetalDrawable> readyDrawable;
    BOOL acquisitionPending;
    BOOL acquisitionEnabled;
    uint64_t acquisitionGeneration;
}
- (id<CAMetalDrawable>)takeReadyDrawable;
- (void)setAcquisitionEnabled:(BOOL)enabled;
- (void)invalidateReadyDrawable;
@end

@implementation YuMetalLayer
- (id<CAMetalDrawable>)acquireDrawable {
    return [super nextDrawable];
}
- (void)invalidateReadyDrawable {
    id<CAMetalDrawable> previous;
    @synchronized (self) {
        acquisitionGeneration += 1;
        previous = readyDrawable;
        readyDrawable = nil;
    }
    [previous release];
}
- (void)setAcquisitionEnabled:(BOOL)enabled {
    @synchronized (self) {
        acquisitionEnabled = enabled;
    }
    [self invalidateReadyDrawable];
}
- (id<CAMetalDrawable>)takeReadyDrawable {
    uint64_t generation;
    @synchronized (self) {
        if (!acquisitionEnabled) return nil;
        if (readyDrawable != nil) {
            id<CAMetalDrawable> drawable = readyDrawable;
            readyDrawable = nil;
            return [drawable autorelease];
        }
        if (acquisitionPending) return nil;
        acquisitionPending = YES;
        generation = acquisitionGeneration;
    }
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INTERACTIVE, 0), ^{
        @autoreleasepool {
            CFTimeInterval acquisitionStart = yu_render_timing_enabled() ? CACurrentMediaTime() : 0;
            id<CAMetalDrawable> drawable = [[self acquireDrawable] retain];
            yu_render_metric("drawable_acquire", self, (CACurrentMediaTime() - acquisitionStart) * 1000);
            @synchronized (self) {
                acquisitionPending = NO;
                if (acquisitionEnabled && acquisitionGeneration == generation) {
                    readyDrawable = drawable;
                    drawable = nil;
                }
            }
            [drawable release];
        }
    });
    return nil;
}
- (void)dealloc {
    [readyDrawable release];
    [super dealloc];
}
@end

// Deterministic probe: hold acquisition until the caller has exercised the
// non-blocking path. No window, GPU or compositor timing is needed.
@interface YuMetalBlockedProbeLayer : YuMetalLayer {
@public
    dispatch_semaphore_t entered;
    dispatch_semaphore_t resume;
    dispatch_semaphore_t finished;
}
@end
@implementation YuMetalBlockedProbeLayer
- (id<CAMetalDrawable>)acquireDrawable {
    dispatch_semaphore_signal(entered);
    dispatch_semaphore_wait(resume, DISPATCH_TIME_FOREVER);
    dispatch_semaphore_signal(finished);
    return nil;
}
- (void)dealloc {
    dispatch_release(entered);
    dispatch_release(resume);
    dispatch_release(finished);
    [super dealloc];
}
@end

int yu_metal_nonblocking_acquisition_probe(void) {
    @autoreleasepool {
        YuMetalBlockedProbeLayer *layer = [[YuMetalBlockedProbeLayer alloc] init];
        layer->entered = dispatch_semaphore_create(0);
        layer->resume = dispatch_semaphore_create(0);
        layer->finished = dispatch_semaphore_create(0);
        [layer setAcquisitionEnabled:YES];
        BOOL passed = [layer takeReadyDrawable] == nil;
        long started = dispatch_semaphore_wait(layer->entered,
            dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC));
        passed = passed && started == 0;
        for (int index = 0; index < 1000 && started == 0; index += 1) {
            passed = passed && [layer takeReadyDrawable] == nil;
        }
        [layer invalidateReadyDrawable];
        [layer setAcquisitionEnabled:NO];
        passed = passed && [layer takeReadyDrawable] == nil;
        dispatch_semaphore_signal(layer->resume);
        long completed = dispatch_semaphore_wait(layer->finished,
            dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC));
        // Re-enable the same layer after the blocked worker has returned. A
        // new generation must be able to start a fresh acquisition; the old
        // worker's completion must not leave acquisitionPending stuck.
        [layer setAcquisitionEnabled:YES];
        passed = passed && [layer takeReadyDrawable] == nil;
        long rebound_started = dispatch_semaphore_wait(layer->entered,
            dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC));
        passed = passed && rebound_started == 0;
        [layer setAcquisitionEnabled:NO];
        dispatch_semaphore_signal(layer->resume);
        long rebound_completed = dispatch_semaphore_wait(layer->finished,
            dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC));
        [layer release];
        return passed && completed == 0 && rebound_completed == 0;
    }
}

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
    uint64_t resource;
    uint32_t image_kind;
} YuMetalDrawCommand;

typedef struct {
    uint32_t page;
    void *texture;
} YuMetalTextureBinding;

typedef struct {
    uint64_t resource;
    uint32_t image_kind;
    void *texture;
} YuMetalImageTextureBinding;

typedef struct {
    float x;
    float y;
    float width;
    float height;
} YuMetalDamageRect;

typedef struct {
    NSView *view;
    CALayer *previous_layer;
    CAMetalLayer *metal_layer;
} YuMetalViewAttachment;

typedef struct {
    NSWindow *window;
    NSView *view;
} YuMetalAppKitProbeHost;

typedef void (*YuMetalAppKitCallback)(void *context);

typedef struct {
    id<MTLRenderPipelineState> clear_pipeline;
    id<MTLRenderPipelineState> solid_pipeline;
    id<MTLRenderPipelineState> glyph_pipeline;
    id<MTLRenderPipelineState> image_pipeline;
    id<MTLSamplerState> sampler;
} YuMetalPipeline;

typedef struct {
    id<MTLTexture> texture;
    id<MTLTexture> scroll_scratch;
    // At most two render command buffers may be waiting for the GPU.  The
    // render host is called from AppKit's main thread, so this gate must be
    // checked with DISPATCH_TIME_NOW: a scroll burst drops a stale frame
    // instead of waiting for an older drawable/command buffer to complete.
    dispatch_semaphore_t in_flight;
    NSUInteger width;
    NSUInteger height;
} YuMetalRenderTarget;

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
    YuMetalLayer *layer = [YuMetalLayer layer];
    if (layer == nil) {
        return 0;
    }
    [layer retain];
    layer.device = (id<MTLDevice>)device_ptr;
    layer.pixelFormat = MTLPixelFormatBGRA8Unorm;
    // The retained target is copied into this texture by a blit encoder.
    layer.framebufferOnly = NO;
    // Bound drawable acquisition waits. This timeout is not a non-blocking API.
    layer.allowsNextDrawableTimeout = YES;
    // Limit GPU queue pressure separately below. GPU completion does not imply
    // that the compositor has released a drawable; nextDrawable may still wait.
    layer.maximumDrawableCount = 3;
    // The product keeps a TextKit source mirror underneath this projection.
    // Transparent untouched pixels let that mirror remain the input and
    // accessibility fallback while Rust contributes only its glyph coverage.
    layer.opaque = NO;
    layer.backgroundColor = NSColor.clearColor.CGColor;
    layer.contentsScale = scale;
    layer.drawableSize = CGSizeMake(pixel_width, pixel_height);
    *out_layer = (void *)layer;
    return 1;
}

int yu_metal_attach_layer_to_view(
    void *layer_ptr,
    void *view_ptr,
    void **out_attachment
) {
    if (layer_ptr == NULL || view_ptr == NULL || out_attachment == NULL) {
        return 0;
    }
    YuMetalViewAttachment *attachment = calloc(1, sizeof(YuMetalViewAttachment));
    if (attachment == NULL) {
        return 0;
    }

    NSView *view = (NSView *)view_ptr;
    CAMetalLayer *metal_layer = (CAMetalLayer *)layer_ptr;
    CALayer *previous_layer = view.layer;
    [view retain];
    [previous_layer retain];
    [metal_layer retain];
    [view setWantsLayer:YES];
    [view setLayer:metal_layer];
    [(YuMetalLayer *)metal_layer setAcquisitionEnabled:YES];

    attachment->view = view;
    attachment->previous_layer = previous_layer;
    attachment->metal_layer = metal_layer;
    *out_attachment = (void *)attachment;
    return 1;
}

void yu_metal_detach_layer_from_view(void *attachment_ptr) {
    if (attachment_ptr == NULL) {
        return;
    }
    YuMetalViewAttachment *attachment = (YuMetalViewAttachment *)attachment_ptr;
    [(YuMetalLayer *)attachment->metal_layer setAcquisitionEnabled:NO];
    if (attachment->view.layer == attachment->metal_layer) {
        [attachment->view setLayer:attachment->previous_layer];
    }
    [attachment->view release];
    [attachment->previous_layer release];
    [attachment->metal_layer release];
    free(attachment);
}

int yu_metal_create_appkit_probe_host(
    double width,
    double height,
    void **out_host,
    void **out_view
) {
    if (width <= 0.0 || height <= 0.0 || out_host == NULL || out_view == NULL) {
        return 0;
    }
    YuMetalAppKitProbeHost *host = calloc(1, sizeof(YuMetalAppKitProbeHost));
    if (host == NULL) {
        return 0;
    }

    NSApplication *application = [NSApplication sharedApplication];
    [application setActivationPolicy:NSApplicationActivationPolicyRegular];
    NSRect frame = NSMakeRect(0.0, 0.0, width, height);
    NSWindow *window = [[NSWindow alloc]
        initWithContentRect:frame
                  styleMask:(NSWindowStyleMaskTitled
                             | NSWindowStyleMaskClosable
                             | NSWindowStyleMaskResizable)
                    backing:NSBackingStoreBuffered
                      defer:NO];
    if (window == nil) {
        free(host);
        return 0;
    }
    NSView *view = [[NSView alloc] initWithFrame:frame];
    if (view == nil) {
        [window release];
        free(host);
        return 0;
    }
    view.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    [window setContentView:view];
    [window center];
    [window makeKeyAndOrderFront:nil];
    [application activateIgnoringOtherApps:YES];
    [window displayIfNeeded];

    host->window = window;
    host->view = view;
    *out_host = (void *)host;
    *out_view = (void *)view;
    return 1;
}

void yu_metal_destroy_appkit_probe_host(void *host_ptr) {
    if (host_ptr == NULL) {
        return;
    }
    YuMetalAppKitProbeHost *host = (YuMetalAppKitProbeHost *)host_ptr;
    [host->window orderOut:nil];
    [host->window close];
    [host->view release];
    [host->window release];
    free(host);
}

void yu_metal_run_appkit_on_main(YuMetalAppKitCallback callback, void *context) {
    if (callback == NULL) {
        return;
    }
    if ([NSThread isMainThread]) {
        @autoreleasepool {
            callback(context);
        }
        return;
    }
    dispatch_sync(dispatch_get_main_queue(), ^{
        @autoreleasepool {
            callback(context);
        }
    });
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
    [(YuMetalLayer *)layer invalidateReadyDrawable];
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

int yu_metal_upload_rgba_texture(
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
    if ((size_t)width > SIZE_MAX / 4 || (size_t)height > SIZE_MAX / ((size_t)width * 4)) {
        return 0;
    }
    size_t bytes_per_row = (size_t)width * 4;
    size_t expected = bytes_per_row * (size_t)height;
    if (expected != pixel_length) {
        return 0;
    }
    MTLTextureDescriptor *descriptor = [MTLTextureDescriptor
        texture2DDescriptorWithPixelFormat:MTLPixelFormatRGBA8Unorm
                                      width:width
                                     height:height
                                  mipmapped:NO];
    if (descriptor == nil) {
        return 0;
    }
    descriptor.storageMode = MTLStorageModeShared;
    descriptor.usage = MTLTextureUsageShaderRead;
    id<MTLTexture> texture = [(id<MTLDevice>)device_ptr newTextureWithDescriptor:descriptor];
    if (texture == nil) {
        return 0;
    }
    MTLRegion region = MTLRegionMake2D(0, 0, width, height);
    [texture replaceRegion:region mipmapLevel:0 withBytes:pixels bytesPerRow:bytes_per_row];
    *out_texture = (void *)texture;
    return 1;
}

int yu_metal_create_render_target(
    void *device_ptr,
    uint32_t width,
    uint32_t height,
    void **out_target
) {
    if (device_ptr == NULL || out_target == NULL || width == 0 || height == 0) {
        return 0;
    }
    MTLTextureDescriptor *descriptor = [MTLTextureDescriptor
        texture2DDescriptorWithPixelFormat:MTLPixelFormatBGRA8Unorm
                                      width:width
                                     height:height
                                  mipmapped:NO];
    if (descriptor == nil) {
        return 0;
    }
    descriptor.storageMode = MTLStorageModePrivate;
    descriptor.usage = MTLTextureUsageRenderTarget | MTLTextureUsageShaderRead;
    id<MTLTexture> texture = [(id<MTLDevice>)device_ptr newTextureWithDescriptor:descriptor];
    if (texture == nil) {
        return 0;
    }
    YuMetalRenderTarget *target = calloc(1, sizeof(YuMetalRenderTarget));
    if (target == NULL) {
        [texture release];
        return 0;
    }
    target->texture = texture;
    target->in_flight = dispatch_semaphore_create(2);
    if (target->in_flight == NULL) {
        [texture release];
        free(target);
        return 0;
    }
    target->width = width;
    target->height = height;
    *out_target = (void *)target;
    return 1;
}

void yu_metal_release_render_target(void *target_ptr) {
    if (target_ptr == NULL) {
        return;
    }
    YuMetalRenderTarget *target = (YuMetalRenderTarget *)target_ptr;
    [target->texture release];
    [target->scroll_scratch release];
    dispatch_release(target->in_flight);
    free(target);
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
    id<CAMetalDrawable> drawable = [(YuMetalLayer *)layer_ptr takeReadyDrawable];
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
    id<MTLFunction> image = [library newFunctionWithName:@"yu_image_fragment"];
    if (vertex == nil || solid == nil || glyph == nil || image == nil) {
        [vertex release];
        [solid release];
        [glyph release];
        [image release];
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
    MTLRenderPipelineDescriptor *image_descriptor = [solid_descriptor copy];
    image_descriptor.fragmentFunction = image;
    MTLRenderPipelineDescriptor *clear_descriptor = [solid_descriptor copy];
    clear_descriptor.colorAttachments[0].blendingEnabled = NO;

    NSError *pipeline_error = nil;
    id<MTLRenderPipelineState> clear_pipeline =
        [device newRenderPipelineStateWithDescriptor:clear_descriptor error:&pipeline_error];
    id<MTLRenderPipelineState> solid_pipeline =
        [device newRenderPipelineStateWithDescriptor:solid_descriptor error:&pipeline_error];
    id<MTLRenderPipelineState> glyph_pipeline =
        [device newRenderPipelineStateWithDescriptor:glyph_descriptor error:&pipeline_error];
    id<MTLRenderPipelineState> image_pipeline =
        [device newRenderPipelineStateWithDescriptor:image_descriptor error:&pipeline_error];

    MTLSamplerDescriptor *sampler_descriptor = [[MTLSamplerDescriptor alloc] init];
    sampler_descriptor.minFilter = MTLSamplerMinMagFilterLinear;
    sampler_descriptor.magFilter = MTLSamplerMinMagFilterLinear;
    sampler_descriptor.sAddressMode = MTLSamplerAddressModeClampToEdge;
    sampler_descriptor.tAddressMode = MTLSamplerAddressModeClampToEdge;
    id<MTLSamplerState> sampler = [device newSamplerStateWithDescriptor:sampler_descriptor];

    [sampler_descriptor release];
    [clear_descriptor release];
    [image_descriptor release];
    [glyph_descriptor release];
    [solid_descriptor release];
    [vertex_descriptor release];
    [vertex release];
    [solid release];
    [glyph release];
    [image release];
    [library release];

    if (clear_pipeline == nil || solid_pipeline == nil || glyph_pipeline == nil
        || image_pipeline == nil || sampler == nil) {
        [clear_pipeline release];
        [solid_pipeline release];
        [glyph_pipeline release];
        [image_pipeline release];
        [sampler release];
        return 0;
    }

    YuMetalPipeline *pipeline = calloc(1, sizeof(YuMetalPipeline));
    if (pipeline == NULL) {
        [clear_pipeline release];
        [solid_pipeline release];
        [glyph_pipeline release];
        [sampler release];
        return 0;
    }
    pipeline->clear_pipeline = clear_pipeline;
    pipeline->solid_pipeline = solid_pipeline;
    pipeline->glyph_pipeline = glyph_pipeline;
    pipeline->image_pipeline = image_pipeline;
    pipeline->sampler = sampler;
    *out_pipeline = (void *)pipeline;
    return 1;
}

static int yu_metal_damage_scissor(
    YuMetalDamageRect damage,
    float scale,
    NSUInteger drawable_width,
    NSUInteger drawable_height,
    MTLScissorRect *out_scissor
) {
    float left = fmaxf(0.0f, damage.x * scale);
    float top = fmaxf(0.0f, damage.y * scale);
    float right = fminf((float)drawable_width, (damage.x + damage.width) * scale);
    float bottom = fminf((float)drawable_height, (damage.y + damage.height) * scale);
    if (!isfinite(left) || !isfinite(top) || !isfinite(right) || !isfinite(bottom)
        || right <= left || bottom <= top) {
        return 0;
    }
    NSUInteger x = (NSUInteger)floorf(left);
    NSUInteger y = (NSUInteger)floorf(top);
    NSUInteger max_right = (NSUInteger)ceilf(right);
    NSUInteger max_bottom = (NSUInteger)ceilf(bottom);
    if (max_right > drawable_width) {
        max_right = drawable_width;
    }
    if (max_bottom > drawable_height) {
        max_bottom = drawable_height;
    }
    if (max_right <= x || max_bottom <= y) {
        return 0;
    }
    out_scissor->x = x;
    out_scissor->y = y;
    out_scissor->width = max_right - x;
    out_scissor->height = max_bottom - y;
    return 1;
}

static int yu_metal_encode_command(
    id<MTLRenderCommandEncoder> encoder,
    YuMetalPipeline *pipeline,
    YuMetalDrawCommand command,
    const YuMetalTextureBinding *textures,
    size_t texture_count,
    const YuMetalImageTextureBinding *image_textures,
    size_t image_texture_count
) {
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
            return 0;
        }
        [encoder setRenderPipelineState:pipeline->glyph_pipeline];
        [encoder setFragmentTexture:(id<MTLTexture>)texture_ptr atIndex:0];
        [encoder setFragmentSamplerState:pipeline->sampler atIndex:0];
    } else if (command.kind == 2) {
        void *texture_ptr = NULL;
        for (size_t texture_index = 0; texture_index < image_texture_count; texture_index += 1) {
            if (image_textures[texture_index].resource == command.resource
                && image_textures[texture_index].image_kind == command.image_kind) {
                texture_ptr = image_textures[texture_index].texture;
                break;
            }
        }
        if (texture_ptr == NULL) {
            return 0;
        }
        [encoder setRenderPipelineState:pipeline->image_pipeline];
        [encoder setFragmentTexture:(id<MTLTexture>)texture_ptr atIndex:0];
        [encoder setFragmentSamplerState:pipeline->sampler atIndex:0];
    } else {
        return 0;
    }

    [encoder setVertexBytes:vertices length:sizeof(vertices) atIndex:0];
    [encoder setFragmentBytes:&primitive length:sizeof(primitive) atIndex:0];
    [encoder drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:6];
    return 1;
}

static void yu_metal_encode_clear_rect(
    id<MTLRenderCommandEncoder> encoder,
    YuMetalPipeline *pipeline,
    YuMetalDamageRect damage,
    YuMetalPrimitiveUniforms background
) {
    YuMetalVertex vertices[6] = {
        {damage.x, damage.y, 0.0f, 0.0f},
        {damage.x + damage.width, damage.y, 0.0f, 0.0f},
        {damage.x, damage.y + damage.height, 0.0f, 0.0f},
        {damage.x + damage.width, damage.y, 0.0f, 0.0f},
        {damage.x + damage.width, damage.y + damage.height, 0.0f, 0.0f},
        {damage.x, damage.y + damage.height, 0.0f, 0.0f},
    };
    [encoder setRenderPipelineState:pipeline->clear_pipeline];
    [encoder setVertexBytes:vertices length:sizeof(vertices) atIndex:0];
    [encoder setFragmentBytes:&background length:sizeof(background) atIndex:0];
    [encoder drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:6];
}

int yu_metal_render_plan(
    void *queue_ptr,
    void *layer_ptr,
    void *pipeline_ptr,
    void *target_ptr,
    float viewport_width,
    float viewport_height,
    float scale,
    int full_clear,
    int32_t scroll_pixels,
    float clear_red,
    float clear_green,
    float clear_blue,
    float clear_alpha,
    const YuMetalDrawCommand *commands,
    size_t command_count,
    const YuMetalDamageRect *damage,
    size_t damage_count,
    const YuMetalTextureBinding *textures,
    size_t texture_count,
    const YuMetalImageTextureBinding *image_textures,
    size_t image_texture_count
) {
    if (queue_ptr == NULL || layer_ptr == NULL || pipeline_ptr == NULL || target_ptr == NULL
        || viewport_width <= 0.0f || viewport_height <= 0.0f || scale <= 0.0f
        || (command_count > 0 && commands == NULL)
        || (damage_count > 0 && damage == NULL)
        || (texture_count > 0 && textures == NULL)
        || (image_texture_count > 0 && image_textures == NULL)) {
        return 0;
    }

    CFTimeInterval encodeStart = yu_render_timing_enabled() ? CACurrentMediaTime() : 0;
    YuMetalPipeline *pipeline = (YuMetalPipeline *)pipeline_ptr;
    YuMetalRenderTarget *target = (YuMetalRenderTarget *)target_ptr;
    if (target->in_flight == NULL
        || dispatch_semaphore_wait(target->in_flight, DISPATCH_TIME_NOW) != 0) {
        // A previous command buffer still owns the available GPU slot.  The
        // caller will submit the newest request on the next display-link
        // tick; never wait on AppKit's main thread here.
        yu_render_metric("gpu_busy", layer_ptr, 0);
        return 2;
    }
    id<CAMetalDrawable> drawable = [(YuMetalLayer *)layer_ptr takeReadyDrawable];
    if (drawable == nil) {
        dispatch_semaphore_signal(target->in_flight);
        yu_render_metric("drawable_unavailable", layer_ptr, 0);
        return 2;
    }
    if (drawable.texture.width != target->width || drawable.texture.height != target->height) {
        dispatch_semaphore_signal(target->in_flight);
        // A resize may race acquisition even when its generation is current.
        // Discard this drawable and request the new dimensions on the next tick.
        return 2;
    }
    id<MTLCommandBuffer> command_buffer = [(id<MTLCommandQueue>)queue_ptr commandBuffer];
    if (command_buffer == nil) {
        dispatch_semaphore_signal(target->in_flight);
        return 3;
    }

    // Encode the move in the same buffer as the redraw. Failed preparation
    // never commits a half-scrolled target. Scratch avoids overlapping copies.
    if (scroll_pixels != 0) {
        NSUInteger amount = (NSUInteger)llabs((long long)scroll_pixels);
        if (full_clear || amount >= target->height) {
            dispatch_semaphore_signal(target->in_flight);
            return 0;
        }
        if (target->scroll_scratch == nil) {
            MTLTextureDescriptor *descriptor = [MTLTextureDescriptor
                texture2DDescriptorWithPixelFormat:target->texture.pixelFormat
                width:target->width height:target->height mipmapped:NO];
            descriptor.storageMode = MTLStorageModePrivate;
            target->scroll_scratch = [target->texture.device newTextureWithDescriptor:descriptor];
            if (target->scroll_scratch == nil) {
                dispatch_semaphore_signal(target->in_flight);
                return 0;
            }
        }
        id<MTLBlitCommandEncoder> move = [command_buffer blitCommandEncoder];
        if (move == nil) {
            dispatch_semaphore_signal(target->in_flight);
            return 7;
        }
        MTLSize overlap = MTLSizeMake(target->width, target->height - amount, 1);
        NSUInteger source_y = scroll_pixels > 0 ? amount : 0;
        NSUInteger destination_y = scroll_pixels > 0 ? 0 : amount;
        [move copyFromTexture:target->texture sourceSlice:0 sourceLevel:0
            sourceOrigin:MTLOriginMake(0, source_y, 0) sourceSize:overlap
            toTexture:target->scroll_scratch destinationSlice:0 destinationLevel:0
            destinationOrigin:MTLOriginMake(0, 0, 0)];
        [move copyFromTexture:target->scroll_scratch sourceSlice:0 sourceLevel:0
            sourceOrigin:MTLOriginMake(0, 0, 0) sourceSize:overlap
            toTexture:target->texture destinationSlice:0 destinationLevel:0
            destinationOrigin:MTLOriginMake(0, destination_y, 0)];
        [move endEncoding];
    }

    MTLRenderPassDescriptor *pass = [MTLRenderPassDescriptor renderPassDescriptor];
    if (pass == nil) {
        dispatch_semaphore_signal(target->in_flight);
        return 4;
    }
    MTLRenderPassColorAttachmentDescriptor *color = pass.colorAttachments[0];
    color.texture = target->texture;
    color.loadAction = full_clear ? MTLLoadActionClear : MTLLoadActionLoad;
    color.storeAction = MTLStoreActionStore;
    color.clearColor = MTLClearColorMake(clear_red, clear_green, clear_blue, clear_alpha);
    id<MTLRenderCommandEncoder> encoder =
        [command_buffer renderCommandEncoderWithDescriptor:pass];
    if (encoder == nil) {
        dispatch_semaphore_signal(target->in_flight);
        return 4;
    }

    YuMetalFrameUniforms frame = {
        viewport_width,
        viewport_height,
        scale,
        0.0f,
    };
    [encoder setVertexBytes:&frame length:sizeof(frame) atIndex:1];

    if (full_clear) {
        MTLScissorRect full_scissor = {
            0,
            0,
            drawable.texture.width,
            drawable.texture.height,
        };
        [encoder setScissorRect:full_scissor];
        for (size_t index = 0; index < command_count; index += 1) {
            if (!yu_metal_encode_command(
                    encoder,
                    pipeline,
                    commands[index],
                    textures,
                    texture_count,
                    image_textures,
                    image_texture_count)) {
                [encoder endEncoding];
                dispatch_semaphore_signal(target->in_flight);
                return 5;
            }
        }
    } else {
        for (size_t damage_index = 0; damage_index < damage_count; damage_index += 1) {
            YuMetalDamageRect damage_rect = damage[damage_index];
            MTLScissorRect scissor;
            if (!yu_metal_damage_scissor(
                    damage_rect,
                    scale,
                    drawable.texture.width,
                    drawable.texture.height,
                    &scissor)) {
                continue;
            }
            [encoder setScissorRect:scissor];
            YuMetalPrimitiveUniforms background = {clear_red, clear_green, clear_blue, clear_alpha};
            yu_metal_encode_clear_rect(encoder, pipeline, damage_rect, background);
            for (size_t index = 0; index < command_count; index += 1) {
                if (!yu_metal_encode_command(
                        encoder,
                        pipeline,
                        commands[index],
                        textures,
                        texture_count,
                        image_textures,
                        image_texture_count)) {
                    [encoder endEncoding];
                    dispatch_semaphore_signal(target->in_flight);
                    return 5;
                }
            }
        }
    }

    [encoder endEncoding];
    id<MTLBlitCommandEncoder> blit = [command_buffer blitCommandEncoder];
    if (blit == nil) {
        dispatch_semaphore_signal(target->in_flight);
        return 7;
    }
    MTLSize copy_size = MTLSizeMake(target->width, target->height, 1);
    [blit copyFromTexture:target->texture
              sourceSlice:0
              sourceLevel:0
             sourceOrigin:MTLOriginMake(0, 0, 0)
               sourceSize:copy_size
                toTexture:drawable.texture
         destinationSlice:0
         destinationLevel:0
        destinationOrigin:MTLOriginMake(0, 0, 0)];
    [blit endEncoding];
    if (yu_render_timing_enabled()) {
        [drawable addPresentedHandler:^(id<MTLDrawable> presented) {
            fprintf(stdout, "yu-render-metric event=present surface=%p time_s=%.9f\n",
                layer_ptr, presented.presentedTime);
            fflush(stdout);
        }];
    }
    [command_buffer presentDrawable:drawable];
    dispatch_semaphore_t in_flight = target->in_flight;
    // The semaphore is captured independently of `target`; this keeps the
    // completion callback valid even if the Rust target wrapper is dropped
    // after the command has been committed.
    dispatch_retain(in_flight);
    [command_buffer addCompletedHandler:^(id<MTLCommandBuffer> _Nonnull _) {
        yu_render_metric("gpu_complete", layer_ptr, 0);
        dispatch_semaphore_signal(in_flight);
        dispatch_release(in_flight);
    }];
    yu_render_metric("gpu_submit", layer_ptr, 0);
    [command_buffer commit];
    yu_render_metric("metal_encode_submit", layer_ptr, (CACurrentMediaTime() - encodeStart) * 1000);
    return 1;
}

void yu_metal_release_pipeline(void *pipeline_ptr) {
    if (pipeline_ptr == NULL) {
        return;
    }
    YuMetalPipeline *pipeline = (YuMetalPipeline *)pipeline_ptr;
    [pipeline->clear_pipeline release];
    [pipeline->solid_pipeline release];
    [pipeline->glyph_pipeline release];
    [pipeline->image_pipeline release];
    [pipeline->sampler release];
    free(pipeline);
}

void yu_metal_release(void *object) {
    if (object != NULL) {
        [(id)object release];
    }
}
