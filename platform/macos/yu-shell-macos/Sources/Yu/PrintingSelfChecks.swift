import AppKit
import PDFKit
import CryptoKit
import Darwin
import YuStorageFFI

/// NSPrintOperation's file-only path. Never submits paper, even with a default
/// printer configured. Actual panel gestures are recorded by the desktop suite.
@MainActor
func runPrintingSelfCheck(input: String, directory: String) -> Never {
    let fm = FileManager.default, root = URL(fileURLWithPath: directory, isDirectory: true)
    var rows: [[String: Any]] = [], ownsRoot = false
    func require(_ condition: Bool, _ message: String) throws {
        if !condition { throw NSError(domain: "Yu.Print.Check", code: 1, userInfo: [NSLocalizedDescriptionKey: message]) }
    }
    func report(_ passed: Bool, _ error: String? = nil) {
        var result: [String: Any] = ["passed": passed, "cases": rows, "physical_jobs_submitted": 0, "evidence_layer": "native"]
        if let error { result["error"] = error }
        let data = try! JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys])
        if ownsRoot { try? data.write(to: root.appendingPathComponent("report.json")) }
        print(String(decoding: data, as: UTF8.self))
    }
    func wait(_ task: NativeHTMLExportTask, commit: Bool) throws -> NativeHTMLExportTask.Status {
        let deadline = Date().addingTimeInterval(120); var submitted = false
        while Date() < deadline {
            let status = try task.status()
            if ["completed", "completed_with_warnings", "failed", "cancelled"].contains(status.phase) { return status }
            if status.phase == "ready", commit, !submitted { try task.commit(allowWarnings: false); submitted = true }
            try require(status.phase != "warnings", "Unexpected warnings: \(status.warnings)")
            Thread.sleep(forTimeInterval: 0.01)
        }
        task.cancel(); throw NSError(domain: "Yu.Print.Check", code: 2)
    }
    do {
        try require(!fm.fileExists(atPath: root.path), "Refusing existing print evidence")
        try fm.createDirectory(at: root, withIntermediateDirectories: false); ownsRoot = true
        let original = try Data(contentsOf: URL(fileURLWithPath: input))
        var noPrinterDenied = false
        do { try NativePrintController.validateSpoolPrinters([]) } catch { noPrinterDenied = true }
        try require(noPrinterDenied, "No-printer preflight did not refuse")
        for name in ["whole", "range", "landscape", "source-alias", "image-alias", "write-denied", "target-change", "publication-cancel"] {
            try autoreleasepool {
            let work = root.appendingPathComponent(name); try fm.createDirectory(at: work, withIntermediateDirectories: false)
            defer { _ = chmod(work.path, 0o700) }
            let prepared = work.appendingPathComponent("prepared.pdf"), spool = work.appendingPathComponent("system.pdf"), target = work.appendingPathComponent("result.pdf")
            let bridge = try StorageBridge(path: input)
            try bridge.setSelection(NSRange(location: (bridge.source as NSString).length, length: 0))
            _ = try bridge.insertText("\nPRINT-UNSAVED-SNAPSHOT\n")
            let captured = bridge.source
            let info = NSPrintInfo(dictionary: [:]); info.paperSize = NSSize(width: 595.2756, height: 841.8898)
            info.leftMargin = 0; info.rightMargin = 0; info.topMargin = 0; info.bottomMargin = 0
            let task = try bridge.beginHTMLExport(to: prepared, config: NativePrintController.preparationConfig(title: "打印测试", untitled: false, resourceBase: nil, info: info))
            defer { task.cancel() }
            try bridge.setSelection(NSRange(location: 0, length: 0)); _ = try bridge.insertText("PRINT-LATER-NOT-IN-SNAPSHOT\n")
            try bridge.setSelectionEndpoints(anchorUTF16: 12, focusUTF16: 2, affinity: 0)
            let source = bridge.source, revision = bridge.revision, selection = bridge.selectionEndpoints
            try require(try wait(task, commit: true).phase == "completed", "Cannot prepare print PDF")
            let pdf = PDFDocument(url: prepared)!
            try require((pdf.string ?? "").contains("PRINT-UNSAVED-SNAPSHOT") && !(pdf.string ?? "").contains("PRINT-LATER-NOT-IN-SNAPSHOT"), "Print snapshot mixed revisions")
            let old = Data("OLD-PRINT-KEEP".utf8)
            if name == "source-alias" { try fm.linkItem(at: URL(fileURLWithPath: input), to: target) }
            else if name == "image-alias" { try fm.linkItem(at: URL(fileURLWithPath: input).deletingLastPathComponent().appendingPathComponent("assets/yu-mark.png"), to: target) }
            else { try old.write(to: target, options: .withoutOverwriting) }
            if ["source-alias", "image-alias"].contains(name) {
                let aliasBytes = try Data(contentsOf: target)
                var denied = false; do { try task.printOutput(target, publish: false) } catch { denied = true }
                try require(denied && Data(contentsOf: target) == aliasBytes, "Source or image alias was not protected")
            } else {
                try task.printOutput(target, publish: false)
                if name == "publication-cancel" {
                    task.cancel(); var denied = false
                    do { try task.printOutput(prepared, publish: true) } catch { denied = true }
                    try require(denied && Data(contentsOf: target) == old, "Cancelled print published")
                } else {
                    if name == "landscape" { info.orientation = .landscape }
                    info.jobDisposition = .save
                    info.dictionary()[NSPrintInfo.AttributeKey.jobSavingURL] = spool
                    if name == "range" {
                        info.dictionary()[NSPrintInfo.AttributeKey.allPages] = false
                        info.dictionary()[NSPrintInfo.AttributeKey.firstPage] = 2
                        info.dictionary()[NSPrintInfo.AttributeKey.lastPage] = 3
                    }
                    let view = NativePrintedPagesView(task: task, sourceSize: pdf.page(at: 0)!.bounds(for: .mediaBox).size, pages: pdf.pageCount)
                    let op = view.makeOperation(info: info)
                    op.showsPrintPanel = false; op.showsProgressPanel = false
                    try view.validatePaper()
                    try require(op.printInfo.jobDisposition == .save && op.run() && !view.drawingFailed, "System file-only operation failed")
                    try require(try Data(contentsOf: target) == old, "System output bypassed staged publication")
                    guard let output = PDFDocument(url: spool) else { throw NSError(domain: "Yu.Print.Check", code: 4) }
                    let expected = name == "range" ? 2 : pdf.pageCount
                    try require(output.pageCount == expected, "System range or pagination differs")
                    for n in 0..<expected {
                        let a = output.page(at: n)?.string ?? "", b = pdf.page(at: n + (name == "range" ? 1 : 0))?.string ?? ""
                        func compact(_ s: String) -> String { s.filter { !$0.isWhitespace } }
                        try require(compact(a) == compact(b), "System output changed page content at \(n)")
                    }
                    let clean = work.appendingPathComponent("checked-output.pdf")
                    try NativePrintController.prepareSystemPDF(spool, output: clean, title: "打印测试")
                    let checked = PDFDocument(url: clean)!
                    try require(checked.pageCount == output.pageCount && checked.documentAttributes?[PDFDocumentAttribute.authorAttribute] == nil, "System output exposes automatic Author metadata")
                    for n in 0..<checked.pageCount { try require(checked.page(at: n)?.string == output.page(at: n)?.string, "Metadata cleanup changed text") }
                    if name == "target-change" { try Data("EXTERNAL-PRINT".utf8).write(to: target) }
                    if name == "write-denied" { try require(chmod(work.path, 0o500) == 0, "Cannot restrict test output directory") }
                    try task.printOutput(clean, publish: true)
                    let status = try wait(task, commit: false)
                    _ = chmod(work.path, 0o700)
                    if name == "target-change" { try require(status.phase == "failed" && Data(contentsOf: target) == Data("EXTERNAL-PRINT".utf8), "External change overwritten") }
                    else if name == "write-denied" { try require(status.phase == "failed" && Data(contentsOf: target) == old, "Permission failure damaged target") }
                    else { try require(status.phase == "completed" && Data(contentsOf: target) == Data(contentsOf: clean), "System file not atomically published") }
                    rows.append(["id": name, "passed": true, "pages": output.pageCount, "source_pages": pdf.pageCount, "phase": status.phase])
                }
            }
            try require(bridge.source == source && bridge.revision == revision && bridge.selectionEndpoints.anchorUTF16 == selection.anchorUTF16 && bridge.selectionEndpoints.focusUTF16 == selection.focusUTF16, "Printing changed editor")
            try require(try Data(contentsOf: URL(fileURLWithPath: input)) == original, "Printing changed disk source")
            _ = try bridge.executeCommand(UInt8(YU_STORAGE_COMMAND_UNDO)); try require(bridge.source == captured, "Printing destroyed undo")
            _ = try bridge.executeCommand(UInt8(YU_STORAGE_COMMAND_REDO)); try require(bridge.source == source, "Printing destroyed redo")
            if ["source-alias", "image-alias", "publication-cancel"].contains(name) { rows.append(["id": name, "passed": true]) }
            }
        }
        report(true); exit(0)
    } catch { report(false, error.localizedDescription); exit(1) }
}
