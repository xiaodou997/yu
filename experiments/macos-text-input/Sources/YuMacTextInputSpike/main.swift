import AppKit
import ApplicationServices
import YuEditorFFI

private let notFoundRange = NSRange(location: NSNotFound, length: 0)

private enum RustCaretAffinity: UInt8 {
    case upstream = 0
    case downstream = 1
}

private enum RustSourceSync: UInt8 {
    case none = 0
    case range = 1
    case full = 2
}

private struct RustCommandResult {
    let revision: UInt64
    let range: NSRange
    let affinity: NSSelectionAffinity
    let changed: Bool
    let sourceSync: RustSourceSync
    let oldSourceRange: NSRange?
    let newSourceRange: NSRange?
}

private struct RustCaretScrollResult {
    let revision: UInt64
    let source: Int
    let block: Int
    let caretX: CGFloat
    let caretY: CGFloat
    let caretWidth: CGFloat
    let caretHeight: CGFloat
    let currentScrollY: CGFloat
    let targetScrollY: CGFloat
    let margin: CGFloat
    let needsScroll: Bool
}

private struct RustProjectionCaret {
    let revision: UInt64
    let source: Int
    let visual: Int
    let roundTripSource: Int
    let affinity: NSSelectionAffinity
}

private struct RustBlockProjectionCaret {
    let revision: UInt64
    let source: Int
    let block: Int
    let visual: Int
    let roundTripSource: Int
    let affinity: NSSelectionAffinity
}

private struct RustBlockShapedCaret {
    let revision: UInt64
    let source: Int
    let block: Int
    let visual: Int
    let roundTripSource: Int
    let line: Int
    let x: CGFloat
    let y: CGFloat
    let width: CGFloat
    let height: CGFloat
    let affinity: NSSelectionAffinity
}

private struct RustViewportMetrics: Equatable {
    let maxWidth: CGFloat
    let lineHeight: CGFloat
    let defaultAdvance: CGFloat
    let estimatedBlockHeight: CGFloat
    let overscan: CGFloat
}

private enum YuViewportApplyResult: Equatable {
    case stale
    case noOp
    case scrolled(CGFloat)
}

/// Native consumer for Rust's absolute document-space caret scroll request.
/// The adapter owns only AppKit viewport state; source/layout remain in Rust.
private final class YuNativeViewportAdapter {
    private let scrollView: NSScrollView
    private(set) var revision: UInt64?
    private(set) var contentHeight: CGFloat = 0

    init(scrollView: NSScrollView) {
        self.scrollView = scrollView
    }

    func configure(revision: UInt64, contentHeight: CGFloat) {
        precondition(contentHeight.isFinite && contentHeight >= 0, "content height must be valid")
        self.revision = revision
        self.contentHeight = contentHeight
        if let documentView = scrollView.documentView {
            var frame = documentView.frame
            let viewportWidth = scrollView.contentView.bounds.width
            if viewportWidth > 0 {
                frame.size.width = viewportWidth
            }
            frame.size.height = contentHeight
            documentView.frame = frame
        }
        scrollView.contentView.needsLayout = true
    }

    func viewportMetrics() -> (scrollY: CGFloat, height: CGFloat) {
        let bounds = scrollView.contentView.bounds
        return (
            max(bounds.origin.y, 0),
            max(bounds.height, 1)
        )
    }

    func apply(
        _ request: RustCaretScrollResult,
        currentRevision: UInt64
    ) -> YuViewportApplyResult {
        guard let revision, revision == currentRevision, request.revision == revision else {
            return .stale
        }
        guard request.needsScroll else { return .noOp }

        let clipView = scrollView.contentView
        let maxScrollY = max(0, contentHeight - clipView.bounds.height)
        let target = min(max(request.targetScrollY, 0), maxScrollY)
        let current = clipView.bounds.origin.y
        guard abs(target - current) > 0.001 else { return .noOp }
        var origin = clipView.bounds.origin
        origin.y = target
        clipView.scroll(to: origin)
        scrollView.reflectScrolledClipView(clipView)
        return .scrolled(target)
    }
}

private enum YuNativeKeyKind {
    static let character = UInt8(YU_KEY_CHARACTER)
    static let enter = UInt8(YU_KEY_ENTER)
    static let tab = UInt8(YU_KEY_TAB)
    static let backspace = UInt8(YU_KEY_BACKSPACE)
    static let delete = UInt8(YU_KEY_DELETE)
    static let left = UInt8(YU_KEY_LEFT)
    static let right = UInt8(YU_KEY_RIGHT)
    static let up = UInt8(YU_KEY_UP)
    static let down = UInt8(YU_KEY_DOWN)
    static let escape = UInt8(YU_KEY_ESCAPE)
}

private enum YuNativeModifier {
    static let command = UInt8(YU_KEY_MODIFIER_COMMAND)
    static let shift = UInt8(YU_KEY_MODIFIER_SHIFT)
    static let control = UInt8(YU_KEY_MODIFIER_CONTROL)
    static let option = UInt8(YU_KEY_MODIFIER_OPTION)
}

private enum YuNativeCommand {
    static let deleteBackward = UInt8(YU_EDITOR_COMMAND_DELETE_BACKWARD)
    static let deleteForward = UInt8(YU_EDITOR_COMMAND_DELETE_FORWARD)
    static let moveLeft = UInt8(YU_EDITOR_COMMAND_MOVE_LEFT)
    static let moveRight = UInt8(YU_EDITOR_COMMAND_MOVE_RIGHT)
    static let moveWordLeft = UInt8(YU_EDITOR_COMMAND_MOVE_WORD_LEFT)
    static let moveWordRight = UInt8(YU_EDITOR_COMMAND_MOVE_WORD_RIGHT)
    static let moveUp = UInt8(YU_EDITOR_COMMAND_MOVE_UP)
    static let moveDown = UInt8(YU_EDITOR_COMMAND_MOVE_DOWN)
    static let moveUpExtend = UInt8(YU_EDITOR_COMMAND_MOVE_UP_EXTEND)
    static let moveDownExtend = UInt8(YU_EDITOR_COMMAND_MOVE_DOWN_EXTEND)
    static let insertNewline = UInt8(YU_EDITOR_COMMAND_INSERT_NEWLINE)
    static let indentList = UInt8(YU_EDITOR_COMMAND_INDENT_LIST)
    static let outdentList = UInt8(YU_EDITOR_COMMAND_OUTDENT_LIST)
    static let undo = UInt8(YU_EDITOR_COMMAND_UNDO)
    static let redo = UInt8(YU_EDITOR_COMMAND_REDO)
}

private final class RustCompositionBridge {
    private var session: OpaquePointer?
    private(set) var hasOverlay = false
    private var viewportMetrics: RustViewportMetrics?

    init(source: String) {
        var created: OpaquePointer?
        let bytes = Array(source.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_composition_session_new(buffer.baseAddress, buffer.count, &created)
        }
        precondition(status == 0 && created != nil, "Rust composition session creation failed")
        session = created
    }

    deinit {
        if let session {
            yu_composition_session_destroy(session)
        }
    }

    func resetSource(_ source: String) {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        let bytes = Array(source.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_composition_session_reset_source(session, buffer.baseAddress, buffer.count)
        }
        precondition(status == 0, "Rust composition source reset failed: \(status)")
        hasOverlay = false
        viewportMetrics = nil
    }

    func begin(replacement: NSRange, preedit: String, selection: NSRange) {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        let bytes = Array(preedit.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_composition_session_begin(
                session,
                UInt64(replacement.location),
                UInt64(NSMaxRange(replacement)),
                buffer.baseAddress,
                buffer.count,
                UInt64(selection.location),
                UInt64(NSMaxRange(selection))
            )
        }
        precondition(status == 0, "Rust composition begin failed: \(status)")
        hasOverlay = true
    }

    func update(preedit: String, selection: NSRange) {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        precondition(hasOverlay, "Rust composition update requires an active overlay")
        let bytes = Array(preedit.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_composition_session_update(
                session,
                buffer.baseAddress,
                buffer.count,
                UInt64(selection.location),
                UInt64(NSMaxRange(selection))
            )
        }
        precondition(status == 0, "Rust composition update failed: \(status)")
    }

    func commit(_ text: String) {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        precondition(hasOverlay, "Rust composition commit requires an active overlay")
        let bytes = Array(text.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_composition_session_commit(session, buffer.baseAddress, buffer.count)
        }
        precondition(status == 0, "Rust composition commit failed: \(status)")
        hasOverlay = false
    }

    func cancel() {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        guard hasOverlay else { return }
        let status = yu_composition_session_cancel(session)
        precondition(status == 0, "Rust composition cancel failed: \(status)")
        hasOverlay = false
    }

    func revision() -> UInt64 {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        var value: UInt64 = 0
        precondition(
            yu_composition_session_revision(session, &value) == 0,
            "Rust composition revision query failed"
        )
        return value
    }

    func selection() -> (
        revision: UInt64,
        range: NSRange,
        affinity: NSSelectionAffinity
    ) {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        var revision: UInt64 = 0
        var start: UInt64 = 0
        var end: UInt64 = 0
        var affinity = RustCaretAffinity.downstream.rawValue
        let status = yu_composition_session_selection(
            session,
            &revision,
            &start,
            &end,
            &affinity
        )
        precondition(status == 0, "Rust composition selection query failed: \(status)")
        precondition(end >= start, "Rust composition selection range must be ordered")
        let nativeAffinity: NSSelectionAffinity
        switch RustCaretAffinity(rawValue: affinity) {
        case .upstream:
            nativeAffinity = .upstream
        case .downstream:
            nativeAffinity = .downstream
        case nil:
            preconditionFailure("Rust composition selection affinity is invalid: \(affinity)")
        }
        return (
            revision,
            NSRange(location: Int(start), length: Int(end - start)),
            nativeAffinity
        )
    }

    func setSelection(
        _ range: NSRange,
        affinity: NSSelectionAffinity = .downstream
    ) {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        precondition(range.location >= 0 && range.length >= 0, "Rust selection must be non-negative")
        let expectedRevision = revision()
        let rustAffinity: RustCaretAffinity = affinity == .upstream ? .upstream : .downstream
        let status = yu_composition_session_set_selection(
            session,
            expectedRevision,
            UInt64(range.location),
            UInt64(NSMaxRange(range)),
            rustAffinity.rawValue
        )
        precondition(status == 0, "Rust composition selection update failed: \(status)")
    }

    func projectionCaret(
        sourceUTF16: Int,
        affinity: NSSelectionAffinity
    ) -> RustProjectionCaret {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        precondition(sourceUTF16 >= 0, "Rust projection caret must be non-negative")
        var result = YuProjectionCaret()
        let rustAffinity: RustCaretAffinity = affinity == .upstream ? .upstream : .downstream
        let status = yu_composition_session_projection_caret(
            session,
            revision(),
            UInt64(sourceUTF16),
            rustAffinity.rawValue,
            &result
        )
        precondition(status == 0, "Rust projection caret query failed: \(status)")
        let nativeAffinity: NSSelectionAffinity =
            result.affinity == UInt8(YU_CARET_AFFINITY_UPSTREAM) ? .upstream : .downstream
        return RustProjectionCaret(
            revision: result.revision,
            source: Int(result.source_utf16),
            visual: Int(result.visual_utf16),
            roundTripSource: Int(result.round_trip_source_utf16),
            affinity: nativeAffinity
        )
    }

    func blockProjectionCaret(
        sourceUTF16: Int,
        affinity: NSSelectionAffinity
    ) -> RustBlockProjectionCaret {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        precondition(sourceUTF16 >= 0, "Rust block projection caret must be non-negative")
        var result = YuBlockProjectionCaret()
        let rustAffinity: RustCaretAffinity = affinity == .upstream ? .upstream : .downstream
        let status = yu_composition_session_block_projection_caret(
            session,
            revision(),
            UInt64(sourceUTF16),
            rustAffinity.rawValue,
            &result
        )
        precondition(status == 0, "Rust block projection caret query failed: \(status)")
        let nativeAffinity: NSSelectionAffinity =
            result.affinity == UInt8(YU_CARET_AFFINITY_UPSTREAM) ? .upstream : .downstream
        return RustBlockProjectionCaret(
            revision: result.revision,
            source: Int(result.source_utf16),
            block: Int(result.block_index),
            visual: Int(result.visual_utf16),
            roundTripSource: Int(result.round_trip_source_utf16),
            affinity: nativeAffinity
        )
    }

    func blockShapedCaret(
        sourceUTF16: Int,
        affinity: NSSelectionAffinity,
        size: CGFloat,
        maxWidth: CGFloat
    ) -> RustBlockShapedCaret {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        precondition(sourceUTF16 >= 0, "Rust block shaped caret must be non-negative")
        var result = YuBlockShapedCaret()
        let rustAffinity: RustCaretAffinity = affinity == .upstream ? .upstream : .downstream
        let status = yu_macos_composition_session_block_shaped_caret(
            session,
            revision(),
            UInt64(sourceUTF16),
            rustAffinity.rawValue,
            Float(size),
            Float(maxWidth),
            &result
        )
        precondition(status == 0, "Rust block shaped caret query failed: \(status)")
        let nativeAffinity: NSSelectionAffinity =
            result.affinity == UInt8(YU_CARET_AFFINITY_UPSTREAM) ? .upstream : .downstream
        return RustBlockShapedCaret(
            revision: result.revision,
            source: Int(result.source_utf16),
            block: Int(result.block_index),
            visual: Int(result.visual_utf16),
            roundTripSource: Int(result.round_trip_source_utf16),
            line: Int(result.line_index),
            x: CGFloat(result.caret_x),
            y: CGFloat(result.caret_y),
            width: CGFloat(result.caret_width),
            height: CGFloat(result.caret_height),
            affinity: nativeAffinity
        )
    }

    func executeCommand(command: UInt8, block: UInt64 = 0) -> RustCommandResult {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        var result = YuEditorCommandResult()
        let status = yu_composition_session_execute_command(
            session,
            command,
            block,
            &result
        )
        precondition(status == 0, "Rust editor command failed: \(status)")
        return commandResult(result)
    }

    func commandAvailable(command: UInt8, block: UInt64 = 0) -> Bool {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        var available: UInt8 = UInt8(YU_COMMAND_UNAVAILABLE)
        let status = yu_composition_session_command_available(
            session,
            command,
            block,
            &available
        )
        precondition(status == 0, "Rust command availability query failed: \(status)")
        return available == UInt8(YU_COMMAND_AVAILABLE)
    }

    func routeKey(
        kind: UInt8,
        value: UInt32 = 0,
        modifiers: UInt8 = 0
    ) -> RustCommandResult? {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        var result = YuEditorCommandResult()
        let status = yu_composition_session_route_key(
            session,
            kind,
            value,
            modifiers,
            &result
        )
        if status == Int32(YU_FFI_KEY_UNHANDLED) {
            return nil
        }
        precondition(status == 0, "Rust native key route failed: \(status)")
        return commandResult(result)
    }

    func setViewportMetrics(_ metrics: RustViewportMetrics) {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        guard viewportMetrics != metrics else { return }
        let status = yu_composition_session_set_viewport_config(
            session,
            revision(),
            Float(metrics.maxWidth),
            Float(metrics.lineHeight),
            Float(metrics.defaultAdvance),
            Float(metrics.estimatedBlockHeight),
            Float(metrics.overscan)
        )
        precondition(status == 0, "Rust viewport metrics update failed: \(status)")
        viewportMetrics = metrics
    }

    func coreTextViewportMetrics(
        family: String,
        size: CGFloat,
        sample: String
    ) -> (lineHeight: CGFloat, defaultAdvance: CGFloat) {
        let familyBytes = Array(family.utf8)
        let sampleBytes = Array(sample.utf8)
        var result = YuCoreTextViewportMetrics()
        let status = familyBytes.withUnsafeBufferPointer { familyBuffer in
            sampleBytes.withUnsafeBufferPointer { sampleBuffer in
                yu_macos_core_text_viewport_metrics(
                    familyBuffer.baseAddress,
                    familyBuffer.count,
                    Float(size),
                    sampleBuffer.baseAddress,
                    sampleBuffer.count,
                    &result
                )
            }
        }
        precondition(status == 0, "CoreText viewport metrics query failed: \(status)")
        return (
            lineHeight: CGFloat(result.line_height),
            defaultAdvance: CGFloat(result.default_advance)
        )
    }

    func coreTextSystemUiViewportMetrics(
        size: CGFloat,
        sample: String
    ) -> (lineHeight: CGFloat, defaultAdvance: CGFloat) {
        let sampleBytes = Array(sample.utf8)
        var result = YuCoreTextViewportMetrics()
        let status = sampleBytes.withUnsafeBufferPointer { sampleBuffer in
            yu_macos_core_text_system_ui_viewport_metrics(
                Float(size),
                sampleBuffer.baseAddress,
                sampleBuffer.count,
                &result
            )
        }
        precondition(status == 0, "CoreText system UI viewport metrics query failed: \(status)")
        return (
            lineHeight: CGFloat(result.line_height),
            defaultAdvance: CGFloat(result.default_advance)
        )
    }

    func coreTextSystemUiShapedLines(
        size: CGFloat,
        maxWidth: CGFloat,
        source: String
    ) -> [YuCoreTextShapedLine] {
        let sourceBytes = Array(source.utf8)
        var required = 0
        let countStatus = sourceBytes.withUnsafeBufferPointer { sourceBuffer in
            yu_macos_core_text_shaped_lines(
                Float(size),
                Float(maxWidth),
                sourceBuffer.baseAddress,
                sourceBuffer.count,
                nil,
                0,
                &required
            )
        }
        precondition(countStatus == 0, "CoreText shaped line count failed: \(countStatus)")
        guard required > 0 else { return [] }

        var lines = [YuCoreTextShapedLine](repeating: YuCoreTextShapedLine(), count: required)
        var written = 0
        let fillStatus = sourceBytes.withUnsafeBufferPointer { sourceBuffer in
            lines.withUnsafeMutableBufferPointer { lineBuffer in
                yu_macos_core_text_shaped_lines(
                    Float(size),
                    Float(maxWidth),
                    sourceBuffer.baseAddress,
                    sourceBuffer.count,
                    lineBuffer.baseAddress,
                    lineBuffer.count,
                    &written
                )
            }
        }
        precondition(fillStatus == 0, "CoreText shaped line fill failed: \(fillStatus)")
        precondition(written == required, "CoreText shaped line count changed during fill")
        return lines
    }

    func coreTextSystemUiProjectedLayout(
        size: CGFloat,
        maxWidth: CGFloat,
        source: String
    ) -> (lines: [YuCoreTextProjectedLine], projected: String) {
        let sourceBytes = Array(source.utf8)
        var requiredLines = 0
        var requiredVisualBytes = 0
        let countStatus = sourceBytes.withUnsafeBufferPointer { sourceBuffer in
            yu_macos_core_text_projected_layout(
                Float(size),
                Float(maxWidth),
                sourceBuffer.baseAddress,
                sourceBuffer.count,
                nil,
                0,
                &requiredLines,
                nil,
                0,
                &requiredVisualBytes
            )
        }
        precondition(countStatus == 0, "CoreText projected layout count failed: \(countStatus)")

        var lines = [YuCoreTextProjectedLine](
            repeating: YuCoreTextProjectedLine(),
            count: requiredLines
        )
        var visualBytes = [UInt8](repeating: 0, count: requiredVisualBytes)
        var writtenLines = 0
        var writtenVisualBytes = 0
        let fillStatus = sourceBytes.withUnsafeBufferPointer { sourceBuffer in
            lines.withUnsafeMutableBufferPointer { lineBuffer in
                visualBytes.withUnsafeMutableBufferPointer { visualBuffer in
                    yu_macos_core_text_projected_layout(
                        Float(size),
                        Float(maxWidth),
                        sourceBuffer.baseAddress,
                        sourceBuffer.count,
                        lineBuffer.baseAddress,
                        lineBuffer.count,
                        &writtenLines,
                        visualBuffer.baseAddress,
                        visualBuffer.count,
                        &writtenVisualBytes
                    )
                }
            }
        }
        precondition(fillStatus == 0, "CoreText projected layout fill failed: \(fillStatus)")
        precondition(writtenLines == requiredLines, "Projected line count changed during fill")
        precondition(
            writtenVisualBytes == requiredVisualBytes,
            "Projected visual byte count changed during fill"
        )
        return (
            lines,
            String(decoding: visualBytes, as: UTF8.self)
        )
    }

    func caretScrollRequest(
        scrollY: CGFloat,
        viewportHeight: CGFloat,
        margin: CGFloat
    ) -> RustCaretScrollResult {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        var result = YuEditorCaretScrollRequest()
        let status = yu_composition_session_caret_scroll_request(
            session,
            revision(),
            Float(scrollY),
            Float(viewportHeight),
            Float(margin),
            &result
        )
        precondition(status == 0, "Rust caret scroll request failed: \(status)")
        return RustCaretScrollResult(
            revision: result.revision,
            source: Int(result.source_utf16),
            block: Int(result.block_index),
            caretX: CGFloat(result.caret_x),
            caretY: CGFloat(result.caret_y),
            caretWidth: CGFloat(result.caret_width),
            caretHeight: CGFloat(result.caret_height),
            currentScrollY: CGFloat(result.current_scroll_y),
            targetScrollY: CGFloat(result.target_scroll_y),
            margin: CGFloat(result.margin),
            needsScroll: result.needs_scroll != 0
        )
    }

    func shapedCaretScrollRequest(
        size: CGFloat,
        maxWidth: CGFloat,
        scrollY: CGFloat,
        viewportHeight: CGFloat,
        margin: CGFloat
    ) -> RustCaretScrollResult {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        var result = YuEditorCaretScrollRequest()
        let status = yu_macos_composition_session_shaped_caret_scroll_request(
            session,
            revision(),
            Float(size),
            Float(maxWidth),
            Float(scrollY),
            Float(viewportHeight),
            Float(margin),
            &result
        )
        precondition(status == 0, "Rust shaped caret scroll request failed: \(status)")
        return RustCaretScrollResult(
            revision: result.revision,
            source: Int(result.source_utf16),
            block: Int(result.block_index),
            caretX: CGFloat(result.caret_x),
            caretY: CGFloat(result.caret_y),
            caretWidth: CGFloat(result.caret_width),
            caretHeight: CGFloat(result.caret_height),
            currentScrollY: CGFloat(result.current_scroll_y),
            targetScrollY: CGFloat(result.target_scroll_y),
            margin: CGFloat(result.margin),
            needsScroll: result.needs_scroll != 0
        )
    }

    func sourceString(utf16Length: Int) -> String {
        precondition(utf16Length >= 0, "Rust composition UTF-16 length must be non-negative")
        return sourceString(utf16Range: NSRange(location: 0, length: utf16Length))
    }

    func sourceString(utf16Range: NSRange) -> String {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        precondition(
            utf16Range.location >= 0 && utf16Range.length >= 0,
            "Rust composition UTF-16 range must be non-negative"
        )
        let expectedRevision = revision()
        var length = 0
        precondition(
            yu_composition_session_source_range_length(
                session,
                expectedRevision,
                UInt64(utf16Range.location),
                UInt64(NSMaxRange(utf16Range)),
                &length
            ) == 0,
            "Rust composition source range length failed"
        )
        var bytes = [UInt8](repeating: 0, count: length)
        var written = 0
        let status = bytes.withUnsafeMutableBufferPointer { buffer in
            yu_composition_session_copy_source_range(
                session,
                expectedRevision,
                UInt64(utf16Range.location),
                UInt64(NSMaxRange(utf16Range)),
                buffer.baseAddress,
                buffer.count,
                &written
            )
        }
        precondition(status == 0 && written == length, "Rust composition source range copy failed: \(status)")
        return String(decoding: bytes, as: UTF8.self)
    }

    func sourceString() -> String {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        var length = 0
        precondition(
            yu_composition_session_source_length(session, &length) == 0,
            "Rust composition source length failed"
        )
        var bytes = [UInt8](repeating: 0, count: length)
        let status = bytes.withUnsafeMutableBufferPointer { buffer in
            yu_composition_session_copy_source(session, buffer.baseAddress, buffer.count)
        }
        precondition(status == 0, "Rust composition source copy failed: \(status)")
        return String(decoding: bytes, as: UTF8.self)
    }

    func overlayString() -> String {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        precondition(hasOverlay, "Rust composition overlay is not active")
        var length = 0
        precondition(
            yu_composition_session_overlay_length(session, &length) == 0,
            "Rust composition overlay length failed"
        )
        var bytes = [UInt8](repeating: 0, count: length)
        let status = bytes.withUnsafeMutableBufferPointer { buffer in
            yu_composition_session_copy_overlay(session, buffer.baseAddress, buffer.count)
        }
        precondition(status == 0, "Rust composition overlay copy failed: \(status)")
        return String(decoding: bytes, as: UTF8.self)
    }

    func overlaySelection() -> NSRange {
        guard let session else { preconditionFailure("Rust composition session is missing") }
        precondition(hasOverlay, "Rust composition overlay is not active")
        var start: UInt64 = 0
        var end: UInt64 = 0
        let status = yu_composition_session_overlay_selection(session, &start, &end)
        precondition(status == 0, "Rust composition selection query failed: \(status)")
        return NSRange(location: Int(start), length: Int(end - start))
    }

    private func commandResult(_ result: YuEditorCommandResult) -> RustCommandResult {
        precondition(
            result.selection_end_utf16 >= result.selection_start_utf16,
            "Rust command selection range must be ordered"
        )
        let nativeAffinity: NSSelectionAffinity
        switch RustCaretAffinity(rawValue: result.affinity) {
        case .upstream:
            nativeAffinity = .upstream
        case .downstream:
            nativeAffinity = .downstream
        case nil:
            preconditionFailure("Rust command selection affinity is invalid: \(result.affinity)")
        }
        guard let sourceSync = RustSourceSync(rawValue: result.source_sync) else {
            preconditionFailure("Rust command source sync kind is invalid: \(result.source_sync)")
        }
        let oldSourceRange: NSRange?
        let newSourceRange: NSRange?
        if sourceSync == .range {
            precondition(
                result.source_old_end_utf16 >= result.source_start_utf16
                    && result.source_new_end_utf16 >= result.source_new_start_utf16,
                "Rust command source range must be ordered"
            )
            oldSourceRange = NSRange(
                location: Int(result.source_start_utf16),
                length: Int(result.source_old_end_utf16 - result.source_start_utf16)
            )
            newSourceRange = NSRange(
                location: Int(result.source_new_start_utf16),
                length: Int(result.source_new_end_utf16 - result.source_new_start_utf16)
            )
        } else {
            oldSourceRange = nil
            newSourceRange = nil
        }
        return RustCommandResult(
            revision: result.revision,
            range: NSRange(
                location: Int(result.selection_start_utf16),
                length: Int(result.selection_end_utf16 - result.selection_start_utf16)
            ),
            affinity: nativeAffinity,
            changed: result.changed != 0,
            sourceSync: sourceSync,
            oldSourceRange: oldSourceRange,
            newSourceRange: newSourceRange
        )
    }
}

final class TextInputView: NSView, NSTextInputClient {
    private let textStorage = NSTextStorage()
    private let layoutManager = NSLayoutManager()
    private let textContainer = NSTextContainer()
    private var selection = NSRange(location: 0, length: 0)
    private var selectionAffinity: NSSelectionAffinity = .downstream
    private var marked = notFoundRange
    private var compositionOriginal: NSAttributedString?
    private var compositionSelectionBefore: NSRange?
    private var compositionAffinityBefore: NSSelectionAffinity?
    private var rustComposition: RustCompositionBridge!
    private var viewportAdapter: YuNativeViewportAdapter?
    private var synchronizingViewport = false

    private let textOrigin = NSPoint(x: 24, y: 24)
    private let maximumTextWidth: CGFloat = 360
    private let defaultAttributes: [NSAttributedString.Key: Any] = [
        .font: NSFont.systemFont(ofSize: 22),
        .foregroundColor: NSColor.labelColor,
    ]

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        textContainer.lineFragmentPadding = 0
        layoutManager.addTextContainer(textContainer)
        textStorage.addLayoutManager(layoutManager)
        replaceStorage(
            range: NSRange(location: 0, length: 0),
            with: NSAttributedString(
                string: "Yu macOS IME spike\n\n请点击这里并输入中文、日文、emoji 或组合字符。\n",
                attributes: defaultAttributes
            )
        )
        selection = NSRange(location: textStorage.length, length: 0)
        rustComposition = RustCompositionBridge(source: textStorage.string)
        setAccessibilityElement(true)
        setAccessibilityRole(.textArea)
        setAccessibilityLabel("Yu Editor document")
        setAccessibilityIdentifier("yu-editor-document")
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    override var acceptsFirstResponder: Bool { true }
    override var isFlipped: Bool { true }

    fileprivate func attachViewportAdapter(_ adapter: YuNativeViewportAdapter) {
        viewportAdapter = adapter
        synchronizeViewport()
    }

    override func updateLayer() {
        layer?.backgroundColor = NSColor.textBackgroundColor.cgColor
    }

    override func layout() {
        super.layout()
        updateContainerSize()
        synchronizeViewport()
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        updateContainerSize()
        layoutManager.ensureLayout(for: textContainer)

        let glyphRange = layoutManager.glyphRange(for: textContainer)
        layoutManager.drawBackground(forGlyphRange: glyphRange, at: textOrigin)
        layoutManager.drawGlyphs(forGlyphRange: glyphRange, at: textOrigin)

        if window?.firstResponder === self {
            let caret = caretRect(at: selection.location, affinity: selectionAffinity)
            NSColor.controlAccentColor.setFill()
            caret.fill()
        }
    }

    override func keyDown(with event: NSEvent) {
        if routeNativeKey(event) {
            return
        }
        if inputContext?.handleEvent(event) != true {
            super.keyDown(with: event)
        }
    }

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        if hasMarkedText() {
            cancelComposition()
        } else {
            inputContext?.discardMarkedText()
            marked = notFoundRange
            rustComposition.cancel()
        }
        let point = convert(event.locationInWindow, from: nil)
        let hit = caretHit(forLocalPoint: point)
        selection = NSRange(location: hit.index, length: 0)
        selectionAffinity = hit.affinity
        rustComposition.setSelection(selection, affinity: selectionAffinity)
        needsDisplay = true
        postSelectionChanged()
        synchronizeViewport()
    }

    func insertText(_ value: Any, replacementRange: NSRange) {
        let inserted = attributedString(from: value, marked: false)
        let target = targetRange(replacementRange)
        print("insertText commit=\(inserted.string.debugDescription) replace=\(target)")
        if !rustComposition.hasOverlay {
            rustComposition.begin(
                replacement: target,
                preedit: "",
                selection: NSRange(location: 0, length: 0)
            )
        }
        rustComposition.commit(inserted.string)
        replaceStorage(range: target, with: inserted)
        let rustSelection = rustComposition.selection()
        let expectedSelection = NSRange(location: target.location + inserted.length, length: 0)
        precondition(
            rustSelection.revision == rustComposition.revision()
                && rustSelection.range == expectedSelection,
            "Rust selection must follow committed source"
        )
        selection = rustSelection.range
        selectionAffinity = .downstream
        marked = notFoundRange
        compositionOriginal = nil
        compositionSelectionBefore = nil
        compositionAffinityBefore = nil
        needsDisplay = true
        postTextChanged()
        synchronizeViewport()
    }

    func setMarkedText(
        _ value: Any,
        selectedRange newSelection: NSRange,
        replacementRange: NSRange
    ) {
        let inserted = attributedString(from: value, marked: true)
        let target = targetRange(replacementRange)
        if !hasMarkedText() {
            compositionOriginal = textStorage.attributedSubstring(from: target)
            compositionSelectionBefore = selection
            compositionAffinityBefore = selectionAffinity
        }
        if rustComposition.hasOverlay {
            rustComposition.update(preedit: inserted.string, selection: newSelection)
        } else {
            rustComposition.begin(
                replacement: target,
                preedit: inserted.string,
                selection: newSelection
            )
        }
        print(
            "setMarkedText preedit=\(inserted.string.debugDescription) "
                + "selection=\(newSelection) replace=\(target)"
        )
        replaceStorage(range: target, with: inserted)
        marked = NSRange(location: target.location, length: inserted.length)

        let relativeLocation = min(newSelection.location, inserted.length)
        let maximumLength = inserted.length - relativeLocation
        selection = NSRange(
            location: target.location + relativeLocation,
            length: min(newSelection.length, maximumLength)
        )
        selectionAffinity = .downstream
        needsDisplay = true
        postTextChanged()
    }

    func unmarkText() {
        print("unmarkText range=\(marked)")
        if hasMarkedText() {
            textStorage.removeAttribute(.underlineStyle, range: marked)
        }
        marked = notFoundRange
        compositionOriginal = nil
        compositionSelectionBefore = nil
        compositionAffinityBefore = nil
        needsDisplay = true
        postSelectionChanged()
    }

    func selectedRange() -> NSRange {
        selection
    }

    func markedRange() -> NSRange {
        marked
    }

    func hasMarkedText() -> Bool {
        marked.location != NSNotFound && marked.length > 0
    }

    func attributedSubstring(
        forProposedRange proposedRange: NSRange,
        actualRange: NSRangePointer?
    ) -> NSAttributedString? {
        let range = clamped(proposedRange)
        actualRange?.pointee = range
        guard range.location != NSNotFound else { return nil }
        return textStorage.attributedSubstring(from: range)
    }

    func validAttributesForMarkedText() -> [NSAttributedString.Key] {
        [.font, .foregroundColor, .underlineStyle]
    }

    func firstRect(
        forCharacterRange characterRange: NSRange,
        actualRange: NSRangePointer?
    ) -> NSRect {
        let range = clamped(characterRange)
        actualRange?.pointee = range
        let localRect: NSRect
        if range.length == 0 {
            let affinity = range == selection ? selectionAffinity : .downstream
            localRect = caretRect(at: range.location, affinity: affinity)
        } else {
            let glyphRange = layoutManager.glyphRange(
                forCharacterRange: range,
                actualCharacterRange: nil
            )
            let bounds = layoutManager.boundingRect(forGlyphRange: glyphRange, in: textContainer)
            localRect = bounds.offsetBy(dx: textOrigin.x, dy: textOrigin.y)
        }

        let windowRect = convert(localRect, to: nil)
        return window?.convertToScreen(windowRect) ?? windowRect
    }

    func characterIndex(for point: NSPoint) -> Int {
        let windowPoint = window?.convertPoint(fromScreen: point) ?? point
        return caretHit(forLocalPoint: convert(windowPoint, from: nil)).index
    }

    func fractionOfDistanceThroughGlyph(for point: NSPoint) -> CGFloat {
        let windowPoint = window?.convertPoint(fromScreen: point) ?? point
        let local = convert(windowPoint, from: nil)
        let containerPoint = NSPoint(x: local.x - textOrigin.x, y: local.y - textOrigin.y)
        updateContainerSize()
        return layoutManager.fractionOfDistanceThroughGlyph(
            for: containerPoint,
            in: textContainer
        )
    }

    override func accessibilityValue() -> Any? {
        textStorage.string
    }

    override func accessibilityNumberOfCharacters() -> Int {
        textStorage.length
    }

    override func accessibilitySelectedText() -> String? {
        let range = validatedAccessibilityRange(selection) ?? NSRange(location: 0, length: 0)
        return (textStorage.string as NSString).substring(with: range)
    }

    override func accessibilitySelectedTextRange() -> NSRange {
        selection
    }

    override func setAccessibilitySelectedTextRange(_ range: NSRange) {
        if hasMarkedText() {
            cancelComposition()
        } else {
            inputContext?.discardMarkedText()
            marked = notFoundRange
            rustComposition.cancel()
        }
        guard let range = validatedAccessibilityRange(range) else { return }
        selection = range
        selectionAffinity = .downstream
        rustComposition.setSelection(selection)
        needsDisplay = true
        postSelectionChanged()
        synchronizeViewport()
    }

    override func accessibilitySelectedTextRanges() -> [NSValue]? {
        [NSValue(range: selection)]
    }

    override func accessibilityVisibleCharacterRange() -> NSRange {
        updateContainerSize()
        layoutManager.ensureLayout(for: textContainer)
        guard layoutManager.numberOfGlyphs > 0 else {
            return NSRange(location: 0, length: 0)
        }
        let visible = NSRect(
            x: bounds.minX - textOrigin.x,
            y: bounds.minY - textOrigin.y,
            width: bounds.width,
            height: bounds.height
        )
        let glyphs = layoutManager.glyphRange(forBoundingRect: visible, in: textContainer)
        return layoutManager.characterRange(forGlyphRange: glyphs, actualGlyphRange: nil)
    }

    override func accessibilityInsertionPointLineNumber() -> Int {
        logicalLine(containing: selection.location)
    }

    override func accessibilityString(for range: NSRange) -> String? {
        guard let range = validatedAccessibilityRange(range) else { return nil }
        return (textStorage.string as NSString).substring(with: range)
    }

    override func accessibilityAttributedString(for range: NSRange) -> NSAttributedString? {
        guard let range = validatedAccessibilityRange(range) else { return nil }
        return textStorage.attributedSubstring(from: range)
    }

    override func accessibilityRange(forLine line: Int) -> NSRange {
        logicalLineRange(line)
    }

    override func accessibilityLine(for index: Int) -> Int {
        guard index >= 0, index <= textStorage.length else { return NSNotFound }
        return logicalLine(containing: index)
    }

    override func accessibilityRange(for index: Int) -> NSRange {
        guard index >= 0, index <= textStorage.length else { return notFoundRange }
        guard index < textStorage.length else {
            return NSRange(location: textStorage.length, length: 0)
        }
        return (textStorage.string as NSString).rangeOfComposedCharacterSequence(at: index)
    }

    override func accessibilityRange(for position: NSPoint) -> NSRange {
        accessibilityRange(for: characterIndex(for: position))
    }

    override func accessibilityFrame(for range: NSRange) -> NSRect {
        guard let range = validatedAccessibilityRange(range) else { return .zero }
        return firstRect(forCharacterRange: range, actualRange: nil)
    }

    override func accessibilityStyleRange(for index: Int) -> NSRange {
        guard index >= 0, index < textStorage.length else { return notFoundRange }
        var effective = NSRange(location: 0, length: 0)
        _ = textStorage.attributes(at: index, effectiveRange: &effective)
        return effective
    }

    func runAccessibilitySelfCheck() {
        let full = NSRange(location: 0, length: accessibilityNumberOfCharacters())
        let firstLine = accessibilityRange(forLine: 0)
        let firstText = accessibilityString(for: firstLine) ?? ""
        let caretFrame = accessibilityFrame(for: accessibilitySelectedTextRange())
        precondition(accessibilityString(for: full) == textStorage.string)
        precondition(firstLine.location == 0 && firstLine.location != NSNotFound)
        precondition(!firstText.isEmpty)
        precondition(!caretFrame.isEmpty)
        print(
            "AX self-check characters=\(accessibilityNumberOfCharacters()) "
                + "selection=\(accessibilitySelectedTextRange()) "
                + "firstLine=\(firstLine) caretFrame=\(caretFrame)"
        )
    }

    func runNativeSelectionSelfCheck() {
        let savedSelection = selection
        let savedAffinity = selectionAffinity
        let source = textStorage.string as NSString
        let probe = source.range(of: "请点击")
        precondition(probe.location != NSNotFound, "selection probe text must exist")

        let base = textStorage.string
        setMarkedText(
            "にほん",
            selectedRange: NSRange(location: 3, length: 0),
            replacementRange: notFoundRange
        )
        precondition(hasMarkedText(), "selection mutation should start from a marked overlay")
        setAccessibilitySelectedTextRange(probe)
        precondition(
            !hasMarkedText() && textStorage.string == base,
            "native selection mutation must cancel the temporary overlay"
        )
        let rustSelection = rustComposition.selection()
        precondition(
            rustSelection.revision == rustComposition.revision()
                && rustSelection.range == probe,
            "native selection mutation must update Rust"
        )
        selectionAffinity = .upstream
        rustComposition.setSelection(selection, affinity: selectionAffinity)
        let upstreamSelection = rustComposition.selection()
        precondition(
            upstreamSelection.range == probe && upstreamSelection.affinity == .upstream,
            "native selection affinity must round-trip through Rust"
        )

        selection = savedSelection
        selectionAffinity = savedAffinity
        rustComposition.setSelection(selection, affinity: selectionAffinity)
        needsDisplay = true
        postSelectionChanged()
        let restored = rustComposition.selection()
        precondition(restored.range == savedSelection, "native selection self-check must restore")
        let affinityName = upstreamSelection.affinity == .upstream ? "upstream" : "downstream"
        print(
            "Native selection self-check probe=\(probe) "
                + "revision=\(rustSelection.revision) affinity=\(affinityName) "
                + "restored=\(restored.range)"
        )
    }

    func runNativeCommandRoutingSelfCheck() {
        let savedStorage = NSAttributedString(attributedString: textStorage)
        let savedSelection = selection
        let savedAffinity = selectionAffinity
        let base = textStorage.string

        marked = notFoundRange
        selection = NSRange(location: textStorage.length, length: 0)
        selectionAffinity = .downstream
        rustComposition.setSelection(selection)
        insertText("z", replacementRange: notFoundRange)
        guard
            let backspace = rustComposition.routeKey(
                kind: YuNativeKeyKind.backspace
            )
        else {
            preconditionFailure("Backspace must be consumed by the Rust command route")
        }
        precondition(backspace.sourceSync == .range, "Backspace should return a local source range")
        applyRustCommandResult(backspace)
        precondition(textStorage.string == base, "Backspace must restore the source")

        insertText("z", replacementRange: notFoundRange)
        precondition(
            rustComposition.commandAvailable(command: YuNativeCommand.deleteBackward),
            "Selector delete should be reported as available"
        )
        doCommand(by: #selector(NSResponder.deleteBackward(_:)))
        precondition(textStorage.string == base, "doCommand delete must restore the source")

        insertText("z", replacementRange: notFoundRange)
        let insertedSelection = selection
        precondition(
            rustComposition.commandAvailable(command: YuNativeCommand.moveWordLeft),
            "Selector word-left should be reported as available"
        )
        doCommand(by: #selector(NSResponder.moveWordLeft(_:)))
        precondition(selection.location < insertedSelection.location, "word-left must move the caret")
        doCommand(by: #selector(NSResponder.moveWordRight(_:)))
        precondition(selection == insertedSelection, "word-right must restore the caret")
        doCommand(by: #selector(NSResponder.deleteBackward(_:)))
        precondition(textStorage.string == base, "word Selector commands must preserve the source")

        insertText("z", replacementRange: notFoundRange)
        let afterInsert = textStorage.string

        guard
            let undo = rustComposition.routeKey(
                kind: YuNativeKeyKind.character,
                value: 0x7a,
                modifiers: YuNativeModifier.command
            )
        else {
            preconditionFailure("Cmd-Z must be consumed by the Rust command route")
        }
        precondition(undo.sourceSync == .full, "grouped Undo should request full source sync")
        applyRustCommandResult(undo)
        precondition(textStorage.string == base, "Cmd-Z must restore the source")

        guard
            let redo = rustComposition.routeKey(
                kind: YuNativeKeyKind.character,
                value: 0x7a,
                modifiers: YuNativeModifier.command | YuNativeModifier.shift
            )
        else {
            preconditionFailure("Cmd-Shift-Z must be consumed by the Rust command route")
        }
        precondition(redo.sourceSync == .full, "grouped Redo should request full source sync")
        applyRustCommandResult(redo)
        precondition(textStorage.string == afterInsert, "Cmd-Shift-Z must restore the edit")

        let verticalSource = "abcdefghij\nxy\n1234567890"
        replaceStorage(
            range: NSRange(location: 0, length: textStorage.length),
            with: attributedString(from: verticalSource, marked: false)
        )
        rustComposition.resetSource(verticalSource)
        selection = NSRange(location: 10, length: 0)
        selectionAffinity = .downstream
        rustComposition.setSelection(selection, affinity: selectionAffinity)
        doCommand(by: #selector(NSResponder.moveDown(_:)))
        precondition(selection.location == 13, "Selector moveDown must use Rust layout hit-test")
        doCommand(by: #selector(NSResponder.moveDown(_:)))
        precondition(
            selection.location == 24,
            "repeated Selector moveDown must retain Rust preferred X"
        )

        let crossBlockSource = "# title\ntext"
        replaceStorage(
            range: NSRange(location: 0, length: textStorage.length),
            with: attributedString(from: crossBlockSource, marked: false)
        )
        rustComposition.resetSource(crossBlockSource)
        selection = NSRange(location: 8, length: 0)
        selectionAffinity = .downstream
        rustComposition.setSelection(selection, affinity: selectionAffinity)
        doCommand(by: #selector(NSResponder.moveUp(_:)))
        precondition(selection.location == 0, "Selector moveUp must cross a Markdown block")
        doCommand(by: #selector(NSResponder.moveDown(_:)))
        precondition(selection.location == 8, "Selector moveDown must return to the next block")

        let extendSource = "one\ntwo\nthree"
        replaceStorage(
            range: NSRange(location: 0, length: textStorage.length),
            with: attributedString(from: extendSource, marked: false)
        )
        rustComposition.resetSource(extendSource)
        selection = NSRange(location: 0, length: 0)
        selectionAffinity = .downstream
        rustComposition.setSelection(selection, affinity: selectionAffinity)
        doCommand(by: #selector(NSResponder.moveDownAndModifySelection(_:)))
        precondition(
            selection == NSRange(location: 0, length: 4),
            "Selector shift-down must extend the Rust selection"
        )
        doCommand(by: #selector(NSResponder.moveDownAndModifySelection(_:)))
        precondition(
            selection == NSRange(location: 0, length: 8),
            "repeated Selector shift-down must preserve the anchor"
        )
        doCommand(by: #selector(NSResponder.moveUpAndModifySelection(_:)))
        precondition(
            selection == NSRange(location: 0, length: 4),
            "Selector shift-up must contract toward the anchor"
        )

        replaceStorage(
            range: NSRange(location: 0, length: textStorage.length),
            with: savedStorage
        )
        selection = savedSelection
        selectionAffinity = savedAffinity
        marked = notFoundRange
        compositionOriginal = nil
        compositionSelectionBefore = nil
        compositionAffinityBefore = nil
        rustComposition.resetSource(textStorage.string)
        rustComposition.setSelection(selection, affinity: selectionAffinity)
        needsDisplay = true
        print(
            "Native command self-check local=Backspace undo=Cmd-Z redo=Cmd-Shift-Z "
                + "source=restored changed=\(backspace.changed)/\(undo.changed)/\(redo.changed)"
        )
    }

    func runViewportScrollSelfCheck() {
        let savedStorage = NSAttributedString(attributedString: textStorage)
        let savedSelection = selection
        let savedAffinity = selectionAffinity
        let source = "one\n\ntwo\n\nthree"

        replaceStorage(
            range: NSRange(location: 0, length: textStorage.length),
            with: attributedString(from: source, marked: false)
        )
        rustComposition.resetSource(source)
        rustComposition.setViewportMetrics(
            RustViewportMetrics(
                maxWidth: 80,
                lineHeight: 1,
                defaultAdvance: 1,
                estimatedBlockHeight: 1,
                overscan: 0
            )
        )
        selection = NSRange(location: source.utf16.count, length: 0)
        selectionAffinity = .downstream
        rustComposition.setSelection(selection, affinity: selectionAffinity)

        let reveal = rustComposition.caretScrollRequest(
            scrollY: 0,
            viewportHeight: 1,
            margin: 0
        )
        precondition(
            reveal.revision == rustComposition.revision()
                && reveal.source == source.utf16.count
                && reveal.block == 4
                && reveal.caretY == 4
                && reveal.targetScrollY == 4
                && reveal.needsScroll,
            "Rust caret scroll request must reveal the focus"
        )

        let nativeScrollView = NSScrollView(
            frame: NSRect(x: 0, y: 0, width: 10, height: 1)
        )
        nativeScrollView.hasVerticalScroller = false
        nativeScrollView.documentView = NSView(
            frame: NSRect(x: 0, y: 0, width: 10, height: 5)
        )
        let viewportAdapter = YuNativeViewportAdapter(scrollView: nativeScrollView)
        viewportAdapter.configure(revision: reveal.revision, contentHeight: 5)
        guard case .scrolled(let nativeTarget) = viewportAdapter.apply(
            reveal,
            currentRevision: reveal.revision
        ) else {
            preconditionFailure("native viewport must consume a visible caret request")
        }
        precondition(
            abs(nativeTarget - reveal.targetScrollY) < 0.001
                && abs(nativeScrollView.contentView.bounds.origin.y - nativeTarget) < 0.001,
            "native viewport must apply the absolute Rust target"
        )
        precondition(
            viewportAdapter.apply(reveal, currentRevision: reveal.revision + 1) == .stale,
            "native viewport must reject stale caret geometry"
        )

        let visible = rustComposition.caretScrollRequest(
            scrollY: reveal.targetScrollY,
            viewportHeight: 1,
            margin: 0
        )
        precondition(
            !visible.needsScroll && visible.targetScrollY == reveal.targetScrollY,
            "visible caret must produce a no-op scroll request"
        )
        precondition(
            viewportAdapter.apply(visible, currentRevision: visible.revision) == .noOp,
            "native viewport must preserve a visible caret without movement"
        )

        selection = NSRange(location: 0, length: 0)
        rustComposition.setSelection(selection)
        let top = rustComposition.caretScrollRequest(
            scrollY: reveal.targetScrollY,
            viewportHeight: 1,
            margin: 0
        )
        precondition(top.needsScroll && top.targetScrollY == 0, "top caret must scroll back")
        guard case .scrolled(let topTarget) = viewportAdapter.apply(
            top,
            currentRevision: top.revision
        ) else {
            preconditionFailure("native viewport must consume the top reveal request")
        }
        precondition(
            abs(topTarget) < 0.001
                && abs(nativeScrollView.contentView.bounds.origin.y) < 0.001,
            "native viewport must scroll back to the document origin"
        )

        replaceStorage(
            range: NSRange(location: 0, length: textStorage.length),
            with: savedStorage
        )
        selection = savedSelection
        selectionAffinity = savedAffinity
        rustComposition.resetSource(textStorage.string)
        rustComposition.setSelection(selection, affinity: selectionAffinity)
        needsDisplay = true
        synchronizeViewport()
        print(
            "Viewport self-check caret-source=\(reveal.source) block=\(reveal.block) "
                + "target=\(reveal.targetScrollY) native=\(nativeTarget) "
                + "stale=rejected noop=\(!visible.needsScroll)"
        )
    }

    func runShapedViewportScrollSelfCheck() {
        let savedStorage = NSAttributedString(attributedString: textStorage)
        let savedSelection = selection
        let savedAffinity = selectionAffinity
        let source = "one\n\ntwo **羽🙂**\n\nthree\n"
        let font = defaultAttributes[.font] as? NSFont ?? NSFont.systemFont(ofSize: 22)
        let nativeMetrics = rustComposition.coreTextSystemUiViewportMetrics(
            size: font.pointSize,
            sample: "M中🙂e\u{301}"
        )

        replaceStorage(
            range: NSRange(location: 0, length: textStorage.length),
            with: attributedString(from: source, marked: false)
        )
        rustComposition.resetSource(source)
        rustComposition.setViewportMetrics(
            RustViewportMetrics(
                maxWidth: 600,
                lineHeight: nativeMetrics.lineHeight,
                defaultAdvance: nativeMetrics.defaultAdvance,
                estimatedBlockHeight: nativeMetrics.lineHeight,
                overscan: 0
            )
        )
        selection = NSRange(location: source.utf16.count, length: 0)
        selectionAffinity = .downstream
        rustComposition.setSelection(selection, affinity: selectionAffinity)

        let request = rustComposition.shapedCaretScrollRequest(
            size: font.pointSize,
            maxWidth: 600,
            scrollY: 0,
            viewportHeight: nativeMetrics.lineHeight,
            margin: 0
        )
        precondition(request.revision == rustComposition.revision())
        precondition(request.source == source.utf16.count && request.block == 4)
        precondition(request.caretX.isFinite && request.caretY.isFinite)
        precondition(request.caretHeight.isFinite && request.caretHeight > 0)
        precondition(request.targetScrollY.isFinite && request.targetScrollY >= 0)
        precondition(request.targetScrollY <= request.caretY + 0.001)
        precondition(request.needsScroll)

        replaceStorage(
            range: NSRange(location: 0, length: textStorage.length),
            with: savedStorage
        )
        selection = savedSelection
        selectionAffinity = savedAffinity
        rustComposition.resetSource(textStorage.string)
        rustComposition.setSelection(selection, affinity: selectionAffinity)
        needsDisplay = true
        synchronizeViewport()
        print(
            "Shaped viewport self-check block=\(request.block) "
                + "caret=(\(String(format: "%.2f", request.caretX)),"
                + "\(String(format: "%.2f", request.caretY))) "
                + "target=\(String(format: "%.2f", request.targetScrollY))"
        )
    }

    func runAttachedViewportSelfCheck() {
        guard let viewportAdapter else {
            preconditionFailure("attached viewport self-check requires an NSScrollView host")
        }
        let savedStorage = NSAttributedString(attributedString: textStorage)
        let savedSelection = selection
        let savedAffinity = selectionAffinity
        let source = (0..<40).map { "line-\($0)" }.joined(separator: "\n") + "\n"

        replaceStorage(
            range: NSRange(location: 0, length: textStorage.length),
            with: attributedString(from: source, marked: false)
        )
        rustComposition.resetSource(source)
        selection = NSRange(location: source.utf16.count, length: 0)
        selectionAffinity = .downstream
        rustComposition.setSelection(selection, affinity: selectionAffinity)
        synchronizeViewport()

        let metrics = viewportAdapter.viewportMetrics()
        let longContentHeight = viewportAdapter.contentHeight
        precondition(viewportAdapter.revision == rustComposition.revision())
        precondition(longContentHeight >= metrics.height)
        if longContentHeight > metrics.height {
            precondition(metrics.scrollY > 0, "attached viewport must reveal the long document caret")
        }

        replaceStorage(
            range: NSRange(location: 0, length: textStorage.length),
            with: savedStorage
        )
        selection = savedSelection
        selectionAffinity = savedAffinity
        rustComposition.resetSource(textStorage.string)
        rustComposition.setSelection(selection, affinity: selectionAffinity)
        synchronizeViewport()
        needsDisplay = true
        print(
            "Attached viewport self-check content=\(longContentHeight) "
                + "viewport=\(metrics.height) scroll=\(metrics.scrollY)"
        )
    }

    func runLayoutRoundTripSelfCheck() {
        updateContainerSize()
        layoutManager.ensureLayout(for: textContainer)
        let glyphRange = layoutManager.glyphRange(for: textContainer)
        var lineFragments = 0
        layoutManager.enumerateLineFragments(forGlyphRange: glyphRange) {
            _, _, _, _, _ in
            lineFragments += 1
        }

        let boundaries = canonicalCaretOffsets()
        var affinitySplits = 0
        var softWrapSplits = 0
        for index in boundaries {
            let downstream = caretRect(at: index, affinity: .downstream)
            let downstreamPoint = probePoint(for: downstream)
            let downstreamHit = caretHit(forLocalPoint: downstreamPoint)
            precondition(
                downstreamHit.index == index,
                "downstream caret round-trip failed at \(index): \(downstreamHit)"
            )
            precondition(
                characterIndex(for: screenPoint(forLocalPoint: downstreamPoint)) == index,
                "screen-space downstream round-trip failed at \(index)"
            )

            let upstream = caretRect(at: index, affinity: .upstream)
            guard !sameVisualLine(upstream, downstream) else { continue }
            affinitySplits += 1
            if index == 0 || (textStorage.string as NSString).character(at: index - 1) != 0x0A {
                softWrapSplits += 1
            }
            let upstreamPoint = probePoint(for: upstream)
            let upstreamHit = caretHit(forLocalPoint: upstreamPoint)
            precondition(
                upstreamHit.index == index && upstreamHit.affinity == .upstream,
                "upstream caret round-trip failed at \(index): \(upstreamHit)"
            )
            precondition(
                characterIndex(for: screenPoint(forLocalPoint: upstreamPoint)) == index,
                "screen-space upstream round-trip failed at \(index)"
            )
        }

        precondition(lineFragments >= 4, "test content must shape into multiple visual lines")
        precondition(affinitySplits > 0, "test content must contain a split caret position")
        precondition(softWrapSplits > 0, "test content must contain a soft-wrap caret split")
        print(
            "Layout self-check lines=\(lineFragments) boundaries=\(boundaries.count) "
                + "affinitySplits=\(affinitySplits) softWrapSplits=\(softWrapSplits)"
        )
    }

    private func textKitLineRanges(for string: String, width: CGFloat) -> [NSRange] {
        let storage = NSTextStorage()
        storage.append(NSAttributedString(string: string, attributes: defaultAttributes))
        let manager = NSLayoutManager()
        let container = NSTextContainer(
            size: NSSize(width: width, height: .greatestFiniteMagnitude)
        )
        container.lineFragmentPadding = 0
        manager.addTextContainer(container)
        storage.addLayoutManager(manager)
        manager.ensureLayout(for: container)
        let glyphRange = manager.glyphRange(for: container)
        var ranges = [NSRange]()
        manager.enumerateLineFragments(forGlyphRange: glyphRange) {
            _, _, _, lineGlyphRange, _ in
            ranges.append(
                manager.characterRange(
                    forGlyphRange: lineGlyphRange,
                    actualGlyphRange: nil
                )
            )
        }
        return ranges
    }

    func runShapedLayoutComparisonSelfCheck() {
        updateContainerSize()
        layoutManager.ensureLayout(for: textContainer)
        let glyphRange = layoutManager.glyphRange(for: textContainer)
        var nativeRanges = [NSRange]()
        layoutManager.enumerateLineFragments(forGlyphRange: glyphRange) {
            _, _, _, lineGlyphRange, _ in
            nativeRanges.append(
                self.layoutManager.characterRange(
                    forGlyphRange: lineGlyphRange,
                    actualGlyphRange: nil
                )
            )
        }

        let font = defaultAttributes[.font] as? NSFont ?? NSFont.systemFont(ofSize: 22)
        let rustLines = rustComposition.coreTextSystemUiShapedLines(
            size: font.pointSize,
            maxWidth: textContainer.containerSize.width,
            source: textStorage.string
        )
        let rustRanges = rustLines
            .map { "\($0.source_start_utf16)..\($0.source_end_utf16)" }
            .joined(separator: ",")
        let nativeRangeDescription = nativeRanges
            .map { "\($0.location)..\(NSMaxRange($0))" }
            .joined(separator: ",")
        let rustSourceLines = rustLines.filter {
            $0.source_start_utf16 != $0.source_end_utf16
        }
        precondition(
            rustLines.allSatisfy {
                $0.source_start_utf16 <= $0.source_end_utf16
                    && $0.width.isFinite
                    && $0.width >= 0
            },
            "Rust shaped lines must have ordered ranges and finite nonnegative widths"
        )
        precondition(
            rustLines.filter { $0.source_start_utf16 == $0.source_end_utf16 }
                .allSatisfy { $0.width == 0 },
            "Rust trailing caret lines must be zero width"
        )
        let rustSourceRangeDescription = rustSourceLines
            .map { "\($0.source_start_utf16)..\($0.source_end_utf16)" }
            .joined(separator: ",")
        precondition(
            rustSourceLines.count == nativeRanges.count,
            "Rust/TextKit shaped source line count mismatch: rust=\(rustSourceLines.count) "
                + "native=\(nativeRanges.count) rustRanges=\(rustSourceRangeDescription) "
                + "nativeRanges=\(nativeRangeDescription)"
        )
        for (index, line) in rustSourceLines.enumerated() {
            let rustRange = NSRange(
                location: Int(line.source_start_utf16),
                length: Int(line.source_end_utf16 - line.source_start_utf16)
            )
            precondition(
                rustRange == nativeRanges[index],
                "Rust/TextKit shaped line range mismatch at \(index): rust=\(rustRange) native=\(nativeRanges[index])"
            )
            precondition(line.width.isFinite && line.width >= 0, "Rust shaped line width is invalid")
        }
        print(
            "Shaped layout self-check sourceLines=\(rustSourceLines.count) "
                + "trailingCaretLines=\(rustLines.count - rustSourceLines.count) "
                + "ranges=\(rustRanges)"
        )
    }

    func runProjectionShapedLayoutSelfCheck() {
        let source = "This is **Yu** and [Rust](https://example.com) with 中文🙂.\nSecond **line**.\n"
        let width: CGFloat = 600
        let font = defaultAttributes[.font] as? NSFont ?? NSFont.systemFont(ofSize: 22)
        let result = rustComposition.coreTextSystemUiProjectedLayout(
            size: font.pointSize,
            maxWidth: width,
            source: source
        )
        let projected = result.projected
        precondition(
            projected == "This is Yu and Rust with 中文🙂.\nSecond line.\n",
            "Markdown projection must hide syntax without rewriting visible source"
        )
        precondition(!projected.contains("**") && !projected.contains("https://"))

        let sourceLength = source.utf16.count
        let visualLength = projected.utf16.count
        precondition(
            result.lines.allSatisfy {
                $0.source_start_utf16 <= $0.source_end_utf16
                    && Int($0.source_end_utf16) <= sourceLength
                    && $0.visual_start_utf16 <= $0.visual_end_utf16
                    && Int($0.visual_end_utf16) <= visualLength
                    && $0.width.isFinite
                    && $0.width >= 0
            },
            "Projected line ranges must stay inside source and visual UTF-16 bounds"
        )
        precondition(
            result.lines.filter {
                $0.visual_start_utf16 == $0.visual_end_utf16
            }.allSatisfy { $0.width == 0 },
            "Projected trailing caret lines must remain zero width"
        )
        precondition(zip(result.lines, result.lines.dropFirst()).allSatisfy { previous, next in
            previous.source_end_utf16 <= next.source_start_utf16
                && previous.visual_end_utf16 <= next.visual_start_utf16
        })
        precondition(
            result.lines.contains {
                $0.source_end_utf16 - $0.source_start_utf16
                    > $0.visual_end_utf16 - $0.visual_start_utf16
            },
            "At least one line must demonstrate hidden Markdown syntax"
        )

        let sourceLines = result.lines.filter {
            $0.visual_start_utf16 != $0.visual_end_utf16
        }
        let nativeRanges = textKitLineRanges(for: projected, width: width)
        precondition(
            sourceLines.count == nativeRanges.count,
            "Projected Rust/TextKit line count mismatch: rust=\(sourceLines.count) native=\(nativeRanges.count)"
        )
        let rustRangeDescription = sourceLines
            .map {
                "\($0.visual_start_utf16)..\($0.visual_end_utf16)(w=\($0.width))"
            }
            .joined(separator: ",")
        let nativeRangeDescription = nativeRanges
            .map { "\($0.location)..\(NSMaxRange($0))" }
            .joined(separator: ",")
        for (index, line) in sourceLines.enumerated() {
            let rustRange = NSRange(
                location: Int(line.visual_start_utf16),
                length: Int(line.visual_end_utf16 - line.visual_start_utf16)
            )
            precondition(
                rustRange == nativeRanges[index],
                "Projected Rust/TextKit range mismatch at \(index): rust=\(rustRange) native=\(nativeRanges[index]) "
                    + "rustRanges=\(rustRangeDescription) nativeRanges=\(nativeRangeDescription) "
                    + "projected=\(projected.debugDescription)"
            )
        }
        let ranges = sourceLines
            .map { "\($0.source_start_utf16)..\($0.source_end_utf16)/\($0.visual_start_utf16)..\($0.visual_end_utf16)" }
            .joined(separator: ",")
        print(
            "Projection shaped self-check sourceLines=\(sourceLines.count) "
                + "trailingCaretLines=\(result.lines.count - sourceLines.count) ranges=\(ranges)"
        )
    }

    func runProjectionCaretSelfCheck() {
        let savedStorage = NSAttributedString(attributedString: textStorage)
        let savedSelection = selection
        let savedAffinity = selectionAffinity
        let source = "before **羽🙂** after\n\nsecond **block**\n"
        let probe = 7

        replaceStorage(
            range: NSRange(location: 0, length: textStorage.length),
            with: attributedString(from: source, marked: false)
        )
        marked = notFoundRange
        compositionOriginal = nil
        compositionSelectionBefore = nil
        compositionAffinityBefore = nil
        rustComposition.resetSource(source)
        selection = NSRange(location: probe, length: 0)
        selectionAffinity = .downstream
        rustComposition.setSelection(selection, affinity: selectionAffinity)

        let upstream = rustComposition.projectionCaret(
            sourceUTF16: probe,
            affinity: .upstream
        )
        let downstream = rustComposition.projectionCaret(
            sourceUTF16: probe,
            affinity: .downstream
        )
        precondition(upstream.revision == rustComposition.revision())
        precondition(downstream.revision == upstream.revision)
        precondition(upstream.source == probe && downstream.source == probe)
        precondition(upstream.visual == 7 && downstream.visual == 7)
        precondition(upstream.roundTripSource == 7, "upstream must stay before hidden syntax")
        precondition(downstream.roundTripSource == 9, "downstream must cross hidden syntax")
        precondition(upstream.affinity == .upstream && downstream.affinity == .downstream)

        let secondStart = (source as NSString).range(of: "second").location
        precondition(secondStart != NSNotFound, "block projection probe text must exist")
        let blockProbe = secondStart + 7
        let blockUpstream = rustComposition.blockProjectionCaret(
            sourceUTF16: blockProbe,
            affinity: .upstream
        )
        let blockDownstream = rustComposition.blockProjectionCaret(
            sourceUTF16: blockProbe,
            affinity: .downstream
        )
        precondition(blockUpstream.revision == upstream.revision)
        precondition(blockUpstream.block > 0 && blockDownstream.block == blockUpstream.block)
        precondition(blockUpstream.source == blockProbe && blockDownstream.source == blockProbe)
        precondition(blockUpstream.visual == 7 && blockDownstream.visual == 7)
        precondition(
            blockUpstream.roundTripSource == blockProbe,
            "block upstream must stay before hidden syntax"
        )
        precondition(
            blockDownstream.roundTripSource == blockProbe + 2,
            "block downstream must cross hidden syntax"
        )

        let shapedUpstream = rustComposition.blockShapedCaret(
            sourceUTF16: blockProbe,
            affinity: .upstream,
            size: 22,
            maxWidth: 600
        )
        let shapedDownstream = rustComposition.blockShapedCaret(
            sourceUTF16: blockProbe,
            affinity: .downstream,
            size: 22,
            maxWidth: 600
        )
        precondition(shapedUpstream.revision == upstream.revision)
        precondition(shapedDownstream.revision == shapedUpstream.revision)
        precondition(shapedUpstream.source == blockProbe && shapedDownstream.source == blockProbe)
        precondition(shapedUpstream.block == blockUpstream.block)
        precondition(shapedDownstream.block == shapedUpstream.block)
        precondition(shapedUpstream.visual == 7 && shapedDownstream.visual == 7)
        precondition(shapedUpstream.line == 0 && shapedDownstream.line == 0)
        precondition(shapedUpstream.roundTripSource == blockProbe)
        precondition(shapedDownstream.roundTripSource == blockProbe + 2)
        precondition(shapedUpstream.x.isFinite && shapedUpstream.y.isFinite)
        precondition(shapedUpstream.height.isFinite && shapedUpstream.height > 0)
        precondition(shapedUpstream.width == 0 && shapedDownstream.width == 0)
        precondition(abs(shapedUpstream.x - shapedDownstream.x) < 0.001)

        replaceStorage(
            range: NSRange(location: 0, length: textStorage.length),
            with: savedStorage
        )
        selection = savedSelection
        selectionAffinity = savedAffinity
        marked = notFoundRange
        compositionOriginal = nil
        compositionSelectionBefore = nil
        compositionAffinityBefore = nil
        rustComposition.resetSource(textStorage.string)
        rustComposition.setSelection(selection, affinity: selectionAffinity)
        synchronizeViewport()
        needsDisplay = true
        print(
            "Projection caret self-check source=\(probe) visual=\(upstream.visual) "
                + "upstream=\(upstream.roundTripSource) downstream=\(downstream.roundTripSource) "
                + "block=\(blockUpstream.block) blockVisual=\(blockUpstream.visual) "
                + "shaped=(x:\(String(format: "%.2f", shapedUpstream.x)), "
                + "y:\(String(format: "%.2f", shapedUpstream.y)), "
                + "lineHeight:\(String(format: "%.2f", shapedUpstream.height)))"
        )
    }

    func runUnicodeCompositionSelfCheck() {
        let savedStorage = NSAttributedString(attributedString: textStorage)
        let savedSelection = selection
        let savedAffinity = selectionAffinity
        let savedMarked = marked
        let base = textStorage.string

        selection = NSRange(location: textStorage.length, length: 0)
        marked = notFoundRange
        setMarkedText(
            "にほんご",
            selectedRange: NSRange(location: 4, length: 0),
            replacementRange: notFoundRange
        )
        precondition(hasMarkedText() && marked.length == 4, "Japanese preedit should be marked")
        precondition(rustComposition.overlayString() == "にほんご")
        precondition(rustComposition.overlaySelection() == NSRange(location: 4, length: 0))
        precondition(rustComposition.sourceString(utf16Length: base.utf16.count) == base)
        setMarkedText(
            "にほんご",
            selectedRange: NSRange(location: 4, length: 0),
            replacementRange: notFoundRange
        )
        precondition(rustComposition.overlayString() == "にほんご")
        insertText("日本語", replacementRange: notFoundRange)
        precondition(!hasMarkedText() && textStorage.string == base + "日本語")
        precondition(
            !rustComposition.hasOverlay
                && rustComposition.sourceString(utf16Length: (base + "日本語").utf16.count)
                    == base + "日本語"
        )

        setMarkedText(
            "\u{301}",
            selectedRange: NSRange(location: 1, length: 0),
            replacementRange: notFoundRange
        )
        setMarkedText(
            "e\u{301}",
            selectedRange: NSRange(location: 2, length: 0),
            replacementRange: notFoundRange
        )
        precondition(rustComposition.overlayString() == "e\u{301}")
        insertText("é", replacementRange: notFoundRange)
        precondition(!hasMarkedText() && textStorage.string == base + "日本語é")
        precondition(
            !rustComposition.hasOverlay
                && rustComposition.sourceString(utf16Length: (base + "日本語é").utf16.count)
                    == base + "日本語é"
        )

        let cancelBase = textStorage.string
        setMarkedText(
            "にほん",
            selectedRange: NSRange(location: 3, length: 0),
            replacementRange: notFoundRange
        )
        precondition(hasMarkedText())
        doCommand(by: #selector(NSResponder.cancelOperation(_:)))
        precondition(!hasMarkedText() && textStorage.string == cancelBase)
        precondition(
            !rustComposition.hasOverlay
                && rustComposition.sourceString(utf16Length: cancelBase.utf16.count) == cancelBase
        )

        replaceStorage(
            range: NSRange(location: 0, length: textStorage.length),
            with: savedStorage
        )
        selection = savedSelection
        selectionAffinity = savedAffinity
        marked = savedMarked
        compositionOriginal = nil
        compositionSelectionBefore = nil
        compositionAffinityBefore = nil
        rustComposition.resetSource(textStorage.string)
        rustComposition.setSelection(selection, affinity: selectionAffinity)
        needsDisplay = true
        print(
            "Unicode composition self-check japanese=日本語 combining=é "
                + "cancel=restored vertical=preferred-x shift-selection"
        )
    }

    override func doCommand(by selector: Selector) {
        let command = NSStringFromSelector(selector)
        if command == "cancel:" || command == "cancelOperation:" {
            cancelComposition()
            return
        }
        let nativeCommand: UInt8
        switch selector {
        case #selector(NSResponder.deleteBackward(_:)):
            nativeCommand = YuNativeCommand.deleteBackward
        case #selector(NSResponder.deleteForward(_:)):
            nativeCommand = YuNativeCommand.deleteForward
        case #selector(NSResponder.moveLeft(_:)):
            nativeCommand = YuNativeCommand.moveLeft
        case #selector(NSResponder.moveRight(_:)):
            nativeCommand = YuNativeCommand.moveRight
        case #selector(NSResponder.moveWordLeft(_:)):
            nativeCommand = YuNativeCommand.moveWordLeft
        case #selector(NSResponder.moveWordRight(_:)):
            nativeCommand = YuNativeCommand.moveWordRight
        case #selector(NSResponder.moveUp(_:)):
            nativeCommand = YuNativeCommand.moveUp
        case #selector(NSResponder.moveDown(_:)):
            nativeCommand = YuNativeCommand.moveDown
        case #selector(NSResponder.moveUpAndModifySelection(_:)):
            nativeCommand = YuNativeCommand.moveUpExtend
        case #selector(NSResponder.moveDownAndModifySelection(_:)):
            nativeCommand = YuNativeCommand.moveDownExtend
        case #selector(NSResponder.insertNewline(_:)):
            nativeCommand = YuNativeCommand.insertNewline
        default:
            super.doCommand(by: selector)
            return
        }
        guard !hasMarkedText() && !rustComposition.hasOverlay else { return }
        guard rustComposition.commandAvailable(command: nativeCommand) else { return }
        applyRustCommandResult(rustComposition.executeCommand(command: nativeCommand))
    }

    private func routeNativeKey(_ event: NSEvent) -> Bool {
        // NSTextInputClient owns marked text. Let the input context consume
        // every key while a preedit is visible; otherwise Cmd-Z could mutate
        // canonical source while the native overlay is still active.
        guard !hasMarkedText() else { return false }

        let key: (kind: UInt8, value: UInt32)
        switch event.keyCode {
        case 36, 76:
            key = (YuNativeKeyKind.enter, 0)
        case 48:
            key = (YuNativeKeyKind.tab, 0)
        case 51:
            key = (YuNativeKeyKind.backspace, 0)
        case 117:
            key = (YuNativeKeyKind.delete, 0)
        case 123:
            key = (YuNativeKeyKind.left, 0)
        case 124:
            key = (YuNativeKeyKind.right, 0)
        case 125:
            key = (YuNativeKeyKind.down, 0)
        case 126:
            key = (YuNativeKeyKind.up, 0)
        case 53:
            key = (YuNativeKeyKind.escape, 0)
        default:
            guard
                let characters = event.charactersIgnoringModifiers,
                characters.unicodeScalars.count == 1,
                let scalar = characters.unicodeScalars.first
            else {
                return false
            }
            key = (YuNativeKeyKind.character, scalar.value)
        }

        var modifiers: UInt8 = 0
        if event.modifierFlags.contains(.command) {
            modifiers |= YuNativeModifier.command
        }
        if event.modifierFlags.contains(.shift) {
            modifiers |= YuNativeModifier.shift
        }
        if event.modifierFlags.contains(.control) {
            modifiers |= YuNativeModifier.control
        }
        if event.modifierFlags.contains(.option) {
            modifiers |= YuNativeModifier.option
        }

        guard let result = rustComposition.routeKey(
            kind: key.kind,
            value: key.value,
            modifiers: modifiers
        ) else {
            return false
        }
        applyRustCommandResult(result)
        return true
    }

    private func applyRustCommandResult(_ result: RustCommandResult) {
        precondition(!hasMarkedText() && !rustComposition.hasOverlay)
        precondition(
            rustComposition.revision() == result.revision,
            "Rust command result must belong to the current revision"
        )
        switch result.sourceSync {
        case .none:
            precondition(
                !result.changed,
                "a changed command must provide a source synchronization scope"
            )
        case .range:
            guard let oldRange = result.oldSourceRange, let newRange = result.newSourceRange else {
                preconditionFailure("range source synchronization requires both ranges")
            }
            precondition(
                oldRange.location >= 0 && NSMaxRange(oldRange) <= textStorage.length,
                "Rust command old source range is outside the native mirror"
            )
            let source = rustComposition.sourceString(utf16Range: newRange)
            replaceStorage(
                range: oldRange,
                with: attributedString(from: source, marked: false)
            )
        case .full:
            let source = rustComposition.sourceString()
            replaceStorage(
                range: NSRange(location: 0, length: textStorage.length),
                with: attributedString(from: source, marked: false)
            )
        }
        guard let selection = validatedAccessibilityRange(result.range) else {
            preconditionFailure("Rust command returned an invalid native selection")
        }
        self.selection = selection
        selectionAffinity = result.affinity
        marked = notFoundRange
        compositionOriginal = nil
        compositionSelectionBefore = nil
        compositionAffinityBefore = nil
        needsDisplay = true
        if result.changed {
            postTextChanged()
        } else {
            postSelectionChanged()
        }
        synchronizeViewport()
    }

    private func attributedString(from value: Any, marked: Bool) -> NSAttributedString {
        let result: NSMutableAttributedString
        if let attributed = value as? NSAttributedString {
            result = NSMutableAttributedString(attributedString: attributed)
            result.addAttributes(defaultAttributes, range: NSRange(location: 0, length: result.length))
        } else {
            result = NSMutableAttributedString(
                string: String(describing: value),
                attributes: defaultAttributes
            )
        }
        if marked, result.length > 0 {
            result.addAttribute(
                .underlineStyle,
                value: NSUnderlineStyle.single.rawValue,
                range: NSRange(location: 0, length: result.length)
            )
        }
        return result
    }

    private func targetRange(_ replacementRange: NSRange) -> NSRange {
        if replacementRange.location != NSNotFound {
            return clamped(replacementRange)
        }
        if hasMarkedText() {
            return clamped(marked)
        }
        return clamped(selection)
    }

    private func clamped(_ range: NSRange) -> NSRange {
        guard range.location != NSNotFound else { return notFoundRange }
        let location = min(range.location, textStorage.length)
        let available = textStorage.length - location
        return NSRange(location: location, length: min(range.length, available))
    }

    private func validatedAccessibilityRange(_ range: NSRange) -> NSRange? {
        guard range.location != NSNotFound, range.location <= textStorage.length else {
            return nil
        }
        let (end, overflow) = range.location.addingReportingOverflow(range.length)
        guard !overflow, end <= textStorage.length else { return nil }
        return range
    }

    private struct CaretHit: CustomStringConvertible {
        let index: Int
        let affinity: NSSelectionAffinity

        var description: String {
            "CaretHit(index: \(index), affinity: \(affinity.rawValue))"
        }
    }

    private func caretHit(forLocalPoint point: NSPoint) -> CaretHit {
        let containerPoint = NSPoint(x: point.x - textOrigin.x, y: point.y - textOrigin.y)
        guard containerPoint.x >= 0, containerPoint.y >= 0 else {
            return CaretHit(index: 0, affinity: .downstream)
        }

        updateContainerSize()
        layoutManager.ensureLayout(for: textContainer)
        var fraction: CGFloat = 0
        let character = min(
            layoutManager.characterIndex(
                for: containerPoint,
                in: textContainer,
                fractionOfDistanceBetweenInsertionPoints: &fraction
            ),
            textStorage.length
        )
        let index: Int
        if character < textStorage.length {
            let cluster = (textStorage.string as NSString)
                .rangeOfComposedCharacterSequence(at: character)
            index = fraction >= 0.5 ? NSMaxRange(cluster) : cluster.location
        } else {
            index = textStorage.length
        }

        let upstream = caretRect(at: index, affinity: .upstream)
        let downstream = caretRect(at: index, affinity: .downstream)
        let affinity: NSSelectionAffinity
        if sameVisualLine(upstream, downstream) {
            affinity = .downstream
        } else {
            affinity = squaredDistance(point, upstream) < squaredDistance(point, downstream)
                ? .upstream : .downstream
        }
        return CaretHit(index: index, affinity: affinity)
    }

    private func canonicalCaretOffsets() -> [Int] {
        let string = textStorage.string as NSString
        var boundaries = [0]
        var index = 0
        while index < string.length {
            index = NSMaxRange(string.rangeOfComposedCharacterSequence(at: index))
            // TextKit canonicalizes a click at the end of a hard line to the
            // position after LF with upstream affinity. The position directly
            // before LF is therefore not an independent visual caret stop.
            if index == string.length || string.character(at: index) != 0x0A {
                boundaries.append(index)
            }
        }
        return boundaries
    }

    private func probePoint(for caret: NSRect) -> NSPoint {
        NSPoint(x: caret.minX + 0.25, y: caret.midY)
    }

    private func screenPoint(forLocalPoint point: NSPoint) -> NSPoint {
        let windowPoint = convert(point, to: nil)
        return window?.convertPoint(toScreen: windowPoint) ?? windowPoint
    }

    private func sameVisualLine(_ lhs: NSRect, _ rhs: NSRect) -> Bool {
        abs(lhs.midY - rhs.midY) < 0.5
    }

    private func squaredDistance(_ point: NSPoint, _ caret: NSRect) -> CGFloat {
        let dx = point.x - caret.minX
        let dy = point.y - caret.midY
        return dx * dx + dy * dy
    }

    private func logicalLine(containing index: Int) -> Int {
        let clampedIndex = min(max(index, 0), textStorage.length)
        let prefix = (textStorage.string as NSString).substring(to: clampedIndex)
        return prefix.utf8.reduce(into: 0) { count, byte in
            if byte == 0x0A { count += 1 }
        }
    }

    private func logicalLineRange(_ requestedLine: Int) -> NSRange {
        guard requestedLine >= 0 else { return notFoundRange }
        let string = textStorage.string as NSString
        var line = 0
        var start = 0

        while line < requestedLine {
            guard start < string.length else { return notFoundRange }
            let range = string.lineRange(for: NSRange(location: start, length: 0))
            let next = NSMaxRange(range)
            guard next > start else { return notFoundRange }
            start = next
            line += 1
        }

        if start == string.length {
            let hasTrailingLine = string.length == 0 || string.character(at: string.length - 1) == 0x0A
            return hasTrailingLine ? NSRange(location: start, length: 0) : notFoundRange
        }
        return string.lineRange(for: NSRange(location: start, length: 0))
    }

    private func postTextChanged() {
        NSAccessibility.post(element: self, notification: .valueChanged)
        postSelectionChanged()
    }

    private func postSelectionChanged() {
        NSAccessibility.post(element: self, notification: .selectedTextChanged)
    }

    private func replaceStorage(range: NSRange, with replacement: NSAttributedString) {
        textStorage.beginEditing()
        textStorage.replaceCharacters(in: range, with: replacement)
        textStorage.endEditing()
    }

    private func cancelComposition() {
        print("cancelComposition range=\(marked)")
        guard hasMarkedText(), let original = compositionOriginal else {
            marked = notFoundRange
            rustComposition.cancel()
            return
        }
        replaceStorage(range: marked, with: original)
        selection = compositionSelectionBefore ?? NSRange(location: marked.location, length: 0)
        selectionAffinity = compositionAffinityBefore ?? .downstream
        marked = notFoundRange
        compositionOriginal = nil
        compositionSelectionBefore = nil
        compositionAffinityBefore = nil
        rustComposition.cancel()
        needsDisplay = true
        postTextChanged()
        synchronizeViewport()
    }

    private func updateContainerSize() {
        textContainer.containerSize = NSSize(
            width: min(max(bounds.width - textOrigin.x * 2, 1), maximumTextWidth),
            height: .greatestFiniteMagnitude
        )
    }

    /// Publishes the native TextKit metrics used by the metrics-only Rust
    /// viewport backend, keeping both sides in the same point-based units.
    private func synchronizeViewport() {
        guard let viewportAdapter, !synchronizingViewport, !hasMarkedText() else { return }
        synchronizingViewport = true
        defer { synchronizingViewport = false }

        updateContainerSize()
        layoutManager.ensureLayout(for: textContainer)
        rustComposition.setViewportMetrics(nativeViewportMetrics())
        let usedRect = layoutManager.usedRect(for: textContainer)
        let extraLineRect = layoutManager.extraLineFragmentUsedRect
        let usedBottom = max(usedRect.maxY, extraLineRect.maxY)
        let viewportHeight = viewportAdapter.viewportMetrics().height
        let contentHeight = max(viewportHeight, textOrigin.y * 2 + usedBottom)
        let revision = rustComposition.revision()
        viewportAdapter.configure(revision: revision, contentHeight: contentHeight)

        let metrics = viewportAdapter.viewportMetrics()
        let font = defaultAttributes[.font] as? NSFont ?? NSFont.systemFont(ofSize: 22)
        let nativeMetrics = nativeViewportMetrics()
        let request = rustComposition.shapedCaretScrollRequest(
            size: font.pointSize,
            maxWidth: nativeMetrics.maxWidth,
            scrollY: metrics.scrollY,
            viewportHeight: metrics.height,
            margin: 8
        )
        _ = viewportAdapter.apply(request, currentRevision: revision)
    }

    private func nativeViewportMetrics() -> RustViewportMetrics {
        let font = defaultAttributes[.font] as? NSFont ?? NSFont.systemFont(ofSize: 22)
        let sample = "M中🙂e\u{301}"
        let coreTextMetrics = rustComposition.coreTextSystemUiViewportMetrics(
            size: font.pointSize,
            sample: sample
        )
        return RustViewportMetrics(
            maxWidth: quantized(max(textContainer.containerSize.width, 1)),
            lineHeight: quantized(coreTextMetrics.lineHeight),
            defaultAdvance: quantized(coreTextMetrics.defaultAdvance),
            estimatedBlockHeight: quantized(coreTextMetrics.lineHeight),
            overscan: quantized(coreTextMetrics.lineHeight * 2)
        )
    }

    private func quantized(_ value: CGFloat) -> CGFloat {
        (value * 100).rounded() / 100
    }

    private func nativeLineHeight() -> CGFloat {
        let font = defaultAttributes[.font] as? NSFont ?? NSFont.systemFont(ofSize: 22)
        return max(layoutManager.defaultLineHeight(for: font), 1)
    }

    private func caretRect(
        at characterIndex: Int,
        affinity: NSSelectionAffinity
    ) -> NSRect {
        let downstream = downstreamCaretRect(at: characterIndex)
        guard affinity == .upstream, characterIndex > 0 else { return downstream }

        let previousCharacter = (textStorage.string as NSString)
            .rangeOfComposedCharacterSequence(at: min(characterIndex, textStorage.length) - 1)
        let previousGlyphs = layoutManager.glyphRange(
            forCharacterRange: previousCharacter,
            actualCharacterRange: nil
        )
        guard previousGlyphs.length > 0 else { return downstream }

        let lastGlyph = NSMaxRange(previousGlyphs) - 1
        let previousLine = layoutManager.lineFragmentUsedRect(
            forGlyphAt: lastGlyph,
            effectiveRange: nil
        )
        guard abs(textOrigin.y + previousLine.midY - downstream.midY) >= 0.5 else {
            return downstream
        }
        let previousBounds = layoutManager.boundingRect(
            forGlyphRange: previousGlyphs,
            in: textContainer
        )
        return NSRect(
            x: textOrigin.x + previousBounds.maxX,
            y: textOrigin.y + previousLine.minY,
            width: 1.5,
            height: max(previousLine.height, 26)
        )
    }

    private func downstreamCaretRect(at characterIndex: Int) -> NSRect {
        updateContainerSize()
        layoutManager.ensureLayout(for: textContainer)

        guard textStorage.length > 0 else {
            return NSRect(x: textOrigin.x, y: textOrigin.y, width: 1.5, height: 26)
        }

        let clampedIndex = min(max(characterIndex, 0), textStorage.length)
        if clampedIndex == textStorage.length,
            (textStorage.string as NSString).hasSuffix("\n"),
            !layoutManager.extraLineFragmentUsedRect.isEmpty
        {
            let extra = layoutManager.extraLineFragmentUsedRect
            return NSRect(
                x: textOrigin.x + extra.minX,
                y: textOrigin.y + extra.minY,
                width: 1.5,
                height: max(extra.height, 26)
            )
        }

        let glyphIndex: Int
        if clampedIndex == textStorage.length {
            glyphIndex = max(layoutManager.numberOfGlyphs - 1, 0)
        } else {
            glyphIndex = layoutManager.glyphIndexForCharacter(at: clampedIndex)
        }
        let line = layoutManager.lineFragmentUsedRect(forGlyphAt: glyphIndex, effectiveRange: nil)
        let glyphLocation = layoutManager.location(forGlyphAt: glyphIndex)
        var x = glyphLocation.x
        if clampedIndex == textStorage.length {
            let finalRange = NSRange(location: glyphIndex, length: 1)
            x = NSMaxX(layoutManager.boundingRect(forGlyphRange: finalRange, in: textContainer))
        }
        return NSRect(
            x: textOrigin.x + x,
            y: textOrigin.y + line.minY,
            width: 1.5,
            height: max(line.height, 26)
        )
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var window: NSWindow?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let frame = NSRect(x: 0, y: 0, width: 760, height: 480)
        let window = NSWindow(
            contentRect: frame,
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Yu — macOS Text Input Spike"
        window.center()

        let scrollView = NSScrollView(frame: frame)
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = false
        scrollView.autohidesScrollers = true
        scrollView.borderType = .noBorder
        scrollView.drawsBackground = true

        let inputView = TextInputView(
            frame: NSRect(origin: .zero, size: scrollView.contentSize)
        )
        inputView.autoresizingMask = [.width]
        scrollView.documentView = inputView
        window.contentView = scrollView
        window.makeKeyAndOrderFront(nil)
        window.makeFirstResponder(inputView)
        inputView.attachViewportAdapter(YuNativeViewportAdapter(scrollView: scrollView))
        inputView.runLayoutRoundTripSelfCheck()
        inputView.runShapedLayoutComparisonSelfCheck()
        inputView.runProjectionShapedLayoutSelfCheck()
        inputView.runProjectionCaretSelfCheck()
        inputView.runUnicodeCompositionSelfCheck()
        inputView.runNativeCommandRoutingSelfCheck()
        inputView.runViewportScrollSelfCheck()
        inputView.runShapedViewportScrollSelfCheck()
        inputView.runAttachedViewportSelfCheck()
        inputView.runNativeSelectionSelfCheck()
        inputView.runAccessibilitySelfCheck()
        NSApp.activate(ignoringOtherApps: true)
        DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + 0.25) {
            runAccessibilityRuntimeProbe()
        }
        self.window = window
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}

private func runAccessibilityRuntimeProbe() {
    let application = AXUIElementCreateApplication(getpid())
    var focusedValue: CFTypeRef?
    let focusedError = AXUIElementCopyAttributeValue(
        application,
        kAXFocusedUIElementAttribute as CFString,
        &focusedValue
    )
    guard focusedError == .success, let focused = focusedValue else {
        print("AX runtime probe focused-element error=\(focusedError.rawValue)")
        return
    }
    let element = focused as! AXUIElement

    var roleValue: CFTypeRef?
    let roleError = AXUIElementCopyAttributeValue(
        element,
        kAXRoleAttribute as CFString,
        &roleValue
    )
    var countValue: CFTypeRef?
    let countError = AXUIElementCopyAttributeValue(
        element,
        kAXNumberOfCharactersAttribute as CFString,
        &countValue
    )
    let count = (countValue as? NSNumber)?.intValue ?? 0
    var firstLine = CFRange(location: 0, length: min(count, 19))
    guard let rangeValue = AXValueCreate(.cfRange, &firstLine) else {
        print("AX runtime probe could not create range value")
        return
    }

    var stringValue: CFTypeRef?
    let stringError = AXUIElementCopyParameterizedAttributeValue(
        element,
        kAXStringForRangeParameterizedAttribute as CFString,
        rangeValue,
        &stringValue
    )
    var boundsValue: CFTypeRef?
    let boundsError = AXUIElementCopyParameterizedAttributeValue(
        element,
        kAXBoundsForRangeParameterizedAttribute as CFString,
        rangeValue,
        &boundsValue
    )

    print(
        "AX runtime probe trusted=\(AXIsProcessTrusted()) "
            + "role=\(String(describing: roleValue)) roleError=\(roleError.rawValue) "
            + "characters=\(count) countError=\(countError.rawValue) "
            + "string=\(String(describing: stringValue)) stringError=\(stringError.rawValue) "
            + "bounds=\(String(describing: boundsValue)) boundsError=\(boundsError.rawValue)"
    )
}

let application = NSApplication.shared
let delegate = AppDelegate()
application.setActivationPolicy(.regular)
application.delegate = delegate
application.run()
