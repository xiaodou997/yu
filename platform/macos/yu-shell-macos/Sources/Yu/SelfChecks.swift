import AppKit
import Darwin
import Foundation
import UniformTypeIdentifiers
import YuStorageFFI

// self-check：验证 Rust↔Swift 边界上的真实行为（剪贴板、selection、undo、
// 投影、命中测试、IME、Accessibility）。
//
// 它们不是产品代码。v1 时期这 3800 行与产品代码混在 main.swift 里，且从未
// 进入 CI，因而无节制地膨胀；现在它们独立成文件，并由
// platform/macos/yu-shell-macos/run-self-checks.sh 在 CI 中执行。
//
// 调用入口（顶层 CommandLine 分发）必须留在 main.swift——Swift 只允许
// main.swift 含有顶层可执行语句。

private func checkTableProjectedGrapheme() throws {
    let path = FileManager.default.temporaryDirectory.appendingPathComponent("yu-grapheme-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: path) }
    for (atom, lower, upper) in [("\\*\u{0301}", 0, 3), ("&#32;\u{0301}", 0, 6), ("<br>\u{0301}", 4, 5)] {
        for forward in [false, true] {
            let source = "| H |\n| --- |\n| a\(atom)z |"
            try source.write(to: path, atomically: true, encoding: .utf8)
            let bridge = try StorageBridge(path: path.path)
            let view = DocumentTextView(bridge: bridge)
            let base = (source as NSString).range(of: atom).location
            let from = base + (forward ? lower : upper)
            view.navigate(toSource: NSRange(location: from, length: 0))
            view.doCommand(by: forward ? #selector(NSResponder.moveRight(_:)) : #selector(NSResponder.moveLeft(_:)))
            precondition(bridge.selection.range == NSRange(location: base + (forward ? upper : lower), length: 0))
            view.navigate(toSource: NSRange(location: from, length: 0))
            view.doCommand(by: forward ? #selector(NSResponder.deleteForward(_:)) : #selector(NSResponder.deleteBackward(_:)))
            let expected = (source as NSString).replacingCharacters(in: NSRange(location: base + lower, length: upper - lower), with: "")
            precondition(bridge.source == expected, "projected combining sequence split")
            view.performUndo()
            precondition(bridge.source == source)
        }
    }
    print("Yu projected table grapheme self-check: escape/entity combining sequences, BR boundary, both arrows/deletion directions and undo passed")
}

private func checkMergedHTMLSelectionClipboard() throws {
    let path = FileManager.default.temporaryDirectory.appendingPathComponent("yu-merged-table-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: path) }
    let source = "<table><tr><td id='a' colspan='2' rowspan='2'>中文🙂</td><td>B</td></tr><tr><td>C</td></tr></table>\r\n"
    let bom = Data([0xef, 0xbb, 0xbf])
    try (bom + Data(source.utf8)).write(to: path)
    let bridge = try StorageBridge(path: path.path)
    let view = DocumentTextView(bridge: bridge)
    let board = NSPasteboard.withUniqueName()
    defer { board.releaseGlobally() }
    let text = source as NSString
    try bridge.selectTableCells(anchor: text.range(of: "中文").location, focus: text.range(of: "C</td>").location, revision: bridge.revision)
    precondition(bridge.tableSelectionColumns == 3)
    precondition(bridge.selectionsIfAvailable?.ranges.count == 3)
    precondition(bridge.selectionsIfAvailable?.primary == 2)
    let fragments = try bridge.copySelectionFragments(revision: bridge.revision)
    precondition(fragments == ["中文🙂", "", "B", "", "", "C"])
    try view.copyToPasteboardForSelfCheck(board)
    precondition(board.string(forType: .string) == "中文🙂\t\tB\n\t\tC")
    let mergedHTML = board.string(forType: .yuHTML) ?? ""
    precondition(mergedHTML == "<table><tr><td id='a' colspan=\"2\" rowspan=\"2\">中文🙂</td><td>B</td></tr><tr><td>C</td></tr></table>")
    let copied = try bridge.copySelectionPayload(revision: bridge.revision)
    guard let tableSource = copied.tableSource else { preconditionFailure("missing merged table structure") }
    let targetPath = FileManager.default.temporaryDirectory.appendingPathComponent("yu-merged-paste-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: targetPath) }
    try Data().write(to: targetPath)
    let targetBridge = try StorageBridge(path: targetPath.path)
    let targetView = DocumentTextView(bridge: targetBridge)
    try targetView.pasteFromPasteboardForSelfCheck(board)
    precondition(targetBridge.source == tableSource)
    precondition(tableSource.contains("id='a'"))
    precondition(tableSource.contains("colspan=\"2\" rowspan=\"2\""))
    targetView.performUndo()
    precondition(targetBridge.source.isEmpty)
    targetView.performRedo()
    precondition(targetBridge.source == tableSource)
    try targetBridge.save()
    let targetReopened = try StorageBridge(path: targetPath.path)
    precondition(targetReopened.source == tableSource)
    for partialSource in [
        "<table><tr><td>LEFT</td><td>TARGET</td><td>OLD</td></tr><tr><td>LEFT2</td><td>E</td><td>F</td></tr><tr><td>BOTTOM</td><td>H</td><td>I</td></tr></table>\r\n",
        "| LEFT | TARGET | OLD |\r\n| --- | --- | --- |\r\n| LEFT2 | E | F |\r\n| BOTTOM | H | I |\r\n"
    ] {
        let partialPath = FileManager.default.temporaryDirectory.appendingPathComponent("yu-merged-overlay-\(UUID().uuidString).md")
        defer { try? FileManager.default.removeItem(at: partialPath) }
        try partialSource.write(to: partialPath, atomically: true, encoding: .utf8)
        let partialBridge = try StorageBridge(path: partialPath.path)
        let partialView = DocumentTextView(bridge: partialBridge)
        partialView.navigate(toSource: NSRange(location: (partialSource as NSString).range(of: "TARGET").location, length: 0))
        try partialView.pasteFromPasteboardForSelfCheck(board)
        let overlaid = partialBridge.source
        let leftHeader = partialSource.hasPrefix("|") ? "<th>LEFT</th>" : "<td>LEFT</td>"
        precondition(overlaid.contains(leftHeader) && overlaid.contains("<td>LEFT2</td>") && overlaid.contains("<td>BOTTOM</td>"))
        let partialCopied = try partialBridge.copySelectionPayload(revision: partialBridge.revision)
        precondition(partialCopied.tableSource == tableSource)
        partialView.performUndo()
        precondition(partialBridge.source == partialSource)
        partialView.performRedo()
        precondition(partialBridge.source == overlaid)
        try partialBridge.save()
        let partialBytes = try Data(contentsOf: partialPath)
        precondition(partialBytes == Data(overlaid.utf8))
        let partialReopened = try StorageBridge(path: partialPath.path)
        precondition(partialReopened.source == overlaid)
    }
    view.doCommand(by: #selector(NSResponder.deleteBackward(_:)))
    let cleared = source.replacingOccurrences(of: "中文🙂", with: "").replacingOccurrences(of: ">B<", with: "><").replacingOccurrences(of: ">C<", with: "><")
    precondition(bridge.source == cleared)
    precondition(bridge.selectionsIfAvailable?.ranges.count == 3)
    view.performUndo()
    precondition(bridge.source == source)
    precondition(bridge.selectionsIfAvailable?.primary == 2)
    board.clearContents()
    board.setString("1\t2\t3\n4\t5\t6", forType: .string)
    try view.pasteFromPasteboardForSelfCheck(board)
    let pasted = bridge.source
    precondition(!pasted.contains("colspan") && !pasted.contains("rowspan"))
    precondition(pasted.components(separatedBy: "id='a'").count == 2)
    precondition(bridge.selectionsIfAvailable?.ranges.count == 6)
    try view.copyToPasteboardForSelfCheck(board)
    precondition(board.string(forType: .string) == "1\t2\t3\n4\t5\t6")
    view.performUndo()
    precondition(bridge.source == source)
    precondition(bridge.selectionsIfAvailable?.ranges.count == 3)
    view.performRedo()
    precondition(bridge.source == pasted)
    try bridge.save()
    let saved = try Data(contentsOf: path)
    precondition(saved == bom + Data(pasted.utf8))
    let reopened = try StorageBridge(path: path.path)
    precondition(reopened.source == pasted)
    print("Yu merged HTML: native UTF16 selection, clipboard text, clear, split paste, undo/redo and BOM/CRLF save/reopen passed")
}

private func checkSparseHTMLSelectionClipboard() throws {
    let path = FileManager.default.temporaryDirectory.appendingPathComponent("yu-sparse-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: path) }
    let source = "<table><tr><td>A</td><td>B</td></tr><tr></tr><tr><td>C</td></tr></table>\r\n"
    try source.write(to: path, atomically: true, encoding: .utf8)
    let bridge = try StorageBridge(path: path.path)
    let view = DocumentTextView(bridge: bridge)
    let board = NSPasteboard.withUniqueName()
    defer { board.releaseGlobally() }
    let ns = source as NSString
    try bridge.selectTableCells(anchor: ns.range(of: "B</td>").location, focus: ns.range(of: "C</td>").location, revision: bridge.revision)
    let fragments = try bridge.copySelectionFragments(revision: bridge.revision)
    precondition(fragments == ["A", "B", "", "", "C", ""])
    try view.copyToPasteboardForSelfCheck(board)
    precondition(board.string(forType: .string) == "A\tB\n\t\nC\t")
    let html = board.string(forType: .yuHTML) ?? ""
    precondition(html.components(separatedBy: "<td>").count == 7)
    view.doCommand(by: #selector(NSResponder.deleteBackward(_:)))
    precondition(bridge.source == source.replacingOccurrences(of: ">A<", with: "><").replacingOccurrences(of: ">B<", with: "><").replacingOccurrences(of: ">C<", with: "><"))
    view.performUndo()
    precondition(bridge.source == source)
    try view.pasteFromPasteboardForSelfCheck(board)
    let padded = source.replacingOccurrences(of: "<tr></tr>", with: "<tr><td></td><td></td></tr>").replacingOccurrences(of: "<td>C</td></tr>", with: "<td>C</td><td></td></tr>")
    precondition(bridge.source == padded)
    view.performUndo()
    precondition(bridge.source == source)
    try bridge.save()
    let reopened = try StorageBridge(path: path.path)
    precondition(reopened.source == source)
    print("Yu sparse HTML clipboard: absent slots, native clipboard roundtrip, clear, undo and exact save/reopen passed")
}

private func checkHTMLCellParagraphs() throws {
    let path = FileManager.default.temporaryDirectory.appendingPathComponent("yu-cell-paragraphs-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: path) }
    let source = "<table><tr><td><p>one</p><p align='right'>中文 two</p></td><td>KEEP</td></tr></table>\r\n"
    try source.write(to: path, atomically: true, encoding: .utf8)
    let bridge = try StorageBridge(path: path.path)
    let view = DocumentTextView(bridge: bridge)
    let board = NSPasteboard.withUniqueName()
    defer { board.releaseGlobally() }
    let ns = source as NSString
    try bridge.selectTableCells(anchor: ns.range(of: "one</p>").location, focus: ns.range(of: "KEEP").location, revision: bridge.revision)
    try view.copyToPasteboardForSelfCheck(board)
    precondition(board.string(forType: .string) == "\"one\n中文 two\"\tKEEP")
    try view.pasteFromPasteboardForSelfCheck(board)
    precondition(bridge.source == source, "HTML format tags escaped during private clipboard roundtrip")
    view.navigate(toSource: NSRange(location: ns.range(of: "中文 two").location, length: 0))
    view.doCommand(by: #selector(NSResponder.deleteBackward(_:)))
    let joined = source.replacingOccurrences(of: "</p><p align='right'>", with: "")
    precondition(bridge.source == joined)
    view.performUndo()
    precondition(bridge.source == source)
    try bridge.save()
    let reopened = try StorageBridge(path: path.path)
    precondition(reopened.source == source)
    print("Yu HTML cell paragraphs: clipboard line breaks, native paragraph join/undo and exact save/reopen passed")
}

private func checkHTMLCellHeadings() throws {
    let path = FileManager.default.temporaryDirectory.appendingPathComponent("yu-cell-headings-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: path) }
    let source = "<table><tr><td><h2 align='center'>中文 heading</h2><p>body</p></td><td>KEEP</td></tr></table>\r\n"
    try source.write(to: path, atomically: true, encoding: .utf8)
    let bridge = try StorageBridge(path: path.path)
    let view = DocumentTextView(bridge: bridge)
    let board = NSPasteboard.withUniqueName()
    defer { board.releaseGlobally() }
    let ns = source as NSString
    try bridge.selectTableCells(anchor: ns.range(of: "中文 heading").location, focus: ns.range(of: "KEEP").location, revision: bridge.revision)
    try view.copyToPasteboardForSelfCheck(board)
    precondition(board.string(forType: .string) == "\"中文 heading\nbody\"\tKEEP")
    try view.pasteFromPasteboardForSelfCheck(board)
    precondition(bridge.source == source)
    view.navigate(toSource: NSRange(location: ns.range(of: "中文 heading").location, length: 0))
    view.insertText("新<&", replacementRange: NSRange(location: NSNotFound, length: 0))
    let edited = source.replacingOccurrences(of: "中文 heading", with: "新&lt;&amp;中文 heading")
    precondition(bridge.source == edited)
    view.performUndo()
    precondition(bridge.source == source)
    view.performRedo()
    precondition(bridge.source == edited)
    view.navigate(toSource: NSRange(location: (edited as NSString).range(of: "body</p>").location, length: 0))
    view.doCommand(by: #selector(NSResponder.deleteBackward(_:)))
    let joined = edited.replacingOccurrences(of: "</h2><p>", with: "").replacingOccurrences(of: "body</p>", with: "body</h2>")
    precondition(bridge.source == joined)
    view.performUndo()
    precondition(bridge.source == edited)
    view.performRedo()
    precondition(bridge.source == joined)
    try bridge.save()
    let reopened = try StorageBridge(path: path.path)
    precondition(reopened.source == joined)
    print("Yu HTML cell headings: clipboard, input, mixed paragraph merge, undo/redo and exact save/reopen passed")
}

private func checkHTMLCellLists() throws {
    let path = FileManager.default.temporaryDirectory.appendingPathComponent("yu-cell-lists-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: path) }
    let original = "<table><tr><td><ol start='3'><li>one<ul><li>nested</li></ul></li><li>two</li></ol></td><td>KEEP</td></tr></table>\r\n"
    try original.write(to: path, atomically: true, encoding: .utf8)
    let bridge = try StorageBridge(path: path.path)
    let view = DocumentTextView(bridge: bridge)
    let board = NSPasteboard.withUniqueName()
    defer { board.releaseGlobally() }
    try bridge.selectTableCells(anchor: (original as NSString).range(of: "one").location, focus: (original as NSString).range(of: "KEEP").location, revision: bridge.revision)
    try view.copyToPasteboardForSelfCheck(board)
    precondition(board.string(forType: .string) == "\"3. one\n  • nested\n4. two\"\tKEEP")
    try view.pasteFromPasteboardForSelfCheck(board)
    precondition(bridge.source == original)
    view.navigate(toSource: NSRange(location: (original as NSString).range(of: "nested").location, length: 0))
    view.insertText("中文<&", replacementRange: NSRange(location: NSNotFound, length: 0))
    let edited = original.replacingOccurrences(of: "nested", with: "中文&lt;&amp;nested")
    precondition(bridge.source == edited)
    view.performUndo()
    precondition(bridge.source == original)
    view.performRedo()
    precondition(bridge.source == edited)
    try bridge.save()
    let reopened = try StorageBridge(path: path.path)
    precondition(reopened.source == edited)
    view.performUndo()
    precondition(bridge.source == original)
    let nestedStart = (original as NSString).range(of: "nested").location
    view.navigate(toSource: NSRange(location: nestedStart + 3, length: 0))
    view.doCommand(by: #selector(NSResponder.insertNewline(_:)))
    let split = original.replacingOccurrences(of: "nested", with: "nes</li><li>ted")
    precondition(bridge.source == split)
    view.insertText("中", replacementRange: NSRange(location: NSNotFound, length: 0))
    let typedSplit = split.replacingOccurrences(of: "<li>ted", with: "<li>中ted")
    precondition(bridge.source == typedSplit)
    view.performUndo()
    precondition(bridge.source == split)
    view.performUndo()
    precondition(bridge.source == original)
    view.performRedo()
    precondition(bridge.source == split)
    try bridge.save()
    let splitReopened = try StorageBridge(path: path.path)
    precondition(splitReopened.source == split)
    print("Yu HTML cell lists: clipboard, typing, native Enter split, undo/redo and exact save/reopen passed")
}

private func checkHTMLListIndent() throws {
    let path = FileManager.default.temporaryDirectory.appendingPathComponent("yu-list-indent-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: path) }
    let original = "<table><tr><td><ul><li>First</li><li>Second</li></ul></td><td>KEEP</td></tr></table>\r\n"
    try original.write(to: path, atomically: true, encoding: .utf8)
    let bridge = try StorageBridge(path: path.path)
    let view = DocumentTextView(bridge: bridge)
    view.navigate(toSource: NSRange(location: (original as NSString).range(of: "Second").location, length: 0))
    let item = NSMenuItem(title: "Indent", action: #selector(DocumentTextView.editListFromMenu(_:)), keyEquivalent: "")
    item.tag = Int(YU_STORAGE_COMMAND_INDENT_LIST)
    precondition(view.validateMenuItem(item))
    view.editListFromMenu(item)
    let nested = original.replacingOccurrences(of: "</li><li>Second</li>", with: "<ul><li>Second</li></ul></li>")
    precondition(bridge.source == nested)
    item.tag = Int(YU_STORAGE_COMMAND_OUTDENT_LIST)
    precondition(view.validateMenuItem(item))
    view.editListFromMenu(item)
    precondition(bridge.source == original)
    view.performUndo()
    precondition(bridge.source == nested)
    view.performUndo()
    precondition(bridge.source == original)
    view.performRedo()
    precondition(bridge.source == nested)
    try bridge.save()
    let reopened = try StorageBridge(path: path.path)
    precondition(reopened.source == nested)
    view.performUndo()
    precondition(bridge.source == original)
    view.navigate(toSource: NSRange(location: (original as NSString).range(of: "Second").location, length: 0))
    view.doCommand(by: #selector(NSResponder.deleteBackward(_:)))
    let exited = original.replacingOccurrences(of: "<li>Second</li></ul>", with: "</ul><p>Second</p>")
    precondition(bridge.source == exited)
    view.insertText("中", replacementRange: NSRange(location: NSNotFound, length: 0))
    precondition(bridge.source == exited.replacingOccurrences(of: "Second", with: "中Second"))
    view.performUndo()
    precondition(bridge.source == exited)
    view.performUndo()
    precondition(bridge.source == original)
    view.performRedo()
    precondition(bridge.source == exited)
    try bridge.save()
    let exitedReopened = try StorageBridge(path: path.path)
    precondition(exitedReopened.source == exited)
    print("Yu HTML list menu indent/outdent, Backspace, history and save/reopen passed")
}

private func checkHTMLListSelections() throws {
    let cases: [(String, String, UInt8)] = [
        ("<ul><li>A</li><li>中文</li><!--gap--><li>C🙂</li></ul>",
         "<ul><li>A<ul><li>中文</li><!--gap--><li>C🙂</li></ul></li></ul>", UInt8(YU_STORAGE_COMMAND_INDENT_LIST)),
        ("<ol start='3'><li>A</li><li id='b'><b>中文</b></li><!--gap--><li><p>C🙂</p></li><li>D</li></ol>",
         "<ol start='3'><li>A</li></ol><p id='b'><b>中文</b></p><!--gap--><div><p>C🙂</p></div><ol start='6'><li>D</li></ol>", UInt8(YU_STORAGE_COMMAND_OUTDENT_LIST)),
        ("<ul><li>P<div align='right'><ol start='3'><li>A</li><li>中文</li><!--gap--><li>C🙂</li><li>D</li></ol>tail</div></li></ul>",
         "<ul><li>P<div align='right'><ol start='3'><li>A</li></ol></div></li><li>中文</li><!--gap--><li>C🙂<div align='right'><ol start='6'><li>D</li></ol>tail</div></li></ul>", UInt8(YU_STORAGE_COMMAND_OUTDENT_LIST)),
    ]
    for (before, after, command) in cases {
        for backward in [false, true] {
            let path = FileManager.default.temporaryDirectory.appendingPathComponent("yu-list-selection-\(UUID().uuidString).md")
            defer { try? FileManager.default.removeItem(at: path) }
            let original = "<table><tr><td>" + before + "</td><td>KEEP</td></tr></table>\r\n"
            let expected = "<table><tr><td>" + after + "</td><td>KEEP</td></tr></table>\r\n"
            let bom = Data([0xef, 0xbb, 0xbf])
            try (bom + Data(original.utf8)).write(to: path)
            let bridge = try StorageBridge(path: path.path)
            let view = DocumentTextView(bridge: bridge)
            func endpoints(_ source: String) -> (UInt64, UInt64) {
                let text = source as NSString
                let start = UInt64(text.range(of: "中文").location)
                let end = UInt64(NSMaxRange(text.range(of: "🙂")))
                return backward ? (end, start) : (start, end)
            }
            let initial = endpoints(original)
            let final = endpoints(expected)
            try bridge.setSelectionEndpoints(anchorUTF16: initial.0, focusUTF16: initial.1)
            let item = NSMenuItem(title: "List", action: #selector(DocumentTextView.editListFromMenu(_:)), keyEquivalent: "")
            item.tag = Int(command)
            precondition(view.validateMenuItem(item))
            view.editListFromMenu(item)
            precondition(bridge.source == expected)
            precondition(bridge.selectionEndpoints.anchorUTF16 == final.0)
            precondition(bridge.selectionEndpoints.focusUTF16 == final.1)
            view.performUndo()
            precondition(bridge.source == original)
            precondition(bridge.selectionEndpoints.anchorUTF16 == initial.0)
            precondition(bridge.selectionEndpoints.focusUTF16 == initial.1)
            view.performRedo()
            precondition(bridge.source == expected)
            precondition(bridge.selectionEndpoints.anchorUTF16 == final.0)
            precondition(bridge.selectionEndpoints.focusUTF16 == final.1)
            try bridge.save()
            let saved = try Data(contentsOf: path)
            precondition(saved == bom + Data(expected.utf8))
            let reopened = try StorageBridge(path: path.path)
            precondition(reopened.source == expected)
        }
    }
    let multiPath = FileManager.default.temporaryDirectory.appendingPathComponent("yu-list-multi-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: multiPath) }
    let multiSource = "<table><tr><td><ul><li><b>中文🙂末</b></li><li>XY</li></ul></td><td>CD</td></tr></table>\r\n\r\n正文尾\r\n"
    let multiExpected = "<table><tr><td><ul><li><b>中文</b></li><li><b>🙂</b></li><li><b>末</b></li><li>X</li><li>Y</li></ul></td><td>C<br>D</td></tr></table>\r\n\r\n正文\r\n尾\r\n"
    try multiSource.write(to: multiPath, atomically: true, encoding: .utf8)
    let multiBridge = try StorageBridge(path: multiPath.path)
    let multiView = DocumentTextView(bridge: multiBridge)
    let needles = ["🙂", "末", "Y</li>", "D</td>", "尾"]
    let cursors = needles.map { NSRange(location: (multiSource as NSString).range(of: $0).location, length: 0) }
    try multiBridge.setSelections(cursors, primary: 1)
    multiView.doCommand(by: #selector(NSResponder.insertNewline(_:)))
    precondition(multiBridge.source == multiExpected)
    multiView.performUndo()
    precondition(multiBridge.source == multiSource)
    multiView.performRedo()
    precondition(multiBridge.source == multiExpected)
    try multiBridge.save()
    let multiSaved = try Data(contentsOf: multiPath)
    precondition(multiSaved == Data(multiExpected.utf8))
    let multiReopened = try StorageBridge(path: multiPath.path)
    precondition(multiReopened.source == multiExpected)
    let indentPath = FileManager.default.temporaryDirectory.appendingPathComponent("yu-multi-indent-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: indentPath) }
    let indentSource = "<table><tr><td><ul><li>首</li><li>中文🙂</li><li>末</li></ul></td></tr></table>\r\n\r\n正文尾\r\n"
    let indentExpected = "<table><tr><td><ul><li>首<ul><li>中文🙂</li><li>末</li></ul></li></ul></td></tr></table>\r\n\r\n正文尾\r\n"
    try indentSource.write(to: indentPath, atomically: true, encoding: .utf8)
    let indentBridge = try StorageBridge(path: indentPath.path)
    let indentView = DocumentTextView(bridge: indentBridge)
    let indentNeedles = ["首", "中文", "🙂", "末", "尾"]
    try indentBridge.setSelections(indentNeedles.map { NSRange(location: (indentSource as NSString).range(of: $0).location, length: 0) }, primary: 0)
    let indentItem = NSMenuItem(title: "Indent", action: #selector(DocumentTextView.editListFromMenu(_:)), keyEquivalent: "")
    indentItem.tag = Int(YU_STORAGE_COMMAND_INDENT_LIST)
    precondition(indentView.validateMenuItem(indentItem))
    indentView.editListFromMenu(indentItem)
    precondition(indentBridge.source == indentExpected)
    indentView.performUndo()
    precondition(indentBridge.source == indentSource)
    indentView.performRedo()
    precondition(indentBridge.source == indentExpected)
    try indentBridge.save()
    let indentSaved = try Data(contentsOf: indentPath)
    precondition(indentSaved == Data(indentExpected.utf8))
    let indentReopened = try StorageBridge(path: indentPath.path)
    precondition(indentReopened.source == indentExpected)
    let outdentPath = FileManager.default.temporaryDirectory.appendingPathComponent("yu-multi-outdent-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: outdentPath) }
    let outdentSource = "<table><tr><td><ul><li>A</li><li>中文🙂</li><li>尾</li></ul></td></tr></table>\r\n"
    let outdentExpected = "<table><tr><td><ul><li>A</li></ul><p>中文🙂</p><p>尾</p></td></tr></table>\r\n"
    try outdentSource.write(to: outdentPath, atomically: true, encoding: .utf8)
    let outdentBridge = try StorageBridge(path: outdentPath.path)
    let outdentView = DocumentTextView(bridge: outdentBridge)
    try outdentBridge.setSelections(["中文🙂", "尾"].map { (outdentSource as NSString).range(of: $0) }, primary: 1)
    let outdentItem = NSMenuItem(title: "Outdent", action: #selector(DocumentTextView.editListFromMenu(_:)), keyEquivalent: "")
    outdentItem.tag = Int(YU_STORAGE_COMMAND_OUTDENT_LIST)
    precondition(outdentView.validateMenuItem(outdentItem))
    outdentView.editListFromMenu(outdentItem)
    precondition(outdentBridge.source == outdentExpected)
    outdentView.performUndo()
    precondition(outdentBridge.source == outdentSource)
    outdentView.performRedo()
    precondition(outdentBridge.source == outdentExpected)
    try outdentBridge.save()
    let outdentSaved = try Data(contentsOf: outdentPath)
    precondition(outdentSaved == Data(outdentExpected.utf8))
    let outdentReopened = try StorageBridge(path: outdentPath.path)
    precondition(outdentReopened.source == outdentExpected)
    let crossPath = FileManager.default.temporaryDirectory.appendingPathComponent("yu-cross-outdent-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: crossPath) }
    let crossSource = "<table><tr><td><ul><li><b>中文🙂</b></li><li><p>End</p></li></ul></td><td><ul><li>Parent<ul><li>Second</li><li>Third</li></ul></li></ul></td></tr></table>\r\n"
    let crossExpected = "<table><tr><td><p><b>中文🙂</b></p><div><p>End</p></div></td><td><ul><li>Parent</li><li>Second</li><li>Third</li></ul></td></tr></table>\r\n"
    try crossSource.write(to: crossPath, atomically: true, encoding: .utf8)
    let crossBridge = try StorageBridge(path: crossPath.path)
    let crossView = DocumentTextView(bridge: crossBridge)
    func crossRanges(_ text: String) -> [NSRange] {
        let source = text as NSString
        return [("中文", "End"), ("Second", "Third")].map { first, last in
            let start = source.range(of: first).location
            return NSRange(location: start, length: NSMaxRange(source.range(of: last)) - start)
        }
    }
    try crossBridge.setSelections(crossRanges(crossSource), primary: 1)
    precondition(crossView.validateMenuItem(outdentItem))
    crossView.editListFromMenu(outdentItem)
    precondition(crossBridge.source == crossExpected)
    precondition(crossBridge.selectionsIfAvailable?.ranges.map { $0.range } == crossRanges(crossExpected))
    precondition(crossBridge.selectionsIfAvailable?.primary == 1)
    crossView.performUndo()
    precondition(crossBridge.source == crossSource)
    precondition(crossBridge.selectionsIfAvailable?.ranges.map { $0.range } == crossRanges(crossSource))
    crossView.performRedo()
    precondition(crossBridge.source == crossExpected)
    precondition(crossBridge.selectionsIfAvailable?.ranges.map { $0.range } == crossRanges(crossExpected))
    precondition(crossBridge.selectionsIfAvailable?.primary == 1)
    try crossBridge.save()
    let crossSaved = try Data(contentsOf: crossPath)
    precondition(crossSaved == Data(crossExpected.utf8))
    let crossReopened = try StorageBridge(path: crossPath.path)
    precondition(crossReopened.source == crossExpected)
    let nestedPath = FileManager.default.temporaryDirectory.appendingPathComponent("yu-nested-multi-outdent-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: nestedPath) }
    let nestedSource = "<table><tr><td><ul><li>Parent<div id='wrap'><ol start='4'><li>A</li><li id='b'>中文🙂</li><!--keep--><li>Second</li><li>Third</li><li>Z</li></ol></div>Tail</li></ul></td></tr></table>\r\n"
    let nestedExpected = "<table><tr><td><ul><li>Parent<div id='wrap'><ol start='4'><li>A</li></ol></div></li><li id='b'>中文🙂</li><!--keep--><li>Second</li><li>Third<div ><ol start='8'><li>Z</li></ol></div>Tail</li></ul></td></tr></table>\r\n"
    let nestedBOM = Data([0xef, 0xbb, 0xbf])
    try (nestedBOM + Data(nestedSource.utf8)).write(to: nestedPath)
    let nestedBridge = try StorageBridge(path: nestedPath.path)
    let nestedView = DocumentTextView(bridge: nestedBridge)
    func nestedRanges(_ text: String) -> [NSRange] {
        let source = text as NSString
        let second = source.range(of: "Second").location
        return [source.range(of: "中文🙂"), NSRange(location: second, length: NSMaxRange(source.range(of: "Third")) - second)]
    }
    func verifyNested(_ text: String) {
        precondition(nestedBridge.source == text)
        precondition(nestedBridge.selectionsIfAvailable?.primary == 1)
        precondition(nestedBridge.selectionsIfAvailable?.ranges.map { $0.range } == nestedRanges(text))
    }
    try nestedBridge.setSelections(nestedRanges(nestedSource), primary: 1)
    precondition(nestedView.validateMenuItem(outdentItem))
    nestedView.editListFromMenu(outdentItem)
    verifyNested(nestedExpected)
    nestedView.performUndo()
    verifyNested(nestedSource)
    nestedView.performRedo()
    verifyNested(nestedExpected)
    try nestedBridge.save()
    let nestedSaved = try Data(contentsOf: nestedPath)
    precondition(nestedSaved == nestedBOM + Data(nestedExpected.utf8))
    let nestedReopened = try StorageBridge(path: nestedPath.path)
    precondition(nestedReopened.source == nestedExpected)
    print("Yu HTML list selections: native menu, directed UTF16 ranges, history and exact save/reopen passed")
}

private func checkMissingFootnoteRecovery() throws {
    let path = FileManager.default.temporaryDirectory.appendingPathComponent("yu-footnote-recovery-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: path) }
    let original = "中文🙂 reference[^missing].\r\n"
    let addition = "\r\n[^missing]: Definition **保留**.\r\n"
    let bom = Data([0xef, 0xbb, 0xbf])
    try (bom + Data(original.utf8)).write(to: path)
    let bridge = try StorageBridge(path: path.path)
    let view = DocumentTextView(bridge: bridge)
    let reference = (original as NSString).range(of: "[^missing]").location
    func verifyMissing(_ session: StorageBridge) throws {
        let diagnostic = try session.documentDiagnostic(at: reference)
        let target = try session.documentReferenceTarget(at: reference)
        precondition(!diagnostic.isEmpty)
        precondition(target == nil)
        precondition(session.source == original)
    }
    try verifyMissing(bridge)
    try bridge.save()
    let missingBytes = try Data(contentsOf: path)
    precondition(missingBytes == bom + Data(original.utf8))
    let missingReopened = try StorageBridge(path: path.path)
    try verifyMissing(missingReopened)
    view.navigate(toSource: NSRange(location: (original as NSString).length, length: 0))
    view.insertText(addition, replacementRange: NSRange(location: NSNotFound, length: 0))
    let corrected = original + addition
    precondition(bridge.source == corrected)
    let diagnostic = try bridge.documentDiagnostic(at: reference)
    let target = try bridge.documentReferenceTarget(at: reference)
    precondition(diagnostic.isEmpty)
    precondition(target?.location == (original as NSString).length + 2)
    view.performUndo()
    try verifyMissing(bridge)
    view.performRedo()
    precondition(bridge.source == corrected)
    let redoDiagnostic = try bridge.documentDiagnostic(at: reference)
    precondition(redoDiagnostic.isEmpty)
    try bridge.save()
    let correctedBytes = try Data(contentsOf: path)
    precondition(correctedBytes == bom + Data(corrected.utf8))
    let reopened = try StorageBridge(path: path.path)
    let reopenedDiagnostic = try reopened.documentDiagnostic(at: reference)
    let reopenedTarget = try reopened.documentReferenceTarget(at: reference)
    precondition(reopened.source == corrected && reopenedDiagnostic.isEmpty)
    precondition(reopenedTarget == target)
    print("Yu missing footnote: diagnostic, repair, history and exact save/reopen passed")
}

private func checkHTMLEmptyListExit() throws {
    let path = FileManager.default.temporaryDirectory.appendingPathComponent("yu-list-exit-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: path) }
    let original = "<table><tr><td><ul><li>parent<ul><li></li></ul></li></ul></td><td>KEEP</td></tr></table>\r\n"
    try original.write(to: path, atomically: true, encoding: .utf8)
    let bridge = try StorageBridge(path: path.path)
    let view = DocumentTextView(bridge: bridge)
    view.navigate(toSource: NSRange(location: (original as NSString).range(of: "<li></li>").location + 4, length: 0))
    view.doCommand(by: #selector(NSResponder.insertNewline(_:)))
    let outdented = "<table><tr><td><ul><li>parent</li><li></li></ul></td><td>KEEP</td></tr></table>\r\n"
    precondition(bridge.source == outdented)
    view.doCommand(by: #selector(NSResponder.insertNewline(_:)))
    let exited = "<table><tr><td><ul><li>parent</li></ul><p></p></td><td>KEEP</td></tr></table>\r\n"
    precondition(bridge.source == exited)
    view.insertText("中文<&", replacementRange: NSRange(location: NSNotFound, length: 0))
    let typed = exited.replacingOccurrences(of: "<p></p>", with: "<p>中文&lt;&amp;</p>")
    precondition(bridge.source == typed)
    for expected in [exited, outdented, original] {
        view.performUndo()
        precondition(bridge.source == expected)
    }
    for expected in [outdented, exited, typed] {
        view.performRedo()
        precondition(bridge.source == expected)
    }
    try bridge.save()
    let reopened = try StorageBridge(path: path.path)
    precondition(reopened.source == typed)
    print("Yu empty HTML list: nested outdent, exit, typing, undo/redo and exact save/reopen passed")
}

private func checkHTMLCellDetails() throws {
    let path = FileManager.default.temporaryDirectory.appendingPathComponent("yu-cell-details-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: path) }
    let original = "<table><tr><td><details><summary>标题</summary><p>Hidden body</p></details></td><td>KEEP</td></tr></table>\r\n"
    try original.write(to: path, atomically: true, encoding: .utf8)
    let bridge = try StorageBridge(path: path.path)
    let view = DocumentTextView(bridge: bridge)
    let board = NSPasteboard.withUniqueName()
    defer { board.releaseGlobally() }
    let title = (original as NSString).range(of: "标题").location
    let header = try bridge.disclosureHeader(at: title, expectedRevision: bridge.revision)
    precondition(header != nil && header?.open == false)
    try bridge.selectTableCells(anchor: title, focus: (original as NSString).range(of: "KEEP").location, revision: bridge.revision)
    try view.copyToPasteboardForSelfCheck(board)
    precondition(board.string(forType: .string) == "标题\tKEEP")
    try view.pasteFromPasteboardForSelfCheck(board)
    precondition(bridge.source == original, "closed details clipboard lost hidden content")
    precondition(view.toggleDisclosure(at: title, revision: bridge.revision))
    let opened = original.replacingOccurrences(of: "<details>", with: "<details open>")
    precondition(bridge.source == opened)
    let visible = try bridge.disclosureHeader(at: (opened as NSString).range(of: "标题").location, expectedRevision: bridge.revision)
    precondition(visible?.open == true)
    view.performUndo()
    precondition(bridge.source == original)
    view.performRedo()
    precondition(bridge.source == opened)
    view.navigate(toSource: NSRange(location: (opened as NSString).range(of: "Hidden body").location, length: 0))
    view.insertText("中文<&", replacementRange: NSRange(location: NSNotFound, length: 0))
    let edited = opened.replacingOccurrences(of: "Hidden body", with: "中文&lt;&amp;Hidden body")
    precondition(bridge.source == edited)
    try bridge.save()
    let reopened = try StorageBridge(path: path.path)
    precondition(reopened.source == edited)
    view.performUndo()
    precondition(bridge.source == opened)
    print("Yu HTML cell details: header query, hidden-source clipboard, native toggle/undo, body edit and exact save/reopen passed")
}

private func checkTypedTableClipboard() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("yu-typed-clipboard-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    for htmlDestination in [false, true] {
        let original = htmlDestination ? "<table><tr><td>KEEP</td></tr></table>\r\n" : "| KEEP |\n| --- |\n"
        let path = directory.appendingPathComponent(htmlDestination ? "html.md" : "markdown.md")
        try original.write(to: path, atomically: true, encoding: .utf8)
        let bridge = try StorageBridge(path: path.path)
        let view = DocumentTextView(bridge: bridge)
        view.navigate(toSource: NSRange(location: (original as NSString).range(of: "KEEP").location, length: 0))
        let format = htmlDestination ? UInt8(YU_STORAGE_FRAGMENT_MARKDOWN) : UInt8(YU_STORAGE_FRAGMENT_HTML)
        let cells = htmlDestination ? ["**中文 bold** [link](https://example.com)"] : ["<p><b>中文 bold</b> <a href='https://example.com'>link</a></p>"]
        _ = try bridge.pasteFragments(cells, columns: 1, format: format)
        let edited = bridge.source
        if htmlDestination {
            precondition(edited.contains("<strong>中文 bold</strong>") && edited.contains("href=\"https://example.com\""))
            precondition(!edited.contains("&lt;p&gt;"))
        } else {
            precondition(edited.contains("**中文 bold**") && edited.contains("[link](https://example.com)"))
        }
        try bridge.save()
        let reopened = try StorageBridge(path: path.path)
        precondition(reopened.source == edited)
        view.performUndo()
        precondition(bridge.source == original)
        view.performRedo()
        precondition(bridge.source == edited)
    }
    print("Yu typed table clipboard: Markdown/HTML format conversion, exact saved bytes and native undo/redo passed")
}

private func checkCellSelectionClipboard() throws {
    try checkTypedTableClipboard()
    try checkHTMLCellParagraphs()
    try checkHTMLCellHeadings()
    try checkHTMLCellLists()
    try checkHTMLEmptyListExit()
    try checkHTMLListIndent()
    try checkHTMLListSelections()
    try checkMissingFootnoteRecovery()
    try checkHTMLCellDetails()
    try checkMergedHTMLSelectionClipboard()
    try checkSparseHTMLSelectionClipboard()
    try checkTableProjectedGrapheme()
    let path = FileManager.default.temporaryDirectory.appendingPathComponent("yu-cell-selection-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: path) }
    let source = "| A | B |\n| --- | --- |\n|  | 🪶 |\n"
    try source.write(to: path, atomically: true, encoding: .utf8)
    let bridge = try StorageBridge(path: path.path)
    let view = DocumentTextView(bridge: bridge)
    let board = NSPasteboard.withUniqueName()
    defer { board.releaseGlobally() }
    let ns = source as NSString
    try bridge.selectTableCells(anchor: ns.range(of: "A").location, focus: ns.range(of: "🪶").location, revision: bridge.revision)
    precondition(bridge.tableSelectionColumns == 2 && bridge.selectionsIfAvailable?.ranges.count == 4)
    try view.copyToPasteboardForSelfCheck(board)
    precondition(board.string(forType: .string) == "A\tB\n\t🪶")
    precondition(board.string(forType: .yuMarkdown) == source.trimmingCharacters(in: .newlines))
    let html = board.string(forType: .yuHTML) ?? ""
    precondition(html.components(separatedBy: "<td>").count == 5 && html.contains("<td></td>"))
    view.doCommand(by: #selector(NSResponder.deleteBackward(_:)))
    let cleared = "|  |  |\n| --- | --- |\n|  |  |\n"
    precondition(bridge.source == cleared, "clearing empty cells must not backspace delimiters")
    view.performUndo()
    precondition(bridge.source == source && bridge.tableSelectionColumns == 2)
    view.performRedo()
    precondition(bridge.source == cleared)
    let positions = bridge.selectionsIfAvailable!.ranges
    try bridge.selectTableCells(anchor: positions.first!.range.location, focus: positions.last!.range.location, revision: bridge.revision)
    try view.pasteFromPasteboardForSelfCheck(board)
    // Empty cells own the start of their existing padding. Paste inserts there
    // without rewriting the two existing spaces (undo above restores exact source).
    let pasted = "|A  |B  |\n| --- | --- |\n|  |🪶  |\n"
    precondition(bridge.source == pasted, "empty copied fragment lost its cell")
    view.performUndo()
    precondition(bridge.source == cleared && bridge.tableSelectionColumns == 2)
    view.performRedo()
    precondition(bridge.source == pasted)
    try bridge.save()
    let reopened = try StorageBridge(path: path.path)
    precondition(reopened.source == pasted)
    // The clipboard carries its own dimensions; a one-cell destination expands
    // instead of joining the four source fragments into one malformed cell.
    let targetPath = FileManager.default.temporaryDirectory.appendingPathComponent("yu-grid-target-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: targetPath) }
    let targetSource = "> | H |\r\n> | :--- |\r\n> | z |"
    try targetSource.write(to: targetPath, atomically: true, encoding: .utf8)
    let target = try StorageBridge(path: targetPath.path)
    let targetView = DocumentTextView(bridge: target)
    let z = (targetSource as NSString).range(of: "z").location
    try target.selectTableCells(anchor: z, focus: z, revision: target.revision)
    try targetView.pasteFromPasteboardForSelfCheck(board)
    let expanded = "> | H |  |\r\n> | :--- | --- |\r\n> | A | B |\r\n> |  | 🪶 |"
    precondition(target.source == expanded, "grid must expand destination rows and columns")
    precondition(target.tableSelectionColumns == 2 && target.selectionsIfAvailable?.ranges.count == 4)
    targetView.performUndo()
    precondition(target.source == targetSource && target.tableSelectionColumns == 1)
    targetView.performRedo()
    precondition(target.source == expanded && target.tableSelectionColumns == 2)
    try target.save()
    let targetReopened = try StorageBridge(path: targetPath.path)
    precondition(targetReopened.source == expanded)
    let invalidBefore = target.source
    do {
        _ = try target.pasteFragments(["one"], columns: 2)
        preconditionFailure("partial grid row accepted")
    } catch BridgeError.operation(let status) {
        precondition(status == 24 && target.source == invalidBefore)
    }
    let plainPath = FileManager.default.temporaryDirectory.appendingPathComponent("yu-grid-plain-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: plainPath) }
    try "".write(to: plainPath, atomically: true, encoding: .utf8)
    let plain = try StorageBridge(path: plainPath.path)
    let plainView = DocumentTextView(bridge: plain)
    try plainView.pasteFromPasteboardForSelfCheck(board)
    precondition(plain.source == source.trimmingCharacters(in: .newlines), "grid pasted outside a table must remain Markdown")
    plainView.performUndo()
    precondition(plain.source.isEmpty)
    print("Yu grid paste clipboard self-check: dimensioned copy, quoted CRLF growth, save/reopen, undo/redo, invalid shape and ordinary-document paste passed")
    let externalBoard = NSPasteboard.withUniqueName()
    defer { externalBoard.releaseGlobally() }
    // No private fragments: these paths exercise external Markdown and accepted
    // HTML followed by the same source parser and grid transaction in Rust.
    let externalMarkdown = "| **A** | B |\n| :---: | ---: |\n| x | 🪶 |\n"
    let externalHTML = "<table><thead><tr><th><strong>A</strong></th><th>B</th></tr></thead><tbody><tr><td>x</td><td>🪶</td></tr></tbody></table>"
    let externalExpected = "> | H |  |\r\n> | :--- | --- |\r\n> | **A** | B |\r\n> | x | 🪶 |"
    let directHTML = "<table><tr><td><strong>A</strong></td><td>B</td></tr><tr><td>x</td><td>🪶</td></tr></table>"
    let bodyOnlyHTML = "<table><tbody><tr><td><strong>A</strong></td><td>B</td></tr><tr><td>x</td><td>🪶</td></tr></tbody></table>"
    for (format, content) in [(NSPasteboard.PasteboardType.yuMarkdown, externalMarkdown), (.yuHTML, externalHTML), (.yuHTML, directHTML), (.yuHTML, bodyOnlyHTML)] {
        targetView.performUndo()
        precondition(target.source == targetSource)
        externalBoard.clearContents()
        precondition(externalBoard.setString("plain fallback must not win", forType: .string))
        precondition(externalBoard.setString(content, forType: format))
        try targetView.pasteFromPasteboardForSelfCheck(externalBoard)
        precondition(target.source == externalExpected, "external table was flattened or its delimiter pasted into a cell")
        precondition(target.tableSelectionColumns == 2 && target.selectionsIfAvailable?.ranges.count == 4)
        try target.save()
        let externalReopened = try StorageBridge(path: targetPath.path)
        precondition(externalReopened.source == externalExpected)
        targetView.performUndo()
        precondition(target.source == targetSource)
        targetView.performRedo()
        precondition(target.source == externalExpected)
    }
    print("Yu external table clipboard self-check: Markdown and thead/direct-tr/tbody HTML precedence, grid growth, CRLF preservation, save/reopen and undo/redo passed")
    targetView.performUndo()
    precondition(target.source == targetSource)
    externalBoard.clearContents()
    precondition(externalBoard.setString("fallback must not win", forType: .string))
    precondition(externalBoard.setString("<table><tr><td><p><strong>first<br>second</strong></p><p>third</p></td><td>&lt;br&gt;<br>end</td></tr></table>", forType: .yuHTML))
    try targetView.pasteFromPasteboardForSelfCheck(externalBoard)
    let multilineHTMLExpected = "> | H |  |\r\n> | :--- | --- |\r\n> | **first<br>second**<br><br>third | \\<br\\><br>end |"
    precondition(target.source == multilineHTMLExpected, "HTML multiline table fell back or split rows")
    let breakStart = (target.source as NSString).range(of: "<br>").location
    targetView.navigate(toSource: NSRange(location: breakStart, length: 0))
    targetView.doCommand(by: #selector(NSResponder.moveRight(_:)))
    precondition(target.selection.range == NSRange(location: breakStart + 4, length: 0))
    targetView.doCommand(by: #selector(NSResponder.moveLeft(_:)))
    precondition(target.selection.range == NSRange(location: breakStart, length: 0))
    targetView.doCommand(by: #selector(NSResponder.moveRightAndModifySelection(_:)))
    precondition(target.selection.range == NSRange(location: breakStart, length: 4))
    try target.save()
    let multilineHTMLReopened = try StorageBridge(path: targetPath.path)
    precondition(multilineHTMLReopened.source == multilineHTMLExpected)
    targetView.performUndo()
    precondition(target.source == targetSource)
    targetView.performRedo()
    precondition(target.source == multilineHTMLExpected)
    targetView.performUndo()
    precondition(target.source == targetSource)
    externalBoard.clearContents()
    precondition(externalBoard.setString("fallback", forType: .string))
    precondition(externalBoard.setString("\" first\r\nsecond \"\t\"a\tb\"\r\n\"<br>\"\t\" \"", forType: .tabularText))
    try targetView.pasteFromPasteboardForSelfCheck(externalBoard)
    let tsvExpected = "> | H |  |\r\n> | :--- | --- |\r\n> | &#32;first<br>second&#32; | a&#9;b |\r\n> | \\<br\\> | &#32; |"
    precondition(target.source == tsvExpected, "TSV field boundaries or literal data lost")
    try targetView.copyToPasteboardForSelfCheck(externalBoard)
    let visibleTSV = "\" first\nsecond \"\t\"a\tb\"\n<br>\t "
    precondition(externalBoard.string(forType: .string) == visibleTSV)
    precondition(externalBoard.string(forType: .tabularText) == visibleTSV)
    // Exercise the external representation alone, without private fragments,
    // Markdown or HTML masking a broken visible-text roundtrip.
    externalBoard.clearContents()
    precondition(externalBoard.setString(visibleTSV, forType: .tabularText))
    try targetView.pasteFromPasteboardForSelfCheck(externalBoard)
    precondition(target.source == tsvExpected, "TSV copy/paste changed cell data")
    precondition(target.tableSelectionColumns == 2 && target.selectionsIfAvailable?.ranges.count == 4)
    try target.save()
    let tsvReopened = try StorageBridge(path: targetPath.path)
    precondition(tsvReopened.source == tsvExpected)
    targetView.performUndo()
    precondition(target.source == targetSource)
    targetView.performRedo()
    precondition(target.source == tsvExpected)
    targetView.performUndo()
    externalBoard.clearContents()
    precondition(externalBoard.setString("a\tb", forType: .string))
    try targetView.pasteFromPasteboardForSelfCheck(externalBoard)
    precondition(target.source == "> | H |  |\r\n> | :--- | --- |\r\n> | a | b |")
    try plainView.pasteFromPasteboardForSelfCheck(externalBoard)
    precondition(plain.source == "a\tb", "ordinary document tabs must remain source")
    plainView.performUndo()
    targetView.performUndo()
    externalBoard.clearContents()
    precondition(externalBoard.setString("\"unclosed", forType: .tabularText))
    do {
        try targetView.pasteFromPasteboardForSelfCheck(externalBoard)
        preconditionFailure("malformed TSV accepted")
    } catch BridgeError.operation(let status) {
        precondition(status == 24 && target.source == targetSource)
    }
    print("Yu TSV clipboard self-check: quoted multiline/tab/space/literal fields, explicit format precedence, plain-text context, rejection, save and history passed")
    // A single empty cell is a real selection and can be copied without text.
    let empty = (pasted as NSString).range(of: "|  |").location + 1
    try bridge.selectTableCells(anchor: empty, focus: empty, revision: bridge.revision)
    precondition(view.hasSourceSelection && bridge.tableSelectionColumns == 1)
    try view.copyToPasteboardForSelfCheck(board)
    precondition(board.string(forType: .yuMarkdown) == "|  |\n| --- |")
    view.doCommand(by: #selector(NSResponder.deleteBackward(_:)))
    precondition(bridge.source == pasted)
    print("Yu rectangular cell clipboard self-check: empty cells, TSV/Markdown/HTML, clear, paste, history and save/reopen passed")
}

func runClipboardSelfCheck(path: String) -> Never {
    do {
        try checkCellSelectionClipboard()
        let bridge = try StorageBridge(path: path)
        let textView = DocumentTextView(bridge: bridge)
        let pasteboard = NSPasteboard.withUniqueName()

        pasteboard.clearContents()
        precondition(
            pasteboard.setString(
                "<h2>Yu</h2><p><strong>羽</strong></p>",
                forType: .yuHTML
            )
        )
        let imported = try textView.sourceFromPasteboardForSelfCheck(pasteboard)
        precondition(imported == "## Yu\n\n**羽**")

        pasteboard.clearContents()
        precondition(
            pasteboard.setString(
                "<script>alert(1)</script>",
                forType: .yuHTML
            )
        )
        let rejected = try textView.sourceFromPasteboardForSelfCheck(pasteboard)
        precondition(rejected == nil)

        pasteboard.clearContents()
        precondition(pasteboard.setString("plain fallback", forType: .string))
        precondition(
            pasteboard.setString(
                "<script>alert(1)</script>",
                forType: .yuHTML
            )
        )
        let plainFallback = try textView.sourceFromPasteboardForSelfCheck(pasteboard)
        precondition(plainFallback == "plain fallback")

        // **纯文本与可用 HTML 同时在时取 HTML。**
        //
        // 这一种组合以前一条断言都没有，而它正是**每一个真实剪贴板**的形状
        // ——浏览器、邮件、文档编辑器都同时放两种。缺了它，「Markdown > 纯文本
        // > HTML」这个顺序绿了很久，而 HTML 导入在生产里一次都没走到过
        // （S7 第七刀 c 的 G 节验收查出来的）。
        //
        // 判据要看得出「取的是哪一份」：纯文本那一份**故意不是** HTML 那一份
        // 的降级文本，否则两条路产出同一个字符串，断言什么都证明不了。
        pasteboard.clearContents()
        precondition(pasteboard.setString("只有纯文本会看到这一行", forType: .string))
        precondition(
            pasteboard.setString(
                "<h2>来自 HTML</h2>",
                forType: .yuHTML
            )
        )
        let preferHTML = try textView.sourceFromPasteboardForSelfCheck(pasteboard)
        precondition(preferHTML == "## 来自 HTML", "纯文本与可用 HTML 同时在时必须取 HTML")

        pasteboard.clearContents()
        precondition(pasteboard.setString("**canonical**", forType: .yuMarkdown))
        precondition(pasteboard.setString("<p>derived</p>", forType: .yuHTML))
        let canonical = try textView.sourceFromPasteboardForSelfCheck(pasteboard)
        precondition(canonical == "**canonical**")

        let fixtureDirectory = URL(fileURLWithPath: path)
            .deletingLastPathComponent()
            .appendingPathComponent("clipboard", isDirectory: true)
        let fixtureCases: [(name: String, accepted: Bool)] = [
            ("semantic-mail", true),
            ("rich-table", true),
            // S7 第七刀 c 的 G 节验收之后翻案：浏览器的信封
            // （<html>/<head>/<body>/<!--StartFragment-->/<div>）不再算
            // 「别人的 HTML」，拆掉信封之后里面那段语义要接得住。以前这里
            // 断言的是「被拒」，而那条断言绿着的同时，Yu 从来没有接住过一次
            // 真实的浏览器粘贴。
            ("browser-wrapper", true),
            // 带行为的标签继续拒——变的是信封，不是这一条。
            ("unsafe", false),
        ]
        for fixture in fixtureCases {
            let htmlURL = fixtureDirectory.appendingPathComponent("\(fixture.name).html")
            let html = try String(contentsOf: htmlURL, encoding: .utf8)
            pasteboard.clearContents()
            precondition(pasteboard.setString(html, forType: .yuHTML))
            let result = try textView.sourceFromPasteboardForSelfCheck(pasteboard)
            if fixture.accepted {
                let expectedURL = fixtureDirectory
                    .appendingPathComponent("\(fixture.name).expected.md")
                let expected = try String(contentsOf: expectedURL, encoding: .utf8)
                    .replacingOccurrences(of: "␠␠", with: "  ")
                    .trimmingCharacters(in: .newlines)
                precondition(result == expected, "fixture \(fixture.name) mismatch")
            } else {
                precondition(result == nil, "fixture \(fixture.name) should be rejected")
            }
        }

        // Disjoint table cells plus an empty primary caret: copy/cut must
        // publish all deleted content and never backspace the empty caret.
        let temporary = FileManager.default.temporaryDirectory.appendingPathComponent("yu-table-cut-\(UUID().uuidString).md")
        defer { try? FileManager.default.removeItem(at: temporary) }
        let original = "| A | B |\n| --- | --- |\n| 中文 | 🪶 |\n\noutside\n"
        try original.write(to: temporary, atomically: true, encoding: .utf8)
        let cutBridge = try StorageBridge(path: temporary.path)
        let cutView = DocumentTextView(bridge: cutBridge)
        let ns = original as NSString
        let first = ns.range(of: "中文")
        let second = ns.range(of: "🪶")
        let empty = NSRange(location: ns.range(of: "outside").location + 3, length: 0)
        try cutBridge.setSelections([first, second, empty], primary: 2)
        precondition(cutView.hasSourceSelection)
        try cutView.copyToPasteboardForSelfCheck(pasteboard)
        precondition(pasteboard.string(forType: .yuMarkdown) == "中文\n🪶")
        let copiedHTML = pasteboard.string(forType: .yuHTML) ?? ""
        precondition(copiedHTML.contains("中文") && copiedHTML.contains("🪶"))
        try cutView.cutToPasteboardForSelfCheck(pasteboard)
        precondition(pasteboard.string(forType: .string) == "中文\n🪶")
        let cutSource = original.replacingOccurrences(of: "中文", with: "").replacingOccurrences(of: "🪶", with: "")
        precondition(cutBridge.source == cutSource)
        _ = try cutBridge.executeCommand(8)
        precondition(cutBridge.source == original)
        precondition(cutBridge.selectionsIfAvailable?.ranges.count == 3)
        precondition(cutBridge.selectionsIfAvailable?.primary == 2)
        _ = try cutBridge.executeCommand(9)
        precondition(cutBridge.source == cutSource)
        precondition(!cutView.hasSourceSelection)
        // Reuse the actual copied envelope, selecting two header cells. The
        // empty source caret was omitted, so the two fragments map one-to-one.
        _ = try cutBridge.executeCommand(8)
        let headerA = ns.range(of: "A")
        let headerB = ns.range(of: "B")
        try cutBridge.setSelections([headerA, headerB], primary: 1)
        try cutView.pasteFromPasteboardForSelfCheck(pasteboard)
        let distributed = original.replacingOccurrences(of: "| A | B |", with: "| 中文 | 🪶 |")
        precondition(cutBridge.source == distributed)
        precondition(cutBridge.selectionsIfAvailable?.ranges.count == 2)
        precondition(cutBridge.selectionsIfAvailable?.primary == 1)
        _ = try cutBridge.insertText("!")
        _ = try cutBridge.executeCommand(8)
        precondition(cutBridge.source == distributed, "undo typing must preserve paste")
        _ = try cutBridge.executeCommand(8)
        precondition(cutBridge.source == original, "one undo restores both pasted cells")
        precondition(cutBridge.selectionsIfAvailable?.ranges.map { $0.range } == [headerA, headerB])
        _ = try cutBridge.executeCommand(9)
        precondition(cutBridge.source == distributed)
        try cutBridge.save()
        let reopened = try StorageBridge(path: temporary.path)
        precondition(reopened.source == distributed)
        // An invalid custom payload must not suppress usable plain text.
        pasteboard.clearContents()
        precondition(pasteboard.setData(Data("invalid".utf8), forType: .yuFragments))
        precondition(pasteboard.setString("fallback", forType: .string))
        try cutBridge.setSelections([NSRange(location: 0, length: 0)], primary: 0)
        try cutView.pasteFromPasteboardForSelfCheck(pasteboard)
        precondition(cutBridge.source == "fallback" + distributed)
        print("Yu fragment paste self-check: per-cell distribution, primary selection, atomic undo/redo, save/reopen and malformed-envelope fallback passed")

        print("Yu multi-selection clipboard self-check: all selected table cells copied/cut, empty primary preserved, undo/redo passed")

        print(
            "Yu Clipboard self-check: Markdown > plain text > strict HTML fallback; "
                + "fixtures=\(fixtureCases.count)"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Clipboard self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

func runSelectionSelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let textView = DocumentTextView(bridge: bridge)
        let source = bridge.source as NSString
        let first = source.range(of: "日本語")
        let second = source.range(of: "Emoji")
        precondition(first.location != NSNotFound)
        precondition(second.location != NSNotFound)

        let firstCaret = NSRange(location: first.location + first.length, length: 0)
        textView.setSelectedRanges(
            [NSValue(range: firstCaret)],
            affinity: .downstream,
            stillSelecting: false
        )
        precondition(bridge.selection.range == firstCaret)

        let secondCaret = NSRange(location: second.location + second.length, length: 0)
        textView.setSelectedRanges(
            [NSValue(range: secondCaret)],
            affinity: .downstream,
            stillSelecting: false
        )
        precondition(bridge.selection.range == secondCaret)
        print("Yu Selection self-check: setSelectedRanges tracks two distinct source positions")
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Selection self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

private func checkEmptyHTMLListInput() throws {
    let path = FileManager.default.temporaryDirectory.appendingPathComponent("yu-empty-html-list-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: path) }
    let original = "<ol start='8'><li></li><li><!--keep--></li><li>尾</li></ol>\r\n"
    try original.write(to: path, atomically: true, encoding: .utf8)
    let bridge = try StorageBridge(path: path.path)
    let view = DocumentTextView(bridge: bridge)
    view.navigate(toSource: NSRange(location: (original as NSString).range(of: "</li>").location, length: 0))
    view.setMarkedText("中文", selectedRange: NSRange(location: 2, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
    precondition(bridge.composition.active && bridge.source == original)
    view.setMarkedText("", selectedRange: NSRange(location: 0, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
    precondition(!bridge.composition.active && bridge.source == original)
    view.setMarkedText("中文", selectedRange: NSRange(location: 2, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
    view.insertText("中文", replacementRange: NSRange(location: NSNotFound, length: 0))
    let edited = original.replacingOccurrences(of: "<li></li>", with: "<li>中文</li>")
    precondition(!bridge.composition.active && bridge.source == edited)
    view.performUndo()
    precondition(bridge.source == original)
    view.performRedo()
    precondition(bridge.source == edited)
    try bridge.save()
    let reopened = try StorageBridge(path: path.path)
    precondition(reopened.source == edited)
    print("Yu empty HTML list: native preedit cancel/commit, undo/redo and exact save/reopen passed")
}

func runUndoSelfCheck(path: String) -> Never {
    do {
        try checkEmptyHTMLListInput()
        let bridge = try StorageBridge(path: path)
        let textView = DocumentTextView(bridge: bridge)
        let original = bridge.source
        let end = NSRange(location: original.utf16.count, length: 0)
        try bridge.setSelection(end)

        textView.insertText("x", replacementRange: end)
        precondition(bridge.source == original + "x")
        precondition(textView.canUndo())

        textView.performUndo()
        precondition(bridge.source == original)
        precondition(textView.canRedo())

        textView.performRedo()
        precondition(bridge.source == original + "x")
        let committedRevision = bridge.revision
        textView.setMarkedText("ceshi", selectedRange: NSRange(location: 5, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
        precondition(bridge.composition.active && bridge.source == original + "x")
        guard let escape = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
            windowNumber: 0, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53) else {
            preconditionFailure("Escape event")
        }
        textView.keyDown(with: escape)
        precondition(!bridge.composition.active && !textView.hasMarkedText())
        precondition(bridge.revision == committedRevision && bridge.source == original + "x")
        textView.performUndo()
        precondition(bridge.source == original, "Cancelled IME must not strand undo behind an empty overlay")
        textView.performRedo()
        precondition(bridge.source == original + "x")
        textView.setMarkedText("y", selectedRange: NSRange(location: 1, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
        textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
        precondition(!bridge.composition.active && !textView.hasMarkedText(), "Deleting the final preedit character must release the overlay")
        precondition(bridge.source == original + "x")
        textView.performUndo()
        precondition(bridge.source == original, "Empty preedit blocked undo")
        textView.performRedo()
        precondition(bridge.source == original + "x")
        print("Yu Undo self-check: Rust history routes undo and redo through the native host")
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Undo self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

/// Verifies the first product-level contract without opening a window:
/// source TextKit construction, Unicode edit, Rust undo/redo, native
/// clipboard payload/paste, save, and a fresh session reopening the exact
/// bytes. The input fixture is copied to a temporary path so the repository
/// file is never modified.
private func checkTableMenuEditing() throws {
    let temporary = FileManager.default.temporaryDirectory.appendingPathComponent("yu-table-menu-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: temporary) }
    let original = "> | A | B | C |\r\n> | --- | --- | --- |\r\n> | 中文 | middle | 🪶 |\r\n"
    try original.write(to: temporary, atomically: true, encoding: .utf8)
    let bridge = try StorageBridge(path: temporary.path)
    let view = DocumentTextView(bridge: bridge)
    let menu = view.makeTableMenu()
    let items = menu.items.filter { !$0.isSeparatorItem }
    precondition(items.count == 10)
    for item in items {
        let cell = (original as NSString).range(of: "middle")
        try bridge.setSelection(NSRange(location: cell.location, length: 0))
        view.refreshFromRust()
        let available = view.validateMenuItem(item)
        precondition(NSApp.sendAction(item.action!, to: item.target, from: item), "native menu target missing")
        let edited = bridge.source
        if item.tag == Int(YU_STORAGE_COMMAND_TABLE_ALIGN_DEFAULT) {
            precondition(!available && edited == original, "default alignment must be a no-op")
            continue
        }
        precondition(available && edited != original, "menu command was not connected")
        precondition(edited.contains("中文") && edited.contains("🪶") || item.tag == Int(YU_STORAGE_COMMAND_TABLE_DELETE_ROW))
        try bridge.save()
        let reopened = try StorageBridge(path: temporary.path)
        precondition(reopened.source == edited)
        view.performUndo()
        precondition(bridge.source == original)
        view.performRedo()
        precondition(bridge.source == edited)
        view.performUndo()
        precondition(bridge.source == original)
    }
    try bridge.setSelection((original as NSString).range(of: "B"))
    let deleteRow = items.first { $0.tag == Int(YU_STORAGE_COMMAND_TABLE_DELETE_ROW) }!
    let above = items.first { $0.tag == Int(YU_STORAGE_COMMAND_TABLE_INSERT_ROW_BEFORE) }!
    precondition(!view.validateMenuItem(deleteRow) && !view.validateMenuItem(above))
    print("Yu table menu self-check: 10 native menu actions, validation, CRLF quote source, undo/redo and save/reopen passed")
}

func runDocumentWorkflowSelfCheck(path: String) -> Never {
    let fileManager = FileManager.default
    let sourceURL = URL(fileURLWithPath: path)
    let temporaryURL = fileManager.temporaryDirectory
        .appendingPathComponent("yu-workflow-\(UUID().uuidString).md")
    do {
        try checkTableMenuEditing()
        try fileManager.copyItem(at: sourceURL, to: temporaryURL)
        defer { try? fileManager.removeItem(at: temporaryURL) }

        let sourceBefore: String
        let savedSource: String
        do {
            let bridge = try StorageBridge(path: temporaryURL.path)
            let textView = DocumentTextView(bridge: bridge)
            sourceBefore = bridge.source
            precondition(sourceBefore.contains("日本語"))
            precondition(sourceBefore.contains("🙂"))

            let addition = "\nYu workflow: 日本語 🙂 é"
            let end = NSRange(location: sourceBefore.utf16.count, length: 0)
            try bridge.setSelection(end)
            textView.insertText(addition, replacementRange: end)
            precondition(bridge.source == sourceBefore + addition)
            precondition(bridge.state.dirty)

            textView.performUndo()
            precondition(bridge.source == sourceBefore)
            textView.performRedo()
            precondition(bridge.source == sourceBefore + addition)

            let insertedRange = NSRange(
                location: sourceBefore.utf16.count,
                length: addition.utf16.count
            )
            try bridge.setSelection(insertedRange)
            let pasteboard = NSPasteboard.withUniqueName()
            try textView.copyToPasteboardForSelfCheck(pasteboard)
            precondition(
                pasteboard.string(forType: .yuMarkdown) == addition,
                "copy must publish canonical source"
            )

            let pasteEnd = NSRange(location: bridge.source.utf16.count, length: 0)
            try bridge.setSelection(pasteEnd)
            try textView.pasteFromPasteboardForSelfCheck(pasteboard)
            precondition(bridge.source == sourceBefore + addition + addition)

            try bridge.save()
            precondition(!bridge.state.dirty)
            savedSource = bridge.source
        }

        let reopened = try StorageBridge(path: temporaryURL.path)
        precondition(reopened.source == savedSource)
        precondition(!reopened.state.dirty)
        print(
            "Yu Document Workflow self-check: open/edit/undo/redo/copy/paste/save/reopen "
                + "passed; UTF-8 source bytes are stable"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Document Workflow self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

/// Exercises the native keyboard/selection route and the file lifecycle
/// states that can otherwise only be reached through a real window:
/// newline/delete/move commands, selection write-back, clean reload,
/// external-change close prompting, and conflict-safe save.
func runDocumentInteractionSelfCheck(path: String) -> Never {
    let fileManager = FileManager.default
    let sourceURL = URL(fileURLWithPath: path)
    let temporaryURL = fileManager.temporaryDirectory
        .appendingPathComponent("yu-interaction-\(UUID().uuidString).md")
    let emptyURL = fileManager.temporaryDirectory
        .appendingPathComponent("yu-empty-\(UUID().uuidString).md")
    do {
        try fileManager.copyItem(at: sourceURL, to: temporaryURL)
        defer {
            try? fileManager.removeItem(at: temporaryURL)
            try? fileManager.removeItem(at: emptyURL)
        }

        do {
            let bridge = try StorageBridge(path: temporaryURL.path)
            let textView = DocumentTextView(bridge: bridge)
            let sourceBefore = bridge.source
            let end = NSRange(location: sourceBefore.utf16.count, length: 0)
            try bridge.setSelection(end)

            textView.insertText("A", replacementRange: end)
            textView.doCommand(by: #selector(NSResponder.insertNewline(_:)))
            textView.insertText(
                "B",
                replacementRange: NSRange(
                    location: bridge.selection.range.location,
                    length: 0
                )
            )
            precondition(bridge.source.hasSuffix("A\nB"))

            textView.doCommand(by: #selector(NSResponder.moveLeft(_:)))
            textView.doCommand(by: #selector(NSResponder.deleteBackward(_:)))
            precondition(bridge.source.hasSuffix("AB"))
            textView.performUndo()
            precondition(bridge.source.hasSuffix("A\nB"))
            textView.performRedo()
            precondition(bridge.source.hasSuffix("AB"))

            let japanese = (bridge.source as NSString).range(of: "日本語")
            precondition(japanese.location != NSNotFound)
            textView.setSelectedRanges(
                [NSValue(range: japanese)],
                affinity: .downstream,
                stillSelecting: false
            )
            precondition(bridge.selection.range == japanese)

            try bridge.save()
            precondition(!bridge.state.dirty)

            let externalSource = bridge.source + "\n外部版本"
            try externalSource.write(
                to: temporaryURL,
                atomically: true,
                encoding: .utf8
            )
            precondition(bridge.state.disk == .changed)
            try bridge.reload()
            textView.refreshFromRust()
            precondition(bridge.source == externalSource)
            precondition(!bridge.state.dirty)

            let localEnd = NSRange(location: bridge.source.utf16.count, length: 0)
            textView.insertText("本地修改", replacementRange: localEnd)
            precondition(bridge.state.dirty)
            let conflictingSource = externalSource + "\n外部再次修改"
            try conflictingSource.write(
                to: temporaryURL,
                atomically: true,
                encoding: .utf8
            )
            precondition(bridge.state.disk == .changed)

            let close = try bridge.requestClose()
            precondition(close.result == 1)
            precondition(close.close_state >= 3)
            try bridge.resolveClose(UInt8(YU_STORAGE_CLOSE_RESOLVE_CANCEL))
            precondition(bridge.state.closeState == 0)

            do {
                try bridge.save()
                preconditionFailure("external conflict save unexpectedly succeeded")
            } catch BridgeError.operation(let status) {
                precondition(status == StorageStatus.externalChange)
            }
            precondition(bridge.state.dirty)
        }

        do {
            let emptyHandle = FileManager.default.createFile(
                atPath: emptyURL.path,
                contents: Data()
            )
            precondition(emptyHandle)
            let emptyBridge = try StorageBridge(path: emptyURL.path)
            let emptyTextView = DocumentTextView(bridge: emptyBridge)
            precondition(emptyBridge.source.isEmpty)
            precondition(emptyTextView.string.isEmpty)
        }

        do {
            _ = try StorageBridge(
                path: temporaryURL.deletingLastPathComponent()
                    .appendingPathComponent("yu-missing-\(UUID().uuidString).md")
                    .path
            )
            preconditionFailure("missing file unexpectedly opened")
        } catch BridgeError.open(let status) {
            precondition(status != StorageStatus.ok)
        }

        print(
            "Yu Document Interaction self-check: keyboard commands, selection, "
                + "clean reload, external conflict and empty/missing paths passed"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Document Interaction self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

func runShapedProjectionHitTestSelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let revision = bridge.revision
        let size: Float = 14.0
        let maxWidth: Float = 500.0
        let textView = DocumentTextView(bridge: bridge)
        textView.frame = NSRect(x: 0.0, y: 0.0, width: CGFloat(maxWidth), height: 600.0)
        textView.font = NSFont.systemFont(ofSize: CGFloat(size))
        let pointerWidth = Float(
            max(textView.bounds.width - 2.0 * textView.contentOrigin.x, 1.0)
        )

        let hit = try bridge.projectionHitTest(
            revision: revision,
            point: CGPoint(x: 0.0, y: 0.0),
            size: size,
            maxWidth: pointerWidth
        )
        precondition(hit.revision == revision)
        precondition(hit.sourceUTF16 <= UInt64(bridge.source.utf16.count))
        precondition(hit.roundTripSourceUTF16 <= UInt64(bridge.source.utf16.count))
        precondition(hit.point.x.isFinite && hit.point.y.isFinite)
        precondition(hit.line == 0)
        precondition(
            textView.applyVisualPointerSelectionForSelfCheck(
                at: NSPoint(x: 0.0, y: 0.0)
            )
        )
        precondition(bridge.selection.range.location == 0)

        // 用 Rust 自己的 caret 几何反推指针坐标，而不是再建一套 TextKit
        // 布局来求点：这样断言的是「caret 几何与 hit-test 互为逆运算」，
        // 属于 Rust 内部自洽性，不引入第二套布局系统（不变量 E1、I5）。
        let sourceEnd = (bridge.source as NSString).range(of: "粗体")
        precondition(sourceEnd.location != NSNotFound)
        let sourceEndUTF16 = UInt64(sourceEnd.location + sourceEnd.length)
        // 由 Rust 自己定位所属块：平台不需要先拿到 viewport 的块列表再挑一个，
        // 那等于把布局几何搬到平台侧（不变量 I3）。
        let endCaret = try bridge.sourceCaret(
            revision: revision,
            sourceUTF16: sourceEndUTF16,
            affinity: 0,
            size: size,
            maxWidth: pointerWidth
        )
        precondition(endCaret.revision == revision)
        precondition(endCaret.point.x.isFinite && endCaret.point.y.isFinite)
        let endPoint = endCaret.point
        precondition(
            textView.applyVisualPointerSelectionForSelfCheck(
                at: endPoint,
                extending: false
            )
        )
        precondition(
            textView.applyVisualPointerSelectionForSelfCheck(
                at: NSPoint(x: 0.0, y: 0.0),
                extending: true
            )
        )
        let endpoints = bridge.selectionEndpoints
        precondition(endpoints.anchorUTF16 > endpoints.focusUTF16)
        precondition(bridge.selection.range.location == Int(endpoints.focusUTF16))

        var staleRejected = false
        do {
            _ = try bridge.projectionHitTest(
                revision: revision + 1,
                point: CGPoint(x: 0.0, y: 0.0),
                size: size,
                maxWidth: pointerWidth
            )
        } catch BridgeError.operation(let status) {
            staleRejected = status == 13
        }
        precondition(staleRejected)
        print(
            "Yu shaped projection hit-test self-check: CoreText point→visual→source "
                + "mapping is Revision-bound (visual UTF-16 \(hit.visualUTF16)); "
                + "reverse drag preserves anchor/focus"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu shaped projection hit-test self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}





func runShapedVerticalSelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let revision = bridge.revision
        let size: Float = 14.0
        let maxWidth: Float = 500.0
        let firstLineEnd = (bridge.source as NSString).range(of: "\n").location
        precondition(firstLineEnd != NSNotFound)
        try bridge.setSelection(
            NSRange(location: firstLineEnd, length: 0),
            affinity: 1
        )
        let first = try bridge.executeShapedVerticalCommand(
            14,
            size: size,
            maxWidth: maxWidth
        )
        precondition(first.revision == revision)
        precondition(first.selection.length == 0)
        precondition(first.selection.location > firstLineEnd)
        let second = try bridge.executeShapedVerticalCommand(
            14,
            size: size,
            maxWidth: maxWidth
        )
        precondition(second.revision == revision)
        precondition(second.selection.location > first.selection.location)
        precondition(bridge.revision == revision)
        print(
            "Yu Shaped Vertical self-check: CoreText line movement preserved "
                + "Revision=\(revision), focus=\(second.selection.location)"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Shaped Vertical self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}












func unwrapSelfCheck<T>(_ value: T?) throws -> T {
    guard let value else {
        throw BridgeError.operation(14)
    }
    return value
}

func runMacosTableResizeCoordinatorSelfCheck(path: String) -> Never {
    do {
        var pointerState = TableResizePointerState()
        precondition(
            pointerState.begin(
                revision: 7,
                kind: UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN)
            )
        )
        precondition(!pointerState.finish(revision: 8))
        precondition(pointerState.isActive)
        precondition(pointerState.cancel(revision: 7))
        precondition(!pointerState.isActive)

        let bridge = try StorageBridge(path: path)
        let revision = bridge.revision
        let size: Float = 14.0
        let maxWidth: Float = 500.0

        let surfaceView = MacosSurfaceHostView(
            frame: NSRect(x: 0.0, y: 0.0, width: 500.0, height: 1000.0)
        )
        let scrollView = NSScrollView(
            frame: NSRect(x: 0.0, y: 0.0, width: 500.0, height: 1000.0)
        )
        scrollView.documentView = NSView(
            frame: NSRect(x: 0.0, y: 0.0, width: 500.0, height: 1000.0)
        )
        let coordinator = MacosSurfaceHostCoordinator(bridge: bridge, fontSize: CGFloat(size))
        coordinator.bind(
            surfaceView: surfaceView,
            scrollView: scrollView,
            fontSize: CGFloat(size)
        )
        coordinator.setHorizontalContentInset(max(surfaceView.bounds.width - CGFloat(maxWidth), 0))
        // Production AX queries wait for an attached, submitted surface even
        // before NSWindow exists. Headless gesture tests obtain their fixture
        // geometry explicitly from Rust; real-window tests cover published AX.
        precondition(coordinator.tableResizeAccessibilityDividers().isEmpty)
        func headlessDividers() throws -> [NativeTableResizeAccessibilityDivider] {
            try bridge.tableResizeAccessibilityDividers(
                revision: bridge.revision, size: size, maxWidth: maxWidth,
                scrollY: 0, viewportHeight: 1000
            )
        }
        let sourceBeforeResize = bridge.source
        let accessibilityDividers = try headlessDividers()
        guard let accessibilityDivider = accessibilityDividers.first(where: {
            $0.kind == UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN)
        }) else {
            throw BridgeError.operation(StorageStatus.invalidSelection)
        }
        precondition(accessibilityDivider.revision == revision)
        precondition(accessibilityDivider.columnCount >= 2)
        precondition(accessibilityDivider.rect.height > 0.0)
        let tableY = accessibilityDivider.rect.midY
        let dividerPoint = NSPoint(x: accessibilityDivider.rect.midX, y: tableY)
        let nearest = try bridge.tableResizeAtDocumentPoint(
            revision: revision,
            action: UInt8(YU_STORAGE_TABLE_RESIZE_PROBE),
            size: size,
            maxWidth: maxWidth,
            point: dividerPoint,
            tolerance: maxWidth
        )
        precondition(nearest.kind == YU_STORAGE_TABLE_RESIZE_COLUMN)
        precondition(nearest.index == accessibilityDivider.index)
        precondition(coordinator.tableResizeHover(at: dividerPoint))
        precondition(
            !coordinator.tableResizeHover(
                at: NSPoint(x: CGFloat(maxWidth) + 100.0, y: tableY)
            )
        )
        precondition(coordinator.beginTableResize(at: dividerPoint))
        precondition(coordinator.tableResizeActiveForSelfCheck)
        precondition(
            coordinator.updateTableResize(
                at: NSPoint(x: dividerPoint.x + 1.0, y: dividerPoint.y)
            )
        )
        precondition(coordinator.finishTableResize())
        precondition(!coordinator.tableResizeActiveForSelfCheck)
        precondition(bridge.source == sourceBeforeResize)

        // Probe before any accessibility query can refresh the layout.
        let resumedResize = coordinator.beginTableResize(at: dividerPoint)
        if !resumedResize {
            let dividers = try headlessDividers()
            fputs("Table resize restart failed: original=\(accessibilityDivider.rect) current=\(dividers.map { $0.rect }) point=\(dividerPoint)\n", stderr)
        }
        precondition(resumedResize)
        precondition(coordinator.cancelTableResize())
        precondition(!coordinator.tableResizeActiveForSelfCheck)

        precondition(coordinator.beginTableResize(at: dividerPoint))
        _ = try bridge.insertText("x")
        coordinator.resetTableResizeAfterDocumentChange()
        precondition(!coordinator.tableResizeActiveForSelfCheck)
        precondition(!coordinator.updateTableResize(at: dividerPoint))
        coordinator.detach()
        precondition(coordinator.tableResizeAccessibilityDividers().isEmpty)
        print(
            "Yu macOS table resize coordinator self-check: document-space CoreText hit, "
                + "Accessibility divider descriptor, mouse update/finish/cancel, stale revision reset "
                + "and deferred pre-surface Accessibility are valid"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu macOS table resize coordinator self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

func runMacosTaskCheckboxSelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let revision = bridge.revision
        let size: Float = 14.0
        let maxWidth: Float = 500.0
        _ = try bridge.macosRenderHostFrame(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 1_000.0,
            surfaceGeneration: 0,
            appearance: UInt8(YU_STORAGE_APPEARANCE_LIGHT)
        )
        // 用 Rust 自己的 point→source 映射找出待办那一行的纵坐标，再沿这一行
        // 向右找出 checkbox 的可命中点。此前是把整份 RenderPlan 取过 ABI 再从
        // 里面挑一条 TASK_CHECKBOX 指令——RenderPlan 不跨 C ABI（不变量 I2）。
        let sourceString = bridge.source as NSString
        let markerRange = sourceString.range(of: "- [ ] todo")
        precondition(markerRange.location != NSNotFound)
        // 边界 bias 会让空块/段尾命中共享边界（即 markerRange 的起点），所以
        // 「第一个映射进 marker 区间的 y」可能是空块顶而不是待办行本身——M3
        // 行高 1.6 + 块间距把两者拉开了 ~68pt，固定探测窗够不着。对每个候选 y
        // 实际探测 checkbox，第一个真正命中的才是待办行；探测本身才是判据，
        // 投影命中只负责把扫描范围圈到这一行附近。
        var found: (point: NSPoint, hit: NativeTaskCheckboxHit)?
        outer: for step in 0..<200 {
            let y = CGFloat(step) * 2.0
            guard let hit = try? bridge.projectionHitTest(
                revision: revision,
                point: CGPoint(x: 1.0, y: y),
                size: size,
                maxWidth: maxWidth
            ), hit.sourceUTF16 >= UInt64(markerRange.location),
               hit.sourceUTF16 <= UInt64(NSMaxRange(markerRange)) else { continue }
            // 在这一行附近扫描出一个真正命中 checkbox 的点。平台已经拿不到任何
            // 绘制几何，只能像用户点击那样去试——这正是这条路径该被测的样子。
            for dy in stride(from: -8.0, through: 24.0, by: 2.0) {
                for dx in stride(from: 0.0, through: 48.0, by: 2.0) {
                    let probe = NSPoint(x: dx, y: y + dy)
                    if let hit = try? bridge.taskCheckboxHitTest(
                        revision: revision,
                        point: probe
                    ) {
                        found = (probe, hit)
                        break outer
                    }
                }
            }
        }
        guard let (point, publishedHit) = found else {
            throw BridgeError.operation(StorageStatus.invalidSelection)
        }
        precondition(publishedHit.revision == revision)
        precondition(publishedHit.markerRange.length == 3)
        precondition(publishedHit.bounds.contains(point))

        let textView = DocumentTextView(bridge: bridge)
        var documentChanges = 0
        textView.onDocumentChange = { documentChanges += 1 }
        textView.onTaskCheckboxPress = { [weak textView] point in
            guard let textView,
                  let hit = try? bridge.taskCheckboxHitTest(
                      revision: bridge.revision,
                      point: point
                  ) else {
                return false
            }
            return textView.toggleTaskPointerHit(hit)
        }
        let sourceBefore = bridge.source
        precondition(textView.pressTaskCheckboxForSelfCheck(at: point))
        precondition(documentChanges == 1)
        precondition(bridge.revision == revision + 1)
        precondition(bridge.source != sourceBefore)
        precondition(bridge.source.contains("- [x] todo"))
        do {
            _ = try bridge.taskCheckboxHitTest(revision: revision, point: point)
            preconditionFailure("stale task checkbox publication was accepted")
        } catch BridgeError.operation(let status) {
            precondition(status == StorageStatus.staleRevision)
        }
        precondition(
            !textView.pressTaskCheckboxForSelfCheck(
                at: NSPoint(x: publishedHit.bounds.maxX + 20.0, y: publishedHit.bounds.midY)
            )
        )
        print(
            "Yu macOS task checkbox self-check: published hit, canonical toggle and stale Revision rejection are valid"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu macOS task checkbox self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

/// 大纲面板的 headless self-check。
///
/// `NSOutlineView` 与 `DocumentTextView` 一样是纯 AppKit 对象：`main.swift`
/// 第一行的 `NSApplication.shared` 已经初始化了 AppKit，`reloadData` /
/// `expandItem` / `selectRowIndexes` 都不需要窗口，也不需要 run loop。真正画
/// 出来的那一层（宽度、深浅色、焦点环）留给人工验收。
///
/// **判据不能来自被测的那条路。**「面板的条数与 FFI 一致」是自证的——面板
/// 本来就是照着那个数组画的。下面四条各有内容：
///
///   1. 还原那棵树：断言落在 **NSOutlineView 眼里的形状**上，反过来核对平表的
///      `parent` 字段。壳按 `child_count` 挂树，判据按 `parent` 核对——两个
///      字段互为对方的参照，挂错父亲、静默把孩子提成根都在这里出。
///   2. 点第 N 行之后光标落在第 N 条标题的正文起点——判据是
///      `bridge.selection`，与面板走的是两条路。
///   3. 那之后 `shapedCaretScrollRequest` 指向那一条的块。这一条压住
///      「面板自己算 y」：滚动必须由 yu-editor::viewport 那条路给出。
///   4. 编辑之后刷新，展开状态与选中行不丢。纯 Swift 状态逻辑，每次
///      `reloadData` 全量重建就会丢，而且不报错。
///
/// **标签那一组断言不在这里了。** 剥标记、折行、身份链都挪进了
/// `yu-editor::OutlineTree`（第七刀 c 的第三块），它们的性质落在
/// `yu-editor/tests/panel.rs`；在壳里再断言一遍就是自证——那两串字是 Rust
/// 直接交下来的。这里只留一条端到端的：真 fixture 经过真 ABI 之后，面板上
/// 那一行确实不带 `**`。
func runOutlinePanelSelfCheck(path: String) -> Never {
    let fileManager = FileManager.default
    let temporaryURL = fileManager.temporaryDirectory
        .appendingPathComponent("yu-outline-\(UUID().uuidString).md")
    do {
        try fileManager.copyItem(at: URL(fileURLWithPath: path), to: temporaryURL)
        defer { try? fileManager.removeItem(at: temporaryURL) }

        // Exercise wrapping at the product's supported 180pt sidebar minimum.
        let outlineSource = try String(contentsOf: temporaryURL, encoding: .utf8)
        try (outlineSource + "\n# Native paragraph acceptance 原生段落验收\n\n# 😀 *斜体* **粗体**\n\n# **&amp;** *&NotEqualTilde;* &#x1F600;\n")
            .write(to: temporaryURL, atomically: true, encoding: .utf8)

        let bridge = try StorageBridge(path: temporaryURL.path)
        let textView = DocumentTextView(bridge: bridge)
        let panel = OutlinePanel()
        // 产品里的接线是同一句（DocumentViewController.loadView）。被测的是
        // `navigateToOutlineItem` 那一份实现，不是这里重写的一份。
        panel.onSelect = { [weak textView] item in
            textView?.navigateToOutlineItem(item)
        }

        let items = try unwrapSelfCheck(bridge.outlineItemsIfAvailable)
        precondition(items.count >= 6, "fixture 里的标题太少，压不住层级")
        let entityHeading = try unwrapSelfCheck(items.first { $0.label == "& ≂\u{338} 😀" })
        precondition(entityHeading.styleRuns.contains { $0.range == NSRange(location: 0, length: 1) && $0.traits == 1 })
        precondition(entityHeading.styleRuns.contains { $0.range == NSRange(location: 2, length: 2) && $0.traits == 2 })
        let styled = try unwrapSelfCheck(items.first { $0.label == "😀 斜体 粗体" })
        let italicRange = (styled.label as NSString).range(of: "斜体")
        let boldRange = (styled.label as NSString).range(of: "粗体")
        precondition(italicRange.location == 3, "emoji must occupy two UTF-16 units")
        precondition(styled.styleRuns.contains { $0.range == italicRange && $0.traits == 2 })
        precondition(styled.styleRuns.contains { $0.range == boldRange && $0.traits == 1 })
        for dark in [false, true] {
            let label = panel.attributedLabelForSelfCheck(styled, dark: dark)
            let italic = try unwrapSelfCheck(label.attribute(.font, at: italicRange.location, effectiveRange: nil) as? NSFont)
            let bold = try unwrapSelfCheck(label.attribute(.font, at: boldRange.location, effectiveRange: nil) as? NSFont)
            precondition(NSFontManager.shared.traits(of: italic).contains(.italicFontMask))
            precondition(NSFontManager.shared.traits(of: bold).contains(.boldFontMask))
            precondition(label.string == styled.label, "native styling must preserve the Rust label")
            let activeLabel = panel.attributedLabelForSelfCheck(styled, dark: dark, active: true)
            let activeItalic = try unwrapSelfCheck(activeLabel.attribute(.font, at: italicRange.location, effectiveRange: nil) as? NSFont)
            let activePlain = try unwrapSelfCheck(activeLabel.attribute(.font, at: 2, effectiveRange: nil) as? NSFont)
            precondition(NSFontManager.shared.traits(of: activeItalic).contains([.boldFontMask, .italicFontMask]))
            precondition(NSFontManager.shared.traits(of: activePlain).contains(.boldFontMask))
            precondition(activeLabel.string == styled.label)
        }
        panel.reload(items: items)
        // Reload/expansion notifications must never become navigation commands.
        var reloadNavigations = 0
        let navigate = panel.onSelect
        panel.onSelect = { _ in reloadNavigations += 1 }
        panel.clickRowForSelfCheck(panel.rowCountForSelfCheck - 1)
        reloadNavigations = 0
        for _ in 0..<20 { panel.reload(items: items) }
        precondition(reloadNavigations == 0, "outline reload must not navigate")
        let selectedBeforeHighlight = panel.selectedIdentityForSelfCheck
        for item in items { panel.highlightHeading(containing: item.labelRange.location) }
        precondition(reloadNavigations == 0, "caret highlight must not navigate")
        precondition(panel.selectedIdentityForSelfCheck == selectedBeforeHighlight, "caret highlight must not alter outline selection")
        panel.onSelect = navigate

        // 视觉结构：行高 30、每级缩进 14（设计稿数值；改设计时与 token 同步改）。
        let metrics = panel.rowMetricsForSelfCheck
        precondition(metrics.height == 24.0, "大纲行高应是 24，实际 \(metrics.height)")
        precondition(metrics.indent == 14.0, "大纲每级缩进应是 14，实际 \(metrics.indent)")
        // 区头计数跟着这一版 reload 走。
        precondition(
            panel.sectionHeaderCountForSelfCheck == items.count,
            "区头计数应是 \(items.count)，实际 \(panel.sectionHeaderCountForSelfCheck)"
        )

        // Real native row rectangles must reflow without losing selection or navigating.
        let selectedBeforeResize = panel.selectedIdentityForSelfCheck
        var resizeNavigations = 0
        panel.onSelect = { _ in resizeNavigations += 1 }
        for dark in [false, true] {
            let wide = panel.rowHeightsForSelfCheck(width: 400, dark: dark)
            let narrow = panel.rowHeightsForSelfCheck(width: 180, dark: dark)
            FileHandle.standardError.write(Data("Outline reflow dark=\(dark) wide=\(wide) narrow=\(narrow)\n".utf8))
            precondition(zip(narrow, wide).allSatisfy { $0 >= $1 }, "narrow outline must not lose lines")
            precondition(zip(narrow, wide).contains { $0 > $1 }, "long outline labels must wrap")
            let restored = panel.rowHeightsForSelfCheck(width: 400, dark: dark)
            precondition(restored == wide, "outline resize must restore row geometry")
            precondition(panel.selectedIdentityForSelfCheck == selectedBeforeResize, "outline resize lost selection")
        }
        precondition(resizeNavigations == 0, "outline resize must not navigate")
        panel.onSelect = navigate

        // 1. 平表 → NSOutlineView 眼里的那棵树。
        var visited: [NativeOutlineItem] = []
        func walk(_ nodes: [OutlineNode], parent: OutlineNode?) {
            for node in nodes {
                if let parent {
                    precondition(
                        node.item.parent == parent.item.index,
                        "\(node.label) 挂在了 \(parent.label) 下，但平表说它的父亲是 \(node.item.parent)"
                    )
                    precondition(
                        node.item.level > parent.item.level,
                        "\(node.label) 的级别不比父亲深"
                    )
                } else {
                    precondition(
                        node.item.parent == UInt32.max,
                        "\(node.label) 被挂成了根级，但平表给了它一个父亲"
                    )
                }
                precondition(
                    node.children.count == node.item.childCount,
                    "\(node.label) 挂了 \(node.children.count) 个孩子，平表说是 \(node.item.childCount)"
                )
                visited.append(node.item)
                walk(node.children, parent: node)
            }
        }
        walk(panel.rootsForSelfCheck, parent: nil)
        precondition(visited.count == items.count, "还原丢了或多出了条目")
        precondition(
            visited.map(\.index) == items.map(\.index),
            "前序遍历与文档顺序不一致"
        )

        // 端到端的那一条：真 fixture 经过真 ABI，面板上那一行不带语法标记。
        let labels = panel.rootsForSelfCheck.flatMap(allLabelsForSelfCheck)
        precondition(
            labels.contains("带 行内标记 的标题"),
            "强调的 `**` 没有被剥掉，实际标签: \(labels)"
        )
        precondition(labels.contains("多行 标题"), "Setext 多行标题没有折成一行: \(labels)")
        precondition(
            labels.allSatisfy { !$0.contains(where: \.isNewline) },
            "面板上不能出现换行"
        )

        // 2 / 3. 点每一行 → 选区落在正文起点 → 滚动请求指向那一条的块。
        let rowCount = panel.rowCountForSelfCheck
        precondition(rowCount == items.count, "默认应当全部展开")
        for row in 0..<rowCount {
            let node = try unwrapSelfCheck(panel.nodeForSelfCheck(row: row))
            panel.clickRowForSelfCheck(row)
            precondition(
                bridge.selection.range
                    == NSRange(location: node.item.labelRange.location, length: 0),
                "点第 \(row) 行之后光标不在 \(node.label) 的正文起点"
            )
            let request = try bridge.shapedCaretScrollRequest(
                revision: bridge.revision,
                size: 14.0,
                maxWidth: 500.0,
                scrollY: 0.0,
                viewportHeight: 200.0
            )
            precondition(
                request.blockIndex == node.item.block,
                "滚动请求指向块 \(request.blockIndex)，而 \(node.label) 在块 \(node.item.block)"
            )
        }

        // 4. 编辑之后刷新，展开状态与选中行不丢。
        let collapsible = try unwrapSelfCheck(
            panel.rootsForSelfCheck.first(where: { !$0.children.isEmpty })
        )
        // A selected descendant disappearing must not move the source caret.
        // Restoring visibility must use the original focus without another edit.
        let child = try unwrapSelfCheck(collapsible.children.first)
        let childRow = try unwrapSelfCheck((0..<panel.rowCountForSelfCheck).first {
            panel.nodeForSelfCheck(row: $0)?.identity == child.identity
        })
        panel.clickRowForSelfCheck(childRow)
        let selectionBeforeCollapse = bridge.selection.range
        var hierarchyNavigations = 0
        panel.onSelect = { item in
            hierarchyNavigations += 1
            navigate?(item)
        }
        for dark in [false, true] {
            _ = panel.rowHeightsForSelfCheck(width: 240, dark: dark)
            panel.highlightHeading(containing: child.item.labelRange.location)
            precondition(panel.activeIdentityForSelfCheck == child.identity)
            panel.collapseForSelfCheck(identity: collapsible.identity)
            FileHandle.standardError.write(Data("Outline collapsed dark=\(dark) active=\(String(describing: panel.activeIdentityForSelfCheck)) expected=\(collapsible.identity) navigations=\(hierarchyNavigations)\n".utf8))
            precondition(panel.activeIdentityForSelfCheck == collapsible.identity,
                         "hidden active heading must highlight its visible ancestor")
            panel.reload(items: items)
            precondition(panel.activeIdentityForSelfCheck == collapsible.identity,
                         "reload must retain the collapsed active ancestor")
            panel.expandForSelfCheck(identity: collapsible.identity)
            FileHandle.standardError.write(Data("Outline expanded dark=\(dark) active=\(String(describing: panel.activeIdentityForSelfCheck)) expected=\(child.identity) navigations=\(hierarchyNavigations)\n".utf8))
            precondition(panel.activeIdentityForSelfCheck == child.identity,
                         "expansion must restore the original active descendant")
        }
        precondition(hierarchyNavigations == 0, "outline collapse/expand must not navigate")
        precondition(bridge.selection.range == selectionBeforeCollapse,
                     "outline hierarchy changes must preserve the source caret")
        panel.onSelect = navigate
        panel.collapseForSelfCheck(identity: collapsible.identity)
        let selectedRow = rowCount - 1
        panel.clickRowForSelfCheck(min(selectedRow, panel.rowCountForSelfCheck - 1))
        let expandedBefore = panel.expandedIdentitiesForSelfCheck
        let selectedBefore = try unwrapSelfCheck(panel.selectedIdentityForSelfCheck)
        precondition(!expandedBefore.contains(collapsible.identity))

        // 在**文档最前面**插一条新标题：这会把后面每一条的 index 与 block
        // 一起推后一位。展开状态与选中行因此不能按下标记，只能按身份记——
        // 在末尾追加字符是压不住这一条的，那种编辑谁都活得下来。
        let revisionBefore = bridge.revision
        let head = NSRange(location: 0, length: 0)
        try bridge.setSelection(head)
        textView.insertText("# 新的顶层\n\n", replacementRange: head)
        precondition(bridge.revision != revisionBefore, "编辑没有推进 Revision")
        let refreshed = try unwrapSelfCheck(bridge.outlineItemsIfAvailable)
        precondition(refreshed.count == items.count + 1, "新标题没有进大纲")
        let shift = refreshed[1].block - items[0].block
        precondition(shift > 0, "新标题没有把后面的块推后")
        precondition(
            zip(items, refreshed.dropFirst()).allSatisfy {
                $0.index + 1 == $1.index && $0.block + shift == $1.block
            },
            "这次编辑应当把每一条的 index 与 block 一起推后"
        )
        panel.reload(items: refreshed)

        precondition(
            panel.sectionHeaderCountForSelfCheck == refreshed.count,
            "刷新之后区头计数没跟上：\(panel.sectionHeaderCountForSelfCheck) != \(refreshed.count)"
        )

        precondition(
            panel.expandedIdentitiesForSelfCheck == expandedBefore,
            "刷新之后展开状态变了: \(panel.expandedIdentitiesForSelfCheck) != \(expandedBefore)"
        )
        precondition(
            panel.selectedIdentityForSelfCheck == selectedBefore,
            "刷新之后选中行丢了"
        )

        print(
            "Yu Outline Panel self-check: items=\(items.count) rows=\(rowCount) "
                + "roots=\(panel.rootsForSelfCheck.count); "
                + "nesting, navigation, viewport-owned scroll and refresh state passed"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Outline Panel self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

/// 搜索面板的 headless self-check。
///
/// **上下文裁剪与剥标记那一组断言不在这里了**：它们随逻辑挪进了
/// `yu-editor::SearchResults`，性质落在 `yu-editor/tests/panel.rs`（含那条
/// 语料造不出来、只能手造的「块比行窄」）。这里留下的都是壳自己的事——
/// 计数标签、点行与高亮的往返、环回、重扫。
func runSearchPanelSelfCheck(path: String) -> Never {
    let fileManager = FileManager.default
    let temporaryURL = fileManager.temporaryDirectory
        .appendingPathComponent("yu-search-\(UUID().uuidString).md")
    do {
        try fileManager.copyItem(at: URL(fileURLWithPath: path), to: temporaryURL)
        defer { try? fileManager.removeItem(at: temporaryURL) }

        let bridge = try StorageBridge(path: temporaryURL.path)
        let textView = DocumentTextView(bridge: bridge)
        let panel = SearchPanel()
        // 产品里的接线是同一句（DocumentViewController.loadView）。被测的是
        // `navigateToSearchMatch` 那一份实现，不是这里重写的一份。
        panel.onSelect = { [weak textView] match in
            textView?.navigateToSearchMatch(match)
        }

        let rowsFor: (String) throws -> [NativeSearchMatch] = { query in
            precondition(bridge.setSearchQuery(query), "设查询失败")
            let matches = try unwrapSelfCheck(bridge.searchMatchesIfAvailable)
            panel.reload(rows: matches, query: query)
            return matches
        }

        // 1. 端到端：真 fixture 经过真 ABI，结果那一行不带语法标记。
        let rows = try rowsFor("标记")
        precondition(rows.count == 6, "fixture 里应当有六处命中，实际 \(rows.count)")
        // 视觉结构：结果行高与大纲同款 30（设计稿数值；改设计时同步这里）。
        precondition(
            panel.rowHeightForSelfCheck == 24.0,
            "搜索结果行高应是 24，实际 \(panel.rowHeightForSelfCheck)"
        )
        precondition(
            panel.rowCountForSelfCheck == rows.count,
            "面板画了 \(panel.rowCountForSelfCheck) 行"
        )
        let labels = rows.map(\.label)
        precondition(
            labels.allSatisfy { !$0.contains("**") },
            "结果那一行还带着强调的 `**`: \(labels)"
        )
        precondition(
            labels.contains("搜索的标记测试"),
            "标题那一处没有剥掉 `**`: \(labels)"
        )
        precondition(
            labels.contains("列表项里的标记"),
            "列表项的 `- ` 没有被剥掉: \(labels)"
        )
        precondition(
            labels.contains("引用块里的标记"),
            "引用块的 `> ` 没有被剥掉: \(labels)"
        )
        // 同一行上的两处命中是两行结果，显示同一段上下文，但指向不同的位置。
        // 「按行去重」会让第二处点不到——不报错，只是少一条。
        let sameLine = rows.filter { $0.label.contains("第二个标记") }
        precondition(sameLine.count == 2, "同一行上的两处命中应当是两行结果")
        precondition(
            sameLine[0].range != sameLine[1].range,
            "两行结果指向了同一处命中"
        )
        precondition(panel.countTextForSelfCheck == "6 处匹配", panel.countTextForSelfCheck)

        // 2. 点第 N 行 → 选区落在第 N 处命中。判据来自 bridge.selection，
        //    与面板走的是两条路。选中而不是只放光标：Rust 侧的「当前命中」
        //    要求选区恰好等于那一段。
        for row in 0..<panel.rowCountForSelfCheck {
            let entry = try unwrapSelfCheck(panel.rowForSelfCheck(row))
            panel.clickRowForSelfCheck(row)
            precondition(
                bridge.selection.range == entry.range,
                "点第 \(row) 行之后选区是 \(bridge.selection.range)，"
                    + "而那一处命中在 \(entry.range)"
            )
            // 3. 「当前命中」由选区推出来，所以列表上高亮的必须是同一行。
            panel.highlightRow(matching: bridge.selection.range)
            precondition(
                panel.selectedRowForSelfCheck == row,
                "选区落在第 \(row) 处命中，列表上高亮的却是第 "
                    + "\(panel.selectedRowForSelfCheck) 行"
            )
        }

        // 4. 「下一个」环回：最后一处之后是第一处。这一条压的是环回那一支，
        //    普通的「往后走一格」压不住它。
        let matches = try unwrapSelfCheck(bridge.searchMatchesIfAvailable)
        let last = try unwrapSelfCheck(matches.last)
        textView.navigateToSearchMatch(last)
        let wrapped = try unwrapSelfCheck(
            SearchResults.next(after: bridge.selection.range, in: matches, forward: true)
        )
        precondition(wrapped.range == matches[0].range, "最后一处的下一个应当环回到第一处")
        // 反向从第一处环回到最后一处。
        textView.navigateToSearchMatch(matches[0])
        let wrappedBack = try unwrapSelfCheck(
            SearchResults.next(after: bridge.selection.range, in: matches, forward: false)
        )
        precondition(wrappedBack.range == last.range, "第一处的上一个应当环回到最后一处")

        // 5. 选区离开任何命中之后，列表上不该还有高亮。
        try bridge.setSelection(NSRange(location: 0, length: 0))
        panel.highlightRow(matching: bridge.selection.range)
        precondition(
            panel.selectedRowForSelfCheck < 0,
            "光标不在任何命中上，列表却还高亮着第 \(panel.selectedRowForSelfCheck) 行"
        )

        // 6. 编辑之后必须重扫。在**文首**插入会把每一处命中一起推后——在末尾
        //    追加是压不住这一条的。
        let before = try unwrapSelfCheck(bridge.searchMatchesIfAvailable)
        let head = NSRange(location: 0, length: 0)
        try bridge.setSelection(head)
        textView.insertText("新", replacementRange: head)
        let after = try unwrapSelfCheck(bridge.searchMatchesIfAvailable)
        precondition(after.count == before.count, "编辑丢掉了命中")
        precondition(
            zip(before, after).allSatisfy { $0.range.location + 1 == $1.range.location },
            "编辑之后命中没有整体推后——不重扫的话它们会停在旧位置"
        )

        // 7. 查不到东西是 0 行，不是错误。
        let none = try rowsFor("这四个字一定不在里面")
        precondition(none.isEmpty, "不该有匹配")
        precondition(panel.rowCountForSelfCheck == 0, "面板没有清空")
        precondition(panel.countTextForSelfCheck == "没有匹配", panel.countTextForSelfCheck)

        print(
            "Yu Search Panel self-check: matches=\(rows.count) "
                + "labels=\(labels.count); stripping, navigation, wrap-around "
                + "and re-scan passed"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Search Panel self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

private func allLabelsForSelfCheck(_ node: OutlineNode) -> [String] {
    [node.label] + node.children.flatMap(allLabelsForSelfCheck)
}

private func checkHiddenContentNavigation() throws {
    let temporary = FileManager.default.temporaryDirectory.appendingPathComponent("yu-hidden-navigation-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: temporary) }
    let source = "[Jump](#target)\n\n<details><summary>First</summary><p id='target'>First hidden</p></details>\n\n<details><summary>Second</summary><p>Second hidden</p></details>"
    try source.write(to: temporary, atomically: true, encoding: .utf8)
    let bridge = try StorageBridge(path: temporary.path)
    let view = DocumentTextView(bridge: bridge)
    let revision = bridge.revision
    let first = (source as NSString).range(of: "First hidden")
    let second = (source as NSString).range(of: "Second hidden")
    let firstHeader = (source as NSString).range(of: "First</summary>").location
    let secondHeader = (source as NSString).range(of: "Second</summary>").location
    view.navigate(toSources: [first, second], primary: 1)
    precondition((try? bridge.disclosureHeader(at: firstHeader, expectedRevision: revision))?.open == true)
    precondition((try? bridge.disclosureHeader(at: secondHeader, expectedRevision: revision))?.open == true)
    precondition(bridge.selectionsIfAvailable?.ranges.count == 2)
    precondition(view.toggleDisclosure(at: firstHeader, revision: revision))
    precondition((try? bridge.disclosureHeader(at: firstHeader, expectedRevision: revision))?.open == false)
    let link = (source as NSString).range(of: "Jump").location
    precondition(view.openDocumentLink(at: link, revision: revision))
    precondition((try? bridge.disclosureHeader(at: firstHeader, expectedRevision: revision))?.open == true)
    precondition(view.selectedRange().location == (source as NSString).range(of: "<p id='target'>").location)
    precondition(bridge.revision == revision && view.string == source)
}

private func checkDisclosureAccessibility() throws {
    let temporary = FileManager.default.temporaryDirectory.appendingPathComponent("yu-disclosure-ax-\(UUID().uuidString).md")
    defer { try? FileManager.default.removeItem(at: temporary) }
    let original = "<details><summary><strong>摘要中文</strong> &amp; <em>内容</em></summary><p>Hidden body</p></details>"
    try original.write(to: temporary, atomically: true, encoding: .utf8)
    let bridge = try StorageBridge(path: temporary.path)
    let view = DocumentTextView(bridge: bridge)
    func disclosure() -> YuAccessibilitySemanticElement? {
        func find(_ elements: [Any]) -> YuAccessibilitySemanticElement? {
            for case let element as YuAccessibilitySemanticElement in elements {
                if element.node.kind == SemanticAccessibilityKind.disclosure.rawValue { return element }
                if let result = find(element.semanticChildren) { return result }
            }
            return nil
        }
        return find(view.accessibilityChildren ?? [])
    }
    guard let closed = disclosure() else { preconditionFailure("Missing disclosure AX element") }
    precondition(closed.accessibilityRole == .disclosureTriangle)
    precondition(closed.accessibilityLabel == "摘要中文 & 内容")
    precondition((closed.accessibilityValue as? NSNumber)?.boolValue == false)
    precondition(closed.accessibilityPerformPress())
    precondition(!closed.accessibilityPerformPress(), "Stale AX action must be rejected")
    precondition((disclosure()?.accessibilityValue as? NSNumber)?.boolValue == true)
    let summary = (view.string as NSString).range(of: "摘要中文")
    view.setSelectedRanges([NSValue(range: NSRange(location: summary.location, length: 0))], affinity: .downstream, stillSelecting: false)
    let item = view.makeDisclosureMenuItem()
    precondition(view.validateMenuItem(item))
    view.toggleDisclosureFromMenu(item)
    precondition((disclosure()?.accessibilityValue as? NSNumber)?.boolValue == false)
    view.undo(nil)
    precondition((disclosure()?.accessibilityValue as? NSNumber)?.boolValue == true)
    view.undo(nil)
    precondition(view.string == original)
    let revision = bridge.revision
    let hidden = (original as NSString).range(of: "Hidden body")
    view.navigate(toSource: hidden)
    precondition(view.selectedRange() == hidden)
    precondition((disclosure()?.accessibilityValue as? NSNumber)?.boolValue == true)
    precondition(bridge.revision == revision && view.string == original)
    precondition(disclosure()?.accessibilityPerformPress() == true)
    precondition((disclosure()?.accessibilityValue as? NSNumber)?.boolValue == false)
    precondition(bridge.revision == revision && view.string == original)
    let visibleCaret = (original as NSString).range(of: "</summary>").location
    precondition(view.selectedRange() == NSRange(location: visibleCaret, length: 0))
    _ = try bridge.insertText("!")
    view.refreshFromRust()
    precondition(view.string == original.replacingOccurrences(of: "</summary>", with: "!</summary>"))
    view.undo(nil)
    precondition(view.string == original)

    try bridge.setSourceMode(true)
    view.refreshFromRust()
    try bridge.setSelection(NSRange(location: hidden.location, length: 0))
    let modeRevision = bridge.revision
    try bridge.setSourceMode(false)
    view.refreshFromRust()
    precondition(view.selectedRange() == NSRange(location: hidden.location, length: 0))
    precondition((disclosure()?.accessibilityValue as? NSNumber)?.boolValue == true)
    precondition(bridge.revision == modeRevision && view.string == original)
    _ = try bridge.insertText("X")
    view.refreshFromRust()
    precondition(disclosure()?.accessibilityPerformPress() == true)
    precondition((disclosure()?.accessibilityValue as? NSNumber)?.boolValue == false)
    view.undo(nil)
    precondition(view.string == original)
    precondition(view.selectedRange() == NSRange(location: hidden.location, length: 0))
    precondition((disclosure()?.accessibilityValue as? NSNumber)?.boolValue == true)
    precondition(disclosure()?.accessibilityPerformPress() == true)
    view.redo(nil)
    precondition(view.string == original.replacingOccurrences(of: "Hidden body", with: "XHidden body"))
    precondition((disclosure()?.accessibilityValue as? NSNumber)?.boolValue == true)
    view.undo(nil)
    precondition(view.string == original)

}

func runAccessibilitySelfCheck(path: String) -> Never {
    do {
        try checkDisclosureAccessibility()
        try checkHiddenContentNavigation()
        let bridge = try StorageBridge(path: path)
        let textView = DocumentTextView(bridge: bridge)
        let initialRevision = bridge.revision
        let initialChildren = (textView.accessibilityChildren ?? [])
            .compactMap { $0 as? YuAccessibilitySemanticElement }

        func validate(
            _ elements: [YuAccessibilitySemanticElement],
            parent: AnyObject,
            revision: UInt64
        ) -> Int {
            var count = 0
            for element in elements {
                precondition(element.node.revision == revision)
                precondition(element.parentObject === parent)
                precondition(element.accessibilityLabel != nil)
                count += 1
                let children = element.semanticChildren
                    .compactMap { $0 as? YuAccessibilitySemanticElement }
                count += validate(children, parent: element, revision: revision)
            }
            return count
        }

        func flatten(_ elements: [YuAccessibilitySemanticElement]) -> [YuAccessibilitySemanticElement] {
            var result: [YuAccessibilitySemanticElement] = []
            for element in elements {
                result.append(element)
                result.append(contentsOf: flatten(element.semanticChildren.compactMap {
                    $0 as? YuAccessibilitySemanticElement
                }))
            }
            return result
        }

        let initialCount = validate(initialChildren, parent: textView, revision: initialRevision)
        let allInitial = flatten(initialChildren)
        let headings = allInitial.filter { $0.node.kind == SemanticAccessibilityKind.heading.rawValue }
        let links = allInitial.filter {
            $0.node.kind == SemanticAccessibilityKind.link.rawValue
                || $0.node.kind == SemanticAccessibilityKind.autolink.rawValue
                || $0.node.kind == SemanticAccessibilityKind.referenceLink.rawValue
        }
        let tasks = allInitial.filter {
            $0.node.kind == SemanticAccessibilityKind.taskListItem.rawValue
        }
        if !headings.isEmpty {
            precondition(headings.allSatisfy { $0.accessibilityRole == .staticText })
        }
        if !links.isEmpty {
            precondition(links.allSatisfy { $0.accessibilityRole == .link })
            precondition(
                links
                    .filter { $0.node.destinationRange != nil }
                    .allSatisfy { $0.accessibilityURL != nil }
            )
        }
        if !tasks.isEmpty {
            precondition(tasks.allSatisfy { $0.accessibilityRole == .checkBox })
            precondition(tasks.allSatisfy { $0.accessibilityValue is NSNumber })
        }
        var openedLinkURLs: [URL] = []
        textView.linkURLOpener = { url in openedLinkURLs.append(url); return true }
        for link in links {
            let expected = link.accessibilityURL
            precondition(expected != nil)
            precondition(link.accessibilityPerformPress())
            precondition(openedLinkURLs.last == expected)
        }
        precondition(openedLinkURLs.count == links.count)
        let linkMenus = links.map { textView.linkMenuItems(at: $0.node.labelRange.location) }
        for (link, items) in zip(links, linkMenus) {
            precondition(items.map(\.title) == ["打开链接", "复制链接地址"])
            precondition(items.allSatisfy { textView.validateMenuItem($0) })
            precondition(NSApp.sendAction(items[0].action!, to: textView, from: items[0]))
            precondition(openedLinkURLs.last == link.accessibilityURL)
            precondition(NSApp.sendAction(items[1].action!, to: textView, from: items[1]))
            precondition(NSPasteboard.general.string(forType: .string) == (try? bridge.linkDestination(at: link.node.labelRange.location, expectedRevision: initialRevision)))
        }
        precondition(
            allInitial
                .filter { $0.node.kind != SemanticAccessibilityKind.taskListItem.rawValue && !links.contains($0) }
                .allSatisfy { !$0.accessibilityPerformPress() }
        )

        let rotors = textView.accessibilityCustomRotors ?? []
        precondition(rotors.count == 2)
        for (index, rotor) in rotors.enumerated() {
            let parameters = NSAccessibilityCustomRotor.SearchParameters()
            parameters.searchDirection = .next
            parameters.filterString = ""
            guard let delegate = rotor.itemSearchDelegate else {
                preconditionFailure("rotor delegate is not retained")
            }
            let result = delegate.rotor(rotor, resultFor: parameters)
            let hasCandidate = index == 0 ? !headings.isEmpty : !links.isEmpty
            if hasCandidate {
                guard let result,
                      let target = result.targetElement as? YuAccessibilitySemanticElement else {
                    preconditionFailure("rotor did not return a semantic target")
                }
                precondition(target.node.revision == initialRevision)
                if index == 0 {
                    precondition(target.node.kind == SemanticAccessibilityKind.heading.rawValue)
                } else {
                    precondition(
                        target.node.kind == SemanticAccessibilityKind.link.rawValue
                            || target.node.kind == SemanticAccessibilityKind.autolink.rawValue
                            || target.node.kind == SemanticAccessibilityKind.referenceLink.rawValue
                    )
                }
                let targetLabel = target.accessibilityLabel ?? ""
                print("  rotor=\(index) target=\(targetLabel)")
            } else {
                precondition(result == nil)
                print("  rotor=\(index) target=<none>")
            }
        }
        print("Yu Accessibility self-check: revision=\(initialRevision) nodes=\(initialCount)")
        for element in initialChildren {
            let label = element.accessibilityLabel ?? ""
            print("  kind=\(element.node.kind) role=\(element.accessibilityRole.rawValue) label=\(label)")
        }

        let actionRevision: UInt64
        let actionChildren: [YuAccessibilitySemanticElement]
        if let task = tasks.first,
           let beforeValue = task.accessibilityValue as? NSNumber,
           let actionBlock = task.node.actionBlock {
            let beforeDone = beforeValue.boolValue
            precondition(task.accessibilityPerformPress())
            actionRevision = bridge.revision
            precondition(actionRevision != initialRevision)
            precondition(task.accessibilityLabel == nil)
            textView.refreshFromRust()
            actionChildren = (textView.accessibilityChildren ?? [])
                .compactMap { $0 as? YuAccessibilitySemanticElement }
            _ = validate(actionChildren, parent: textView, revision: actionRevision)
            let toggledTask = flatten(actionChildren).first {
                $0.node.actionBlock == actionBlock
            }
            guard let toggledTask,
                  let afterValue = toggledTask.accessibilityValue as? NSNumber else {
                preconditionFailure("toggled task child is missing")
            }
            precondition(afterValue.boolValue != beforeDone)
            print("Yu Accessibility self-check: task press revision=\(actionRevision)")
        } else {
            actionRevision = initialRevision
            actionChildren = initialChildren
        }

        if bridge.revision != initialRevision {
            let before = openedLinkURLs.count
            for item in linkMenus.flatMap({ $0 }) {
                precondition(!textView.validateMenuItem(item))
                if let action = item.action { _ = NSApp.sendAction(action, to: textView, from: item) }
            }
            precondition(openedLinkURLs.count == before)
        }

        // Headless splitter contract: the real coordinator supplies these
        // descriptors from CoreText geometry, while this self-check injects
        // one scalar descriptor to verify AppKit role/action/lifecycle
        // behavior without requiring a window or VoiceOver session.
        let splitterRevision = bridge.revision
        let splitterDescriptor = NativeTableResizeAccessibilityDivider(
            revision: splitterRevision,
            blockIndex: 0,
            kind: UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN),
            index: 0,
            columnCount: 2,
            rect: NSRect(x: 100.0, y: 10.0, width: 2.0, height: 20.0),
            tableSourceRange: NSRange(location: 0, length: 0)
        )
        var splitterActions: [Int] = []
        textView.tableResizeAccessibilityProvider = {
            bridge.revision == splitterRevision ? [splitterDescriptor] : []
        }
        textView.tableResizeAccessibilityFrameProvider = { _ in
            NSRect(x: 1.0, y: 2.0, width: 3.0, height: 20.0)
        }
        textView.onTableResizeAccessibilityAction = { descriptor, direction in
            guard descriptor.revision == splitterRevision else { return false }
            splitterActions.append(direction)
            return true
        }
        textView.refreshTableResizeAccessibility(postNotification: true)
        guard let splitter = (textView.accessibilitySplitters ?? []).first
            as? YuAccessibilityTableResizeElement else {
            preconditionFailure("table splitter accessibility child is missing")
        }
        precondition(splitter.accessibilityRole == .splitter)
        precondition(splitter.accessibilityLabel() != nil)
        precondition(
            splitter.accessibilityIdentifier()
                == "yu-table-divider-\(splitterRevision)-0-0"
        )
        precondition(splitter.parentObject === textView)
        precondition(splitter.accessibilityFrame() == NSRect(x: 1.0, y: 2.0, width: 3.0, height: 20.0))
        precondition(splitter.accessibilityPerformIncrement())
        precondition(splitter.accessibilityPerformDecrement())
        precondition(splitterActions == [1, -1])
        print(
            "Yu Accessibility self-check: splitter role/action revision=\(splitterRevision)"
        )

        _ = try bridge.insertText("\n")
        if let staleCandidate = actionChildren.first {
            precondition(staleCandidate.accessibilityLabel == nil)
        }
        textView.refreshFromRust()
        let nextRevision = bridge.revision
        precondition(!splitter.accessibilityPerformIncrement())
        precondition((textView.accessibilitySplitters ?? []).isEmpty)
        let nextChildren = (textView.accessibilityChildren ?? [])
            .compactMap { $0 as? YuAccessibilitySemanticElement }
        precondition(nextRevision != actionRevision)
        precondition(nextChildren.allSatisfy { $0.node.revision == nextRevision })
        print("Yu Accessibility self-check: refreshed revision=\(nextRevision)")
        // Exercise capacity reuse across stable size, growth and shrinkage.
        // Every result must contain exactly this revision's nodes, not unused
        // zero-filled slots or stale metadata left over from the larger tree.
        for source in ["plain", String(repeating: "# 标题\n\n**内容** [链接](https://example.com)\n\n", count: 20), "small"] {
            try bridge.setSelection(NSRange(location: 0, length: (bridge.source as NSString).length))
            _ = try bridge.insertText(source)
            textView.refreshFromRust()
            guard let nodes = bridge.accessibilitySemanticNodesIfAvailable else {
                preconditionFailure("Semantic capacity query failed")
            }
            precondition(nodes.allSatisfy { $0.revision == bridge.revision })
            precondition(nodes.enumerated().allSatisfy { Int($0.element.index) == $0.offset })
            if source == "plain" || source == "small" {
                precondition(nodes.count == 2)
                precondition(bridge.copySourceRangeIfAvailable(nodes[1].labelRange, revision: bridge.revision) == source)
            } else {
                precondition(nodes.count > 40)
            }
            let children = (textView.accessibilityChildren ?? []).compactMap { $0 as? YuAccessibilitySemanticElement }
            precondition(validate(children, parent: textView, revision: bridge.revision) == nodes.count - 1)
        }
        print("Yu Accessibility self-check: capacity grow/shrink and revision checks passed")
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Accessibility self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

/// 代码高亮：颜色真的进了这一帧的字形。
///
/// # 判据落在哪
///
/// **数的是场景图元的颜色**（`highlightedGlyphCount`，由 Rust 在组装这一帧时
/// 从 `Primitive::Glyph` 数出来），不是 `TextRole`、不是装饰、不是着色器。
/// 判据不来自被测的那条路。
///
/// **主判据是一个差分**：同一段代码，一份带语言名、一份不带，两份文档除此之外
/// 一个字节都不差。「带语言名的那份高亮字形更多」不可能来自别的原因；而单看
/// 一个绝对数字压不住「所有字形都被算成高亮」——那种错法数字更大，一样过。
///
/// Rust 侧的 `yu-workspace` 用例走的是等宽假 shaper。这里走的是**真的
/// CoreText**：字形的数量与分段都不同，颜色是不是按 run 正确分配只有这条路
/// 看得见。
///
/// headless 压不住的只有一条：**这一帧真的上了屏**。那要 Metal surface，
/// 挂在 `--launch-window-self-check` 上。
func runCodeHighlightSelfCheck(path: String) -> Never {
    let fileManager = FileManager.default
    let directory = fileManager.temporaryDirectory
        .appendingPathComponent("yu-code-highlight-\(UUID().uuidString)")
    do {
        try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? fileManager.removeItem(at: directory) }

        let fixture = try String(contentsOfFile: path, encoding: .utf8)
        precondition(
            fixture.contains("```rust"),
            "fixture 必须有一个带语言名的代码块"
        )

        /// 一份源码渲染成一帧之后带高亮颜色的字形数。
        func highlightedGlyphs(of source: String, name: String) throws -> Int {
            let url = directory.appendingPathComponent("\(name).md")
            try source.write(to: url, atomically: true, encoding: .utf8)
            let bridge = try StorageBridge(path: url.path)
            let snapshot = try bridge.macosRenderHostFrame(
                revision: bridge.revision,
                size: 14.0,
                maxWidth: 500.0,
                scrollY: 0.0,
                viewportHeight: 2_000.0,
                surfaceGeneration: 0,
            appearance: UInt8(YU_STORAGE_APPEARANCE_LIGHT)
            )
            precondition(snapshot.published, "\(name) 这一帧没有发布")
            precondition(snapshot.commandCount > 0, "\(name) 这一帧没有任何绘制指令")
            return snapshot.highlightedGlyphCount
        }

        // 差分的两边：把每一个语言名去掉，其余一个字节不动。
        let tagged = fixture
        let untagged = fixture
            .replacingOccurrences(of: "```rust", with: "```")
            .replacingOccurrences(of: "```json", with: "```")

        let withLanguage = try highlightedGlyphs(of: tagged, name: "tagged")
        let withoutLanguage = try highlightedGlyphs(of: untagged, name: "untagged")

        precondition(
            withoutLanguage == 0,
            "去掉语言名之后仍有 \(withoutLanguage) 个字形带着高亮颜色"
        )
        precondition(
            withLanguage > 0,
            "带语言名的代码块一个高亮字形都没有"
        )

        // 高亮只覆盖代码块里的一部分字形——全都覆盖说明颜色被无差别地刷上去了，
        // 而那种错法上面两条都过。fixture 里另有标题与正文，它们必须保持正文色。
        let plainDocument = try highlightedGlyphs(
            of: "# 只有正文\n\n一段普通文字，没有代码块。\n",
            name: "plain"
        )
        precondition(
            plainDocument == 0,
            "没有代码块的文档里出现了 \(plainDocument) 个高亮字形"
        )

        // 认不出的语言与带参数的语言名：前者不着色，后者照常着色。
        // `Language::from_info` 只取第一个词，这一条是它在真实路径上的判据。
        let unknown = try highlightedGlyphs(
            of: "```brainfuck\n+++[->+++<]\n```\n",
            name: "unknown"
        )
        precondition(unknown == 0, "认不出的语言不该着色，实际 \(unknown)")
        let withArguments = try highlightedGlyphs(
            of: "```rust,ignore\nfn a() { let x = 1; }\n```\n",
            name: "arguments"
        )
        precondition(
            withArguments > 0,
            "带参数的语言名没有被认出来"
        )

        // 在代码块里打字，颜色必须跟着改。
        //
        // 判据是**数量的确定变化**，不是「变了就行」：把 `let` 改成 `lets`，
        // 那三个字形从关键字变回普通标识符，高亮字形数正好少 3。只断「不相等」
        // 的话，一个每次编辑都把整块颜色清掉的实现也能过。
        //
        // 这一条本来在人工验收清单里，但真人按键盘那条路很难把光标准确放进代码
        // 块（方向键会滚动视口，合成事件又驱动不了 AppKit）。放在这里更强：
        // 它可复现，而且断的是一个数。
        let typingURL = directory.appendingPathComponent("typing.md")
        let typingSource = "```rust\nfn a() { let x = 1; }\n```\n"
        try typingSource.write(to: typingURL, atomically: true, encoding: .utf8)
        let typingBridge = try StorageBridge(path: typingURL.path)
        func renderCount(_ bridge: StorageBridge) throws -> Int {
            let snapshot = try bridge.macosRenderHostFrame(
                revision: bridge.revision,
                size: 14.0,
                maxWidth: 500.0,
                scrollY: 0.0,
                viewportHeight: 2_000.0,
                surfaceGeneration: 0,
            appearance: UInt8(YU_STORAGE_APPEARANCE_LIGHT)
            )
            precondition(snapshot.published, "打字用的这一帧没有发布")
            return snapshot.highlightedGlyphCount
        }
        let beforeTyping = try renderCount(typingBridge)
        precondition(beforeTyping > 0, "打字前就没有高亮，这一条压不住任何东西")
        // 光标落在 `let` 之后，插一个 `s`。
        let mirror = typingBridge.source as NSString
        let keyword = mirror.range(of: "let")
        precondition(keyword.location != NSNotFound, "语料里没有 let")
        try typingBridge.setSelection(NSRange(location: NSMaxRange(keyword), length: 0))
        _ = try typingBridge.insertText("s")
        precondition(
            typingBridge.source.contains("lets x"),
            "插入没有落在 let 后面：\(typingBridge.source)"
        )
        let afterTyping = try renderCount(typingBridge)
        precondition(
            afterTyping == beforeTyping - 3,
            "把 let 改成 lets 之后高亮字形应当少 3 个，"
                + "实际 \(beforeTyping) → \(afterTyping)"
        )

        print(
            "Yu code highlight self-check: highlighted glyphs \(withLanguage) with language, "
                + "\(withoutLanguage) without; typing let→lets \(beforeTyping)→\(afterTyping); "
                + "unknown language and plain documents stay uncolored"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu code highlight self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

/// 多光标：一组选区在 Rust↔Swift 边界上的行为。
///
/// # 判据落在哪
///
/// - **「N 根光标真的都在编辑」的判据是 canonical source**，不是选区数组。
///   两条路分开：选区是命令的输入，源码是 `TextBuffer` 的输出。数选区的条数
///   只能证明「设进去了」，证不了「用上了」。
/// - **「⌥ 点加了一根」的判据是 Rust 归一化之后的那一组**，不是 Swift 这边
///   拼出来的数组——后者是被测的那条路。
/// - **AX 复数属性单独断**：它以前是从单数推出来的一条假复数，改错了不报错，
///   只是读屏少认几根光标。
///
/// headless 压不住的只有一条：**⌥ 点的坐标→源码那一步**，它要已发布的
/// viewport 几何。那一条挂在 `--launch-window-self-check` 上。
func runMultiCursorSelfCheck(path: String) -> Never {
    let fileManager = FileManager.default
    let temporaryURL = fileManager.temporaryDirectory
        .appendingPathComponent("yu-multi-cursor-\(UUID().uuidString).md")
    do {
        try fileManager.copyItem(at: URL(fileURLWithPath: path), to: temporaryURL)
        defer { try? fileManager.removeItem(at: temporaryURL) }

        let bridge = try StorageBridge(path: temporaryURL.path)
        let textView = DocumentTextView(bridge: bridge)
        textView.refreshFromRust()

        let selections: () throws -> (ranges: [NSRange], primary: Int) = {
            let value = try unwrapSelfCheck(bridge.selectionsIfAvailable)
            return (value.ranges.map { $0.range }, value.primary)
        }

        // 1. 开门就是一条选区。「一个光标都没有」在这个模型里不存在。
        let initial = try selections()
        precondition(initial.ranges.count == 1, "初始应当只有一条选区")
        precondition(initial.primary == 0, "初始 primary 必须是 0")

        // 2. **⌥ 点加一根光标。** 判据是 Rust 归一化之后那一组，不是这边送进去
        //    的数组。走的是产品里同一个入口 `addCaret(atSource:)`。
        let mirror = bridge.source as NSString
        let firstTarget = mirror.range(of: "alpha").location
        let secondTarget = mirror.range(of: "gamma").location
        textView.navigate(toSource: NSRange(location: firstTarget, length: 0))
        precondition(textView.addCaret(atSource: secondTarget), "加光标失败")
        let two = try selections()
        precondition(
            two.ranges == [
                NSRange(location: firstTarget, length: 0),
                NSRange(location: secondTarget, length: 0),
            ],
            "两根光标的位置不对: \(two.ranges)"
        )
        precondition(two.primary == 1, "primary 必须是刚加的那一根")

        // 3. **重叠的输入要被归一化掉，而且归一化在 Rust 侧。**
        //    在同一个偏移上再点一次 ⌥：光标数不变，不是变成三根。这条压住的是
        //    「Swift 侧自己先排一遍」那种第二份合并实现。
        precondition(textView.addCaret(atSource: secondTarget), "重复加光标不应失败")
        let afterDuplicate = try selections()
        precondition(
            afterDuplicate.ranges.count == 2,
            "同一个偏移点两次不该变成三根光标"
        )

        // 3b. **AppKit 递过来的一组区间要整组送给 Rust。**
        //
        //     `setSelectedRanges` 是 **AppKit 发起**那条路（鼠标、以及 AX 给
        //     `AXSelectedTextRanges` 赋值）。Yu 自己发起的多光标走
        //     `navigate(toSources:primary:)`，**不经过这个 override**——所以
        //     上面那几条压不住它。只送第一条的表现是：读屏或辅助工具设了三段
        //     选区，Rust 只收到一段，另外两段悄悄消失。
        let appKitRanges = [
            NSValue(range: NSRange(location: firstTarget, length: 0)),
            NSValue(range: NSRange(location: secondTarget, length: 0)),
        ]
        textView.setSelectedRanges(appKitRanges, affinity: .downstream, stillSelecting: false)
        let fromAppKit = try selections()
        precondition(
            fromAppKit.ranges.count == 2,
            "AppKit 递过来两段，Rust 只收到 \(fromAppKit.ranges.count) 段"
        )
        //     这条路上的 primary 只能问 AppKit 自己认哪一条（`NSTextView` 对
        //     不连续选区没有「主」的概念），所以下一条断言之前先用显式入口把
        //     primary 摆回来。
        textView.navigate(
            toSources: [
                NSRange(location: firstTarget, length: 0),
                NSRange(location: secondTarget, length: 0),
            ],
            primary: 1
        )

        // 4. **AX 的复数属性报的是真复数。**
        //    它以前是 `[单数]`——屏幕上有两根光标，读屏只知道一根，不报错。
        let axRanges = (textView.accessibilitySelectedTextRanges() ?? []).map { $0.rangeValue }
        precondition(axRanges.count == 2, "AXSelectedTextRanges 报了 \(axRanges.count) 条")
        // 单数属性给的是 primary——那不是降级，那是另一个属性。
        precondition(
            textView.accessibilitySelectedTextRange()
                == NSRange(location: secondTarget, length: 0),
            "AXSelectedTextRange 必须是 primary"
        )

        // 5. **两根光标真的都在编辑。判据是源码。**
        //    只断选区条数的话，「除了 primary 谁都没插进去」会静默通过。
        let before = bridge.source
        _ = try bridge.insertText("X")
        let after = bridge.source
        // 期望值按**偏移**精确构造，不用 `replacingOccurrences`——后者会把
        // fixture 里别处的同名词一起换掉，于是这条断言压不住「插错了地方」。
        // 从后往前插，前一次插入才不会推移后一个偏移。
        let expected = NSMutableString(string: before)
        expected.insert("X", at: secondTarget)
        expected.insert("X", at: firstTarget)
        precondition(
            after == expected as String,
            "两处都要插上 X，实际: \(after)"
        )
        // 落点：各自停在自己插进去的那个字后面。
        let landed = try selections()
        precondition(landed.ranges.count == 2, "编辑之后仍然是两根光标")
        precondition(
            landed.ranges.allSatisfy { $0.length == 0 },
            "插入之后每一条都该是光标"
        )

        // 6. **一次 undo 收回两处。** 一条命令一个 Transaction，所以 history 里
        //    只有一条。改成「一个光标一个 Transaction」的话这里会红：撤销只
        //    收回一处，源码剩一半——不报错，只是撤销撤不干净。
        _ = try bridge.executeCommand(8)  // Undo。`Command` 是 DocumentTextView 的私有枚举。
        precondition(bridge.source == before, "一次 undo 必须把两处一起收回")

        // 7. **选中全部匹配。** 匹配已经是有序、互不重叠的一组，恰好是
        //    `Selections` 要的形状。相邻的两处不能被并掉。
        precondition(bridge.setSearchQuery("aa"), "设查询失败")
        let matches = try unwrapSelfCheck(bridge.searchMatchesIfAvailable)
        precondition(matches.count >= 2, "fixture 里 `aa` 至少要有两处，压不住就白写")
        textView.navigate(toSources: matches.map { $0.range }, primary: 0)
        let selected = try selections()
        precondition(
            selected.ranges == matches.map { $0.range },
            "全部匹配没有一一对上: \(selected.ranges) vs \(matches.map { $0.range })"
        )

        // 8. **塌回一条。** 单数入口仍然是「平台送来一个选区」那条路。
        textView.navigate(toSource: NSRange(location: 0, length: 0))
        let collapsed = try selections()
        precondition(collapsed.ranges.count == 1, "单数导航必须塌回一条")

        // 9. **组字期间只剩一条。** `CompositionOverlay` 是一个 preedit 覆盖一个
        //    区间；留着 N 条会在屏幕上留下几根不动的假光标。这是一笔登记在案的
        //    降级，理由与还债条件写在 `EditorDocument::begin_composition` 上。
        textView.navigate(toSources: matches.map { $0.range }, primary: 0)
        let beforeComposition = try selections()
        precondition(beforeComposition.ranges.count >= 2, "组字前应当是多光标")
        try bridge.beginComposition(
            replacementRange: NSRange(location: 0, length: 0),
            preedit: "n",
            selection: NSRange(location: 1, length: 0)
        )
        let composing = try selections()
        precondition(composing.ranges.count == 1, "组字期间必须只剩一条选区")
        try bridge.cancelComposition()

        print("Yu multi-cursor self-check: carets=2 matches=\(matches.count)")
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu multi-cursor self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

/// Control/FFI contract check; deliberately not a real sheet interaction claim.
func runImagePropertiesSelfCheck(path: String) -> Never {
    do {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("yu-image-properties-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let document = root.appendingPathComponent("note.md")
        let imageName = "图 100%.png"
        try FileManager.default.copyItem(at: URL(fileURLWithPath: path), to: root.appendingPathComponent(imageName))
        let uri = try StorageBridge.imageURI(forLocalPath: imageName)
        precondition(uri == "%E5%9B%BE%20100%25.png")
        let source = "# 保留\r\n\r\n![图](\(uri))\r\n"
        let bytes = Data([0xef, 0xbb, 0xbf]) + Data(source.utf8)
        try bytes.write(to: document)
        let bridge = try StorageBridge(path: document.path)
        let offset = (source as NSString).range(of: "![图]").location
        let properties = try bridge.imageProperties(at: offset)
        precondition(properties.alternative == "图" && properties.destination == uri)
        var finished = false
        let panel = ImagePropertiesPanel(properties: properties, document: document,
            apply: { _ = try bridge.updateImageProperties($0) }, finished: { finished = true })
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        let views = descendants(panel.window!.contentView!)
        func field(_ name: String) -> NSTextField {
            views.first { $0.identifier?.rawValue == "yu-image-" + name } as! NSTextField
        }
        func button(_ title: String) -> NSButton { views.compactMap { $0 as? NSButton }.first { $0.title == title }! }
        func change(_ name: String, _ value: String) {
            let target = field(name)
            target.stringValue = value
            let event = Notification(name: Notification.Name("control-check"), object: target)
            panel.controlTextDidChange(event)
            panel.controlTextDidEndEditing(event)
        }
        for appearance in [NSAppearance.Name.aqua, .darkAqua] {
            panel.window?.appearance = NSAppearance(named: appearance)
            let content = panel.window!.contentView!
            content.layoutSubtreeIfNeeded()
            let right = button("应用").convert(button("应用").bounds, to: content).maxX
            precondition(abs(right - (content.bounds.maxX - 24)) < 1, "sheet actions must align to the trailing inset")
            precondition(field("destination").cell?.usesSingleLineMode == true)
        }
        precondition(field("destination").stringValue == imageName)
        let replacementName = "副本 %.png"
        try FileManager.default.copyItem(at: URL(fileURLWithPath: path), to: root.appendingPathComponent(replacementName))
        change("destination", replacementName)
        change("width", "0")
        button("应用").performClick(nil)
        precondition(bridge.source == source && !finished)
        change("width", "128")
        precondition(field("height").stringValue == "128")
        change("alternative", "羽 <图>")
        button("应用").performClick(nil)
        let changed = bridge.source
        precondition(finished && changed.contains("width=\"128\" height=\"128\""))
        precondition(changed.contains("alt=\"羽 &lt;图&gt;\""))
        do { _ = try bridge.updateImageProperties(properties); preconditionFailure("stale property edit accepted") }
        catch {}
        _ = try bridge.executeCommand(8)
        precondition(bridge.source == source)
        _ = try bridge.executeCommand(9)
        precondition(bridge.source == changed)
        try bridge.save()
        let reopened = try StorageBridge(path: document.path)
        let restored = try reopened.imageProperties(at: offset)
        precondition(restored.identity.width == 128 && restored.identity.height == 128)
        precondition(restored.alternative == "羽 <图>" && reopened.source == changed)
        let replacementURI = try StorageBridge.imageURI(forLocalPath: replacementName)
        precondition(restored.destination == replacementURI)
        let saved = try Data(contentsOf: document)
        precondition(saved == Data([0xef, 0xbb, 0xbf]) + Data(changed.utf8))
        let cancelled = ImagePropertiesPanel(properties: restored, document: document,
            apply: { _ in preconditionFailure("cancel applied properties") }, finished: {})
        let cancel = descendants(cancelled.window!.contentView!).compactMap { $0 as? NSButton }.first { $0.title == "取消" }!
        cancel.performClick(nil)
        precondition(reopened.source == changed)
        // Actual ImageIO failures and AppKit target/action, without posting
        // system mouse events or claiming right-click window acceptance.
        let retryDocument = root.appendingPathComponent("retry.md")
        let retrySource = "![missing](restored.png)\r\n"
        let retryBytes = Data([0xef, 0xbb, 0xbf]) + Data(retrySource.utf8)
        try retryBytes.write(to: retryDocument)
        let retryBridge = try StorageBridge(path: retryDocument.path)
        let retryView = DocumentTextView(bridge: retryBridge)
        retryView.isEditable = true
        var refreshes = 0
        retryView.onResourceChange = { refreshes += 1 }
        let retryProperties = try retryBridge.imageProperties(at: 0)
        func settleImage() throws -> NativeMacosRenderHostSnapshot {
            let deadline = Date().addingTimeInterval(5)
            repeat {
                let frame = try retryBridge.macosRenderHostFrame(revision: retryBridge.revision,
                    size: 16, maxWidth: 500, scrollY: 0, viewportHeight: 240,
                    surfaceGeneration: 0, appearance: UInt8(YU_STORAGE_APPEARANCE_LIGHT))
                if !frame.resourceRefreshPending { return frame }
                Thread.sleep(forTimeInterval: 0.005)
            } while Date() < deadline
            throw CocoaError(.coderInvalidValue)
        }
        let failedFrame = try settleImage()
        let failedStatus = try retryBridge.imageResourceStatus(retryProperties)
        precondition(failedStatus == UInt8(YU_STORAGE_IMAGE_RESOURCE_FAILED) && !failedFrame.resourceRetryPending)
        let retryItem = retryView.imageResourceMenuItem(at: 0)!
        precondition(retryItem.title == "图片加载失败，重试" && retryView.validateMenuItem(retryItem))
        try FileManager.default.copyItem(at: URL(fileURLWithPath: path), to: root.appendingPathComponent("restored.png"))
        let retryMenu = NSMenu()
        retryMenu.addItem(retryItem)
        retryMenu.performActionForItem(at: 0)
        precondition(refreshes == 1)
        let readyFrame = try settleImage()
        precondition(!readyFrame.resourceRetryPending)
        let readyStatus = try retryBridge.imageResourceStatus(retryProperties)
        precondition(readyStatus == UInt8(YU_STORAGE_IMAGE_RESOURCE_READY))
        precondition(retryView.imageResourceMenuItem(at: 0) == nil && !retryView.validateMenuItem(retryItem))
        precondition(retryBridge.source == retrySource && retryBridge.revision == 0 && !retryBridge.commandAvailable(8))
        let retrySaved = try Data(contentsOf: retryDocument)
        precondition(retrySaved == retryBytes)
        try FileManager.default.removeItem(at: root)
        print("Yu image properties self-check: native controls, ratio, invalid dimensions, revision-bound FFI, undo/redo, BOM/CRLF save/reopen and failed-image menu target/action retry passed; real sheet/right-click events require window acceptance")
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu image properties self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

/// Tests shared settings notifications against multiple native document hosts.
/// Window geometry/scrolling still requires the external event suite.
func runReadingPreferencesSelfCheck(path: String) -> Never {
    do {
        try { () throws -> Void in
            let defaults = UserDefaults.standard
            let saved = defaults.volatileDomain(forName: UserDefaults.argumentDomain)
            defer { defaults.setVolatileDomain(saved, forName: UserDefaults.argumentDomain) }
            func preferences(_ size: Double, _ column: Double) {
                defaults.setVolatileDomain(["Yu.bodyFontSize": size, "Yu.readingColumnWidth": column,
                    "Yu.readingTheme": 0, "Yu.focusMode": size > 16], forName: UserDefaults.argumentDomain)
                NotificationCenter.default.post(name: NativeTheme.didChange, object: nil)
            }
            preferences(16, 0)
            let root = FileManager.default.temporaryDirectory.appendingPathComponent("yu-reading-preferences-" + UUID().uuidString)
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: root) }
            let original = try Data(contentsOf: URL(fileURLWithPath: path))
            var bridges: [StorageBridge] = []
            var controllers: [DocumentViewController] = []
            for index in 0..<2 {
                let file = root.appendingPathComponent("note-\(index).md")
                try original.write(to: file)
                let bridge = try StorageBridge(path: file.path)
                let controller = DocumentViewController(bridge: bridge)
                _ = controller.view
                controller.withFileInputForSelfCheck { precondition($0.font?.pointSize == 16) }
                bridges.append(bridge)
                controllers.append(controller)
            }
            let revealCoordinator = MacosSurfaceHostCoordinator(bridge: bridges[0])
            revealCoordinator.verifyDeferredCaretRevealForSelfCheck()
            withExtendedLifetime(revealCoordinator) {}
            let sources = bridges.map { $0.source }
            let revisions = bridges.map { $0.revision }
            preferences(20, 600)
            for controller in controllers {
                controller.withFileInputForSelfCheck { precondition($0.font?.pointSize == 20) }
            }
            let geometry = NativeTheme.reading(width: 1200, windowWidth: 1600)
            precondition(geometry.content_width == 600 && geometry.origin_x == 300)
            precondition(bridges.map { $0.source } == sources && bridges.map { $0.revision } == revisions)
            let future = DocumentViewController(bridge: try StorageBridge(path: root.appendingPathComponent("note-0.md").path))
            _ = future.view
            future.withFileInputForSelfCheck { precondition($0.font?.pointSize == 20) }
            preferences(16, 0)
            for controller in controllers + [future] {
                controller.withFileInputForSelfCheck { precondition($0.font?.pointSize == 16) }
            }
            precondition(NativeTheme.reading(width: 1200, windowWidth: 1600).content_width == 760)
            for index in 0..<2 {
                let bytes = try Data(contentsOf: root.appendingPathComponent("note-\(index).md"))
                precondition(bytes == original)
            }
        }()
        print("Yu reading preferences self-check: existing/future native hosts share font settings, Rust owns custom/default columns, source/revision/file bytes unchanged; visible window acceptance pending")
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu reading preferences self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

func runImageBatchSelfCheck(path: String) -> Never {
    do {
        try { () throws -> Void in
            let root = FileManager.default.temporaryDirectory.appendingPathComponent("yu-image-batch-" + UUID().uuidString)
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: root) }
            let defaults = UserDefaults.standard
            let old = defaults.volatileDomain(forName: UserDefaults.argumentDomain)
            defaults.setVolatileDomain(["Yu.imagePolicy": "copy", "Yu.imageDirectory": "assets"], forName: UserDefaults.argumentDomain)
            defer { defaults.setVolatileDomain(old, forName: UserDefaults.argumentDomain) }
            let image = root.appendingPathComponent("图 %.png")
            let bytes = try Data(contentsOf: URL(fileURLWithPath: path))
            try bytes.write(to: image)
            let draft = try NativeDocumentLocations.current.newDraftURL()
            defer { try? FileManager.default.removeItem(at: draft) }
            let bridge = try StorageBridge(path: draft.path, mode: .untitled)
            let controller = DocumentViewController(bridge: bridge)
            _ = controller.view
            var textView: DocumentTextView!
            controller.withFileInputForSelfCheck { textView = $0 }
            controller.savePanelDecision = { _ in nil }
            let board = NSPasteboard.withUniqueName()
            defer { board.releaseGlobally() }
            board.setData(bytes, forType: .png)
            board.setString("DO NOT INSERT FALLBACK", forType: .string)
            try textView.pasteFromPasteboardForSelfCheck(board)
            precondition(bridge.source.isEmpty && bridge.revision == 0 && controller.persistence.isUntitled)
            precondition(!FileManager.default.fileExists(atPath: draft.deletingLastPathComponent().appendingPathComponent("assets").path))
            let destination = root.appendingPathComponent("saved.md")
            controller.savePanelDecision = { _ in destination }
            let imported = try textView.onImageImport!([.file(image), .data(bytes)], nil)
            precondition(imported && !controller.persistence.isUntitled && bridge.path == destination.path)
            let source = bridge.source
            precondition(source.components(separatedBy: "![").count == 3)
            let assets = try FileManager.default.contentsOfDirectory(at: root.appendingPathComponent("assets"), includingPropertiesForKeys: nil)
            precondition(assets.count == 2 && assets[0] != assets[1])
            for asset in assets { let data = try Data(contentsOf: asset); precondition(data == bytes) }
            _ = try bridge.executeCommand(8)
            precondition(bridge.source.isEmpty)
            _ = try bridge.executeCommand(9)
            precondition(bridge.source == source)
            try bridge.save()
            let reopened = try StorageBridge(path: destination.path)
            precondition(reopened.source == source)
            let revision = bridge.revision
            do {
                _ = try textView.onImageImport!([.file(image), .data(Data("broken".utf8))], nil)
                preconditionFailure("invalid second image imported")
            } catch NativeImageResources.Failure.invalidImage {}
            precondition(bridge.source == source && bridge.revision == revision)
            let after = try FileManager.default.contentsOfDirectory(at: root.appendingPathComponent("assets"), includingPropertiesForKeys: nil)
            precondition(Set(after) == Set(assets))
        }()
        print("Yu image batch self-check: cancelled first save consumes image paste without text fallback; successful first save, atomic multi-image undo/redo, reopen and failure without side effects passed")
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu image batch self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}


func runSpellingCoordinatorSelfCheck(path: String) -> Never {
    do {
        try { () throws -> Void in
            let root = FileManager.default.temporaryDirectory.appendingPathComponent("yu-spelling-coordinator-" + UUID().uuidString)
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: root) }
            let file = root.appendingPathComponent("note.md")
            let source = "speling and `codde` <https://exampel.com>\r\n"
            let original = Data(source.utf8)
            try original.write(to: file)
            let bridge = try StorageBridge(path: file.path)
            var publications = 0
            let coordinator = NativeSpellingCoordinator(bridge: bridge) { publications += 1 }
            func wait(_ predicate: () -> Bool) {
                let deadline = Date().addingTimeInterval(10)
                while !predicate() && Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.02)) }
                precondition(predicate(), "Spelling publication timed out")
            }
            coordinator.update(source: source, visible: NSRange(location: 0, length: source.utf16.count), focus: 0, enabled: true)
            wait { publications > 0 }
            precondition(coordinator.publishedDiagnosticCount == 1)
            precondition(bridge.source == source && bridge.revision == 0)
            coordinator.update(source: source, visible: nil, focus: 0, enabled: false)
            precondition(coordinator.publishedDiagnosticCount == 0)
            let disabledPublications = publications
            coordinator.update(source: source, visible: nil, focus: 0, enabled: true)
            coordinator.update(source: source, visible: nil, focus: 0, enabled: false)
            RunLoop.current.run(until: Date().addingTimeInterval(0.5))
            precondition(publications == disabledPublications + 1 && coordinator.publishedDiagnosticCount == 0, "Cancelled work republished diagnostics")
            let bytes = try Data(contentsOf: file)
            precondition(bytes == original && bridge.source == source && bridge.revision == 0)
            withExtendedLifetime(coordinator) {}

            let sentence = "This is a good sentence. "
            let target = 4094
            let prefix = String(repeating: sentence, count: target / sentence.utf16.count)
                + String(repeating: " ", count: target % sentence.utf16.count)
            let longSource = prefix + "speling " + String(repeating: sentence, count: 200)
            let longFile = root.appendingPathComponent("boundary.md")
            try Data(longSource.utf8).write(to: longFile)
            let longBridge = try StorageBridge(path: longFile.path)
            var chunkPublications = 0
            let bounded = NativeSpellingCoordinator(bridge: longBridge) { chunkPublications += 1 }
            bounded.update(source: longSource, visible: NSRange(location: 4090, length: 30), focus: 0, enabled: true)
            wait { chunkPublications >= 2 }
            precondition(bounded.publishedDiagnosticCount == 1, "Overlap must preserve one complete diagnostic across a chunk boundary")
            precondition(longBridge.source == longSource && longBridge.revision == 0)
            withExtendedLifetime(bounded) {}

            // A distant caret and viewport must not schedule every intervening
            // chunk in a long document. The OS still checks real prose here.
            let sparseSource = String(repeating: sentence + "\n\n", count: 42_000) + "This sentence contains a speling mistake.\n"
            let sparseFile = root.appendingPathComponent("sparse-long.md")
            let sparseBytes = Data(sparseSource.utf8)
            try sparseBytes.write(to: sparseFile)
            let sparseBridge = try StorageBridge(path: sparseFile.path)
            var sparsePublications = 0
            let sparse = NativeSpellingCoordinator(bridge: sparseBridge) { sparsePublications += 1 }
            sparse.update(source: sparseSource, visible: NSRange(location: 0, length: 100),
                focus: sparseSource.utf16.count - 3, enabled: true)
            wait { sparsePublications >= 2 }
            RunLoop.current.run(until: Date().addingTimeInterval(0.4))
            guard sparsePublications == 2 && sparse.publishedDiagnosticCount == 1 else {
                throw NSError(domain: "Yu.SpellingSelfCheck", code: 1, userInfo: [NSLocalizedDescriptionKey:
                    "Distant viewport/caret: expected 2 publications and 1 diagnostic; got \(sparsePublications) and \(sparse.publishedDiagnosticCount)"])
            }
            precondition(sparseBridge.source == sparseSource && sparseBridge.revision == 0)
            let sparseSaved = try Data(contentsOf: sparseFile)
            precondition(sparseSaved == sparseBytes)
            withExtendedLifetime(sparse) {}

            let clippedSource = String(repeating: "x", count: 9_000) + " This sentence contains a speling mistake."
            let clippedFile = root.appendingPathComponent("clipped-word.md")
            try Data(clippedSource.utf8).write(to: clippedFile)
            let clippedBridge = try StorageBridge(path: clippedFile.path)
            var clippedPublications = 0
            let clipped = NativeSpellingCoordinator(bridge: clippedBridge) { clippedPublications += 1 }
            clipped.update(source: clippedSource, visible: NSRange(location: 0, length: 20),
                focus: clippedSource.utf16.count - 2, enabled: true)
            wait { clippedPublications >= 2 }
            precondition(clipped.publishedDiagnosticCount == 1,
                "An oversized word clipped at a chunk boundary must not publish partial-word errors")
            withExtendedLifetime(clipped) {}

            // Exercise generation cancellation across the actual Rust IME
            // bridge. This is a contract test, not real keyboard/IME evidence.
            let beforeIME = publications
            coordinator.update(source: source, visible: nil, focus: 0, enabled: true)
            try bridge.beginComposition(replacementRange: NSRange(location: 0, length: 0),
                preedit: "pin", selection: NSRange(location: 3, length: 0))
            coordinator.update(source: source, visible: nil, focus: 0, enabled: true)
            RunLoop.current.run(until: Date().addingTimeInterval(0.5))
            precondition(publications == beforeIME && bridge.source == source,
                "A queued check must not publish into active composition")
            try bridge.cancelComposition()
            coordinator.update(source: source, visible: nil, focus: 0, enabled: true)
            wait { publications > beforeIME }
            precondition(coordinator.publishedDiagnosticCount == 1 && bridge.source == source)
            coordinator.update(source: source, visible: nil, focus: 0, enabled: false)
            coordinator.update(source: source, visible: nil, focus: 0, enabled: true)
            try bridge.setSelection(NSRange(location: 0, length: 7))
            _ = try bridge.insertText("spelling")
            let edited = bridge.source
            let beforeEditPublication = publications
            coordinator.update(source: edited, visible: nil, focus: 0, enabled: true)
            wait { publications > beforeEditPublication }
            RunLoop.current.run(until: Date().addingTimeInterval(0.4))
            precondition(publications == beforeEditPublication + 1 && coordinator.publishedDiagnosticCount == 0,
                "Stale source requests must not restore corrected diagnostics")
            precondition(bridge.source == source.replacingOccurrences(of: "speling", with: "spelling"))
            let unchangedSaved = try Data(contentsOf: file)
            precondition(unchangedSaved == original)
        }()
        print("Yu spelling coordinator self-check: bounded async publication, code/URL exclusion, disable and cancellation, unchanged source/revision/file bytes")
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu spelling coordinator self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}


func runCalendarSelfCheck() {
    let parser = ISO8601DateFormatter()
    let midnight = parser.date(from: "2026-01-01T00:00:00Z")!
    let utc = TimeZone(secondsFromGMT: 0)!
    let perth = TimeZone(identifier: "Australia/Perth")!
    let losAngeles = TimeZone(identifier: "America/Los_Angeles")!
    let base = StorageBridge.referenceDay(at: midnight, timeZone: utc)!
    precondition(StorageBridge.referenceDay(at: midnight, timeZone: perth) == base)
    precondition(StorageBridge.referenceDay(at: midnight, timeZone: losAngeles) == base - 1)
    let evening = parser.date(from: "2026-01-01T20:00:00Z")!
    precondition(StorageBridge.referenceDay(at: evening, timeZone: perth) == base + 1)
    let springBefore = parser.date(from: "2026-03-08T09:59:00Z")!
    let springAfter = parser.date(from: "2026-03-08T10:01:00Z")!
    precondition(StorageBridge.referenceDay(at: springBefore, timeZone: losAngeles) == StorageBridge.referenceDay(at: springAfter, timeZone: losAngeles))
    let path = FileManager.default.temporaryDirectory.appendingPathComponent("yu-calendar-\(UUID().uuidString).md")
    let source = "# 日期测试\n"
    try! source.write(to: path, atomically: true, encoding: .utf8)
    defer { try? FileManager.default.removeItem(at: path) }
    let bridge = try! StorageBridge(path: path.path)
    precondition(try! bridge.updateRenderCalendar(at: midnight, timeZone: utc))
    precondition(!(try! bridge.updateRenderCalendar(at: midnight.addingTimeInterval(3600), timeZone: utc)))
    precondition(try! bridge.updateRenderCalendar(at: evening, timeZone: perth))
    precondition(!(try! bridge.updateRenderCalendar(at: evening.addingTimeInterval(30), timeZone: perth)))
    precondition(try! bridge.updateRenderCalendar(at: midnight, timeZone: losAngeles))
    precondition(bridge.source == source)
    print("Yu calendar self-check: local day, timezone, DST and cached FFI updates passed")
}
