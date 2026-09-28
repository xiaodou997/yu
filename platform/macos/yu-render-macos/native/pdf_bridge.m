#import <AppKit/AppKit.h>
#import <CoreText/CoreText.h>
#import <CoreGraphics/CoreGraphics.h>
#import <Foundation/Foundation.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>

// This adapter receives resolved flow, never Markdown or an editable session.
// Public NSImage drawing into a non-screen Quartz context preserves supported
// SVG vectors. There are no private CoreSVG selectors or bitmap page captures.
typedef bool (*YuPDFCheckpoint)(void *);
typedef struct { NSMutableData *data; YuPDFCheckpoint check; void *owner; bool failed; } YuPDFSink;
static size_t pdf_write(void *info, const void *bytes, size_t count) {
    YuPDFSink *sink = info;
    if (sink->failed || !sink->check(sink->owner) || count > 256u*1024u*1024u - sink->data.length) { sink->failed=true;return 0; }
    [sink->data appendBytes:bytes length:count];return count;
}
typedef struct { CGFloat width, ascent, descent; } YuPDFImageMetrics;
static void image_metrics_free(void *p) {free(p);}
static CGFloat image_ascent(void *p) {return ((YuPDFImageMetrics *)p)->ascent;}
static CGFloat image_descent(void *p) {return ((YuPDFImageMetrics *)p)->descent;}
static CGFloat image_width(void *p) {return ((YuPDFImageMetrics *)p)->width;}

@interface YuPDFComposer : NSObject
@property NSDictionary *packet;
@property NSMutableArray *images;
@property NSMutableSet *destinations;
@property NSString *failure;
@property CGContextRef context;
@property YuPDFCheckpoint check;
@property void *owner;
@property CGFloat width, height, margin, bottom, y;
@property NSUInteger pages, maxPages;
@property BOOL numbers;
@property BOOL recording;
@property NSMutableArray *pageCommands;
@end
@implementation YuPDFComposer
// Retain the original shaped CTLines and frozen images, not PDF-page redraws:
// Quartz/PDFKit page replay can lose ToUnicode mappings on system PDF output.
- (void)emit:(void (^)(CGContextRef))draw {
    draw(_context);
    if (_recording) [_pageCommands.lastObject addObject:[draw copy]];
}
- (void)fill:(CGRect)rect r:(CGFloat)r g:(CGFloat)g b:(CGFloat)b {
    [self emit:^(CGContextRef c){CGContextSetRGBFillColor(c,r,g,b,1);CGContextFillRect(c,rect);}];
}
- (void)border:(CGRect)rect {
    [self emit:^(CGContextRef c){CGContextSetRGBStrokeColor(c,0.65,0.65,0.65,1);CGContextSetLineWidth(c,0.5);CGContextStrokeRect(c,rect);}];
}
- (void)strike:(CGRect)box height:(CGFloat)h {
    [self emit:^(CGContextRef c){CGContextSetRGBStrokeColor(c,0,0,0,1);CGContextSetLineWidth(c,0.6);CGContextMoveToPoint(c,box.origin.x,box.origin.y+h*0.55);CGContextAddLineToPoint(c,CGRectGetMaxX(box),box.origin.y+h*0.55);CGContextStrokePath(c);}];
}
- (void)textLine:(CTLineRef)line x:(CGFloat)x y:(CGFloat)y {
    id retained=(__bridge id)line;
    [self emit:^(CGContextRef c){CGContextSetTextMatrix(c,CGAffineTransformIdentity);CGContextSetTextPosition(c,x,y);CTLineDraw((__bridge CTLineRef)retained,c);}];
}
- (void)picture:(NSImage *)image rect:(CGRect)box {
    [self emit:^(CGContextRef c){
        [NSGraphicsContext saveGraphicsState];NSGraphicsContext.currentContext=[NSGraphicsContext graphicsContextWithCGContext:c flipped:NO];
        @try{[image drawInRect:box fromRect:NSZeroRect operation:NSCompositingOperationSourceOver fraction:1 respectFlipped:NO hints:nil];}
        @finally{[NSGraphicsContext restoreGraphicsState];}
    }];
}

- (BOOL)checkpoint {if (!_check(_owner)) {_failure=@"PDF 已取消或超过准备时间预算";return NO;}return YES;}
- (BOOL)beginPage {
    if (![self checkpoint]) return NO;
    if (_pages >= _maxPages) {_failure=@"PDF 超过 1000 页预算";return NO;}
    if (_pages) CGPDFContextEndPage(_context);
    CGPDFContextBeginPage(_context,NULL);_pages++;_y=_margin;
    if(_recording){if(!_pageCommands)_pageCommands=NSMutableArray.array;[_pageCommands addObject:NSMutableArray.array];}
    [self fill:CGRectMake(0,0,_width,_height) r:1 g:1 b:1];
    if (_numbers) {
        NSAttributedString *text=[[NSAttributedString alloc] initWithString:[NSString stringWithFormat:@"%lu",(unsigned long)_pages]
            attributes:@{NSFontAttributeName:[NSFont systemFontOfSize:9],NSForegroundColorAttributeName:NSColor.grayColor}];
        CTLineRef line=CTLineCreateWithAttributedString((__bridge CFAttributedStringRef)text);
        double width=CTLineGetTypographicBounds(line,NULL,NULL,NULL);
        [self textLine:line x:(_width-width)/2 y:_margin/2];CFRelease(line);
    }
    return YES;
}
- (NSImage *)image:(NSUInteger)index {
    if (index>=_images.count) {_failure=@"PDF 图片索引无效";return nil;}
    id cached=_images[index];if (cached!=NSNull.null) return cached;
    NSDictionary *resource=_packet[@"images"][index];NSString *uri=resource[@"uri"];
    NSRange separator=[uri rangeOfString:@","];
    if (separator.location==NSNotFound) {_failure=@"PDF 图片数据无效";return nil;}
    NSData *bytes=[[NSData alloc] initWithBase64EncodedString:[uri substringFromIndex:separator.location+1] options:0];
    NSImage *image=[[NSImage alloc] initWithData:bytes];
    if (!image || !isfinite(image.size.width) || !isfinite(image.size.height) || image.size.width<=0 || image.size.height<=0) {
        _failure=@"PDF 无法绘制当前图片或 SVG；没有使用旧图";return nil;
    }
    image.cacheMode=NSImageCacheNever;_images[index]=image;return image;
}
- (NSDictionary *)layout:(NSArray *)runs width:(CGFloat)available style:(NSString *)kind {
    if (![self checkpoint]) return nil;
    CGFloat base=[kind isEqual:@"code"]?10.5:12;
    BOOL heading=[kind hasPrefix:@"h"] && kind.length==2;
    if (heading) {NSUInteger level=[[kind substringFromIndex:1] integerValue];base=level==1?22:level==2?17:14;}
    NSMutableAttributedString *text=[[NSMutableAttributedString alloc] initWithString:@""];
    NSMutableArray *pictures=NSMutableArray.array,*anchors=NSMutableArray.array;
    for (NSDictionary *run in runs) {
        NSUInteger offset=text.length;
        if ([run[@"anchor"] isKindOfClass:NSString.class]) [anchors addObject:@{@"offset":@(offset),@"id":run[@"anchor"]}];
        NSString *value=run[@"text"]?:@"";
        CGFloat factor=run[@"scale"]?[run[@"scale"] doubleValue]:1;
        BOOL bold=heading||[run[@"bold"] boolValue],mono=[kind isEqual:@"code"]||[run[@"mono"] boolValue];
        NSFont *font=mono?[NSFont monospacedSystemFontOfSize:base*factor weight:bold?NSFontWeightBold:NSFontWeightRegular]:[NSFont systemFontOfSize:base*factor weight:bold?NSFontWeightBold:NSFontWeightRegular];
        if ([run[@"italic"] boolValue]) {
            CTFontRef italic=CTFontCreateCopyWithSymbolicTraits((__bridge CTFontRef)font,0,NULL,kCTFontItalicTrait,kCTFontItalicTrait);
            if(italic)font=CFBridgingRelease(italic);
        }
        NSMutableDictionary *attrs=[@{NSFontAttributeName:font,NSForegroundColorAttributeName:NSColor.blackColor,
            NSBaselineOffsetAttributeName:run[@"rise"]?:@0} mutableCopy];
        NSString *link=[run[@"link"] isKindOfClass:NSString.class]?run[@"link"]:nil;
        if (link) {attrs[@"yu-link"]=link;attrs[NSForegroundColorAttributeName]=[NSColor colorWithSRGBRed:0.12 green:0.28 blue:0.58 alpha:1];attrs[NSUnderlineStyleAttributeName]=@1;}
        if ([run[@"underline"] boolValue]) attrs[NSUnderlineStyleAttributeName]=@1;
        if ([run[@"strike"] boolValue]) attrs[@"yu-strike"]=@YES;
        if ([run[@"highlight"] boolValue]) attrs[@"yu-highlight"]=@YES;
        if (run[@"image"]) {
            NSUInteger index=[run[@"image"] unsignedIntegerValue];NSImage *image=[self image:index];if (!image)return nil;
            NSDictionary *resource=_packet[@"images"][index];
            BOOL hasWidth=[resource[@"width"] isKindOfClass:NSNumber.class],hasHeight=[resource[@"height"] isKindOfClass:NSNumber.class];
            CGFloat w=hasWidth?[resource[@"width"] doubleValue]*0.75:image.size.width*0.75;
            CGFloat h=hasHeight?[resource[@"height"] doubleValue]*0.75:image.size.height*0.75;
            if([resource[@"widthPercent"] isKindOfClass:NSNumber.class]){w=available*[resource[@"widthPercent"] doubleValue]/100;hasWidth=YES;}
            if([resource[@"heightPercent"] isKindOfClass:NSNumber.class]){h=(_bottom-_margin)*[resource[@"heightPercent"] doubleValue]/100;hasHeight=YES;}
            if(hasWidth&&!hasHeight)h=w*image.size.height/image.size.width;
            if(hasHeight&&!hasWidth)w=h*image.size.width/image.size.height;
            // A CSS logical px maps to 0.75 pt; text is the same 16px/12pt paper style.
            CGFloat scale=MIN(1,MIN(available/w,(_bottom-_margin-8)/h));
            if (!isfinite(scale)||scale<[_packet[@"minFigureScale"] doubleValue]) {_failure=@"图片或公式需要缩小至 25% 以下，请调整纸张方向或页边距";return nil;}
            w*=scale;h*=scale;CGFloat descent=MIN(h,[resource[@"descent"] doubleValue]*0.75*scale);
            YuPDFImageMetrics *metrics=malloc(sizeof(*metrics));if(!metrics){_failure=@"无法分配图片度量";return nil;}
            *metrics=(YuPDFImageMetrics){w,h-descent,descent};
            CTRunDelegateCallbacks callbacks={kCTRunDelegateCurrentVersion,image_metrics_free,image_ascent,image_descent,image_width};
            CTRunDelegateRef delegate=CTRunDelegateCreate(&callbacks,metrics);
            if(!delegate){free(metrics);_failure=@"无法创建图片排版度量";return nil;}
            attrs[(__bridge NSString *)kCTRunDelegateAttributeName]=(__bridge id)delegate;
            attrs[NSForegroundColorAttributeName]=NSColor.clearColor;value=@"\uFFFC";
            [pictures addObject:@{@"offset":@(offset),@"index":@(index),@"width":@(w),@"height":@(h),@"descent":@(descent)}];
            [text appendAttributedString:[[NSAttributedString alloc] initWithString:value attributes:attrs]];CFRelease(delegate);
        } else if (value.length) [text appendAttributedString:[[NSAttributedString alloc] initWithString:value attributes:attrs]];
    }
    NSMutableArray *lines=NSMutableArray.array;CGFloat total=0;
    CTTypesetterRef setter=CTTypesetterCreateWithAttributedString((__bridge CFAttributedStringRef)text);
    NSUInteger offset=0;
    while(offset<text.length) {
        if (![self checkpoint]) {CFRelease(setter);return nil;}
        CFIndex count=CTTypesetterSuggestLineBreak(setter,(CFIndex)offset,available);
        if(count<=0){CFRelease(setter);_failure=@"PDF 无法在当前宽度放入完整字形，请调整纸张";return nil;}
        CTLineRef line=CTTypesetterCreateLine(setter,CFRangeMake((CFIndex)offset,count));
        CGFloat ascent=0,descent=0,leading=0;double used=CTLineGetTypographicBounds(line,&ascent,&descent,&leading);
        CGFloat height=MAX(base*1.4,ascent+descent+MAX(leading,2));
        if (!isfinite(height)||height>_bottom-_margin) {CFRelease(line);CFRelease(setter);_failure=@"单行内容无法完整放入 PDF 页面";return nil;}
        [lines addObject:@{@"line":(__bridge id)line,@"start":@(offset),@"length":@(count),@"ascent":@(ascent),@"height":@(height),@"width":@(used)}];CFRelease(line);
        total+=height;offset+=(NSUInteger)count;
    }
    CFRelease(setter);
    if(!lines.count) total=base*1.4;
    return @{@"text":text,@"lines":lines,@"pictures":pictures,@"anchors":anchors,@"height":@(total)};
}
- (void)anchor:(NSString *)name x:(CGFloat)x y:(CGFloat)y {
    if ([_destinations containsObject:name])return;
    [_destinations addObject:name];CGPDFContextAddDestinationAtPoint(_context,(__bridge CFStringRef)name,CGPointMake(x,_height-y));
}
- (BOOL)drawLine:(NSDictionary *)line layout:(NSDictionary *)layout x:(CGFloat)x y:(CGFloat)y align:(NSString *)align available:(CGFloat)available {
    if (![self checkpoint]) return NO;
    CTLineRef native=(__bridge CTLineRef)line[@"line"];
    CGFloat h=[line[@"height"] doubleValue],ascent=[line[@"ascent"] doubleValue];
    CGFloat spare=MAX(0,available-[line[@"width"] doubleValue]);
    if([align isEqual:@"center"])x+=spare/2;else if([align isEqual:@"right"])x+=spare;
    NSRange range=NSMakeRange([line[@"start"] unsignedIntegerValue],[line[@"length"] unsignedIntegerValue]);
    NSAttributedString *text=layout[@"text"];
    [text enumerateAttributesInRange:range options:0 usingBlock:^(NSDictionary *attrs,NSRange part,BOOL *stop){
        CGFloat start=CTLineGetOffsetForStringIndex(native,part.location,NULL),end=CTLineGetOffsetForStringIndex(native,NSMaxRange(part),NULL);
        CGRect box=CGRectMake(x+MIN(start,end),self.height-y-h,MAX(1,fabs(end-start)),h);
        if([attrs[@"yu-highlight"] boolValue])[self fill:box r:1 g:0.95 b:0.65];
        if([attrs[@"yu-strike"] boolValue])[self strike:box height:h];
        NSString *link=attrs[@"yu-link"];
        if([link hasPrefix:@"#"])CGPDFContextSetDestinationForRect(self.context,(__bridge CFStringRef)[link substringFromIndex:1],box);
        else if([link hasPrefix:@"https:"]||[link hasPrefix:@"http:"]||[link hasPrefix:@"mailto:"]){NSURL *url=[NSURL URLWithString:link];if(url)CGPDFContextSetURLForRect(self.context,(__bridge CFURLRef)url,box);}
    }];
    CGFloat baseline=_height-y-ascent;
    [self textLine:native x:x y:baseline];
    for(NSDictionary *picture in layout[@"pictures"]){NSUInteger index=[picture[@"offset"] unsignedIntegerValue];if(!NSLocationInRange(index,range))continue;
        NSImage *image=[self image:[picture[@"index"] unsignedIntegerValue]];if(!image)return NO;
        CGFloat offset=CTLineGetOffsetForStringIndex(native,index,NULL);
        CGRect box=CGRectMake(x+offset,baseline-[picture[@"descent"] doubleValue],[picture[@"width"] doubleValue],[picture[@"height"] doubleValue]);
        [self picture:image rect:box];
    }
    for(NSDictionary *anchor in layout[@"anchors"]){NSUInteger offset=[anchor[@"offset"] unsignedIntegerValue];
        if(NSLocationInRange(offset,range)||(offset==text.length&&NSMaxRange(range)==text.length))[self anchor:anchor[@"id"] x:x+CTLineGetOffsetForStringIndex(native,offset,NULL) y:y];
    }
    return YES;
}
- (BOOL)paragraph:(NSDictionary *)block {
    CGFloat x=_margin+MIN(8,[block[@"indent"] unsignedIntegerValue])*14;
    CGFloat width=_width-_margin-x;NSString *kind=block[@"style"];
    NSDictionary *layout=[self layout:block[@"runs"] width:width style:kind];if(!layout)return NO;
    CGFloat before=[kind hasPrefix:@"h"]?12:4;
    NSArray *lines=layout[@"lines"];
    CGFloat keep=lines.count?[lines[0][@"height"] doubleValue]:0;if([kind hasPrefix:@"h"])keep+=30;
    if(_y+before+keep>_bottom && ![self beginPage])return NO;_y+=before;
    if(!lines.count){for(NSDictionary *anchor in layout[@"anchors"])[self anchor:anchor[@"id"] x:x y:_y];return YES;}
    for(NSDictionary *line in lines){CGFloat h=[line[@"height"] doubleValue];if(_y+h>_bottom && ![self beginPage])return NO;
        if([kind isEqual:@"code"])[self fill:CGRectMake(x-3,_height-_y-h,width+6,h) r:0.95 g:0.95 b:0.95];
        if([kind isEqual:@"quote"])[self fill:CGRectMake(x-7,_height-_y-h,2,h) r:0.65 g:0.65 b:0.65];
        if(![self drawLine:line layout:layout x:x y:_y align:block[@"align"] available:width])return NO;_y+=h;
    }
    _y+=6;return YES;
}
- (BOOL)table:(NSDictionary *)table {
    NSUInteger columns=[table[@"columns"] unsignedIntegerValue],rows=[table[@"rows"] unsignedIntegerValue],headers=[table[@"headers"] unsignedIntegerValue];
    CGFloat width=_width-2*_margin,columnWidth=width/columns;
    if(!columns||!rows||columnWidth<24){_failure=@"PDF 表格列宽不足 24 pt，请选择横向纸张或减小页边距";return NO;}
    NSMutableArray *heights=NSMutableArray.array,*layouts=NSMutableArray.array;
    for(NSUInteger n=0;n<rows;n++)[heights addObject:@22];
    NSArray *cells=table[@"cells"];
    for(NSDictionary *cell in cells){CGFloat w=[cell[@"columns"] unsignedIntegerValue]*columnWidth-12;
        NSDictionary *layout=[self layout:cell[@"runs"] width:w style:@"body"];if(!layout)return NO;[layouts addObject:layout];
        if([cell[@"rows"] unsignedIntegerValue]==1){NSUInteger row=[cell[@"row"] unsignedIntegerValue];heights[row]=@(MAX([heights[row] doubleValue],[layout[@"height"] doubleValue]+12));}
    }
    for(NSUInteger n=0;n<cells.count;n++){NSDictionary *cell=cells[n];NSUInteger start=[cell[@"row"] unsignedIntegerValue],span=[cell[@"rows"] unsignedIntegerValue];CGFloat sum=0;
        for(NSUInteger r=start;r<start+span;r++)sum+=[heights[r] doubleValue];CGFloat need=[layouts[n][@"height"] doubleValue]+12;
        if(need>sum)heights[start+span-1]=@([heights[start+span-1] doubleValue]+need-sum);
    }
    CGFloat headerHeight=0;for(NSUInteger r=0;r<headers;r++)headerHeight+=[heights[r] doubleValue];
    BOOL (^drawBand)(NSUInteger,NSUInteger)=^BOOL(NSUInteger first,NSUInteger end){
        NSMutableArray *positions=NSMutableArray.array;CGFloat top=self.y;
        for(NSUInteger r=first;r<end;r++){[positions addObject:@(top)];top+=[heights[r] doubleValue];}
        for(NSUInteger n=0;n<cells.count;n++){NSDictionary *cell=cells[n];NSUInteger row=[cell[@"row"] unsignedIntegerValue];if(row<first||row>=end)continue;
            CGFloat x=self.margin+[cell[@"column"] unsignedIntegerValue]*columnWidth,w=[cell[@"columns"] unsignedIntegerValue]*columnWidth;
            CGFloat y=[positions[row-first] doubleValue],h=0;for(NSUInteger r=row;r<row+[cell[@"rows"] unsignedIntegerValue];r++)h+=[heights[r] doubleValue];
            CGRect box=CGRectMake(x,self.height-y-h,w,h);if([cell[@"header"] boolValue])[self fill:box r:0.94 g:0.94 b:0.94];
            [self border:box];
            CGFloat lineY=y+6;for(NSDictionary *line in layouts[n][@"lines"]){if(![self drawLine:line layout:layouts[n] x:x+6 y:lineY align:cell[@"align"] available:w-12])return NO;lineY+=[line[@"height"] doubleValue];}
        }
        self.y=top;return YES;
    };
    _y+=6;
    for(NSArray *band in table[@"bands"]){if(![self checkpoint])return NO;NSUInteger first=[band[0] unsignedIntegerValue],end=[band[1] unsignedIntegerValue];CGFloat h=0;
        for(NSUInteger r=first;r<end;r++)h+=[heights[r] doubleValue];CGFloat repeat=first>=headers?headerHeight:0;
        if(h+repeat>_bottom-_margin){_failure=@"表格完整跨行合并组无法放入单页，请调整纸张方向或页边距";return NO;}
        if(_y+h>_bottom){if(![self beginPage])return NO;if(first>=headers&&headers&&!drawBand(0,headers))return NO;}
        if(!drawBand(first,end))return NO;
    }
    _y+=10;return YES;
}
- (BOOL)render {
    _width=[_packet[@"width"] doubleValue];_height=[_packet[@"height"] doubleValue];_margin=[_packet[@"margin"] doubleValue];_numbers=[_packet[@"pageNumbers"] boolValue];
    _bottom=_height-_margin;_maxPages=[_packet[@"maxPages"] unsignedIntegerValue];_images=NSMutableArray.array;_destinations=NSMutableSet.set;
    for(NSUInteger n=0;n<[_packet[@"images"] count];n++)[_images addObject:NSNull.null];
    if(![self beginPage])return NO;
    for(NSDictionary *block in _packet[@"blocks"]){@autoreleasepool {
        if([block[@"kind"] isEqual:@"paragraph"]){if(![self paragraph:block])return NO;}
        else if([block[@"kind"] isEqual:@"table"]){if(![self table:block])return NO;}
        else {if(_y+16>_bottom&&![self beginPage])return NO;[self fill:CGRectMake(_margin,_height-_y-8,_width-2*_margin,0.5) r:0.6 g:0.6 b:0.6];_y+=16;}
    }}
    return YES;
}
@end

int yu_macos_export_pdf(const uint8_t *json,size_t length,void *owner,YuPDFCheckpoint checkpoint,void **out_bytes,size_t *out_length,uint32_t *out_pages,void **out_print_plan,char *error,size_t error_capacity) {
    if(!json||!length||!checkpoint||!out_bytes||!out_length||!out_pages||!error||!error_capacity)return 0;
    *out_bytes=NULL;*out_length=0;*out_pages=0;error[0]=0;
    if(out_print_plan)*out_print_plan=NULL;
    @autoreleasepool {
        NSDictionary *packet=[NSJSONSerialization JSONObjectWithData:[NSData dataWithBytesNoCopy:(void *)json length:length freeWhenDone:NO] options:0 error:nil];
        if(![packet isKindOfClass:NSDictionary.class])return 0;
        CGFloat w=[packet[@"width"] doubleValue],h=[packet[@"height"] doubleValue],m=[packet[@"margin"] doubleValue];
        if(!isfinite(w)||!isfinite(h)||w<500||h<500||w>900||h>900||m<18||m>144)return 0;
        YuPDFSink sink={NSMutableData.data,checkpoint,owner,false};CGDataConsumerCallbacks callbacks={pdf_write,NULL};
        CGDataConsumerRef consumer=CGDataConsumerCreate(&sink,&callbacks);CGRect media=CGRectMake(0,0,w,h);
        CGContextRef context=CGPDFContextCreate(consumer,&media,(__bridge CFDictionaryRef)@{(__bridge NSString *)kCGPDFContextTitle:packet[@"title"]?:@"Yu 文档",(__bridge NSString *)kCGPDFContextCreator:@"Yu"});CGDataConsumerRelease(consumer);
        if(!context)return 0;
        YuPDFComposer *composer=YuPDFComposer.new;composer.packet=packet;composer.context=context;composer.check=checkpoint;composer.owner=owner;composer.recording=out_print_plan!=NULL;
        BOOL okay=NO;
        @try {okay=[composer render];} @catch(NSException *exception) {composer.failure=@"原生 PDF 绘制异常；未提交输出";}
        if(composer.pages)CGPDFContextEndPage(context);CGPDFContextClose(context);CGContextRelease(context);
        if(!okay||sink.failed||!checkpoint(owner)) {
            NSString *message=composer.failure?:@"PDF 输出超限、取消或无法完成";strlcpy(error,message.UTF8String,error_capacity);return 0;
        }
        void *result=malloc(sink.data.length);if(!result){strlcpy(error,"PDF allocation failed",error_capacity);return 0;}
        memcpy(result,sink.data.bytes,sink.data.length);*out_bytes=result;*out_length=sink.data.length;*out_pages=(uint32_t)composer.pages;
        if(out_print_plan)*out_print_plan=(void *)CFBridgingRetain([composer.pageCommands copy]);
        return 1;
    }
}
void yu_macos_export_pdf_free(void *bytes){free(bytes);}
int yu_macos_print_plan_draw(void *plan,uint32_t page,void *context){
    if(!plan||!context)return 0;
    NSArray *pages=(__bridge NSArray *)plan;if(page>=pages.count)return 0;
    @autoreleasepool{@try{for(void (^draw)(CGContextRef) in pages[page])draw((CGContextRef)context);return 1;}
        @catch(NSException *e){return 0;}}
}
void yu_macos_print_plan_free(void *plan){if(plan)CFRelease(plan);}
