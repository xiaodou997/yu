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

// Work readiness does not change resource geometry or document identity.
// Wake pending hosts once; settled windows ignore this notification.
void yu_metal_notify_frame_work_ready(void) {
    static atomic_bool pending = false;
    if (atomic_exchange(&pending, true)) return;
    dispatch_async(dispatch_get_main_queue(), ^{
        atomic_store(&pending, false);
        [[NSNotificationCenter defaultCenter]
            postNotificationName:@"YuRenderWorkReady" object:nil];
    });
}

// Only the acquisition worker may wait in CAMetalLayer. The main thread
// exchanges owned drawables under a short lock that never encloses Metal calls.
@interface YuMetalLayer : CAMetalLayer {
    id<CAMetalDrawable> readyDrawable;
    BOOL acquisitionPending;
    BOOL acquisitionEnabled;
    uint64_t acquisitionGeneration;
    uint64_t submittedSerial;
    uint64_t presentedSerial;
    uint64_t droppedSerial;
    uint64_t droppedGeneration;
    CFTimeInterval latestPresentedTime;
}
- (id<CAMetalDrawable>)takeReadyDrawable;
- (void)trackPresentation:(id<CAMetalDrawable>)drawable;
- (uint64_t)beginPresentation;
- (void)didPresentSerial:(uint64_t)serial atTime:(CFTimeInterval)time;
- (BOOL)hasPresentedLatest;
- (BOOL)needsPresentationRecovery;
- (BOOL)recordDroppedSerial:(uint64_t)serial generation:(uint64_t)generation;
- (CFTimeInterval)latestPresentationTime;
- (void)setAcquisitionEnabled:(BOOL)enabled;
- (void)invalidateReadyDrawable;
@end

@implementation YuMetalLayer
- (void)trackPresentation:(id<CAMetalDrawable>)drawable {
    uint64_t serial = [self beginPresentation];
    uint64_t generation;
    @synchronized (self) { generation = acquisitionGeneration; }
    if (serial == 1 && yu_render_timing_enabled()) {
        fprintf(stdout, "yu-render-metric event=presentation_policy surface=%p vsync=%d transaction=%d drawable_count=%lu\n",
            self, self.displaySyncEnabled, self.presentsWithTransaction,
            (unsigned long)self.maximumDrawableCount);
        fflush(stdout);
    }
    [drawable addPresentedHandler:^(id<MTLDrawable> presented) {
        if (presented.presentedTime == 0) {
            if ([self recordDroppedSerial:serial generation:generation]) {
                dispatch_async(dispatch_get_main_queue(), ^{
                    // A newer submission, resize or detach may supersede this
                    // callback before the main thread handles it.
                    @synchronized (self) {
                        if (submittedSerial != serial || acquisitionGeneration != generation
                            || ![self needsPresentationRecovery]) return;
                    }
                    yu_render_metric("presentation_dropped", self, 0);
                    [[NSNotificationCenter defaultCenter]
                        postNotificationName:@"YuRenderPresentationDropped" object:self];
                });
            }
            return;
        }
        [self didPresentSerial:serial atTime:presented.presentedTime];
    }];
}
- (uint64_t)beginPresentation {
    @synchronized (self) { return ++submittedSerial; }
}
- (void)didPresentSerial:(uint64_t)serial atTime:(CFTimeInterval)time {
    if (!isfinite(time) || time <= 0) return;
    @synchronized (self) {
        if (serial > presentedSerial) {
            presentedSerial = serial;
            latestPresentedTime = time;
        }
    }
}
- (BOOL)recordDroppedSerial:(uint64_t)serial generation:(uint64_t)generation {
    @synchronized (self) {
        if (!acquisitionEnabled || acquisitionGeneration != generation
            || serial != submittedSerial || serial <= presentedSerial
            || droppedSerial == serial) return NO;
        droppedSerial = serial;
        droppedGeneration = generation;
        return YES;
    }
}
- (BOOL)needsPresentationRecovery {
    @synchronized (self) {
        return acquisitionEnabled && droppedSerial != 0
            && droppedSerial == submittedSerial && presentedSerial < droppedSerial
            && droppedGeneration == acquisitionGeneration;
    }
}
- (BOOL)hasPresentedLatest {
    return [self latestPresentationTime] > 0;
}
- (CFTimeInterval)latestPresentationTime {
    @synchronized (self) {
        return submittedSerial != 0 && presentedSerial == submittedSerial ? latestPresentedTime : 0;
    }
}
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
            yu_metal_notify_frame_work_ready();
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
    // 与 crates/yu-render/src/backend.rs 的 DrawCommand 末尾逐字段对应，
    // 两侧同改：这是静态链接的内部 ABI，没有头文件替你查。
    float radius;
    float rect_offset_x;
    float rect_offset_y;
    float rect_width;
    float rect_height;
    float shadow_offset_x;
    float shadow_offset_y;
    float shadow_blur;
    uint32_t shadow_color;
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
    id<MTLRenderPipelineState> rounded_pipeline;
    id<MTLRenderPipelineState> polyline_pipeline;
    id<MTLSamplerState> sampler;
} YuMetalPipeline;

typedef struct {
    id<MTLTexture> texture;
    id<MTLTexture> scroll_scratch;
    NSMutableArray<id<MTLBuffer>> *vertex_pool;
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

// 圆角矩形：一个外扩 quad 承载填充 + 阴影，字段语义见 Rust 侧 DrawCommand
// 注释与 yu_shaders.metal 的 YuRoundedUniforms。逻辑像素。
typedef struct {
    float quad_width;
    float quad_height;
    float rect_offset_x;
    float rect_offset_y;
    float rect_width;
    float rect_height;
    float radius;
    float shadow_blur;
    float shadow_offset_x;
    float shadow_offset_y;
    // MSL float4 is 16-byte aligned: the colors start at 48, not 40.
    float color_alignment_padding[2];
    float shadow_red;
    float shadow_green;
    float shadow_blue;
    float shadow_alpha;
    float fill_red;
    float fill_green;
    float fill_blue;
    float fill_alpha;
} YuMetalRoundedUniforms;

// 图片：颜色乘子 + quad 尺寸 + 裁剪圆角。逻辑像素。
typedef struct {
    float red;
    float green;
    float blue;
    float alpha;
    float width;
    float height;
    float radius;
    float padding;
} YuMetalImageUniforms;

// 布局断言：YuMetalDrawCommand 与 crates/yu-render 的 DrawCommand 是两边
// 手写同步的内部 ABI，Rust 侧有同值的单元测试（draw_command_layout_matches_
// metal_bridge），这里在编译期把 C 侧钉死——任一边漂移都会在构建时炸。
_Static_assert(sizeof(YuMetalDrawCommand) == 104,
    "YuMetalDrawCommand must stay in sync with yu-render DrawCommand");
_Static_assert(offsetof(YuMetalDrawCommand, resource) == 56,
    "DrawCommand field order drifted");
_Static_assert(offsetof(YuMetalDrawCommand, radius) == 68,
    "DrawCommand field order drifted");
_Static_assert(offsetof(YuMetalDrawCommand, shadow_color) == 100,
    "DrawCommand field order drifted");
_Static_assert(sizeof(YuMetalRoundedUniforms) == 80,
    "YuMetalRoundedUniforms must match yu_shaders.metal");
_Static_assert(offsetof(YuMetalRoundedUniforms, shadow_red) == 48,
    "Metal shadow float4 must start at byte 48");
_Static_assert(offsetof(YuMetalRoundedUniforms, fill_red) == 64,
    "Metal fill float4 must start at byte 64");
_Static_assert(sizeof(YuMetalImageUniforms) == 32,
    "YuMetalImageUniforms must match yu_shaders.metal");

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
    // Two drawables reduce compositor queue latency for interactive writing.
    // Acquisition still runs off the main thread and vertical sync stays on.
    // GPU completion does not imply that the compositor released a drawable.
    layer.maximumDrawableCount = 2;
    // The document surface owns every pixel. Glass is provided by AppKit chrome.
    layer.opaque = YES;
    layer.backgroundColor = NSColor.textBackgroundColor.CGColor;
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

int yu_metal_presentation_serial_self_check(void) {
    @autoreleasepool {
        YuMetalLayer *layer = [YuMetalLayer layer];
        if ([layer hasPresentedLatest]) return 0;
        uint64_t first = [layer beginPresentation];
        if ([layer hasPresentedLatest]) return 0;
        uint64_t second = [layer beginPresentation];
        [layer didPresentSerial:first atTime:1.25];
        if ([layer hasPresentedLatest]) return 0;
        [layer didPresentSerial:second atTime:0];
        if ([layer hasPresentedLatest]) return 0;
        [layer didPresentSerial:second atTime:2.5];
        if (![layer hasPresentedLatest]) return 0;
        if ([layer latestPresentationTime] != 2.5) return 0;
        [layer didPresentSerial:first atTime:3.0];
        if (![layer hasPresentedLatest]) return 0;
        if ([layer latestPresentationTime] != 2.5) return 0;
        [layer beginPresentation];
        return ![layer hasPresentedLatest];
    }
}

double yu_metal_layer_presentation_time(void *layer_ptr) {
    return layer_ptr != NULL ? [(YuMetalLayer *)layer_ptr latestPresentationTime] : 0;
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
    target->vertex_pool = [[NSMutableArray alloc] init];
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
    [target->vertex_pool release];
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
    [(YuMetalLayer *)layer_ptr trackPresentation:drawable];
    [command_buffer presentDrawable:drawable];
    [command_buffer commit];
    return 1;
}

int yu_metal_create_pipeline(
    void *device_ptr,
    const uint8_t *library_bytes,
    size_t library_length,
    void **out_pipeline
) {
    if (device_ptr == NULL || library_bytes == NULL || library_length == 0 || out_pipeline == NULL) return 0;
    id<MTLDevice> device = (id<MTLDevice>)device_ptr;
    void *owned = malloc(library_length);
    if (owned == NULL) return 0;
    memcpy(owned, library_bytes, library_length);
    dispatch_data_t data = dispatch_data_create(owned, library_length,
        dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), DISPATCH_DATA_DESTRUCTOR_FREE);
    NSError *library_error = nil;
    id<MTLLibrary> library = [device newLibraryWithData:data error:&library_error];
    dispatch_release(data);
    if (library_error != nil) NSLog(@"Yu Metal library: %@", library_error.localizedDescription);
    if (library == nil) {
        return 0;
    }

    id<MTLFunction> vertex = [library newFunctionWithName:@"yu_vertex"];
    id<MTLFunction> solid = [library newFunctionWithName:@"yu_solid_fragment"];
    id<MTLFunction> glyph = [library newFunctionWithName:@"yu_glyph_fragment"];
    id<MTLFunction> image = [library newFunctionWithName:@"yu_image_fragment"];
    id<MTLFunction> rounded = [library newFunctionWithName:@"yu_rounded_fragment"];
    id<MTLFunction> polyline = [library newFunctionWithName:@"yu_polyline_fragment"];
    if (vertex == nil || solid == nil || glyph == nil || image == nil || rounded == nil || polyline == nil) {
        [vertex release];
        [solid release];
        [glyph release];
        [image release];
        [rounded release];
        [polyline release];
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
    MTLRenderPipelineDescriptor *rounded_descriptor = [solid_descriptor copy];
    rounded_descriptor.fragmentFunction = rounded;
    MTLRenderPipelineDescriptor *polyline_descriptor = [solid_descriptor copy];
    polyline_descriptor.fragmentFunction = polyline;
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
    id<MTLRenderPipelineState> rounded_pipeline =
        [device newRenderPipelineStateWithDescriptor:rounded_descriptor error:&pipeline_error];
    id<MTLRenderPipelineState> polyline_pipeline =
        [device newRenderPipelineStateWithDescriptor:polyline_descriptor error:&pipeline_error];

    MTLSamplerDescriptor *sampler_descriptor = [[MTLSamplerDescriptor alloc] init];
    sampler_descriptor.minFilter = MTLSamplerMinMagFilterLinear;
    sampler_descriptor.magFilter = MTLSamplerMinMagFilterLinear;
    sampler_descriptor.sAddressMode = MTLSamplerAddressModeClampToEdge;
    sampler_descriptor.tAddressMode = MTLSamplerAddressModeClampToEdge;
    id<MTLSamplerState> sampler = [device newSamplerStateWithDescriptor:sampler_descriptor];

    [sampler_descriptor release];
    [clear_descriptor release];
    [image_descriptor release];
    [rounded_descriptor release];
    [polyline_descriptor release];
    [glyph_descriptor release];
    [solid_descriptor release];
    [vertex_descriptor release];
    [vertex release];
    [solid release];
    [glyph release];
    [image release];
    [rounded release];
    [polyline release];
    [library release];

    if (clear_pipeline == nil || solid_pipeline == nil || glyph_pipeline == nil
        || image_pipeline == nil || rounded_pipeline == nil || polyline_pipeline == nil || sampler == nil) {
        [clear_pipeline release];
        [solid_pipeline release];
        [glyph_pipeline release];
        [image_pipeline release];
        [rounded_pipeline release];
        [polyline_pipeline release];
        [sampler release];
        return 0;
    }

    YuMetalPipeline *pipeline = calloc(1, sizeof(YuMetalPipeline));
    if (pipeline == NULL) {
        [clear_pipeline release];
        [solid_pipeline release];
        [glyph_pipeline release];
        [image_pipeline release];
        [rounded_pipeline release];
        [polyline_pipeline release];
        [sampler release];
        return 0;
    }
    pipeline->clear_pipeline = clear_pipeline;
    pipeline->solid_pipeline = solid_pipeline;
    pipeline->glyph_pipeline = glyph_pipeline;
    pipeline->image_pipeline = image_pipeline;
    pipeline->rounded_pipeline = rounded_pipeline;
    pipeline->polyline_pipeline = polyline_pipeline;
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
    size_t image_texture_count,
    id<MTLBuffer> vertex_buffer, NSUInteger vertex_offset, NSUInteger vertex_count
) {
    YuMetalVertex vertices[6] = {
        {command.x, command.y, command.u0, command.v0},
        {command.x + command.width, command.y, command.u1, command.v0},
        {command.x, command.y + command.height, command.u0, command.v1},
        {command.x + command.width, command.y, command.u1, command.v0},
        {command.x + command.width, command.y + command.height, command.u1, command.v1},
        {command.x, command.y + command.height, command.u0, command.v1},
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
    } else if (command.kind == 4) {
        [encoder setRenderPipelineState:pipeline->polyline_pipeline];
    } else if (command.kind == 3) {
        // 圆角矩形：纯着色器绘制，无纹理；quad 外扩与几何偏移已随命令带来。
        [encoder setRenderPipelineState:pipeline->rounded_pipeline];
    } else {
        return 0;
    }

    if (vertex_buffer != nil) {
        [encoder setVertexBuffer:vertex_buffer offset:vertex_offset atIndex:0];
    } else {
        [encoder setVertexBytes:vertices length:sizeof(vertices) atIndex:0];
    }
    // fragment uniform 按 kind 区分布局：solid/glyph 是颜色四元组，image 多
    // 带 quad 尺寸与裁剪圆角，rounded 是填充 + 阴影的完整参数块。
    if (command.kind == 4) {
        // Matches YuPolylineUniforms: local points and width, no textures.
        float polyline[16] = {
            command.width, command.height,
            command.rect_offset_x, command.rect_offset_y,
            command.rect_width, command.rect_height,
            command.shadow_offset_x, command.shadow_offset_y,
            command.radius, 0.0f, 0.0f, 0.0f,
            command.red, command.green, command.blue, command.alpha
        };
        [encoder setFragmentBytes:polyline length:sizeof(polyline) atIndex:0];
    } else if (command.kind == 3) {
        // shadow_color 是打包 RGBA8（大端 [r,g,b,a]），在此归一化。
        YuMetalRoundedUniforms rounded = {
            command.width,
            command.height,
            command.rect_offset_x,
            command.rect_offset_y,
            command.rect_width,
            command.rect_height,
            command.radius,
            command.shadow_blur,
            command.shadow_offset_x,
            command.shadow_offset_y,
            {0.0f, 0.0f},
            (float)((command.shadow_color >> 24) & 0xffu) / 255.0f,
            (float)((command.shadow_color >> 16) & 0xffu) / 255.0f,
            (float)((command.shadow_color >> 8) & 0xffu) / 255.0f,
            (float)(command.shadow_color & 0xffu) / 255.0f,
            command.red,
            command.green,
            command.blue,
            command.alpha,
        };
        [encoder setFragmentBytes:&rounded length:sizeof(rounded) atIndex:0];
    } else if (command.kind == 2) {
        YuMetalImageUniforms image = {
            command.red,
            command.green,
            command.blue,
            command.alpha,
            command.width,
            command.height,
            command.radius,
            0.0f,
        };
        [encoder setFragmentBytes:&image length:sizeof(image) atIndex:0];
    } else {
        YuMetalPrimitiveUniforms primitive = {
            command.red,
            command.green,
            command.blue,
            command.alpha,
        };
        [encoder setFragmentBytes:&primitive length:sizeof(primitive) atIndex:0];
    }
    [encoder drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:vertex_count];
    return 1;
}

// Merge adjacent primitives only. Color, atlas and scissor order are preserved.
static size_t yu_metal_batch_end(const YuMetalDrawCommand *commands, size_t start, size_t count) {
    YuMetalDrawCommand first = commands[start];
    size_t end = start + 1;
    if (first.kind != 0 && first.kind != 1) return end;
    while (end < count) {
        YuMetalDrawCommand next = commands[end];
        if (next.kind != first.kind || (first.kind == 1 && next.page != first.page)
            || next.red != first.red || next.green != first.green
            || next.blue != first.blue || next.alpha != first.alpha) break;
        end++;
    }
    return end;
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

    if (command_count > NSUIntegerMax / (6 * sizeof(YuMetalVertex))) {
        [encoder endEncoding];
        dispatch_semaphore_signal(target->in_flight);
        return 5;
    }
    NSUInteger vertex_bytes = MAX(1, command_count * 6 * sizeof(YuMetalVertex));
    NSMutableArray<id<MTLBuffer>> *pool = target->vertex_pool;
    id<MTLBuffer> vertex_buffer = nil;
    @synchronized(pool) {
        if (pool.count > 0) {
            vertex_buffer = [[[pool lastObject] retain] autorelease];
            [pool removeLastObject];
        }
    }
    if (vertex_buffer == nil || vertex_buffer.length < vertex_bytes) {
        vertex_buffer = [[target->texture.device newBufferWithLength:vertex_bytes
            options:MTLResourceStorageModeShared] autorelease];
    }
    if (vertex_buffer == nil) {
        [encoder endEncoding];
        dispatch_semaphore_signal(target->in_flight);
        return 5;
    }
    YuMetalVertex *vertices = vertex_buffer.contents;
    for (size_t i = 0; i < command_count; i++) {
        YuMetalDrawCommand c = commands[i];
        YuMetalVertex quad[6] = {
            {c.x, c.y, c.u0, c.v0}, {c.x+c.width, c.y, c.u1, c.v0},
            {c.x, c.y+c.height, c.u0, c.v1}, {c.x+c.width, c.y, c.u1, c.v0},
            {c.x+c.width, c.y+c.height, c.u1, c.v1}, {c.x, c.y+c.height, c.u0, c.v1}
        };
        memcpy(vertices + i * 6, quad, sizeof(quad));
    }
    if (full_clear) {
        MTLScissorRect full_scissor = {
            0,
            0,
            drawable.texture.width,
            drawable.texture.height,
        };
        [encoder setScissorRect:full_scissor];
        for (size_t index = 0, end = 0; index < command_count; index = end) {
            end = yu_metal_batch_end(commands, index, command_count);
            if (!yu_metal_encode_command(
                    encoder,
                    pipeline,
                    commands[index],
                    textures,
                    texture_count,
                    image_textures,
                    image_texture_count, vertex_buffer, index * 6 * sizeof(YuMetalVertex), (end - index) * 6)) {
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
            for (size_t index = 0, end = 0; index < command_count; index = end) {
                end = yu_metal_batch_end(commands, index, command_count);
            if (!yu_metal_encode_command(
                        encoder,
                        pipeline,
                        commands[index],
                        textures,
                        texture_count,
                        image_textures,
                        image_texture_count, vertex_buffer, index * 6 * sizeof(YuMetalVertex), (end - index) * 6)) {
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
    [(YuMetalLayer *)layer_ptr trackPresentation:drawable];
    [command_buffer presentDrawable:drawable];
    dispatch_semaphore_t in_flight = target->in_flight;
    // The semaphore is captured independently of `target`; this keeps the
    // completion callback valid even if the Rust target wrapper is dropped
    // after the command has been committed.
    dispatch_retain(in_flight);
    [command_buffer addCompletedHandler:^(id<MTLCommandBuffer> _Nonnull _) {
        yu_render_metric("gpu_complete", layer_ptr, 0);
        @synchronized(pool) { if (pool.count < 2) [pool addObject:vertex_buffer]; }
        dispatch_semaphore_signal(in_flight);
        yu_metal_notify_frame_work_ready();
        dispatch_release(in_flight);
    }];
    yu_render_metric("gpu_submit", layer_ptr, 0);
    [command_buffer commit];
    yu_render_metric("metal_encode_submit", layer_ptr, (CACurrentMediaTime() - encodeStart) * 1000);
    return 1;
}

// Resolve the real production presented callback without compositor timing.
@interface YuMetalPresentationProbeDrawable : NSObject <CAMetalDrawable> {
    CFTimeInterval time;
    MTLDrawablePresentedHandler completion;
}
- (void)resolve:(CFTimeInterval)value;
@end
@implementation YuMetalPresentationProbeDrawable
- (id<MTLTexture>)texture { return nil; }
- (CAMetalLayer *)layer { return nil; }
- (CFTimeInterval)presentedTime { return time; }
- (NSUInteger)drawableID { return 0; }
- (void)present {}
- (void)presentAtTime:(CFTimeInterval)value { (void)value; }
- (void)presentAfterMinimumDuration:(CFTimeInterval)value { (void)value; }
- (void)addPresentedHandler:(MTLDrawablePresentedHandler)handler {
    [completion release]; completion = [handler copy];
}
- (void)resolve:(CFTimeInterval)value { time = value; completion(self); }
- (void)dealloc { [completion release]; [super dealloc]; }
@end

int yu_metal_presentation_recovery_self_check(void) {
    @autoreleasepool {
        YuMetalLayer *layer = [YuMetalLayer layer];
        [layer setAcquisitionEnabled:YES];
        if ([layer needsPresentationRecovery]) return 0;
        YuMetalPresentationProbeDrawable *first = [[[YuMetalPresentationProbeDrawable alloc] init] autorelease];
        YuMetalPresentationProbeDrawable *second = [[[YuMetalPresentationProbeDrawable alloc] init] autorelease];
        [layer trackPresentation:first];
        [layer trackPresentation:second];
        [first resolve:0];
        if ([layer needsPresentationRecovery]) return 0;
        [second resolve:0];
        if (![layer needsPresentationRecovery]) return 0;
        [second resolve:2.5];
        if ([layer needsPresentationRecovery] || ![layer hasPresentedLatest]) return 0;
        [second resolve:0];
        if ([layer needsPresentationRecovery]) return 0;
        [layer trackPresentation:first];
        [layer invalidateReadyDrawable];
        [first resolve:0];
        if ([layer needsPresentationRecovery]) return 0;
        [layer trackPresentation:second];
        [layer setAcquisitionEnabled:NO];
        [second resolve:0];
        if ([layer needsPresentationRecovery]) return 0;
        [layer setAcquisitionEnabled:YES];
        [layer trackPresentation:first];
        [second resolve:0];
        if ([layer needsPresentationRecovery]) return 0;
        [first resolve:0];
        if (![layer needsPresentationRecovery]) return 0;
        [layer beginPresentation];
        return ![layer needsPresentationRecovery];
    }
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
    [pipeline->rounded_pipeline release];
    [pipeline->polyline_pipeline release];
    [pipeline->sampler release];
    free(pipeline);
}

void yu_metal_release(void *object) {
    if (object != NULL) {
        [(id)object release];
    }
}

// Real GPU pixel oracle for the C/MSL uniform contract. Test-only callers use
// an offscreen target; no window or screen-recording permission is involved.
// Test-only caller: exercise the production image encoder after a glyph batch.
// Readback/wait are confined to this off-screen regression, never frame submission.
int yu_metal_image_pixel_probe(void *device_ptr, void *pipeline_ptr, void *image_ptr,
                               uint32_t variant, uint32_t *out_ink) {
    if (!device_ptr || !pipeline_ptr || !image_ptr || !out_ink) return 0;
    @autoreleasepool {
        id<MTLDevice> device = (id<MTLDevice>)device_ptr;
        YuMetalPipeline *pipeline = (YuMetalPipeline *)pipeline_ptr;
        const NSUInteger width = 1320, height = 1136, count = 22;
        MTLTextureDescriptor *desc = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatBGRA8Unorm width:width height:height mipmapped:NO];
        desc.storageMode = MTLStorageModePrivate;
        desc.usage = MTLTextureUsageRenderTarget | MTLTextureUsageShaderRead;
        id<MTLTexture> target = [[device newTextureWithDescriptor:desc] autorelease];
        id<MTLCommandQueue> queue = [[device newCommandQueue] autorelease];
        id<MTLBuffer> vertices = [[device newBufferWithLength:count*6*sizeof(YuMetalVertex) options:MTLResourceStorageModeShared] autorelease];
        const NSUInteger row = 256, sample_height = 40;
        id<MTLBuffer> sample = [[device newBufferWithLength:row*sample_height options:MTLResourceStorageModeShared] autorelease];
        if (!target || !queue || !vertices || !sample) return 0;
        YuMetalDrawCommand commands[22] = {0};
        commands[0] = (YuMetalDrawCommand){.kind=0,.width=660,.height=568,.red=1,.green=1,.blue=1,.alpha=1};
        for (NSUInteger i=1; i<20; i++)
            commands[i] = (YuMetalDrawCommand){.kind=1,.x=(float)i*5,.y=55,.width=3,.height=12,.u1=1,.v1=1,.red=1,.green=1,.blue=1,.alpha=1,.page=0};
        commands[20] = (YuMetalDrawCommand){.kind=2,.x=24,.y=106.518875f,.width=25,.height=19,.u1=1,.v1=1,.red=1,.green=1,.blue=1,.alpha=1,.resource=1,.image_kind=1};
        commands[21] = (YuMetalDrawCommand){.kind=0,.x=24,.y=48,.width=1,.height=38.4,.alpha=1};
        YuMetalVertex *values = vertices.contents;
        for (NSUInteger i=0; i<count; i++) {
            YuMetalDrawCommand c = commands[i];
            YuMetalVertex quad[6] = {{c.x,c.y,c.u0,c.v0},{c.x+c.width,c.y,c.u1,c.v0},{c.x,c.y+c.height,c.u0,c.v1},{c.x+c.width,c.y,c.u1,c.v0},{c.x+c.width,c.y+c.height,c.u1,c.v1},{c.x,c.y+c.height,c.u0,c.v1}};
            memcpy(values+i*6,quad,sizeof(quad));
        }
        void *glyph_ptr = NULL;
        uint8_t *glyph_pixels = calloc(1024*1024, 4);
        if (!glyph_pixels) return 0;
        int uploaded = yu_metal_upload_rgba_texture(device_ptr,1024,1024,glyph_pixels,1024*1024*4,&glyph_ptr);
        free(glyph_pixels);
        if (!uploaded || !glyph_ptr) return 0;
        [(id)glyph_ptr autorelease];
        YuMetalTextureBinding glyph = {0, glyph_ptr};
        YuMetalImageTextureBinding image = {1,1,image_ptr};
        id<MTLCommandBuffer> buffer = [queue commandBuffer];
        MTLRenderPassDescriptor *pass = [MTLRenderPassDescriptor renderPassDescriptor];
        pass.colorAttachments[0].texture = target;
        pass.colorAttachments[0].loadAction = MTLLoadActionClear;
        pass.colorAttachments[0].storeAction = MTLStoreActionStore;
        pass.colorAttachments[0].clearColor = MTLClearColorMake(1,1,1,1);
        id<MTLRenderCommandEncoder> encoder = [buffer renderCommandEncoderWithDescriptor:pass];
        if (!buffer || !encoder) return 0;
        YuMetalFrameUniforms frame = {660,568,2,0};
        [encoder setVertexBytes:&frame length:sizeof(frame) atIndex:1];
        for (size_t i=0; i<count;) {
            size_t end = yu_metal_batch_end(commands,i,count);
            BOOL inline_image = (variant & 1) && commands[i].kind == 2;
            if (!yu_metal_encode_command(encoder,pipeline,commands[i],&glyph,1,&image,1,
                inline_image ? nil : vertices, i*6*sizeof(YuMetalVertex),(end-i)*6)) {
                [encoder endEncoding]; return 0;
            }
            i=end;
        }
        [encoder endEncoding];
        id<MTLBlitCommandEncoder> blit = [buffer blitCommandEncoder];
        if (!blit) return 0;
        [blit copyFromTexture:target sourceSlice:0 sourceLevel:0 sourceOrigin:MTLOriginMake(48,212,0) sourceSize:MTLSizeMake(50,sample_height,1) toBuffer:sample destinationOffset:0 destinationBytesPerRow:row destinationBytesPerImage:row*sample_height];
        [blit endEncoding];
        [buffer commit]; [buffer waitUntilCompleted];
        if (buffer.status != MTLCommandBufferStatusCompleted) return 0;
        uint32_t ink = 0;
        const uint8_t *pixels = sample.contents;
        for (NSUInteger y=0; y<sample_height; y++) for (NSUInteger x=0; x<50; x++) {
            const uint8_t *p = pixels+y*row+x*4;
            if (p[0]<190 && p[1]<190 && p[2]<190) ink++;
        }
        *out_ink=ink;
        return 1;
    }
}

int yu_metal_rounded_pixel_probe(void *device_ptr, void *pipeline_ptr, uint8_t *rgba) {
    if (!device_ptr || !pipeline_ptr || !rgba) return 0;
    @autoreleasepool {
        id<MTLDevice> device = (id<MTLDevice>)device_ptr;
        id<MTLCommandQueue> queue = [device newCommandQueue];
        MTLTextureDescriptor *descriptor = [MTLTextureDescriptor
            texture2DDescriptorWithPixelFormat:MTLPixelFormatBGRA8Unorm
            width:32 height:32 mipmapped:NO];
        descriptor.storageMode = MTLStorageModeShared;
        descriptor.usage = MTLTextureUsageRenderTarget;
        id<MTLTexture> target = [device newTextureWithDescriptor:descriptor];
        if (!queue || !target) { [queue release]; [target release]; return 0; }
        id<MTLCommandBuffer> buffer = [queue commandBuffer];
        MTLRenderPassDescriptor *pass = [MTLRenderPassDescriptor renderPassDescriptor];
        pass.colorAttachments[0].texture = target;
        pass.colorAttachments[0].loadAction = MTLLoadActionClear;
        pass.colorAttachments[0].storeAction = MTLStoreActionStore;
        pass.colorAttachments[0].clearColor = MTLClearColorMake(0, 0, 0, 0);
        id<MTLRenderCommandEncoder> encoder = [buffer renderCommandEncoderWithDescriptor:pass];
        YuMetalFrameUniforms frame = {32, 32, 1, 0};
        [encoder setVertexBytes:&frame length:sizeof(frame) atIndex:1];
        YuMetalDrawCommand command = {0};
        command.kind = 3;
        command.x = 4; command.y = 4; command.width = 24; command.height = 24;
        command.u1 = 1; command.v1 = 1;
        command.rect_width = 24; command.rect_height = 24; command.radius = 4;
        command.red = 49.0f/255; command.green = 127.0f/255; command.blue = 185.0f/255;
        command.alpha = 1;
        int encoded = yu_metal_encode_command(encoder, (YuMetalPipeline *)pipeline_ptr,
            command, NULL, 0, NULL, 0, nil, 0, 6);
        [encoder endEncoding];
        [buffer commit];
        [buffer waitUntilCompleted];
        uint8_t pixel[4] = {0};
        [target getBytes:pixel bytesPerRow:4 fromRegion:MTLRegionMake2D(16,16,1,1) mipmapLevel:0];
        rgba[0] = pixel[2]; rgba[1] = pixel[1]; rgba[2] = pixel[0]; rgba[3] = pixel[3];
        int ok = encoded && buffer.status == MTLCommandBufferStatusCompleted;
        [queue release]; [target release];
        return ok;
    }
}
