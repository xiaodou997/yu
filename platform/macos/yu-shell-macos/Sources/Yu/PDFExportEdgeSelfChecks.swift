import AppKit
import PDFKit
import CryptoKit
import Darwin
import YuStorageFFI

/// Integrated production PDF refusals. Every file is owned by this invocation.
@MainActor
func runPDFExportEdgeSelfCheck(directory: String) -> Never {
    let root = URL(fileURLWithPath: directory, isDirectory: true), fm = FileManager.default
    var rows: [[String: Any]] = [], ownsRoot = false
    func require(_ condition: Bool, _ message: String) throws {
        if !condition { throw NSError(domain: "Yu.PDF.Edges", code: 1, userInfo: [NSLocalizedDescriptionKey: message]) }
    }
    func save(_ passed: Bool, error: String? = nil) {
        var report: [String: Any] = ["passed": passed, "evidence_layer": "native", "cases": rows]
        if let error { report["error"] = error }
        if let data = try? JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys]) {
            if ownsRoot { try? data.write(to: root.appendingPathComponent("report.json")) }
            print(String(decoding: data, as: UTF8.self))
        }
    }
    do {
        try require(!fm.fileExists(atPath: root.path), "Refusing existing edge directory")
        try fm.createDirectory(at: root, withIntermediateDirectories: false); ownsRoot = true
        let png = Data(base64Encoded: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a5WQAAAAASUVORK5CYII=")!
        let modes = ["figure-exact", "figure-over", "columns-exact", "columns-over", "merged-group-over", "source-alias", "image-alias", "untitled-no-base"]
        for mode in modes {
            let work = root.appendingPathComponent(mode, isDirectory: true)
            try fm.createDirectory(at: work, withIntermediateDirectories: false)
            let input = work.appendingPathComponent("source.md"), target = work.appendingPathComponent("output.pdf"), image = work.appendingPathComponent("image.png")
            try png.write(to: image, options: .withoutOverwriting)
            var source = "# PDF 边界\n\nEDGE-END\n"
            var config: [String: Any] = ["exportFormat": "pdf", "title": "PDF Edges", "paper": "Letter", "margin": 36, "pageNumbers": false, "referenceDay": 20724, "replaceExisting": true]
            if mode.hasPrefix("figure-") { source += "\n<img src=\"image.png\" width=\"\(mode == "figure-exact" ? 2880 : 2881)\">\n" }
            if mode.hasPrefix("columns-") {
                config["margin"] = 18
                let count = mode == "columns-exact" ? 24 : 25
                source += "\n<table><tr>" + String(repeating: "<td>a</td>", count: count) + "</tr></table>\n"
            }
            if mode == "merged-group-over" {
                source += "\n<table><tr><td rowspan=\"40\">MERGED</td><td>ROW-0</td></tr>" + (1..<40).map { "<tr><td>ROW-\($0)</td></tr>" }.joined() + "</table>\n"
            }
            if ["image-alias", "untitled-no-base"].contains(mode) { source += "\n![本地](image.png)\n" }
            if mode == "untitled-no-base" { config["untitled"] = true }
            let original = Data(source.utf8)
            try original.write(to: input, options: .withoutOverwriting)
            if mode == "source-alias" { try fm.linkItem(at: input, to: target) }
            else if mode == "image-alias" { try fm.linkItem(at: image, to: target) }
            else { try Data("OLD-PDF-KEEP".utf8).write(to: target, options: .withoutOverwriting) }
            let old = try Data(contentsOf: target)
            let bridge = try StorageBridge(path: input.path)
            try bridge.setSelectionEndpoints(anchorUTF16: 6, focusUTF16: 1, affinity: 0)
            let revision = bridge.revision, state = bridge.state, selection = bridge.selectionEndpoints
            let task = try bridge.beginHTMLExport(to: target, config: config)
            defer { task.cancel() }
            var final: NativeHTMLExportTask.Status?, committed = false
            let deadline = Date().addingTimeInterval(60)
            while Date() < deadline {
                let status = try task.status()
                if ["completed", "completed_with_warnings", "cancelled", "failed"].contains(status.phase) { final = status; break }
                if status.phase == "ready", !committed { try task.commit(allowWarnings: false); committed = true }
                try require(status.phase != "warnings", "Budget or valid content must not become an ordinary warning")
                Thread.sleep(forTimeInterval: 0.005)
            }
            guard let final else { throw NSError(domain: "Yu.PDF.Edges", code: 2, userInfo: [NSLocalizedDescriptionKey: "PDF edge timed out: \(mode)"]) }
            let success = mode.hasSuffix("exact")
            try require(final.phase == (success ? "completed" : "failed"), "\(mode): \(final.phase) \(final.message)")
            if success {
                guard let pdf = PDFDocument(url: target) else { throw NSError(domain: "Yu.PDF.Edges", code: 3) }
                try require(pdf.pageCount > 0 && (pdf.string ?? "").contains("EDGE-END"), "Boundary output lost content")
            } else { try require(try Data(contentsOf: target) == old, "Failure overwrote target: \(mode)") }
            let messageNeedles = ["figure-over": "25%", "columns-over": "24 pt", "merged-group-over": "合并组"]
            if let needle = messageNeedles[mode] { try require(final.message.contains(needle), "Unexpected rejection: \(final.message)") }
            try require(bridge.source == source && bridge.revision == revision && bridge.state.dirty == state.dirty, "Export changed source identity")
            try require(bridge.selectionEndpoints.anchorUTF16 == selection.anchorUTF16 && bridge.selectionEndpoints.focusUTF16 == selection.focusUTF16, "Export changed reversed selection")
            try require(try Data(contentsOf: input) == original && Data(contentsOf: image) == png, "Export changed input or resource")
            try require(!(try fm.contentsOfDirectory(atPath: work.path)).contains(where: { $0.hasPrefix(".yu-export-") }), "Temporary file remained")
            rows.append(["id": mode, "passed": true, "phase": final.phase, "message": final.message, "source_selection_resources_preserved": true])
        }
        save(true); exit(0)
    } catch { save(false, error: error.localizedDescription); exit(1) }
}
