import AppKit
import CryptoKit
import Darwin
import YuStorageFFI
import PDFKit

/// Production task/FFI failure contracts on this invocation's own fixtures.
/// No window, system clipboard, or global permission setting is involved.
@MainActor
func runHTMLExportSafetySelfCheck(directory: String, pdf: Bool = false) -> Never {
    let root = URL(fileURLWithPath: directory, isDirectory: true)
    let fm = FileManager.default
    var rows: [[String: Any]] = []
    var ownsRoot = false
    func require(_ value: Bool, _ message: String) throws {
        if !value { throw NSError(domain: "Yu.Export.Safety", code: 1, userInfo: [NSLocalizedDescriptionKey: message]) }
    }
    func sha(_ data: Data) -> String { SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined() }
    func identity(_ bridge: StorageBridge) throws -> Data {
        guard let selections = bridge.selectionsIfAvailable else { throw NSError(domain: "Yu.Export.Safety", code: 2) }
        let state = bridge.state, endpoints = bridge.selectionEndpoints
        return try JSONSerialization.data(withJSONObject: [
            "source": bridge.source, "revision": state.revision, "saved": state.savedRevision,
            "dirty": state.dirty, "bom": state.bom, "close": state.closeState,
            "anchor": endpoints.anchorUTF16, "focus": endpoints.focusUTF16,
            "primary": selections.primary,
            "ranges": selections.ranges.map { [$0.range.location, $0.range.length, Int($0.affinity)] },
            "tableColumns": bridge.tableSelectionColumns,
            "selectionMarkdown": try bridge.copySelectionMarkdown(revision: bridge.revision),
            "undo": bridge.commandAvailable(UInt8(YU_STORAGE_COMMAND_UNDO)),
            "redo": bridge.commandAvailable(UInt8(YU_STORAGE_COMMAND_REDO))
        ], options: [.sortedKeys])
    }
    func saveReport(_ passed: Bool, _ error: String? = nil) {
        var result: [String: Any] = ["passed": passed, "evidence_layer": "native", "format": pdf ? "pdf" : "html", "cases": rows]
        if let error { result["error"] = error }
        if let data = try? JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys]) {
            if ownsRoot { try? data.write(to: root.appendingPathComponent("report.json"), options: .atomic) }
            print(String(decoding: data, as: UTF8.self))
        }
    }
    let base = "# Safety 中文🙂\n\n| A | B |\n| --- | --- |\n| CELL_ONE | CELL_TWO |\n\nSAFETY-END\n"
    do {
        try require(!fm.fileExists(atPath: root.path), "Safety output directory already exists")
        try fm.createDirectory(at: root, withIntermediateDirectories: false)
        ownsRoot = true
        let png = Data(base64Encoded: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a5WQAAAAASUVORK5CYII=")!
        let modes = ["success-multi", "success-reverse", "success-table", "warning-cancel", "write-denied", "target-changed", "resource-changed", "resource-byte-budget"]
        for mode in modes {
            let work = root.appendingPathComponent(mode, isDirectory: true)
            try fm.createDirectory(at: work, withIntermediateDirectories: false)
            let path = work.appendingPathComponent("source.md")
            let target = work.appendingPathComponent(pdf ? "output.pdf" : "output.html")
            let image = work.appendingPathComponent("image.png")
            var source = base
            if ["warning-cancel", "write-denied", "target-changed"].contains(mode) { source += "\n![missing](missing.png)\n" }
            if mode == "resource-changed" {
                try png.write(to: image, options: .withoutOverwriting)
                source += "\n![local](image.png)\n" + (0..<220).map { "\n$x_{\($0)}^2+1$\n" }.joined()
            }
            if mode == "resource-byte-budget" {
                try Data().write(to: image, options: .withoutOverwriting)
                let handle = try FileHandle(forWritingTo: image)
                try handle.truncate(atOffset: 32 * 1024 * 1024 + 1); try handle.close()
                source += "\n![large](image.png)\n"
            }
            let sourceBytes = Data(source.utf8)
            try sourceBytes.write(to: path, options: .withoutOverwriting)
            let original = Data("OLD-OUTPUT-KEEP".utf8)
            try original.write(to: target, options: .withoutOverwriting)
            let bridge = try StorageBridge(path: path.path)
            let unsaved = "\nUNSAVED-SAFETY-SNAPSHOT\n", redo = "\nREDO-SAFETY-BRANCH\n"
            try bridge.setSelection(NSRange(location: (bridge.source as NSString).length, length: 0))
            _ = try bridge.insertText(unsaved)
            // A real cursor move separates typing groups. Two consecutive inserts
            // otherwise legitimately coalesce and undo both seed strings.
            try bridge.setSelection(NSRange(location: 0, length: 0))
            try bridge.setSelection(NSRange(location: (bridge.source as NSString).length, length: 0))
            _ = try bridge.insertText(redo)
            _ = try bridge.executeCommand(UInt8(YU_STORAGE_COMMAND_UNDO))
            try require(bridge.source == source + unsaved, "History seed did not retain the unsaved snapshot")
            try require(bridge.commandAvailable(UInt8(YU_STORAGE_COMMAND_REDO)), "Missing redo branch")
            if mode == "success-reverse" {
                try bridge.setSelectionEndpoints(anchorUTF16: 10, focusUTF16: 2, affinity: 0)
            } else if mode == "success-table" {
                let first = (bridge.source as NSString).range(of: "CELL_ONE")
                let last = (bridge.source as NSString).range(of: "CELL_TWO")
                try bridge.selectTableCells(anchor: first.location, focus: last.location, revision: bridge.revision)
                try require(bridge.tableSelectionColumns == 2, "Table rectangle was not established")
            } else {
                try bridge.setSelections([NSRange(location: 0, length: 1), NSRange(location: 3, length: 1)], primary: 1, affinities: [0, 1])
            }
            let before = try identity(bridge)
            let task = try bridge.beginHTMLExport(to: target, config: ["title": "Safety", "fontSize": 16,
                "width": 800, "referenceDay": 20724, "replaceExisting": true, "exportFormat": pdf ? "pdf" : "html"])
            var committed = false, changed = false, final: NativeHTMLExportTask.Status?
            let deadline = Date().addingTimeInterval(90)
            defer { task.cancel(); _ = chmod(work.path, 0o700) }
            while Date() < deadline {
                let status = try task.status()
                if ["completed", "completed_with_warnings", "failed", "cancelled"].contains(status.phase) { final = status; break }
                if mode == "resource-changed", !changed, status.message.hasPrefix("准备公式和图表") {
                    try (png + Data([0])).write(to: image, options: .atomic); changed = true
                }
                if status.phase == "ready" || status.phase == "warnings" {
                    if mode == "warning-cancel" {
                        var denied = false
                        do { try task.commit(allowWarnings: false) } catch { denied = true }
                        try require(denied, "Warnings published without consent")
                        try require(try Data(contentsOf: target) == original, "Unconfirmed warnings changed target")
                        task.cancel()
                    } else if !committed {
                        if mode == "write-denied" { try require(chmod(work.path, 0o500) == 0, "Cannot deny own fixture directory") }
                        if mode == "target-changed" { try Data("EXTERNAL-OUTPUT".utf8).write(to: target); changed = true }
                        try task.commit(allowWarnings: status.phase == "warnings"); committed = true
                    }
                }
                Thread.sleep(forTimeInterval: 0.002)
            }
            _ = chmod(work.path, 0o700)
            guard let final else { throw NSError(domain: "Yu.Export.Safety", code: 3, userInfo: [NSLocalizedDescriptionKey: "Safety case timed out: \(mode)"]) }
            let expected = mode.hasPrefix("success-") ? "completed" : (mode == "warning-cancel" ? "cancelled" : "failed")
            try require(final.phase == expected, "\(mode): \(final.phase), \(final.message)")
            let messages = ["write-denied": "创建导出临时文件", "target-changed": "目标已改变", "resource-changed": "图片改变", "resource-byte-budget": "32 MiB"]
            if let expectedMessage = messages[mode] { try require(final.message.contains(expectedMessage), "Unexpected failure: \(final.message)") }
            if mode == "resource-changed" { try require(changed, "Resource change was not observed during preparation") }
            try require(try identity(bridge) == before, "\(mode) changed full selection/history/source identity")
            try require(try Data(contentsOf: path) == sourceBytes, "\(mode) changed disk source")
            let output = try Data(contentsOf: target)
            if expected == "completed" {
                let text: String
                if pdf {
                    guard let document = PDFDocument(data: output), document.pageCount > 0 else {
                        throw NSError(domain: "Yu.Export.Safety", code: 4, userInfo: [NSLocalizedDescriptionKey: "Invalid PDF"])
                    }
                    text = document.string ?? ""
                } else { text = String(decoding: output, as: UTF8.self) }
                try require(text.contains("UNSAVED-SAFETY-SNAPSHOT") && !text.contains("REDO-SAFETY-BRANCH"), "Wrong history snapshot exported")
            } else {
                let kept = mode == "target-changed" ? Data("EXTERNAL-OUTPUT".utf8) : original
                try require(output == kept, "\(mode) did not preserve existing target")
            }
            _ = try bridge.executeCommand(UInt8(YU_STORAGE_COMMAND_REDO))
            try require(bridge.source == source + unsaved + redo, "\(mode) destroyed existing redo")
            _ = try bridge.executeCommand(UInt8(YU_STORAGE_COMMAND_UNDO))
            try require(bridge.source == source + unsaved, "\(mode) destroyed subsequent undo")
            try bridge.setSelection(NSRange(location: (bridge.source as NSString).length, length: 0))
            _ = try bridge.insertText("CONTINUE-EDITING")
            try require(bridge.source.hasSuffix("CONTINUE-EDITING"), "Cannot continue editing")
            _ = try bridge.executeCommand(UInt8(YU_STORAGE_COMMAND_UNDO))
            try require(bridge.source == source + unsaved, "Continued edit cannot be undone")
            try require(!(try fm.contentsOfDirectory(atPath: work.path)).contains(where: { $0.hasPrefix(".yu-export-") }), "Temporary output remained")
            rows.append(["id": mode, "passed": true, "phase": final.phase, "message": final.message,
                "full_identity_preserved": true, "redo_undo_executed": true, "continued_edit_undo": true,
                "output_sha256": sha(output), "resource_mutated_by_test": mode == "resource-changed"])
        }
        saveReport(true); exit(0)
    } catch {
        saveReport(false, error.localizedDescription); exit(1)
    }
}
