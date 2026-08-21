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
        let revision = bridge.state.revision
        let size: Float = 14.0
        let maxWidth: Float = 500.0
        let textView = DocumentTextView(bridge: bridge)
        textView.frame = NSRect(x: 0.0, y: 0.0, width: CGFloat(maxWidth), height: 600.0)
        textView.font = NSFont.systemFont(ofSize: CGFloat(size))
        let pointerWidth = Float(
            max(textView.bounds.width - 2.0 * textView.textContainerOrigin.x, 1.0)
        )

        let hit = try bridge.macosProjectionHitTest(
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
        let endCaret = try bridge.macosSourceCaret(
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
        // 分隔线的位置由 Rust 自己的 Accessibility 描述符给出。平台不需要先取
        // viewport 的块列表、再逐块找出哪个是表格——那是把布局几何搬到平台侧
        // （不变量 I3）。这条路径同时就是 VoiceOver 用的那一条。
        let sourceBeforeResize = bridge.source
        let accessibilityDividers = coordinator.tableResizeAccessibilityDividers()
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
        let nearest = try bridge.macosTableResizeHitTestAtDocumentPoint(
            revision: revision,
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
        _ = try bridge.macosRenderHostFrame(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 1_000.0,
            surfaceGeneration: 0
        )
        // 用 Rust 自己的 point→source 映射找出待办那一行的纵坐标，再沿这一行
        // 向右找出 checkbox 的可命中点。此前是把整份 RenderPlan 取过 ABI 再从
        // 里面挑一条 TASK_CHECKBOX 指令——RenderPlan 不跨 C ABI（不变量 I2）。
        let sourceString = bridge.source as NSString
        let markerRange = sourceString.range(of: "- [ ] todo")
        precondition(markerRange.location != NSNotFound)
        var taskLineY: CGFloat?
        for step in 0..<200 {
            let y = CGFloat(step) * 2.0
            guard let hit = try? bridge.macosProjectionHitTest(
                revision: revision,
                point: CGPoint(x: 1.0, y: y),
                size: size,
                maxWidth: maxWidth
            ) else { continue }
            if hit.sourceUTF16 >= UInt64(markerRange.location),
               hit.sourceUTF16 <= UInt64(NSMaxRange(markerRange)) {
                taskLineY = y
                break
            }
        }
        guard let taskLineY else {
            throw BridgeError.operation(StorageStatus.invalidSelection)
        }
        // 在这一行附近扫描出一个真正命中 checkbox 的点。平台已经拿不到任何
        // 绘制几何，只能像用户点击那样去试——这正是这条路径该被测的样子。
        var found: (point: NSPoint, hit: NativeTaskCheckboxHit)?
        outer: for dy in stride(from: -8.0, through: 24.0, by: 2.0) {
            for dx in stride(from: 0.0, through: 48.0, by: 2.0) {
                let probe = NSPoint(x: dx, y: taskLineY + dy)
                if let hit = try? bridge.macosTaskCheckboxHitTest(
                    revision: revision,
                    point: probe
                ) {
                    found = (probe, hit)
                    break outer
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
