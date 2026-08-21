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

func runClipboardSelfCheck(path: String) -> Never {
    do {
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
            ("browser-wrapper", false),
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

func runUndoSelfCheck(path: String) -> Never {
    do {
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
func runDocumentWorkflowSelfCheck(path: String) -> Never {
    let fileManager = FileManager.default
    let sourceURL = URL(fileURLWithPath: path)
    let temporaryURL = fileManager.temporaryDirectory
        .appendingPathComponent("yu-workflow-\(UUID().uuidString).md")
    do {
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
            try bridge.cancelClose()
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

func runProjectionSelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let revision = bridge.state.revision
        let source = bridge.source
        let projected = try bridge.projectedSource(revision: revision)
        precondition(projected.contains("粗体"))
        precondition(projected.contains("强调"))
        precondition(projected.contains("链接"))
        precondition(!projected.contains("**粗体**"))
        precondition(!projected.contains("*强调*"))
        precondition(!projected.contains("[链接](https://example.com)"))

        let strongMarker = (source as NSString).range(of: "**粗体**")
        precondition(strongMarker.location != NSNotFound)
        let insideStrong = UInt64(strongMarker.location + 2)
        let caret = try bridge.projectionCaret(
            revision: revision,
            sourceUTF16: insideStrong,
            affinity: 1
        )
        precondition(caret.revision == revision)
        precondition(caret.roundTripSourceUTF16 == insideStrong)
        precondition(caret.visualUTF16 < UInt64(projected.utf16.count))

        let end = try bridge.projectionCaret(
            revision: revision,
            sourceUTF16: UInt64(source.utf16.count),
            affinity: 1
        )
        precondition(end.visualUTF16 == UInt64(projected.utf16.count))
        precondition(end.roundTripSourceUTF16 == UInt64(source.utf16.count))
        print(
            "Yu Projection self-check: source UTF-16 \(source.utf16.count) -> "
                + "visual UTF-16 \(projected.utf16.count); caret round-trips"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Projection self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

func runProjectionHitTestSelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let revision = bridge.state.revision
        let source = bridge.source
        let projected = try bridge.projectedSource(revision: revision)
        let sourceStrong = (source as NSString).range(of: "**粗体**")
        let visualStrong = (projected as NSString).range(of: "粗体")
        precondition(sourceStrong.location != NSNotFound)
        precondition(visualStrong.location != NSNotFound)

        let selection = try bridge.projectionSelection(
            revision: revision,
            sourceRange: sourceStrong,
            affinity: 1
        )
        precondition(selection.revision == revision)
        precondition(selection.sourceRange == sourceStrong)
        precondition(selection.visualRange == visualStrong)
        precondition(selection.roundTripSourceRange == sourceStrong)

        let visualPrefix = (projected as NSString).substring(to: visualStrong.location)
        let line = UInt64(visualPrefix.components(separatedBy: "\n").count - 1)
        let linePrefix = (visualPrefix as NSString).substring(
            from: (visualPrefix as NSString).range(of: "\n", options: .backwards).location + 1
        )
        let point = CGPoint(x: CGFloat(linePrefix.utf16.count) + 0.1, y: CGFloat(line))
        let hit = try bridge.projectionHitTest(revision: revision, point: point)
        precondition(hit.revision == revision)
        precondition(hit.line == line)
        precondition(hit.sourceUTF16 == UInt64(sourceStrong.location))
        precondition(hit.visualUTF16 == UInt64(visualStrong.location))
        precondition(hit.roundTripSourceUTF16 == UInt64(sourceStrong.location))
        precondition(abs(hit.point.x - CGFloat(linePrefix.utf16.count)) < 0.001)
        precondition(abs(hit.point.y - CGFloat(line)) < 0.001)

        var staleRejected = false
        do {
            _ = try bridge.projectionHitTest(revision: revision + 1, point: point)
        } catch {
            staleRejected = true
        }
        precondition(staleRejected)
        print(
            "Yu Projection hit-test self-check: visual selection and "
                + "point↔source round-trip are Revision-bound"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Projection hit-test self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

func runShapedProjectionHitTestSelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let revision = bridge.state.revision
        let size: Float = 14.0
        let maxWidth: Float = 500.0
        let textView = DocumentTextView(bridge: bridge)
        textView.frame = NSRect(x: 0.0, y: 0.0, width: CGFloat(maxWidth), height: 600.0)
        textView.font = NSFont.systemFont(ofSize: CGFloat(size))
        let pointerWidth = Float(
            max(textView.bounds.width - 2.0 * textView.textContainerOrigin.x, 1.0)
        )

        let projected = try bridge.projectedSource(revision: revision)
        let hit = try bridge.macosProjectionHitTest(
            revision: revision,
            point: CGPoint(x: 0.0, y: 0.0),
            size: size,
            maxWidth: pointerWidth
        )
        precondition(hit.revision == revision)
        precondition(hit.sourceUTF16 <= UInt64(bridge.source.utf16.count))
        precondition(hit.visualUTF16 <= UInt64(projected.utf16.count))
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
        let (_, viewportBlocks) = try bridge.macosShapedViewportBlocks(
            revision: revision,
            size: size,
            maxWidth: pointerWidth,
            scrollY: 0.0,
            viewportHeight: 600.0
        )
        guard let targetBlock = viewportBlocks.first(where: {
            UInt64($0.sourceRange.location) <= sourceEndUTF16
                && sourceEndUTF16 <= UInt64(NSMaxRange($0.sourceRange))
        }) else {
            preconditionFailure("no viewport block contains the target source offset")
        }
        let endCaret = try bridge.macosBlockCaret(
            revision: revision,
            blockIndex: targetBlock.blockIndex,
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
            _ = try bridge.macosProjectionHitTest(
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





func runCompositionHitTestSelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let revision = bridge.state.revision
        let source = bridge.source as NSString
        let replacementStart = source.range(of: "x")
        let replacementEnd = source.range(of: "日本語")
        precondition(replacementStart.location != NSNotFound)
        precondition(replacementEnd.location != NSNotFound)
        let replacement = NSRange(
            location: replacementStart.location,
            length: replacementEnd.location + 2 - replacementStart.location
        )
        try bridge.beginComposition(
            replacementRange: replacement,
            preedit: "日本🙂",
            selection: NSRange(location: 2, length: 2)
        )

        let size: Float = 14.0
        let maxWidth: Float = 500.0
        let (_, blocks) = try bridge.macosShapedViewportBlocks(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 1_000.0
        )
        let secondBlock = blocks.first { block in
            block.sourceRange.location <= replacementEnd.location
                && NSMaxRange(block.sourceRange) >= replacementEnd.location
        } ?? blocks.last!
        let point = CGPoint(
            x: CGFloat(maxWidth - 1.0),
            y: secondBlock.y + secondBlock.height * 0.5
        )
        let projection = try bridge.compositionProjection(revision: revision)
        let hit = try bridge.macosCompositionProjectionHitTest(
            revision: revision,
            generation: projection.generation,
            point: point,
            size: size,
            maxWidth: maxWidth
        )
        precondition(hit.revision == revision)
        precondition(hit.generation == projection.generation)
        precondition(hit.blockIndex == secondBlock.blockIndex)
        precondition(hit.point.x.isFinite && hit.point.y.isFinite)
        precondition(hit.visualSelection == projection.visualSelection)
        precondition(hit.visualReplacement == projection.visualReplacementRange)
        precondition(hit.visualUTF16 >= UInt64(projection.visualReplacementRange.location))
        do {
            _ = try bridge.macosCompositionProjectionHitTest(
                revision: revision,
                generation: projection.generation + 1,
                point: point,
                size: size,
                maxWidth: maxWidth
            )
            preconditionFailure("stale composition hit unexpectedly succeeded")
        } catch BridgeError.operation(let status) {
            precondition(status == 16)
        }
        try bridge.cancelComposition()
        precondition(!bridge.composition.active)
        precondition(bridge.state.revision == revision)
        print(
            "Yu Composition Hit-Test self-check: cross-block transient point mapped "
                + "at block \(hit.blockIndex), generation \(hit.generation)"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Composition Hit-Test self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

func runBlockProjectionSelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let revision = bridge.state.revision
        let count = try bridge.projectionBlockCount(revision: revision)
        precondition(count > 0)

        var previousEnd = 0
        var kinds = Set<UInt8>()
        var visualTexts: [String] = []
        for index in 0..<count {
            let (block, visual) = try bridge.projectedBlock(
                revision: revision,
                blockIndex: UInt64(index)
            )
            precondition(block.revision == revision)
            precondition(block.blockIndex == UInt64(index))
            precondition(block.sourceRange.location >= previousEnd)
            previousEnd = NSMaxRange(block.sourceRange)
            precondition(block.visualUTF8Length == visual.utf8.count)
            precondition(block.visualUTF16Length == visual.utf16.count)
            kinds.insert(block.kind)
            visualTexts.append(visual)
            print(
                "  block=\(index) kind=\(block.kind) projection=\(block.projectionKind) "
                    + "source=\(block.sourceRange) visualUTF16=\(block.visualUTF16Length)"
            )
        }

        precondition(kinds.contains(3), "heading block missing")
        precondition(kinds.contains(5), "blockquote block missing")
        precondition(kinds.contains(6), "list block missing")
        precondition(kinds.contains(4), "fenced-code block missing")
        precondition(kinds.contains(7), "task-list block missing")
        precondition(visualTexts.contains { $0.contains("粗体") })
        precondition(visualTexts.contains { $0.contains("链接") })
        precondition(visualTexts.contains { $0.contains("任务") })
        precondition(visualTexts.contains { $0.contains("fn main") })
        precondition(visualTexts.contains { $0.contains("引用块") })
        precondition(visualTexts.contains { $0.contains("有序列表") })
        precondition(visualTexts.contains { $0.contains("Projection blocks") })
        precondition(visualTexts.allSatisfy { !$0.contains("# Projection blocks") })
        precondition(visualTexts.allSatisfy { !$0.contains("> 引用块") })
        precondition(visualTexts.allSatisfy { !$0.contains("**粗体**") })
        precondition(visualTexts.allSatisfy { !$0.contains("[链接](https://example.com)") })

        var tableIndex: Int?
        for index in 0..<count {
            let (block, _) = try bridge.projectedBlock(
                revision: revision,
                blockIndex: UInt64(index)
            )
            if block.projectionKind == 7 {
                tableIndex = index
                break
            }
        }
        if let tableIndex {
            let cells = try bridge.projectedTableCells(
                revision: revision,
                blockIndex: UInt64(tableIndex)
            )
            precondition(cells.count == 6, "table cell count mismatch")
            precondition(cells.map(\.row) == [0, 0, 1, 1, 2, 2])
            precondition(cells.map(\.column) == [0, 1, 0, 1, 0, 1])
            precondition(cells.allSatisfy { $0.source_start_utf16 <= $0.source_end_utf16 })

            let layoutCells = try bridge.tableLayoutCells(
                revision: revision,
                blockIndex: UInt64(tableIndex),
                maxWidth: 20.0,
                lineHeight: 2.0,
                defaultAdvance: 1.0
            )
            precondition(layoutCells.count == 4, "table layout cell count mismatch")
            precondition(layoutCells.map(\.row) == [0, 0, 1, 1])
            precondition(layoutCells.map(\.column) == [0, 1, 0, 1])
            precondition(layoutCells[0].y == 0.0)
            precondition(layoutCells[2].y == 2.0)
            precondition(layoutCells[1].alignment == YU_STORAGE_TABLE_ALIGNMENT_CENTER)
            let hit = try bridge.tableCellHitTest(
                revision: revision,
                blockIndex: UInt64(tableIndex),
                point: CGPoint(x: 3.5, y: 2.5),
                maxWidth: 20.0,
                lineHeight: 2.0,
                defaultAdvance: 1.0
            )
            precondition(hit.row == 1 && hit.column == 1, "table hit-test mismatch")
            precondition(hit.x == 3.0 && hit.y == 2.0)

            let sourceBeforeResize = bridge.source
            let columnDividerX = layoutCells[0].x + layoutCells[0].width
            let columnResize = try bridge.tableResizeHitTest(
                revision: revision,
                blockIndex: UInt64(tableIndex),
                point: CGPoint(x: CGFloat(columnDividerX + 0.1), y: 0.5),
                tolerance: 0.2,
                maxWidth: 20.0,
                lineHeight: 2.0,
                defaultAdvance: 1.0
            )
            precondition(columnResize.revision == revision)
            precondition(columnResize.block_index == UInt64(tableIndex))
            precondition(columnResize.kind == YU_STORAGE_TABLE_RESIZE_COLUMN)
            precondition(columnResize.index == 0)
            precondition(abs(columnResize.position - columnDividerX) < 0.0001)

            let begunResize = try bridge.tableResizeBegin(
                revision: revision,
                blockIndex: UInt64(tableIndex),
                point: CGPoint(x: CGFloat(columnDividerX + 0.1), y: 0.5),
                tolerance: 0.2,
                pointerPosition: Float(columnDividerX + 0.1),
                maxWidth: 20.0,
                lineHeight: 2.0,
                defaultAdvance: 1.0
            )
            precondition(begunResize.revision == columnResize.revision)
            precondition(begunResize.block_index == columnResize.block_index)
            precondition(begunResize.kind == columnResize.kind)
            precondition(begunResize.index == columnResize.index)
            precondition(abs(begunResize.position - columnResize.position) < 0.0001)
            let preview = try bridge.tableResizeAction(
                revision: revision,
                action: UInt8(YU_STORAGE_TABLE_RESIZE_UPDATE),
                pointerPosition: Float(columnDividerX + 1.1)
            )
            precondition(preview.revision == revision)
            precondition(preview.blockIndex == UInt64(tableIndex))
            precondition(preview.kind == YU_STORAGE_TABLE_RESIZE_COLUMN)
            precondition(preview.index == 0)
            precondition(abs(preview.delta - 1.0) < 0.0001)
            let finishedResize = try bridge.tableResizeAction(
                revision: revision,
                action: UInt8(YU_STORAGE_TABLE_RESIZE_FINISH)
            )
            precondition(finishedResize == preview)

            let resizedLayoutCells = try bridge.tableLayoutCellsWithResize(
                revision: revision,
                blockIndex: UInt64(tableIndex),
                resizeKind: UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN),
                resizeIndex: columnResize.index,
                resizeDelta: 1.0,
                maxWidth: 20.0,
                lineHeight: 2.0,
                defaultAdvance: 1.0
            )
            precondition(resizedLayoutCells.count == 4)
            precondition(resizedLayoutCells[0].width == 4.0)
            precondition(resizedLayoutCells[1].x == 4.0)
            precondition(resizedLayoutCells[1].width == 2.0)
            precondition(resizedLayoutCells[3].x == 4.0)
            let canonicalLayoutCells = try bridge.tableLayoutCells(
                revision: revision,
                blockIndex: UInt64(tableIndex),
                maxWidth: 20.0,
                lineHeight: 2.0,
                defaultAdvance: 1.0
            )
            precondition(canonicalLayoutCells[0].width == 3.0)
            precondition(canonicalLayoutCells[1].x == 3.0)
            precondition(bridge.source == sourceBeforeResize)

            let rowDividerY = layoutCells[0].y + layoutCells[0].height
            let rowResize = try bridge.tableResizeHitTest(
                revision: revision,
                blockIndex: UInt64(tableIndex),
                point: CGPoint(x: 1.0, y: CGFloat(rowDividerY + 0.1)),
                tolerance: 0.2,
                maxWidth: 20.0,
                lineHeight: 2.0,
                defaultAdvance: 1.0
            )
            precondition(rowResize.revision == revision)
            precondition(rowResize.kind == YU_STORAGE_TABLE_RESIZE_ROW)
            precondition(rowResize.index == 0)
            precondition(abs(rowResize.position - rowDividerY) < 0.0001)
            let begunRowResize = try bridge.tableResizeBegin(
                revision: revision,
                blockIndex: UInt64(tableIndex),
                point: CGPoint(x: 1.0, y: CGFloat(rowDividerY + 0.1)),
                tolerance: 0.2,
                pointerPosition: Float(rowDividerY + 0.1),
                maxWidth: 20.0,
                lineHeight: 2.0,
                defaultAdvance: 1.0
            )
            precondition(begunRowResize.revision == rowResize.revision)
            precondition(begunRowResize.block_index == rowResize.block_index)
            precondition(begunRowResize.kind == rowResize.kind)
            precondition(begunRowResize.index == rowResize.index)
            precondition(abs(begunRowResize.position - rowResize.position) < 0.0001)
            let rowPreview = try bridge.tableResizeAction(
                revision: revision,
                action: UInt8(YU_STORAGE_TABLE_RESIZE_UPDATE),
                pointerPosition: Float(rowDividerY + 0.2)
            )
            precondition(rowPreview.kind == YU_STORAGE_TABLE_RESIZE_ROW)
            try bridge.tableResizeAction(
                revision: revision,
                action: UInt8(YU_STORAGE_TABLE_RESIZE_CANCEL)
            )
            do {
                _ = try bridge.tableResizeHitTest(
                    revision: revision,
                    blockIndex: UInt64(tableIndex),
                    point: CGPoint(x: CGFloat(columnDividerX + 0.1), y: 0.5),
                    tolerance: 0.0,
                    maxWidth: 20.0,
                    lineHeight: 2.0,
                    defaultAdvance: 1.0
                )
                preconditionFailure("outside resize tolerance unexpectedly succeeded")
            } catch BridgeError.operation(let status) {
                precondition(status == 14)
            }
            precondition(bridge.source == sourceBeforeResize)
        } else {
            preconditionFailure("table projection missing")
        }

        do {
            _ = try bridge.projectedBlock(revision: revision, blockIndex: UInt64(count))
            preconditionFailure("out-of-bounds block unexpectedly succeeded")
        } catch BridgeError.operation(let status) {
            precondition(status == 14)
        }

        _ = try bridge.insertText("x")
        do {
            _ = try bridge.projectionBlockCount(revision: revision)
            preconditionFailure("stale block count unexpectedly succeeded")
        } catch BridgeError.operation(let status) {
            precondition(status == 13)
        }
        if let tableIndex {
            do {
                _ = try bridge.tableResizeHitTest(
                    revision: revision,
                    blockIndex: UInt64(tableIndex),
                    point: CGPoint(x: 3.1, y: 0.5),
                    tolerance: 0.2,
                    maxWidth: 20.0,
                    lineHeight: 2.0,
                    defaultAdvance: 1.0
                )
                preconditionFailure("stale table resize hit unexpectedly succeeded")
            } catch BridgeError.operation(let status) {
                precondition(status == 13)
            }
            do {
                _ = try bridge.tableLayoutCellsWithResize(
                    revision: revision,
                    blockIndex: UInt64(tableIndex),
                    resizeKind: UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN),
                    resizeIndex: 0,
                    resizeDelta: 1.0,
                    maxWidth: 20.0,
                    lineHeight: 2.0,
                    defaultAdvance: 1.0
                )
                preconditionFailure("stale table resize layout unexpectedly succeeded")
            } catch BridgeError.operation(let status) {
                precondition(status == 13)
            }
        }
        print(
            "Yu Block Projection self-check: revision=\(revision) blocks=\(count) "
                + "source ranges and visual lengths are revision-bound"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Block Projection self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

func runBlockLayoutSelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let revision = bridge.state.revision
        let blockIndex: UInt64 = 2
        let (block, _) = try bridge.projectedBlock(
            revision: revision,
            blockIndex: blockIndex
        )
        precondition(block.kind == 2, "paragraph block missing")

        let metrics = try bridge.blockLayout(
            revision: revision,
            blockIndex: blockIndex,
            maxWidth: 80.0,
            lineHeight: 1.0,
            defaultAdvance: 1.0
        )
        precondition(metrics.revision == revision)
        precondition(metrics.blockIndex == blockIndex)
        precondition(!metrics.shaped)
        precondition(metrics.lineCount > 0)
        precondition(metrics.height > 0.0)
        precondition(metrics.visualUTF16Length == block.visualUTF16Length)

        let shaped = try bridge.macosBlockLayout(
            revision: revision,
            blockIndex: blockIndex,
            size: 14.0,
            maxWidth: 500.0
        )
        precondition(shaped.revision == revision)
        precondition(shaped.blockIndex == blockIndex)
        precondition(shaped.shaped)
        precondition(shaped.lineCount > 0)
        precondition(shaped.height > 0.0)
        precondition(shaped.lineHeight > 0.0)
        precondition(shaped.defaultAdvance > 0.0)
        precondition(shaped.visualUTF16Length == block.visualUTF16Length)

        let source = bridge.source
        let marker = (source as NSString).range(of: "**粗体**")
        precondition(marker.location != NSNotFound)
        let caret = try bridge.macosBlockCaret(
            revision: revision,
            blockIndex: blockIndex,
            sourceUTF16: UInt64(marker.location),
            affinity: 0,
            size: 14.0,
            maxWidth: 500.0
        )
        precondition(caret.revision == revision)
        precondition(caret.blockIndex == blockIndex)
        precondition(caret.sourceUTF16 == UInt64(marker.location))
        precondition(caret.shaped)
        precondition(caret.height == shaped.lineHeight)
        precondition(caret.point.x.isFinite && caret.point.y.isFinite)

        _ = try bridge.insertText("x")
        do {
            _ = try bridge.macosBlockLayout(
                revision: revision,
                blockIndex: blockIndex,
                size: 14.0,
                maxWidth: 500.0
            )
            preconditionFailure("stale block layout unexpectedly succeeded")
        } catch BridgeError.operation(let status) {
            precondition(status == 13)
        }
        print(
            "Yu Block Layout self-check: metrics/CoreText block geometry and caret "
                + "are Revision-bound"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Block Layout self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

func runShapedViewportSelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let revision = bridge.state.revision
        let size: Float = 14.0
        let maxWidth: Float = 500.0
        let shaped = try bridge.macosBlockLayout(
            revision: revision,
            blockIndex: 2,
            size: size,
            maxWidth: maxWidth
        )

        let (snapshot, blocks) = try bridge.macosShapedViewportBlocks(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 1_000.0
        )
        precondition(snapshot.revision == revision)
        precondition(snapshot.blockRange.count == blocks.count)
        precondition(!blocks.isEmpty)
        precondition(snapshot.contentHeight >= shaped.lineHeight)
        precondition(blocks.allSatisfy { block in
            block.revision == revision
                && block.height > 0.0
                && block.y.isFinite
                && block.height.isFinite
                && block.sourceRange.location >= 0
        })
        let ordered = zip(blocks, blocks.dropFirst()).allSatisfy { first, second -> Bool in
            let sourceOrdered = NSMaxRange(first.sourceRange) <= second.sourceRange.location
            return first.blockIndex < second.blockIndex
                && sourceOrdered
                && first.y < second.y
        }
        precondition(ordered)
        precondition(blocks.contains { $0.kind == 3 }, "heading block missing")
        precondition(blocks.contains { $0.kind == 7 }, "task-list block missing")
        precondition(blocks.contains { $0.kind == 4 }, "fenced-code block missing")
        precondition(blocks.allSatisfy { $0.measured })

        _ = try bridge.insertText("x")
        do {
            _ = try bridge.macosShapedViewportBlocks(
                revision: revision,
                size: size,
                maxWidth: maxWidth,
                scrollY: 0.0,
                viewportHeight: 1_000.0
            )
            preconditionFailure("stale shaped viewport unexpectedly succeeded")
        } catch BridgeError.operation(let status) {
            precondition(status == 13)
        }
        print(
            "Yu Shaped Viewport self-check: revision=\(revision) blocks=\(blocks.count) "
                + "document-space origins/heights and source ranges are revision-bound"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Shaped Viewport self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

func runShapedVerticalSelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let revision = bridge.state.revision
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
        precondition(bridge.state.revision == revision)
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
        let revision = bridge.state.revision
        let size: Float = 14.0
        let maxWidth: Float = 500.0
        let (_, blocks) = try bridge.macosShapedViewportBlocks(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 1000.0
        )
        let blockCount = try bridge.projectionBlockCount(revision: revision)
        let tableBlockIndex = try (0..<blockCount).compactMap { index -> UInt64? in
            let (block, _) = try bridge.projectedBlock(
                revision: revision,
                blockIndex: UInt64(index)
            )
            return block.projectionKind == UInt8(YU_STORAGE_PROJECTION_TABLE)
                ? block.blockIndex
                : nil
        }.first
        guard let tableBlockIndex,
              let tableBlock = blocks.first(where: { $0.blockIndex == tableBlockIndex }) else {
            throw BridgeError.operation(StorageStatus.invalidSelection)
        }
        var tableY = tableBlock.y + 0.5
        var nearest = try bridge.macosTableResizeHitTestAtDocumentPoint(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            point: CGPoint(x: 0.0, y: tableY),
            tolerance: maxWidth
        )
        precondition(nearest.kind == YU_STORAGE_TABLE_RESIZE_COLUMN)
        var dividerPoint = NSPoint(
            x: CGFloat(nearest.position + 0.1),
            y: tableY
        )

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
        coordinator.setContentWidth(CGFloat(maxWidth))
        // Setting the viewport policy can invalidate measured block heights.
        // Re-read the table y/divider after the coordinator has prepared the
        // same metrics contract used by the product pointer path.
        _ = coordinator.visualDecorationGeometry()
        let (_, currentBlocks) = try bridge.macosShapedViewportBlocks(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 1000.0
        )
        guard let currentTableBlock = currentBlocks.first(where: {
            $0.blockIndex == tableBlockIndex
        }) else {
            throw BridgeError.operation(StorageStatus.invalidSelection)
        }
        tableY = currentTableBlock.y + 0.5
        nearest = try bridge.macosTableResizeHitTestAtDocumentPoint(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            point: CGPoint(x: 0.0, y: tableY),
            tolerance: maxWidth
        )
        precondition(nearest.kind == YU_STORAGE_TABLE_RESIZE_COLUMN)
        dividerPoint = NSPoint(x: CGFloat(nearest.position + 0.1), y: tableY)
        let sourceBeforeResize = bridge.source
        let accessibilityDividers = coordinator.tableResizeAccessibilityDividers()
        guard let accessibilityDivider = accessibilityDividers.first(where: {
            $0.blockIndex == tableBlockIndex && $0.index == nearest.index
        }) else {
            throw BridgeError.operation(StorageStatus.invalidSelection)
        }
        precondition(accessibilityDivider.revision == revision)
        precondition(accessibilityDivider.kind == YU_STORAGE_TABLE_RESIZE_COLUMN)
        precondition(accessibilityDivider.columnCount >= 2)
        precondition(accessibilityDivider.rect.height > 0.0)
        precondition(accessibilityDivider.rect.contains(
            NSPoint(x: accessibilityDivider.rect.midX, y: tableY)
        ))
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

        precondition(coordinator.beginTableResize(at: dividerPoint))
        precondition(coordinator.cancelTableResize())
        precondition(!coordinator.tableResizeActiveForSelfCheck)

        precondition(coordinator.beginTableResize(at: dividerPoint))
        _ = try bridge.insertText("x")
        coordinator.resetTableResizeAfterDocumentChange()
        precondition(!coordinator.tableResizeActiveForSelfCheck)
        precondition(!coordinator.updateTableResize(at: dividerPoint))
        coordinator.detach()
        print(
            "Yu macOS table resize coordinator self-check: document-space CoreText hit, "
                + "Accessibility divider descriptor, mouse update/finish/cancel, stale revision reset "
                + "and headless surface fallback are valid"
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
        let revision = bridge.state.revision
        let size: Float = 14.0
        let maxWidth: Float = 500.0
        let (_, commands, _, _) = try bridge.macosVisualRenderPlan(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 1_000.0
        )
        guard let task = commands.first(where: {
            $0.kind == UInt8(YU_STORAGE_RENDER_COMMAND_TASK_CHECKBOX)
        }) else {
            throw BridgeError.operation(StorageStatus.invalidSelection)
        }
        _ = try bridge.macosRenderHostFrame(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 1_000.0,
            surfaceGeneration: 0
        )
        let point = NSPoint(x: task.bounds.midX, y: task.bounds.midY)
        let publishedHit = try bridge.macosTaskCheckboxHitTest(
            revision: revision,
            point: point
        )
        precondition(publishedHit.revision == revision)
        precondition(publishedHit.blockIndex == task.blockIndex)
        precondition(publishedHit.markerRange.length == 3)
        precondition(publishedHit.bounds.contains(point))

        let textView = DocumentTextView(bridge: bridge)
        var documentChanges = 0
        textView.onDocumentChange = { documentChanges += 1 }
        textView.onTaskCheckboxPress = { [weak textView] point in
            guard let textView,
                  let hit = try? bridge.macosTaskCheckboxHitTest(
                      revision: bridge.state.revision,
                      point: point
                  ) else {
                return false
            }
            return textView.toggleTaskPointerHit(hit)
        }
        let sourceBefore = bridge.source
        precondition(textView.pressTaskCheckboxForSelfCheck(at: point))
        precondition(documentChanges == 1)
        precondition(bridge.state.revision == revision + 1)
        precondition(bridge.source != sourceBefore)
        precondition(bridge.source.contains("- [x] todo"))
        do {
            _ = try bridge.macosTaskCheckboxHitTest(revision: revision, point: point)
            preconditionFailure("stale task checkbox publication was accepted")
        } catch BridgeError.operation(let status) {
            precondition(status == StorageStatus.staleRevision)
        }
        precondition(
            !textView.pressTaskCheckboxForSelfCheck(
                at: NSPoint(x: task.bounds.maxX + 20.0, y: task.bounds.midY)
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

func runCompositionProjectionSelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let sourceBefore = bridge.source
        let revision = bridge.state.revision
        let strong = (sourceBefore as NSString).range(of: "**粗体**")
        precondition(strong.location != NSNotFound)
        let replacement = NSRange(location: strong.location + 2, length: 2)
        try bridge.beginComposition(
            replacementRange: replacement,
            preedit: "日本🙂",
            selection: NSRange(location: 2, length: 2)
        )

        let initial = try bridge.compositionProjection(revision: revision)
        precondition(initial.revision == revision)
        precondition(initial.generation == bridge.composition.generation)
        precondition(initial.replacementRange == replacement)
        precondition(initial.preeditSelection == NSRange(location: 2, length: 2))
        let projected = try bridge.copyCompositionProjection(
            revision: initial.revision,
            generation: initial.generation
        )
        precondition(projected.contains("日本🙂"))
        precondition(!projected.contains("**粗体**"))
        precondition(initial.projectedUTF8Length == projected.utf8.count)
        precondition(initial.projectedUTF16Length == projected.utf16.count)
        precondition(initial.visualSelection.length == 2)

        let caret = try bridge.compositionCaret(
            revision: initial.revision,
            generation: initial.generation,
            sourceUTF16: UInt64(replacement.location),
            affinity: 1
        )
        precondition(caret.revision == revision)
        precondition(caret.generation == initial.generation)
        precondition(caret.sourceUTF16 == UInt64(replacement.location))
        precondition(caret.visualSelection == initial.visualSelection)
        precondition(caret.visualUTF16 == UInt64(initial.visualSelection.location + initial.visualSelection.length))

        try bridge.updateComposition(
            preedit: "日本語",
            selection: NSRange(location: 3, length: 0)
        )
        let updated = try bridge.compositionProjection(revision: revision)
        precondition(updated.generation != initial.generation)
        precondition(updated.preeditSelection == NSRange(location: 3, length: 0))
        do {
            _ = try bridge.copyCompositionProjection(
                revision: revision,
                generation: initial.generation
            )
            preconditionFailure("stale composition projection unexpectedly succeeded")
        } catch BridgeError.operation(let status) {
            precondition(status == 16)
        }
        let updatedCaret = try bridge.compositionCaret(
            revision: revision,
            generation: updated.generation,
            sourceUTF16: UInt64(replacement.location),
            affinity: 1
        )
        precondition(updatedCaret.visualSelection.length == 0)
        precondition(updatedCaret.visualUTF16 == UInt64(updated.visualSelection.location))

        try bridge.cancelComposition()
        precondition(!bridge.composition.active)
        precondition(bridge.state.revision == revision)
        precondition(bridge.source == sourceBefore)
        do {
            _ = try bridge.compositionProjection(revision: revision)
            preconditionFailure("cancelled composition unexpectedly projected")
        } catch BridgeError.operation(let status) {
            precondition(status == 15)
        }
        print(
            "Yu Composition Projection self-check: revision=\(revision) "
                + "generation \(initial.generation)->\(updated.generation); source preserved"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Composition Projection self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

func runAccessibilitySelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let textView = DocumentTextView(bridge: bridge)
        let initialRevision = bridge.state.revision
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
        precondition(
            allInitial
                .filter { $0.node.kind != SemanticAccessibilityKind.taskListItem.rawValue }
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
            actionRevision = bridge.state.revision
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

        // Headless splitter contract: the real coordinator supplies these
        // descriptors from CoreText geometry, while this self-check injects
        // one scalar descriptor to verify AppKit role/action/lifecycle
        // behavior without requiring a window or VoiceOver session.
        let splitterRevision = bridge.state.revision
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
            bridge.state.revision == splitterRevision ? [splitterDescriptor] : []
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
        let nextRevision = bridge.state.revision
        precondition(!splitter.accessibilityPerformIncrement())
        precondition((textView.accessibilitySplitters ?? []).isEmpty)
        let nextChildren = (textView.accessibilityChildren ?? [])
            .compactMap { $0 as? YuAccessibilitySemanticElement }
        precondition(nextRevision != actionRevision)
        precondition(nextChildren.allSatisfy { $0.node.revision == nextRevision })
        print("Yu Accessibility self-check: refreshed revision=\(nextRevision)")
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Accessibility self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}
