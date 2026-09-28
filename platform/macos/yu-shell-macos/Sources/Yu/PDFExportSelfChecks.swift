import AppKit
import PDFKit
import CryptoKit
import YuStorageFFI

/// Uses the real snapshot/task/native backend and publishes an actual PDF.
/// PDFKit checks here are native evidence, not a substitute for an external viewer.
@MainActor
func runPDFExportSelfCheck(input: String, output: String) -> Never {
    func require(_ value: Bool, _ message: String) throws {
        if !value { throw NSError(domain: "Yu.Export.PDF.Check", code: 1, userInfo: [NSLocalizedDescriptionKey: message]) }
    }
    func sha(_ data: Data) -> String { SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined() }
    func identity(_ bridge: StorageBridge) throws -> Data {
        let state = bridge.state, endpoints = bridge.selectionEndpoints
        guard let selections = bridge.selectionsIfAvailable else { throw NSError(domain: "Yu.Export.PDF.Check", code: 2) }
        return try JSONSerialization.data(withJSONObject: ["source": bridge.source,
            "revision": state.revision, "saved": state.savedRevision, "dirty": state.dirty,
            "bom": state.bom, "anchor": endpoints.anchorUTF16, "focus": endpoints.focusUTF16,
            "primary": selections.primary, "selections": selections.ranges.map { [$0.range.location, $0.range.length, Int($0.affinity)] },
            "undo": bridge.commandAvailable(UInt8(YU_STORAGE_COMMAND_UNDO)),
            "redo": bridge.commandAvailable(UInt8(YU_STORAGE_COMMAND_REDO))], options: [.sortedKeys])
    }
    let target = URL(fileURLWithPath: output)
    let args = CommandLine.arguments
    let cancelled = args.contains("--pdf-cancel"), warnings = args.contains("--pdf-warnings")
    let landscape = args.contains("--pdf-landscape"), letter = args.contains("--pdf-letter")
    var report: [String: Any] = ["passed": false, "evidence_layer": "native", "test": "pdf-export-self-check"]
    do {
        let original = try Data(contentsOf: URL(fileURLWithPath: input))
        let previous = try? Data(contentsOf: target)
        let bridge = try StorageBridge(path: input)
        let config: [String: Any] = ["exportFormat": "pdf", "title": "Yu PDF 综合导出", "paper": letter ? "Letter" : "A4",
            "landscape": landscape, "margin": landscape ? 36 : 44, "pageNumbers": true,
            "referenceDay": 20724, "replaceExisting": previous != nil]
        try bridge.beginComposition(replacementRange: NSRange(location: 0, length: 0), preedit: "未提交", selection: NSRange(location: 3, length: 0))
        let composing = try identity(bridge)
        var rejected = false
        do { let task = try bridge.beginHTMLExport(to: target, config: config); task.cancel() }
        catch { rejected = true }
        try require(rejected && bridge.composition.active && bridge.copyComposition(bridge.composition) == "未提交", "PDF did not refuse live composition")
        try require(try identity(bridge) == composing, "Composition refusal changed source/history")
        try bridge.cancelComposition()
        try bridge.setSelection(NSRange(location: (bridge.source as NSString).length, length: 0))
        _ = try bridge.insertText("\n\nPDF-UNSAVED-SNAPSHOT 中文\n")
        let revision = bridge.revision
        let task = try bridge.beginHTMLExport(to: target, config: config)
        defer { task.cancel() }
        try bridge.setSelection(NSRange(location: 0, length: 0))
        _ = try bridge.insertText("PDF-LATER-EDIT-NOT-EXPORTED\n")
        try bridge.setSelections([NSRange(location: 0, length: 1), NSRange(location: 4, length: 2)], primary: 1)
        let before = try identity(bridge)
        if cancelled { task.cancel() }
        var terminal: NativeHTMLExportTask.Status?, committed = false
        let deadline = Date().addingTimeInterval(330)
        while Date() < deadline {
            let status = try task.status()
            if ["completed", "completed_with_warnings", "cancelled", "failed"].contains(status.phase) { terminal = status; break }
            if status.phase == "ready", !committed { try task.commit(allowWarnings: false); committed = true }
            if status.phase == "warnings", !committed {
                try require(warnings, "Unexpected PDF warnings: \(status.warnings)")
                try require((try? Data(contentsOf: target)) == previous, "Warning preparation published output")
                try task.commit(allowWarnings: true); committed = true
            }
            Thread.sleep(forTimeInterval: 0.01)
        }
        guard let terminal else { throw NSError(domain: "Yu.Export.PDF.Check", code: 3, userInfo: [NSLocalizedDescriptionKey: "PDF self-check timeout"]) }
        report["phase"] = terminal.phase; report["message"] = terminal.message
        report["warnings"] = terminal.warnings; report["images"] = terminal.images; report["embedded"] = terminal.embedded
        try require(terminal.revision == revision, "Wrong PDF snapshot revision")
        try require(try identity(bridge) == before, "PDF changed source/revision/full selection/history")
        try require(try Data(contentsOf: URL(fileURLWithPath: input)) == original, "PDF changed original file")
        if cancelled {
            try require(terminal.phase == "cancelled", "PDF cancellation failed")
            try require((try? Data(contentsOf: target)) == previous, "Cancellation changed old output")
        } else {
            try require(terminal.phase == (warnings ? "completed_with_warnings" : "completed"), "PDF failed: \(terminal.message)")
            let bytes = try Data(contentsOf: target)
            guard let document = PDFDocument(data: bytes) else { throw NSError(domain: "Yu.Export.PDF.Check", code: 4) }
            let text = document.string ?? ""
            try require(text.contains("PDF-UNSAVED-SNAPSHOT") && !text.contains("PDF-LATER-EDIT-NOT-EXPORTED"), "Wrong PDF snapshot text")
            try require(!text.contains("GROUP5-PRIVATE-METADATA-MUST-NOT-LEAK"), "Private metadata leaked")
            try require(document.pageCount > 0 && document.pageCount == terminal.pages, "PDF page count mismatch")
            var pages: [[String: Any]] = []
            for n in 0..<document.pageCount {
                guard let page = document.page(at: n) else { throw NSError(domain: "Yu.Export.PDF.Check", code: 5) }
                let rect = page.bounds(for: .mediaBox)
                let expected = letter ? CGSize(width: 612, height: 792) : CGSize(width: 595.2756, height: 841.8898)
                try require(abs(rect.width - (landscape ? expected.height : expected.width)) < 0.02 && abs(rect.height - (landscape ? expected.width : expected.height)) < 0.02, "Paper dimensions mismatch")
                pages.append(["index": n, "width": rect.width, "height": rect.height, "text": page.string ?? "", "annotations": page.annotations.count])
            }
            report["pages"] = pages; report["page_count"] = document.pageCount
            report["output_sha256"] = sha(bytes); report["output_bytes"] = bytes.count
        }
        let old = bridge.source
        try bridge.setSelection(NSRange(location: 0, length: 0))
        _ = try bridge.insertText("PDF-CONTINUE-EDIT\n")
        _ = try bridge.executeCommand(UInt8(YU_STORAGE_COMMAND_UNDO))
        try require(bridge.source == old, "Continued edit/undo failed")
        report["source_identity_preserved"] = true; report["continued_edit_undo"] = true
        report["passed"] = true
    } catch { report["error"] = error.localizedDescription }
    if let bytes = try? JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys]) {
        try? bytes.write(to: URL(fileURLWithPath: output + ".report.json"), options: .withoutOverwriting)
        print(String(decoding: bytes, as: UTF8.self))
    }
    exit(report["passed"] as? Bool == true ? 0 : 1)
}
