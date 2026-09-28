#import <CoreGraphics/CoreGraphics.h>
#import <AppKit/AppKit.h>
#import <Foundation/Foundation.h>
#import <ImageIO/ImageIO.h>

#include <stdint.h>
#include <stdlib.h>
#include <string.h>

/*
 * ImageIO is deliberately kept on the macOS side of the boundary. The Rust
 * caller receives only owned RGBA8 bytes and dimensions; CGImage/CF objects
 * never enter shared editor state or cross the FFI as retained handles.
 */
int yu_macos_image_decode_file(
    const uint8_t *path_bytes,
    size_t path_length,
    uint32_t max_pixel_dimension,
    uint32_t *out_intrinsic_width,
    uint32_t *out_intrinsic_height,
    uint32_t *out_width,
    uint32_t *out_height,
    void **out_pixels,
    size_t *out_pixel_length
) {
    if (path_bytes == NULL || path_length == 0 || out_width == NULL || out_height == NULL
        || out_pixels == NULL || out_pixel_length == NULL
        || out_intrinsic_width == NULL || out_intrinsic_height == NULL) {
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
    NSDictionary *properties = [(NSDictionary *)CGImageSourceCopyPropertiesAtIndex(source, 0, NULL) autorelease];
    uint32_t intrinsic_width = [properties[(id)kCGImagePropertyPixelWidth] unsignedIntValue];
    uint32_t intrinsic_height = [properties[(id)kCGImagePropertyPixelHeight] unsignedIntValue];
    NSInteger orientation = [properties[(id)kCGImagePropertyOrientation] integerValue];
    if (orientation >= 5 && orientation <= 8) {
        uint32_t swap = intrinsic_width; intrinsic_width = intrinsic_height; intrinsic_height = swap;
    }
    NSDictionary *options = @{
        (id)kCGImageSourceCreateThumbnailFromImageAlways: @YES,
        (id)kCGImageSourceCreateThumbnailWithTransform: @YES,
        (id)kCGImageSourceThumbnailMaxPixelSize: @(MAX(1, MIN(4096, max_pixel_dimension))),
        (id)kCGImageSourceShouldCacheImmediately: @YES
    };
    CGImageRef image = CGImageSourceCreateThumbnailAtIndex(source, 0, (CFDictionaryRef)options);
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

    *out_intrinsic_width = intrinsic_width ?: (uint32_t)width;
    *out_intrinsic_height = intrinsic_height ?: (uint32_t)height;
    *out_width = (uint32_t)width;
    *out_height = (uint32_t)height;
    *out_pixels = pixels;
    *out_pixel_length = pixel_length;
    return 1;
}

void yu_macos_image_free_bytes(void *pixels) {
    free(pixels);
}

/* Export decodes frozen bytes, never reopens a changing path. ImageIO handles
 * orientation and PNG encoding. Original metadata (GPS, author, filenames) is
 * not copied. Budget checks precede decoded-image allocation; no downsampling
 * quietly discards document image detail. */
int yu_macos_export_image_png(const uint8_t *bytes, size_t length,
    uint64_t max_pixels, void **out_bytes, size_t *out_length,
    uint32_t *out_width, uint32_t *out_height) {
    if (!bytes || !length || !out_bytes || !out_length || !out_width || !out_height) return 0;
    *out_bytes = NULL; *out_length = 0;
    @autoreleasepool {
        NSData *data = [NSData dataWithBytesNoCopy:(void *)bytes length:length freeWhenDone:NO];
        CGImageSourceRef source = CGImageSourceCreateWithData((CFDataRef)data,
            (CFDictionaryRef)@{(id)kCGImageSourceShouldCache: @NO});
        if (!source) return 0;
        NSDictionary *properties = [(NSDictionary *)CGImageSourceCopyPropertiesAtIndex(source, 0, NULL) autorelease];
        uint64_t width = [properties[(id)kCGImagePropertyPixelWidth] unsignedLongLongValue];
        uint64_t height = [properties[(id)kCGImagePropertyPixelHeight] unsignedLongLongValue];
        if (!width || !height || width > UINT32_MAX || height > UINT32_MAX || width > max_pixels / height) {
            CFRelease(source); return -1;
        }
        CGImageRef image = CGImageSourceCreateThumbnailAtIndex(source, 0, (CFDictionaryRef)@{
            (id)kCGImageSourceCreateThumbnailFromImageAlways: @YES,
            (id)kCGImageSourceCreateThumbnailWithTransform: @YES,
            (id)kCGImageSourceThumbnailMaxPixelSize: @(MAX(width, height)),
            (id)kCGImageSourceShouldCacheImmediately: @YES
        });
        CFRelease(source);
        if (!image) return 0;
        size_t actual_width = CGImageGetWidth(image), actual_height = CGImageGetHeight(image);
        if (!actual_width || !actual_height || actual_width > max_pixels / actual_height) {
            CGImageRelease(image); return -1;
        }
        NSMutableData *png = [NSMutableData data];
        CGImageDestinationRef destination = CGImageDestinationCreateWithData((CFMutableDataRef)png, CFSTR("public.png"), 1, NULL);
        if (!destination) { CGImageRelease(image); return 0; }
        CGImageDestinationAddImage(destination, image, NULL);
        bool written = CGImageDestinationFinalize(destination);
        CFRelease(destination); CGImageRelease(image);
        if (!written || png.length == 0 || png.length > 32 * 1024 * 1024) return -1;
        void *copy = malloc(png.length);
        if (!copy) return -1;
        memcpy(copy, png.bytes, png.length);
        *out_bytes = copy; *out_length = png.length;
        *out_width = (uint32_t)actual_width; *out_height = (uint32_t)actual_height;
        return 1;
    }
}

/*
 * AppKit owns the system SVG decoder on macOS. The bridge deliberately
 * rasterizes into caller-selected bounded dimensions and returns only copied
 * RGBA8 bytes; NSImage/CGImage objects never cross into Rust or the shared
 * editor crates.
 */
int yu_macos_svg_rasterize(
    const uint8_t *markup_bytes,
    size_t markup_length,
    uint32_t width,
    uint32_t height,
    void **out_pixels,
    size_t *out_pixel_length
) {
    if (markup_bytes == NULL || markup_length == 0 || width == 0 || height == 0
        || out_pixels == NULL || out_pixel_length == NULL) {
        return 0;
    }

    int status = 0;
    @autoreleasepool {
        NSData *data = [[NSData alloc] initWithBytes:markup_bytes length:markup_length];
        NSImage *image = [[NSImage alloc] initWithData:data];
        [data release];
        if (image == nil) {
            [image release];
            return 0;
        }

        NSRect proposed = NSMakeRect(0.0, 0.0, (CGFloat)width, (CGFloat)height);
        CGImageRef cg_image = [image CGImageForProposedRect:&proposed context:nil hints:nil];
        if (cg_image == NULL) {
            [image release];
            return 0;
        }

        if ((size_t)width > SIZE_MAX / 4 || (size_t)height > SIZE_MAX / ((size_t)width * 4)) {
            [image release];
            return 0;
        }
        size_t bytes_per_row = (size_t)width * 4;
        size_t pixel_length = bytes_per_row * (size_t)height;
        void *pixels = calloc(1, pixel_length);
        if (pixels == NULL) {
            [image release];
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
            [image release];
            return 0;
        }

        CGContextDrawImage(context, CGRectMake(0.0, 0.0, (CGFloat)width, (CGFloat)height), cg_image);
        CGContextRelease(context);
        [image release];
        *out_pixels = pixels;
        *out_pixel_length = pixel_length;
        status = 1;
    }
    return status;
}
