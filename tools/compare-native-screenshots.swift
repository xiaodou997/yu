// Compare already captured images without resizing or aligning away layout errors.
// Usage: swift tools/compare-native-screenshots.swift reference.png actual.png output-directory
import Foundation
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers

struct ComparisonFailure: Error { let message: String }
func pixels(_ path: String) throws -> (Int, Int, [UInt8]) {
    guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil),
          let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else {
        throw ComparisonFailure(message: "Cannot decode image: \(path)")
    }
    let width = image.width, height = image.height
    var bytes = [UInt8](repeating: 0, count: width * height * 4)
    let rendered = bytes.withUnsafeMutableBytes { raw -> Bool in
        guard let context = CGContext(data: raw.baseAddress, width: width, height: height,
            bitsPerComponent: 8, bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        return true
    }
    guard rendered else { throw ComparisonFailure(message: "Cannot rasterize image") }
    var colors = Set<UInt32>()
    for index in stride(from: 0, to: bytes.count, by: 4) {
        let red = UInt32(bytes[index]) << 16
        let green = UInt32(bytes[index + 1]) << 8
        colors.insert(red | green | UInt32(bytes[index + 2]))
        if colors.count > 16 { break }
    }
    guard colors.count > 16 else { throw ComparisonFailure(message: "Blank or insufficient-content screenshot: \(path)") }
    return (width, height, bytes)
}
let args = CommandLine.arguments
if args.count != 4 {
    fputs("Usage: compare-native-screenshots.swift reference.png actual.png output-directory\n", stderr)
    exit(2)
}
let output = URL(fileURLWithPath: args[3], isDirectory: true)
try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
var report: [String: Any] = ["reference": args[1], "actual": args[2], "visual_parity_passed": false,
    "note": "Pixel differences do not certify wrapping, geometry, input behavior or antialiasing equivalence."]
var status: Int32 = 0
do {
    let (width, height, reference) = try pixels(args[1])
    let (actualWidth, actualHeight, actual) = try pixels(args[2])
    report["reference_pixels"] = [width, height]
    report["actual_pixels"] = [actualWidth, actualHeight]
    guard width == actualWidth, height == actualHeight else {
        throw ComparisonFailure(message: "Dimension mismatch; normalize capture conditions, not screenshot pixels")
    }
    var difference = [UInt8](repeating: 255, count: reference.count)
    var changed = 0, lowMagnitude = 0, totalError: Int64 = 0
    for index in stride(from: 0, to: reference.count, by: 4) {
        var maximum = 0
        for channel in 0..<3 {
            let delta = abs(Int(reference[index + channel]) - Int(actual[index + channel]))
            difference[index + channel] = UInt8(delta)
            maximum = max(maximum, delta)
            totalError += Int64(delta)
        }
        if maximum > 0 { changed += 1 }
        if maximum > 0 && maximum <= 8 { lowMagnitude += 1 }
    }
    let data = Data(difference) as CFData
    guard let provider = CGDataProvider(data: data), let image = CGImage(width: width, height: height,
        bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
        space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
        provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent),
        let destination = CGImageDestinationCreateWithURL(output.appendingPathComponent("difference.png") as CFURL,
            UTType.png.identifier as CFString, 1, nil) else { throw ComparisonFailure(message: "Cannot encode difference") }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination) else { throw ComparisonFailure(message: "Cannot write difference") }
    report["status"] = "compared-requires-geometry-review"
    report["changed_pixels"] = changed
    report["low_magnitude_pixels_1_to_8"] = lowMagnitude
    report["mean_channel_error"] = Double(totalError) / Double(width * height * 3)
} catch {
    report["status"] = "rejected"
    report["reason"] = String(describing: error)
    status = 1
}
try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
    .write(to: output.appendingPathComponent("comparison.json"), options: .atomic)
exit(status)
