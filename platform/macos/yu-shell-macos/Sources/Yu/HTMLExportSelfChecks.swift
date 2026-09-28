import AppKit
import CryptoKit
import Darwin
import YuStorageFFI

/// Exercises the production bridge, worker, bundled helper and filesystem path.
/// This is native evidence, NOT a menu gesture or independent-viewer result.
@MainActor
func runHTMLExportSelfCheck(input: String, output: String) -> Never {
    func check(_ condition: Bool, _ message: String) throws {
        if !condition { throw NSError(domain: "Yu.Export.SelfCheck", code: 1, userInfo: [NSLocalizedDescriptionKey: message]) }
    }
    func sha(_ data: Data) -> String { SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined() }
    func identity(_ bridge: StorageBridge) throws -> Data {
        let state = bridge.state
        let endpoints = bridge.selectionEndpoints
        guard let selections = bridge.selectionsIfAvailable else { throw NSError(domain: "Yu.Export.SelfCheck", code: 2) }
        return try JSONSerialization.data(withJSONObject: [
            "source": bridge.source, "revision": state.revision, "saved": state.savedRevision,
            "dirty": state.dirty, "bom": state.bom, "close": state.closeState,
            "anchor": endpoints.anchorUTF16, "focus": endpoints.focusUTF16,
            "primary": selections.primary,
            "ranges": selections.ranges.map { [$0.range.location, $0.range.length, Int($0.affinity)] },
            "undo": bridge.commandAvailable(UInt8(YU_STORAGE_COMMAND_UNDO)),
            "redo": bridge.commandAvailable(UInt8(YU_STORAGE_COMMAND_REDO))
        ], options: [.sortedKeys])
    }
    var report: [String: Any] = ["evidence_layer": "native", "test": "html-export-self-check"]
    let outputURL = URL(fileURLWithPath: output)
    do {
        let originalDisk = try Data(contentsOf: URL(fileURLWithPath: input))
        let oldOutput = try? Data(contentsOf: outputURL)
        let bridge = try StorageBridge(path: input)
        let arguments = CommandLine.arguments
        let dark = arguments.contains("--export-dark")
        let cancel = arguments.contains("--export-cancel")
        let history = arguments.contains("--export-history")
        let allowWarnings = arguments.contains("--export-warnings")
        let theme = NativeTheme.spec(resolved: UInt8(dark ? YU_STORAGE_APPEARANCE_DARK : YU_STORAGE_APPEARANCE_LIGHT))
        let referenceDay = RenderCalendarContext.referenceDay(at: Date(), timeZone: .current) ?? 0
        let config: [String: Any] = ["title": "Yu 第五组综合导出", "foreground": theme.text,
            "background": theme.background, "link": theme.link, "fontSize": 16,
            "width": 800, "dark": dark, "referenceDay": referenceDay,
            "replaceExisting": oldOutput != nil]
        // Synthetic native composition: prove refusal preserves the overlay.
        try bridge.beginComposition(replacementRange: NSRange(location: 0, length: 0), preedit: "未提交组字", selection: NSRange(location: 5, length: 0))
        let beforeComposition = try identity(bridge)
        var refusedComposition = false
        do { let unexpected = try bridge.beginHTMLExport(to: outputURL, config: config); unexpected.cancel() }
        catch { refusedComposition = true }
        try check(refusedComposition && bridge.composition.active && bridge.copyComposition(bridge.composition) == "未提交组字", "导出必须拒绝且保留预编辑")
        try check(try identity(bridge) == beforeComposition, "预编辑拒绝改变了正文/历史/选区")
        try bridge.cancelComposition() // Test explicitly ends its own synthetic overlay.
        let unsaved = "\n\nGROUP5-UNSAVED-SNAPSHOT-中文🙂\n"
        let later = "\nGROUP5-LATER-EDIT-NOT-IN-EXPORT\n"
        try bridge.setSelection(NSRange(location: (bridge.source as NSString).length, length: 0))
        _ = try bridge.insertText(unsaved)
        if history {
            try bridge.setSelection(NSRange(location: (bridge.source as NSString).length, length: 0))
            _ = try bridge.insertText(later)
            _ = try bridge.executeCommand(UInt8(YU_STORAGE_COMMAND_UNDO))
            try check(!bridge.source.contains(later), "历史种子撤销失败")
            try check(bridge.commandAvailable(UInt8(YU_STORAGE_COMMAND_REDO)), "没有建立重做分支")
        }
        let capturedRevision = bridge.revision
        let task = try bridge.beginHTMLExport(to: outputURL, config: config)
        if !history && !cancel {
            try bridge.setSelection(NSRange(location: (bridge.source as NSString).length, length: 0))
            _ = try bridge.insertText(later)
        }
        try bridge.setSelections([NSRange(location: 0, length: 1), NSRange(location: 3, length: 1)], primary: 1)
        let expectedIdentity = try identity(bridge)
        if cancel { task.cancel() }
        let deadline = Date().addingTimeInterval(330)
        var committed = false
        var final: NativeHTMLExportTask.Status?
        while Date() < deadline {
            let status = try task.status()
            if ["completed", "completed_with_warnings", "cancelled", "failed"].contains(status.phase) { final = status; break }
            if status.phase == "ready" && !committed { try task.commit(allowWarnings: false); committed = true }
            if status.phase == "warnings" && !committed {
                try check(allowWarnings, "正常语料出现非预期警告：\(status.warnings.joined(separator: "；"))")
                try task.commit(allowWarnings: true); committed = true
            }
            Thread.sleep(forTimeInterval: 0.02)
        }
        guard let final else { task.cancel(); throw NSError(domain: "Yu.Export.SelfCheck", code: 3, userInfo: [NSLocalizedDescriptionKey: "导出自检超时"]) }
        report["phase"] = final.phase; report["warnings"] = final.warnings
        report["images"] = final.images; report["embedded"] = final.embedded
        report["captured_revision"] = capturedRevision; report["reference_day"] = referenceDay
        try check(final.revision == capturedRevision, "任务 revision 不匹配")
        try check(try identity(bridge) == expectedIdentity, "导出改变正文、版本、选区、脏状态或历史能力")
        try check(try Data(contentsOf: URL(fileURLWithPath: input)) == originalDisk, "导出改变磁盘原件")
        if cancel {
            try check(final.phase == "cancelled", "取消未生效")
            try check((try? Data(contentsOf: outputURL)) == oldOutput, "取消改变已有输出")
        } else {
            try check(final.phase == (allowWarnings ? "completed_with_warnings" : "completed"), "错误的结束状态：\(final.phase)：\(final.message)")
            let bytes = try Data(contentsOf: outputURL)
            let html = String(decoding: bytes, as: UTF8.self)
            try check(html.contains("GROUP5-UNSAVED-SNAPSHOT-中文🙂"), "未保存内容缺失")
            try check(!html.contains("GROUP5-LATER-EDIT-NOT-IN-EXPORT"), "启动后编辑混入产物")
            try check(!html.contains("<script") && !html.contains("src=\"https:"), "产物包含主动内容或远程图片")
            report["output_sha256"] = sha(bytes); report["output_bytes"] = bytes.count
        }
        if history {
            _ = try bridge.executeCommand(UInt8(YU_STORAGE_COMMAND_REDO))
            try check(bridge.source.contains(later), "导出破坏了原有重做分支")
            _ = try bridge.executeCommand(UInt8(YU_STORAGE_COMMAND_UNDO))
            try check(!bridge.source.contains(later), "重做后的撤销无效")
        }
        report["passed"] = true
        report["checks"] = ["native_preedit_refusal", "source_disk_unchanged", "source_revision_selection_dirty_history_unchanged", "snapshot_revision", cancel ? "cancel_preserves_target" : "unsaved_snapshot_not_later_edits"] + (history ? ["existing_redo_branch_executed"] : [])
        let json = try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
        try json.write(to: URL(fileURLWithPath: output + ".report.json"), options: .withoutOverwriting)
        print(String(decoding: json, as: UTF8.self))
        exit(0)
    } catch {
        report["passed"] = false; report["error"] = error.localizedDescription
        if let json = try? JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys]) {
            try? json.write(to: URL(fileURLWithPath: output + ".report.json"), options: .withoutOverwriting)
            FileHandle.standardError.write(json)
        }
        exit(1)
    }
}
