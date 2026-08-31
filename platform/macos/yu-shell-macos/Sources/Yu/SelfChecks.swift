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
        let nearest = try bridge.macosTableResizeAtDocumentPoint(
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
///   1. 扁平 → 嵌套那次转换：断言落在**树的形状**上，反过来核对平表的
///      `parent` 字段。「挂错父亲」「静默地把孩子提成根」都在这里出。
///   2. 点第 N 行之后光标落在第 N 条标题的正文起点——判据是
///      `bridge.selection`，与面板走的是两条路。
///   3. 那之后 `macosShapedCaretScrollRequest` 指向那一条的块。这一条压住
///      「面板自己算 y」：滚动必须由 yu-editor::viewport 那条路给出。
///   4. 编辑之后刷新，展开状态与选中行不丢。纯 Swift 状态逻辑，每次
///      `reloadData` 全量重建就会丢，而且不报错。
func runOutlinePanelSelfCheck(path: String) -> Never {
    let fileManager = FileManager.default
    let temporaryURL = fileManager.temporaryDirectory
        .appendingPathComponent("yu-outline-\(UUID().uuidString).md")
    do {
        try fileManager.copyItem(at: URL(fileURLWithPath: path), to: temporaryURL)
        defer { try? fileManager.removeItem(at: temporaryURL) }

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
        let mirror = bridge.source as NSString
        let hiddenFor: (NativeOutlineItem) -> [NSRange]? = { item in
            bridge.blockHiddenSpans(block: item.block, in: item.labelRange)
        }
        panel.reload(items: items, source: mirror, hidden: hiddenFor)

        // 0. 「拿镜像减区间」那一步的性质。
        try checkPanelLabelProperties(items: items, mirror: mirror, hidden: hiddenFor)

        // 1. 扁平 → 嵌套。
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
                visited.append(node.item)
                walk(node.children, parent: node)
            }
        }
        walk(panel.rootsForSelfCheck, parent: nil)
        precondition(visited.count == items.count, "转换丢了或多出了条目")
        precondition(
            visited.map(\.index) == items.map(\.index),
            "前序遍历与文档顺序不一致"
        )

        // label 是源码区间**减掉被藏的那几段**：第三刀开了回报区间的 FFI，
        // 行内标记不再显示在面板上。唯一的纯呈现例外是 Setext 折成一行。
        let labels = panel.rootsForSelfCheck.flatMap(allLabelsForSelfCheck)
        precondition(
            labels.contains("带 行内标记 的标题"),
            "强调的 `**` 没有被剥掉，实际标签: \(labels)"
        )
        precondition(
            labels.contains("带 链接 的标题"),
            "链接的方括号与目标没有被剥掉，实际标签: \(labels)"
        )
        precondition(labels.contains("收尾串"), "ATX 收尾串没有被树剥掉: \(labels)")
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
            let request = try bridge.macosShapedCaretScrollRequest(
                revision: bridge.state.revision,
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
        panel.collapseForSelfCheck(identity: collapsible.identity)
        let selectedRow = rowCount - 1
        panel.clickRowForSelfCheck(min(selectedRow, panel.rowCountForSelfCheck - 1))
        let expandedBefore = panel.expandedIdentitiesForSelfCheck
        let selectedBefore = try unwrapSelfCheck(panel.selectedIdentityForSelfCheck)
        precondition(!expandedBefore.contains(collapsible.identity))

        // 在**文档最前面**插一条新标题：这会把后面每一条的 index 与 block
        // 一起推后一位。展开状态与选中行因此不能按下标记，只能按身份记——
        // 在末尾追加字符是压不住这一条的，那种编辑谁都活得下来。
        let revisionBefore = bridge.state.revision
        let head = NSRange(location: 0, length: 0)
        try bridge.setSelection(head)
        textView.insertText("# 新的顶层\n\n", replacementRange: head)
        precondition(bridge.state.revision != revisionBefore, "编辑没有推进 Revision")
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
        panel.reload(items: refreshed, source: bridge.source as NSString, hidden: hiddenFor)

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

        let rowsFor: (String) throws -> [SearchResultRow] = { query in
            precondition(bridge.setSearchQuery(query), "设查询失败")
            let matches = try unwrapSelfCheck(bridge.searchMatchesIfAvailable)
            let mirror = bridge.source as NSString
            let rows = matches.map { match in
                SearchResults.row(for: match, in: mirror) { block, range in
                    bridge.blockHiddenSpans(block: block, in: range)
                }
            }
            panel.reload(rows: rows, query: query)
            return rows
        }

        // 1. 结果那一行**剥掉了语法标记**，而且没有越出它所在的块。
        //
        //    两条都不是自证的：前者的判据是「Markdown 的 `**` 是标记」这件
        //    人知道的事，后者的判据是 FFI 报的块区间，与算上下文那条路分开。
        let rows = try rowsFor("标记")
        precondition(rows.count == 6, "fixture 里应当有六处命中，实际 \(rows.count)")
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
            sameLine[0].match.range != sameLine[1].match.range,
            "两行结果指向了同一处命中"
        )
        precondition(
            labels.allSatisfy { !$0.contains(where: \.isNewline) },
            "结果列表上不能出现换行: \(labels)"
        )
        let mirror = bridge.source as NSString
        for row in rows {
            let context = SearchResults.contextRange(for: row.match, in: mirror)
            precondition(
                context.location >= row.match.blockRange.location
                    && context.location + context.length
                        <= row.match.blockRange.location + row.match.blockRange.length,
                "上下文 \(context) 越出了块 \(row.match.blockRange)——"
                    + "回报隐藏区间的 FFI 会拒绝它，于是那一行悄悄带回语法标记"
            )
            precondition(
                NSIntersectionRange(context, row.match.range).length == row.match.range.length,
                "上下文 \(context) 没有盖住命中 \(row.match.range)"
            )
        }
        precondition(panel.countTextForSelfCheck == "6 处匹配", panel.countTextForSelfCheck)

        // 1b. 块比行窄的时候必须裁。
        //
        //     **语料造不出这个情形**：块的边界今天是按行划的，所以一行必然落在
        //     一个块里，删掉那个交集全部用例照样绿。但它不是死代码——理由写在
        //     `SearchResults.contextRange` 上（AppKit 与块扫描器对「一行」的
        //     定义不同；「块的边界还没合并」那道闸门一旦打开就更不成立）。
        //     所以这里手造一条：一行完整，块只覆盖它的后半截。
        var narrow = YuStorageSearchMatch()
        narrow.block = 0
        narrow.start_utf16 = 4
        narrow.end_utf16 = 6
        narrow.block_start_utf16 = 3
        narrow.block_end_utf16 = 7
        let clipped = SearchResults.contextRange(
            for: NativeSearchMatch(narrow),
            in: "abcdefghij" as NSString
        )
        precondition(
            clipped == NSRange(location: 3, length: 4),
            "块比行窄时没有裁：\(clipped)。不裁的后果是回报隐藏区间的 FFI 拒绝这个"
                + "请求，那一行悄悄带回语法标记"
        )

        // 2. 点第 N 行 → 选区落在第 N 处命中。判据来自 bridge.selection，
        //    与面板走的是两条路。选中而不是只放光标：Rust 侧的「当前命中」
        //    要求选区恰好等于那一段。
        for row in 0..<panel.rowCountForSelfCheck {
            let entry = try unwrapSelfCheck(panel.rowForSelfCheck(row))
            panel.clickRowForSelfCheck(row)
            precondition(
                bridge.selection.range == entry.match.range,
                "点第 \(row) 行之后选区是 \(bridge.selection.range)，"
                    + "而那一处命中在 \(entry.match.range)"
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
                + "labels=\(labels.count); stripping, block-clipped context, "
                + "navigation, wrap-around and re-scan passed"
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

/// 「拿镜像减区间」那一步的判据。
///
/// **「藏对了没有」不在这里证**——那是 `yu-decoration/src/hidden.rs` 的线性
/// 参照实现与 `extension_decorations.rs` 那 45 条压住的事，Swift 侧再证一遍
/// 只会得到一份自证的用例。这一层可能错的是别的：UTF-16 偏移、区间重叠、
/// 逆序、越界。所以判据是**性质**，加上一组手造的畸形输入。
///
/// 也**不**拿「大纲面板与搜索面板显示同一个字符串」当判据——两边都从这一份
/// 定义来，那同样是自证的。
private func checkPanelLabelProperties(
    items: [NativeOutlineItem],
    mirror: NSString,
    hidden: (NativeOutlineItem) -> [NSRange]?
) throws {
    var stripped = 0
    for item in items {
        let spans = try unwrapSelfCheck(hidden(item))
        let raw = mirror.substring(with: item.labelRange)
        let label = PanelLabel.stripping(spans, from: mirror, in: item.labelRange)

        // 区间本身：升序、不重叠、不越界。FFI 承诺了这个形状，这里反过来核对。
        precondition(
            PanelLabel.isWellFormed(spans, within: item.labelRange),
            "回报的区间不是升序不重叠的：\(spans) 不在 \(item.labelRange) 里"
        )
        // 长度：结果恰好少掉藏起来的那些字。少减/多减都在这一条下面。
        let removed = spans.reduce(0) { $0 + $1.length }
        precondition(
            label.utf16.count == item.labelRange.length - removed,
            "「\(label)」有 \(label.utf16.count) 个 UTF-16 单元，"
                + "而 \(item.labelRange.length) 减去藏掉的 \(removed) 是 "
                + "\(item.labelRange.length - removed)"
        )
        // 子序列：只允许**删**字节，不允许改写或换顺序。
        precondition(
            isSubsequenceForSelfCheck(label, of: raw),
            "「\(label)」不是「\(raw)」的子序列"
        )
        // 拿不到区间（Revision 撞上刷新）时退回**显示源码**。空串会让面板上
        // 那一行整条消失——不报错，只是空了。
        if !raw.contains(where: \.isNewline) {
            precondition(
                OutlineTree.displayLabel(item.labelRange, in: mirror, hidden: nil) == raw,
                "拿不到区间时应当显示源码「\(raw)」"
            )
        }
        if removed > 0 { stripped += 1 }
    }
    precondition(stripped > 0, "fixture 里没有一条标题带行内标记，这几条压不住任何东西")

    // 手造的畸形输入。它们不可能从 FFI 来（那边有自己的用例），但这一步是
    // 公开的、两个面板共用，一个错的调用方不该换来一个谁也说不清的字符串。
    let source = "abcdef" as NSString
    let whole = NSRange(location: 0, length: 6)
    precondition(PanelLabel.stripping([], from: source, in: whole) == "abcdef", "空区间")
    precondition(
        PanelLabel.stripping([NSRange(location: 0, length: 6)], from: source, in: whole) == "",
        "整段被藏"
    )
    precondition(
        PanelLabel.stripping(
            [NSRange(location: 1, length: 2), NSRange(location: 4, length: 1)],
            from: source,
            in: whole
        ) == "adf",
        "两段各藏一截"
    )
    precondition(
        PanelLabel.stripping([NSRange(location: 6, length: 0)], from: source, in: whole)
            == "abcdef",
        "空区间不藏任何东西"
    )
    // 下面四种都必须原样返回：显示源码是一件真事，按自相矛盾的区间去减不是。
    for (name, bad) in [
        ("逆序", [NSRange(location: 3, length: 1), NSRange(location: 1, length: 1)]),
        ("重叠", [NSRange(location: 1, length: 3), NSRange(location: 2, length: 2)]),
        ("越界", [NSRange(location: 4, length: 5)]),
        ("负长度", [NSRange(location: 2, length: -1)]),
    ] {
        precondition(
            PanelLabel.stripping(bad, from: source, in: whole) == "abcdef",
            "\(name)的区间必须整段原样返回，实际得到"
                + "「\(PanelLabel.stripping(bad, from: source, in: whole))」"
        )
    }
    // 请求区间只覆盖一部分镜像时，藏的偏移是**绝对**的，不是相对起点的。
    precondition(
        PanelLabel.stripping(
            [NSRange(location: 3, length: 1)],
            from: source,
            in: NSRange(location: 2, length: 4)
        ) == "cef",
        "区间偏移必须是镜像里的绝对位置"
    )
}

private func isSubsequenceForSelfCheck(_ candidate: String, of source: String) -> Bool {
    var remaining = Substring(source)
    for character in candidate {
        guard let index = remaining.firstIndex(of: character) else { return false }
        remaining = remaining[remaining.index(after: index)...]
    }
    return true
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
                revision: bridge.state.revision,
                size: 14.0,
                maxWidth: 500.0,
                scrollY: 0.0,
                viewportHeight: 2_000.0,
                surfaceGeneration: 0
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
                revision: bridge.state.revision,
                size: 14.0,
                maxWidth: 500.0,
                scrollY: 0.0,
                viewportHeight: 2_000.0,
                surfaceGeneration: 0
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
