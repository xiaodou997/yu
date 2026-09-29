// Included by pdf_bridge.m: shares the existing flow composer, not PDF replay.
#import <ImageIO/ImageIO.h>
#include <stdio.h>

static NSColor *png_color(uint32_t rgba) {
    return [NSColor colorWithSRGBRed:((rgba>>24)&255)/255.0 green:((rgba>>16)&255)/255.0 blue:((rgba>>8)&255)/255.0 alpha:1];
}
int yu_macos_png_prepare(const uint8_t *json,size_t length,void *owner,YuPDFCheckpoint check,void **out_plan,uint32_t *count,char *error,size_t capacity) {
    if(!json||!length||!check||!out_plan||!count||!error||!capacity)return 0;
    *out_plan=NULL;*count=0;error[0]=0;
    @autoreleasepool { @try {
        NSDictionary *packet=[NSJSONSerialization JSONObjectWithData:[NSData dataWithBytesNoCopy:(void *)json length:length freeWhenDone:NO] options:0 error:nil];
        NSDictionary *settings=packet[@"png"];
        NSUInteger scale=[settings[@"scale"] unsignedIntegerValue],width=[settings[@"pixelWidth"] unsignedIntegerValue];
        CGFloat logical=[packet[@"width"] doubleValue]*4/3,plane=[packet[@"height"] doubleValue];
        if(scale<1||scale>2||width!=logical*scale||logical<320||logical>2048||!isfinite(plane)||plane<=36||plane>24576){strlcpy(error,"PNG dimension parameters invalid",capacity);return 0;}
        YuPDFComposer *c=YuPDFComposer.new;c.packet=packet;c.check=check;c.owner=owner;c.raster=YES;c.recording=YES;
        c.foreground=png_color([settings[@"foreground"] unsignedIntValue]);c.background=png_color([settings[@"background"] unsignedIntValue]);c.linkColor=png_color([settings[@"link"] unsignedIntValue]);
        if(![c render]){strlcpy(error,(c.failure?:@"PNG layout failed").UTF8String,capacity);return 0;}
        NSMutableArray *sizes=NSMutableArray.array;uint64_t total=0;
        for(NSNumber *used in c.segmentHeights){
            uint64_t height=(uint64_t)ceil(used.doubleValue*4/3)*scale;
            if(!height||height>32768||height*width>16ull*1024*1024||total+height*width>128ull*1024*1024){strlcpy(error,"PNG 超过单段16 Mi/总128 Mi像素预算",capacity);return 0;}
            total+=height*width;[sizes addObject:@[@(width),@(height)]];
        }
        if(!sizes.count||sizes.count>64||sizes.count!=c.pageCommands.count||!check(owner)){strlcpy(error,"PNG 分段数量无效、超限或已取消",capacity);return 0;}
        NSDictionary *plan=@{@"commands":[c.pageCommands copy],@"sizes":sizes,@"scale":@(scale),@"plane":@(plane),@"background":c.background};
        *out_plan=(void *)CFBridgingRetain(plan);*count=(uint32_t)sizes.count;return 1;
    } @catch(NSException *e){strlcpy(error,"PNG 布局异常，未输出文件",capacity);return 0;} }
}
void yu_macos_png_free(void *plan){if(plan)CFRelease(plan);}
int yu_macos_png_size(void *plan,uint32_t index,uint32_t *width,uint32_t *height){
    if(!plan||!width||!height)return 0;NSDictionary *p=(__bridge NSDictionary *)plan;
    NSArray *sizes=p[@"sizes"];if(index>=sizes.count)return 0;
    *width=[sizes[index][0] unsignedIntValue];*height=[sizes[index][1] unsignedIntValue];return 1;
}
typedef struct {NSMutableData *data;YuPDFCheckpoint check;void *owner;size_t limit;bool failed;} YuPNGSink;
static size_t png_write(void *info,const void *bytes,size_t count){
    YuPNGSink *s=info;
    if(s->failed||!s->check(s->owner)||s->data.length>s->limit||count>s->limit-s->data.length){s->failed=true;return 0;}
    [s->data appendBytes:bytes length:count];return count;
}
int yu_macos_png_encode(void *plan,uint32_t index,void *owner,YuPDFCheckpoint check,size_t limit,void **bytes,size_t *length,char *error,size_t capacity){
    if(!plan||!check||!bytes||!length||!error||!capacity)return 0;
    *bytes=NULL;*length=0;error[0]=0;CGContextRef context=NULL;CGImageRef image=NULL;CGImageDestinationRef dest=NULL;CGDataConsumerRef consumer=NULL;
    int success=0;
    @autoreleasepool { @try {
        NSDictionary *p=(__bridge NSDictionary *)plan;uint32_t w=0,h=0;
        if(!yu_macos_png_size(plan,index,&w,&h)||!w||!h||h>32768||(uint64_t)w*h>16ull*1024*1024||!check(owner)){strlcpy(error,"PNG 像素超限或已取消",capacity);return 0;}
        CGColorSpaceRef space=CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        context=CGBitmapContextCreate(NULL,w,h,8,(size_t)w*4,space,kCGImageAlphaNoneSkipLast);CGColorSpaceRelease(space);
        if(!context){strlcpy(error,"无法分配PNG离屏位图",capacity);return 0;}
        CGContextSetFillColorWithColor(context,((NSColor *)p[@"background"]).CGColor);CGContextFillRect(context,CGRectMake(0,0,w,h));
        CGFloat factor=[p[@"scale"] doubleValue]*4/3;
        CGContextScaleCTM(context,factor,factor);CGContextTranslateCTM(context,0,h/factor-[p[@"plane"] doubleValue]);
        for(void (^draw)(CGContextRef) in p[@"commands"][index]){
            if(!check(owner)){strlcpy(error,"已取消PNG绘制",capacity);return 0;} draw(context);
        }
        image=CGBitmapContextCreateImage(context);if(!image){strlcpy(error,"无法取得PNG离屏结果",capacity);return 0;}
        YuPNGSink sink={NSMutableData.data,check,owner,MIN(limit,256u*1024u*1024u),false};CGDataConsumerCallbacks callbacks={png_write,NULL};
        consumer=CGDataConsumerCreate(&sink,&callbacks);dest=CGImageDestinationCreateWithDataConsumer(consumer,CFSTR("public.png"),1,NULL);
        if(!dest){strlcpy(error,"无法建立PNG编码器",capacity);return 0;}
        CGImageDestinationAddImage(dest,image,NULL);
        if(!CGImageDestinationFinalize(dest)||sink.failed||!check(owner)){strlcpy(error,"PNG编码超限、失败或已取消",capacity);return 0;}
        void *result=malloc(sink.data.length);if(!result){strlcpy(error,"PNG编码结果分配失败",capacity);return 0;}
        memcpy(result,sink.data.bytes,sink.data.length);*bytes=result;*length=sink.data.length;success=1;
    } @catch(NSException *e){strlcpy(error,"PNG绘制异常，未提交文件",capacity);}
      @finally{if(dest)CFRelease(dest);if(consumer)CGDataConsumerRelease(consumer);if(image)CGImageRelease(image);if(context)CGContextRelease(context);}
    }
    return success;
}
// The kernel rejects any pre-existing final directory atomically, including an
// empty one created between preflight and commit. No merge or overwrite fallback.
int yu_macos_move_directory_exclusive(const char *source,const char *target){
    if(!source||!target)return -1;return renamex_np(source,target,RENAME_EXCL);
}
