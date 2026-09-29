// Independent ImageIO pixel check for the fixed text-only long fixture.
// Counts actual ink bands, without OCR or trusting the exporter's layout report.
import AppKit
import ImageIO
import CryptoKit
import UniformTypeIdentifiers
let args=Array(CommandLine.arguments.dropFirst())
guard args.count==3,let expected=Int(args[2]),expected>0 else{exit(1)}
let directory=URL(fileURLWithPath:args[0]),out=URL(fileURLWithPath:args[1])
guard !FileManager.default.fileExists(atPath:out.path) else{exit(2)}
try FileManager.default.createDirectory(at:out,withIntermediateDirectories:false)
let files=try FileManager.default.contentsOfDirectory(at:directory,includingPropertiesForKeys:nil).filter{$0.pathExtension=="png"}.sorted{$0.lastPathComponent<$1.lastPathComponent}
var records:[[String:Any]]=[],total=0
func save(_ image:CGImage,_ name:String)throws{
    let url=out.appendingPathComponent(name)
    guard let target=CGImageDestinationCreateWithURL(url as CFURL,UTType.png.identifier as CFString,1,nil)else{throw NSError(domain:"PNG.Check",code:1)}
    CGImageDestinationAddImage(target,image,nil);guard CGImageDestinationFinalize(target)else{throw NSError(domain:"PNG.Check",code:2)}
}
for (index,url) in files.enumerated(){try autoreleasepool{
    let data=try Data(contentsOf:url)
    guard let src=CGImageSourceCreateWithData(data as CFData,nil),let image=CGImageSourceCreateImageAtIndex(src,0,nil),image.width*image.height<=16*1024*1024 else{exit(3)}
    let w=image.width,h=image.height
    guard let context=CGContext(data:nil,width:w,height:h,bitsPerComponent:8,bytesPerRow:w*4,space:CGColorSpace(name:CGColorSpace.sRGB)!,bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue),let ptr=context.data?.assumingMemoryBound(to:UInt8.self) else{exit(4)}
    context.draw(image,in:CGRect(x:0,y:0,width:w,height:h))
    var bands=0,lastInk = -1000,minInk=h,maxInk = -1
    for y in 0..<h {
        var ink=false
        for x in stride(from:0,to:w,by:2){let n=y*w*4+x*4;if Int(ptr[n])+Int(ptr[n+1])+Int(ptr[n+2])<600 {ink=true;break}}
        if ink {if y-lastInk>4{bands+=1};lastInk=y;minInk=min(minInk,y);maxInk=max(maxInk,y)}
    }
    total+=bands
    records.append(["file":url.lastPathComponent,"width":w,"height":h,"ink_bands":bands,"first_ink_row":minInk,"last_ink_row":maxInk,"sha256":SHA256.hash(data:data).map{String(format:"%02x",$0)}.joined()])
    let cut=min(h,260)
    if let top=image.cropping(to:CGRect(x:0,y:0,width:min(w,1100),height:cut)){try save(top,"part-\(index+1)-top.png")}
    if let bottom=image.cropping(to:CGRect(x:0,y:h-cut,width:min(w,1100),height:cut)){try save(bottom,"part-\(index+1)-bottom.png")}
}}
let report:[String:Any]=["passed":total==expected,"observed_ink_bands":total,"expected_fixed_fixture_lines":expected,"segments":records,"method":"decoded pixels; ink row bands, no OCR"]
let bytes=try JSONSerialization.data(withJSONObject:report,options:[.prettyPrinted,.sortedKeys]);try bytes.write(to:out.appendingPathComponent("report.json"));print(String(decoding:bytes,as:UTF8.self));exit(total==expected ? 0:1)
