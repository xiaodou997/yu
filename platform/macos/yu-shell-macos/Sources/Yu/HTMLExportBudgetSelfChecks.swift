import AppKit
import CryptoKit
import ImageIO

/// Actual production ImageIO/task boundaries using this run's generated inputs.
/// Budget guards in Rust are tested separately; this is not a GUI interaction.
@MainActor
func runHTMLExportBudgetSelfCheck(input: String, output: String) -> Never {
    let inputs = URL(fileURLWithPath: input, isDirectory: true)
    let root = URL(fileURLWithPath: output, isDirectory: true)
    let fm = FileManager.default
    var rows: [[String: Any]] = [], ownsRoot = false
    func require(_ condition: Bool, _ message: String) throws {
        if !condition { throw NSError(domain: "Yu.Export.Budget", code: 1, userInfo: [NSLocalizedDescriptionKey: message]) }
    }
    func sha(_ data: Data) -> String { SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined() }
    func report(_ passed: Bool, _ error: String? = nil) {
        var result: [String: Any] = ["passed": passed, "evidence_layer": "native", "cases": rows]
        if let error { result["error"] = error }
        if let data = try? JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys]) {
            if ownsRoot { try? data.write(to: root.appendingPathComponent("report.json"), options: .atomic) }
            print(String(decoding: data, as: UTF8.self))
        }
    }
    do {
        try require(!fm.fileExists(atPath: root.path), "Budget output directory already exists")
        try fm.createDirectory(at: root, withIntermediateDirectories: false); ownsRoot = true
        for (name, imageName, expected, message) in [
            ("pixels-below", "pixels-below.png", "completed", ""),
            ("pixels-exact", "pixels-exact.png", "completed", ""),
            ("pixels-over", "pixels-over.png", "failed", "32 Mi"),
            ("svg-bytes-over", "svg-bytes-over.svg", "failed", "4 MiB"),
            ("svg-nodes-over", "svg-nodes-over.svg", "failed", "100000")
        ] {
            let image = inputs.appendingPathComponent(imageName)
            let originalImage = try Data(contentsOf: image)
            let source = "# Budget \(name)\n\n![boundary](\(image.path))\n\nBUDGET-END\n"
            let file = root.appendingPathComponent(name + ".md")
            let target = root.appendingPathComponent(name + ".html")
            let original = Data("OLD-OUTPUT-KEEP".utf8)
            try Data(source.utf8).write(to: file, options: .withoutOverwriting)
            try original.write(to: target, options: .withoutOverwriting)
            let bridge = try StorageBridge(path: file.path)
            let before = bridge.state
            try bridge.setSelection(NSRange(location: 2, length: 3))
            let task = try bridge.beginHTMLExport(to: target, config: ["title": name, "fontSize": 16,
                "width": 800, "referenceDay": 20724, "replaceExisting": true])
            defer { task.cancel() }
            var final: NativeHTMLExportTask.Status?, committed = false
            let deadline = Date().addingTimeInterval(90)
            while Date() < deadline {
                let status = try task.status()
                if ["failed", "cancelled", "completed", "completed_with_warnings", "warnings"].contains(status.phase) {
                    final = status; break
                }
                if status.phase == "ready", !committed { try task.commit(allowWarnings: false); committed = true }
                Thread.sleep(forTimeInterval: 0.005)
            }
            guard let final else { throw NSError(domain: "Yu.Export.Budget", code: 2, userInfo: [NSLocalizedDescriptionKey: "Budget task timeout: \(name)"]) }
            try require(final.phase == expected, "\(name): expected \(expected), got \(final.phase): \(final.message)")
            if !message.isEmpty { try require(final.message.contains(message), "Wrong budget diagnostic: \(final.message)") }
            try require(bridge.source == source && bridge.state.revision == before.revision && bridge.state.dirty == before.dirty, "Document changed")
            try require(bridge.selection.range == NSRange(location: 2, length: 3), "Selection changed")
            try require(try Data(contentsOf: image) == originalImage, "Original image changed")
            try require(try Data(contentsOf: file) == Data(source.utf8), "Disk source changed")
            let bytes = try Data(contentsOf: target)
            var row: [String: Any] = ["id": name, "passed": true, "phase": final.phase,
                "message": final.message, "output_sha256": sha(bytes), "input_sha256": sha(originalImage), "source_selection_unchanged": true]
            if expected == "failed" { try require(bytes == original, "Rejected budget changed old output") }
            else {
                let html = String(decoding: bytes, as: UTF8.self)
                try require(html.contains("BUDGET-END") && final.images == 1 && final.warnings.isEmpty, "Incomplete image output")
                guard let start = html.range(of: "data:image/png;base64,"),
                      let end = html[start.upperBound...].firstIndex(of: "\""),
                      let png = Data(base64Encoded: String(html[start.upperBound..<end])),
                      let imageSource = CGImageSourceCreateWithData(png as CFData, nil),
                      let properties = CGImageSourceCopyPropertiesAtIndex(imageSource, 0, nil) as? [CFString: Any],
                      let width = properties[kCGImagePropertyPixelWidth] as? NSNumber,
                      let height = properties[kCGImagePropertyPixelHeight] as? NSNumber else {
                    throw NSError(domain: "Yu.Export.Budget", code: 3)
                }
                let expectedHeight = name == "pixels-exact" ? 4096 : 4095
                try require(width.intValue == 8192 && height.intValue == expectedHeight, "Normalization changed boundary image dimensions")
                row["width"] = width; row["height"] = height
            }
            rows.append(row)
            try require(!(try fm.contentsOfDirectory(atPath: root.path)).contains(where: { $0.hasPrefix(".yu-export-") }), "Temporary files remain")
        }
        report(true); exit(0)
    } catch { report(false, error.localizedDescription); exit(1) }
}
