// Read-only ImageIO observation, including actual decode and background pixels.
import AppKit
import ImageIO
import CryptoKit
let url=URL(fileURLWithPath:CommandLine.arguments[1])
let bytes=try Data(contentsOf:url)
guard let source=CGImageSourceCreateWithData(bytes as CFData,nil),CGImageSourceGetCount(source)==1,
      let image=CGImageSourceCreateImageAtIndex(source,0,nil),image.width<=32768,image.height<=32768,
      image.width*image.height<=16*1024*1024 else {exit(1)}
let space=CGColorSpace(name:CGColorSpace.sRGB)!
guard let context=CGContext(data:nil,width:1,height:1,bitsPerComponent:8,bytesPerRow:4,space:space,bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue) else {exit(2)}
context.draw(image,in:CGRect(x:0,y:0,width:image.width,height:image.height))
let pixel=context.data!.assumingMemoryBound(to:UInt8.self)
let value:[String:Any]=["width":image.width,"height":image.height,"corner_rgba":(0..<4).map{Int(pixel[$0])},
    "sha256":SHA256.hash(data:bytes).map{String(format:"%02x",$0)}.joined(),"bytes":bytes.count,"decoded":true]
let data=try JSONSerialization.data(withJSONObject:value,options:[.prettyPrinted,.sortedKeys])
print(String(decoding:data,as:UTF8.self))
