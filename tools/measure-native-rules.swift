// Detect visible vertical rules from a screenshot ROI, independently of app geometry.
import Foundation
import CoreGraphics
import ImageIO
let args = Array(CommandLine.arguments.dropFirst())
guard args.count == 5, let source = CGImageSourceCreateWithURL(URL(fileURLWithPath:args[0]) as CFURL,nil),
      let image = CGImageSourceCreateImageAtIndex(source,0,nil),
      let left=Int(args[1]), let top=Int(args[2]), let right=Int(args[3]), let bottom=Int(args[4]),
      left >= 3, top >= 0, right < image.width-3, bottom <= image.height, left < right, top < bottom else {
    fputs("Usage: measure-native-rules PNG left top right bottom (pixels)\n",stderr); exit(1)
}
let width=image.width, height=image.height
var pixels=[UInt8](repeating:0,count:width*height*4)
pixels.withUnsafeMutableBytes { buffer in
    let context=CGContext(data:buffer.baseAddress,width:width,height:height,bitsPerComponent:8,bytesPerRow:width*4,
        space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue)!
    context.draw(image,in:CGRect(x:0,y:0,width:width,height:height))
}
func difference(_ x:Int,_ other:Int,_ y:Int)->Int {
    (0..<3).map { abs(Int(pixels[(y*width+x)*4+$0])-Int(pixels[(y*width+other)*4+$0])) }.max()!
}
var columns=[Int]()
for x in left..<right {
    var count=0
    for y in top..<bottom {
        if difference(x,x-3,y) >= 6 && difference(x,x+3,y) >= 6 { count += 1 }
    }
    if Double(count)/Double(bottom-top) > 0.7 { columns.append(x) }
}
var groups=[[Int]]()
for x in columns {
    if let last=groups.last?.last, x-last<=2 { groups[groups.count-1].append(x) }
    else { groups.append([x]) }
}
let result:[String:Any] = ["width":width,"height":height,"vertical_rules":groups.map { Double($0.reduce(0,+))/Double($0.count) }]
let data=try JSONSerialization.data(withJSONObject:result,options:[.prettyPrinted,.sortedKeys])
print(String(data:data,encoding:.utf8)!)
