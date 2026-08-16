#import <CoreGraphics/CoreGraphics.h>
#import <Foundation/Foundation.h>
#import <ImageIO/ImageIO.h>

#include <stdint.h>
#include <stdlib.h>

/*
 * ImageIO is deliberately kept on the macOS side of the boundary. The Rust
 * caller receives only owned RGBA8 bytes and dimensions; CGImage/CF objects
 * never enter shared editor state or cross the FFI as retained handles.
 */
int yu_macos_image_decode_file(
    const uint8_t *path_bytes,
    size_t path_length,
    uint32_t *out_width,
    uint32_t *out_height,
    void **out_pixels,
    size_t *out_pixel_length
) {
    if (path_bytes == NULL || path_length == 0 || out_width == NULL || out_height == NULL
        || out_pixels == NULL || out_pixel_length == NULL) {
        return 0;
    }

    NSString *path = [[NSString alloc]
        initWithBytes:path_bytes
               length:path_length
             encoding:NSUTF8StringEncoding];
    if (path == nil) {
        return 0;
    }
    NSURL *url = [NSURL fileURLWithPath:path];
    [path release];
    if (url == nil) {
        return 0;
    }

    CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)url, NULL);
    if (source == NULL) {
        return 0;
    }
    CGImageRef image = CGImageSourceCreateImageAtIndex(source, 0, NULL);
    CFRelease(source);
    if (image == NULL) {
        return 0;
    }

    size_t width = CGImageGetWidth(image);
    size_t height = CGImageGetHeight(image);
    if (width == 0 || height == 0 || width > UINT32_MAX || height > UINT32_MAX
        || width > SIZE_MAX / 4 || height > SIZE_MAX / (width * 4)) {
        CGImageRelease(image);
        return 0;
    }
    size_t bytes_per_row = width * 4;
    size_t pixel_length = bytes_per_row * height;
    void *pixels = calloc(1, pixel_length);
    if (pixels == NULL) {
        CGImageRelease(image);
        return 0;
    }

    CGColorSpaceRef color_space = CGColorSpaceCreateDeviceRGB();
    CGContextRef context = CGBitmapContextCreate(
        pixels,
        width,
        height,
        8,
        bytes_per_row,
        color_space,
        kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big);
    if (color_space != NULL) {
        CGColorSpaceRelease(color_space);
    }
    if (context == NULL) {
        free(pixels);
        CGImageRelease(image);
        return 0;
    }

    CGContextDrawImage(context, CGRectMake(0.0, 0.0, (CGFloat)width, (CGFloat)height), image);
    CGContextRelease(context);
    CGImageRelease(image);

    *out_width = (uint32_t)width;
    *out_height = (uint32_t)height;
    *out_pixels = pixels;
    *out_pixel_length = pixel_length;
    return 1;
}

void yu_macos_image_free_bytes(void *pixels) {
    free(pixels);
}
