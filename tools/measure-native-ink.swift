// Read-only screenshot diagnostics. Ink bands are not line boxes or a parity verdict.
import Foundation
import CoreGraphics
import ImageIO

func fail(_ message: String) -> Never {
    fputs(message + "\n", stderr)
    exit(2)
}
let args = CommandLine.arguments
guard args.count == 6 || args.count == 8 else {
    fail("image scale x-min-pt x-max-pt output.json [y-min-pt y-max-pt]")
}
guard let scale = Double(args[2]), scale.isFinite, scale > 0,
      let x0 = Double(args[3]), x0.isFinite,
      let x1 = Double(args[4]), x1.isFinite,
      let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: args[1]) as CFURL, nil),
      let img = CGImageSourceCreateImageAtIndex(source, 0, nil) else {
    fail("Invalid scale, horizontal bounds, or image")
}
let w = img.width, h = img.height
let y0 = args.count == 8 ? Double(args[6]) ?? .nan : 40
let y1 = args.count == 8 ? Double(args[7]) ?? .nan : Double(h) / scale
guard y0.isFinite, y1.isFinite, x0 >= 0, x1 > x0, x1 <= Double(w) / scale,
      y0 >= 0, y1 > y0, y1 <= Double(h) / scale else { fail("ROI outside image or empty") }
let left = Int(ceil(x0 * scale)), right = Int(floor(x1 * scale))
let top = Int(ceil(y0 * scale)), bottom = Int(floor(y1 * scale))
guard left < right, top < bottom else { fail("ROI contains no complete pixels") }
var data = [UInt8](repeating: 0, count: w*h*4)
data.withUnsafeMutableBytes { ptr in
    guard let c = CGContext(data: ptr.baseAddress, width: w, height: h,
                            bitsPerComponent: 8, bytesPerRow: w*4,
                            space: CGColorSpaceCreateDeviceRGB(),
                            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else {
        fail("Unable to decode image")
    }
    c.draw(img, in: CGRect(x: 0, y: 0, width: w, height: h))
}
// The most frequent sampled RGB in the supplied text ROI is its background.
// Sampling within the ROI avoids using a differently colored sidebar as canvas.
var histogram: [Int: Int] = [:]
for y in stride(from: top, to: bottom, by: 4) {
    for x in stride(from: left, to: right, by: 4) {
        let i = (y*w+x)*4
        let rgb = Int(data[i]) << 16 | Int(data[i+1]) << 8 | Int(data[i+2])
        histogram[rgb, default: 0] += 1
    }
}
let color = histogram.max { a, b in a.value == b.value ? a.key < b.key : a.value < b.value }!.key
let bg = (color >> 16, (color >> 8) & 255, color & 255)
var bands: [[String: Any]] = [], start: Int?
func appendBand(_ s: Int, _ end: Int) {
    if end-s >= 2 {
        bands.append(["top": Double(s)/scale, "bottom": Double(end)/scale,
                      "clipped": s == top || end == bottom])
    }
}
for y in top..<bottom {
    var count = 0
    for x in left..<right {
        let i = (y*w+x)*4
        let d = max(abs(Int(data[i])-bg.0), abs(Int(data[i+1])-bg.1), abs(Int(data[i+2])-bg.2))
        if d > 70 { count += 1 }
    }
    if count >= 12 {
        if start == nil { start = y }
    } else if let s = start {
        appendBand(s, y)
        start = nil
    }
}
if let s = start { appendBand(s, bottom) }
let report: [String: Any] = [
    "image": args[1], "scale": scale, "roi_pt": [x0, y0, x1, y1],
    "sampled_background_rgb": [bg.0, bg.1, bg.2], "bands": bands,
    "note": "Ink diagnostics for text-dominant ROI; not line boxes or geometry acceptance. Clipped bands are incomplete."
]
try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
    .write(to: URL(fileURLWithPath: args[5]))
