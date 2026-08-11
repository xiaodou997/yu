#import <Metal/Metal.h>
#import <QuartzCore/CAMetalLayer.h>

#include <stddef.h>
#include <stdint.h>

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

void yu_metal_release(void *object) {
    if (object != NULL) {
        [(id)object release];
    }
}
