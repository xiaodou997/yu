// Locate a known test phrase in actual rendered pixels, not Yu layout geometry.
import Foundation
import ImageIO
import Vision
let args = Array(CommandLine.arguments.dropFirst())
guard (args.count == 2 || (args.count == 3 && args[2] == "--accurate")),
      let source = CGImageSourceCreateWithURL(URL(fileURLWithPath:args[0]) as CFURL,nil),
      let image = CGImageSourceCreateImageAtIndex(source,0,nil) else {
    fputs("Usage: locate-native-text PNG phrase [--accurate]\n",stderr); exit(1)
}
func locate(_ level: VNRequestTextRecognitionLevel) throws -> [[String: Any]] {
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = level
    request.usesLanguageCorrection = false
    request.recognitionLanguages = ["en-US"]
    try VNImageRequestHandler(cgImage: image, options: [:]).perform([request])
    var matches = [[String: Any]]()
    for observation in request.results ?? [] {
        guard let text = observation.topCandidates(1).first,
              let range = text.string.range(of: args[1]),
              let box = try text.boundingBox(for: range)?.boundingBox else { continue }
        matches.append(["text": text.string, "x": box.minX * Double(image.width),
            "y": (1 - box.maxY) * Double(image.height), "width": box.width * Double(image.width),
            "height": box.height * Double(image.height), "confidence": text.confidence])
    }
    return matches
}
// Small math labels can be missed by the fast recognizer. Retry the same exact
// phrase with accurate recognition; never substitute a layout-derived position.
var matches: [[String: Any]]
var recognition = args.count == 3 ? "accurate" : "fast"
do {
    matches = try locate(args.count == 3 ? .accurate : .fast)
    if matches.isEmpty && args.count == 2 {
        recognition = "accurate"
        matches = try locate(.accurate)
    }
} catch {
    fputs("Screenshot OCR failed: \(error)\n", stderr)
    exit(2)
}
let data = try JSONSerialization.data(withJSONObject: ["image_width": image.width, "image_height": image.height,
    "recognition": recognition, "matches": matches], options: [.prettyPrinted, .sortedKeys])
print(String(data: data, encoding: .utf8)!)
