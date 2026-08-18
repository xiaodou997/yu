import AppKit
import Darwin
import UniformTypeIdentifiers
import YuStorageFFI

private extension NSPasteboard.PasteboardType {
    /// The de-facto Markdown pasteboard UTI used by macOS Markdown editors.
    /// The payload is always the canonical source selected in Rust, never the
    /// TextKit projection (which may contain a transient IME overlay).
    static let yuMarkdown = NSPasteboard.PasteboardType("net.daringfireball.markdown")
    /// Semantic HTML generated from the same Rust-owned source selection.
    static let yuHTML = NSPasteboard.PasteboardType(UTType.html.identifier)
}

private enum StorageStatus {
    static let ok: Int32 = 0
    static let staleRevision: Int32 = 13
    static let externalChange: Int32 = 4
    static let unsavedChanges: Int32 = 5
    static let htmlImportRejected: Int32 = 18
    static let invalidSelection: Int32 = 14
    static let invalidViewport: Int32 = 20
    static let tableResizeNotActive: Int32 = 22
}

private enum DiskState: UInt8 {
    case unchanged = 0
    case changed = 1
    case missing = 2

    var label: String {
        switch self {
        case .unchanged: return "磁盘一致"
        case .changed: return "外部已修改"
        case .missing: return "文件不存在"
        }
    }
}

private struct NativeStorageState {
    let revision: UInt64
    let savedRevision: UInt64
    let dirty: Bool
    let disk: DiskState
    let bom: Bool
    let closeState: UInt8

    init(_ value: YuStorageState) {
        revision = value.revision
        savedRevision = value.saved_revision
        dirty = value.dirty != 0
        disk = DiskState(rawValue: value.disk_state) ?? .changed
        bom = value.bom != 0
        closeState = value.close_state
    }
}

private struct NativeSelection {
    let revision: UInt64
    let range: NSRange
    let affinity: UInt8

    init(_ value: YuStorageSelection) {
        revision = value.revision
        range = NSRange(
            location: Int(value.start_utf16),
            length: Int(value.end_utf16 - value.start_utf16)
        )
        affinity = value.affinity
    }
}

private struct NativeSelectionEndpoints {
    let revision: UInt64
    let anchorUTF16: UInt64
    let focusUTF16: UInt64
    let affinity: UInt8

    init(_ value: YuStorageSelectionEndpoints) {
        revision = value.revision
        anchorUTF16 = value.anchor_utf16
        focusUTF16 = value.focus_utf16
        affinity = value.affinity
    }
}

private struct NativeProjectionCaret {
    let revision: UInt64
    let sourceUTF16: UInt64
    let visualUTF16: UInt64
    let roundTripSourceUTF16: UInt64
    let affinity: UInt8

    init(_ value: YuStorageProjectionCaret) {
        revision = value.revision
        sourceUTF16 = value.source_utf16
        visualUTF16 = value.visual_utf16
        roundTripSourceUTF16 = value.round_trip_source_utf16
        affinity = value.affinity
    }
}

private struct NativeProjectionSelection {
    let revision: UInt64
    let sourceRange: NSRange
    let visualRange: NSRange
    let roundTripSourceRange: NSRange
    let affinity: UInt8

    init(_ value: YuStorageProjectionSelection) {
        revision = value.revision
        sourceRange = NSRange(
            location: Int(value.source_start_utf16),
            length: Int(value.source_end_utf16 - value.source_start_utf16)
        )
        visualRange = NSRange(
            location: Int(value.visual_start_utf16),
            length: Int(value.visual_end_utf16 - value.visual_start_utf16)
        )
        roundTripSourceRange = NSRange(
            location: Int(value.round_trip_source_start_utf16),
            length: Int(value.round_trip_source_end_utf16 - value.round_trip_source_start_utf16)
        )
        affinity = value.affinity
    }
}

private struct NativeProjectionSourceCaret {
    let revision: UInt64
    let visualUTF16: UInt64
    let sourceUTF16: UInt64
    let roundTripVisualUTF16: UInt64
    let affinity: UInt8

    init(_ value: YuStorageProjectionSourceCaret) {
        revision = value.revision
        visualUTF16 = value.visual_utf16
        sourceUTF16 = value.source_utf16
        roundTripVisualUTF16 = value.round_trip_visual_utf16
        affinity = value.affinity
    }
}

private struct NativeProjectionSourceSelection {
    let revision: UInt64
    let visualRange: NSRange
    let sourceRange: NSRange
    let roundTripVisualRange: NSRange
    let affinity: UInt8

    init(_ value: YuStorageProjectionSourceSelection) {
        revision = value.revision
        visualRange = NSRange(
            location: Int(value.visual_start_utf16),
            length: Int(value.visual_end_utf16 - value.visual_start_utf16)
        )
        sourceRange = NSRange(
            location: Int(value.source_start_utf16),
            length: Int(value.source_end_utf16 - value.source_start_utf16)
        )
        roundTripVisualRange = NSRange(
            location: Int(value.round_trip_visual_start_utf16),
            length: Int(value.round_trip_visual_end_utf16 - value.round_trip_visual_start_utf16)
        )
        affinity = value.affinity
    }
}

private struct NativeProjectionHit {
    let revision: UInt64
    let sourceUTF16: UInt64
    let visualUTF16: UInt64
    let roundTripSourceUTF16: UInt64
    let imageSourceRange: NSRange?
    let line: UInt64
    let point: CGPoint
    let affinity: UInt8

    init(_ value: YuStorageProjectionHit) {
        revision = value.revision
        sourceUTF16 = value.source_utf16
        visualUTF16 = value.visual_utf16
        roundTripSourceUTF16 = value.round_trip_source_utf16
        if value.image_source_start_utf16 == YU_STORAGE_IMAGE_DESTINATION_NONE
            || value.image_source_end_utf16 == YU_STORAGE_IMAGE_DESTINATION_NONE {
            imageSourceRange = nil
        } else {
            imageSourceRange = NSRange(
                location: Int(value.image_source_start_utf16),
                length: Int(value.image_source_end_utf16 - value.image_source_start_utf16)
            )
        }
        line = value.line
        point = CGPoint(x: CGFloat(value.x), y: CGFloat(value.y))
        affinity = value.affinity
    }
}

private struct NativeProjectionBlock {
    let revision: UInt64
    let blockIndex: UInt64
    let sourceRange: NSRange
    let visualUTF8Length: Int
    let visualUTF16Length: Int
    let kind: UInt8
    let projectionKind: UInt8

    init(_ value: YuStorageProjectionBlock) {
        revision = value.revision
        blockIndex = value.block_index
        sourceRange = NSRange(
            location: Int(value.source_start_utf16),
            length: Int(value.source_end_utf16 - value.source_start_utf16)
        )
        visualUTF8Length = Int(value.visual_utf8_length)
        visualUTF16Length = Int(value.visual_utf16_length)
        kind = value.kind
        projectionKind = value.projection_kind
    }
}

private struct NativeTableResizeCommit: Equatable {
    let revision: UInt64
    let blockIndex: UInt64
    let kind: UInt8
    let index: UInt64
    let initialPosition: Float
    let finalPosition: Float
    let delta: Float

    init(_ value: YuStorageTableResizeCommit) {
        revision = value.revision
        blockIndex = value.block_index
        kind = value.kind
        index = value.index
        initialPosition = value.initial_position
        finalPosition = value.final_position
        delta = value.delta
    }
}

private struct NativeBlockLayout {
    let revision: UInt64
    let blockIndex: UInt64
    let sourceRange: NSRange
    let visualUTF16Length: Int
    let lineCount: UInt64
    let width: CGFloat
    let height: CGFloat
    let lineHeight: CGFloat
    let defaultAdvance: CGFloat
    let kind: UInt8
    let projectionKind: UInt8
    let shaped: Bool

    init(_ value: YuStorageBlockLayout) {
        revision = value.revision
        blockIndex = value.block_index
        sourceRange = NSRange(
            location: Int(value.source_start_utf16),
            length: Int(value.source_end_utf16 - value.source_start_utf16)
        )
        visualUTF16Length = Int(value.visual_utf16_length)
        lineCount = value.line_count
        width = CGFloat(value.width)
        height = CGFloat(value.height)
        lineHeight = CGFloat(value.line_height)
        defaultAdvance = CGFloat(value.default_advance)
        kind = value.kind
        projectionKind = value.projection_kind
        shaped = value.shaped != 0
    }
}

private struct NativeMacosFontMetrics {
    let revision: UInt64
    let size: CGFloat
    let lineHeight: CGFloat
    let defaultAdvance: CGFloat

    init(_ value: YuStorageMacosFontMetrics) {
        revision = value.revision
        size = CGFloat(value.size)
        lineHeight = CGFloat(value.line_height)
        defaultAdvance = CGFloat(value.default_advance)
    }
}

private struct NativeBlockCaret {
    let revision: UInt64
    let sourceUTF16: UInt64
    let blockIndex: UInt64
    let visualUTF16: UInt64
    let roundTripSourceUTF16: UInt64
    let lineIndex: UInt64
    let point: CGPoint
    let height: CGFloat
    let affinity: UInt8
    let shaped: Bool

    init(_ value: YuStorageBlockCaret) {
        revision = value.revision
        sourceUTF16 = value.source_utf16
        blockIndex = value.block_index
        visualUTF16 = value.visual_utf16
        roundTripSourceUTF16 = value.round_trip_source_utf16
        lineIndex = value.line_index
        point = CGPoint(x: CGFloat(value.caret_x), y: CGFloat(value.caret_y))
        height = CGFloat(value.caret_height)
        affinity = value.affinity
        shaped = value.shaped != 0
    }
}

private struct NativeShapedViewportBlock {
    let revision: UInt64
    let blockIndex: UInt64
    let sourceRange: NSRange
    let y: CGFloat
    let height: CGFloat
    let measured: Bool
    let kind: UInt8

    init(_ value: YuStorageShapedViewportBlock) {
        revision = value.revision
        blockIndex = value.block_index
        sourceRange = NSRange(
            location: Int(value.source_start_utf16),
            length: Int(value.source_end_utf16 - value.source_start_utf16)
        )
        y = CGFloat(value.y)
        height = CGFloat(value.height)
        measured = value.measured != 0
        kind = value.kind
    }
}

private struct NativeShapedViewportSnapshot {
    let revision: UInt64
    let blockRange: Range<UInt64>
    let contentHeight: CGFloat
    let scrollY: CGFloat
    let viewportHeight: CGFloat
    let maxScrollY: CGFloat

    init(_ value: YuStorageShapedViewportSnapshot) {
        revision = value.revision
        blockRange = value.block_start..<value.block_end
        contentHeight = CGFloat(value.content_height)
        scrollY = CGFloat(value.scroll_y)
        viewportHeight = CGFloat(value.viewport_height)
        maxScrollY = CGFloat(value.max_scroll_y)
    }
}

private struct NativeVisualDecorationSnapshot {
    let revision: UInt64
    let compositionGeneration: UInt64
    let selectionCount: Int
    let caretPresent: Bool
    let contentHeight: CGFloat
    let scrollY: CGFloat
    let viewportHeight: CGFloat
    let maxScrollY: CGFloat
    let viewportWidth: CGFloat

    init(_ value: YuStorageMacosVisualDecorationSnapshot) {
        revision = value.revision
        compositionGeneration = value.composition_generation
        selectionCount = Int(value.selection_count)
        caretPresent = value.caret_present != 0
        contentHeight = CGFloat(value.content_height)
        scrollY = CGFloat(value.scroll_y)
        viewportHeight = CGFloat(value.viewport_height)
        maxScrollY = CGFloat(value.max_scroll_y)
        viewportWidth = CGFloat(value.viewport_width)
    }
}

private struct NativeVisualDecorationRect {
    let revision: UInt64
    let blockIndex: UInt64
    let lineIndex: UInt64
    let rect: CGRect
    let kind: UInt8

    init(_ value: YuStorageMacosVisualDecorationRect) {
        revision = value.revision
        blockIndex = value.block_index
        lineIndex = value.line_index
        rect = CGRect(
            x: CGFloat(value.x),
            y: CGFloat(value.y),
            width: CGFloat(value.width),
            height: CGFloat(value.height)
        )
        kind = value.kind
    }
}

private struct NativeVisualDecorationCaret {
    let revision: UInt64
    let blockIndex: UInt64
    let lineIndex: UInt64
    let rect: CGRect
    let affinity: UInt8
    let present: Bool

    init(_ value: YuStorageMacosVisualDecorationCaret) {
        revision = value.revision
        blockIndex = value.block_index
        lineIndex = value.line_index
        rect = CGRect(
            x: CGFloat(value.x),
            y: CGFloat(value.y),
            width: CGFloat(value.width),
            height: CGFloat(value.height)
        )
        affinity = value.affinity
        present = value.present != 0
    }
}

private struct NativeVisualSceneSnapshot {
    let revision: UInt64
    let blockRange: Range<UInt64>
    let primitiveCount: Int
    let contentHeight: CGFloat
    let scrollY: CGFloat
    let viewportHeight: CGFloat
    let maxScrollY: CGFloat
    let viewportWidth: CGFloat

    init(_ value: YuStorageVisualSceneSnapshot) {
        revision = value.revision
        blockRange = value.block_start..<value.block_end
        primitiveCount = Int(value.primitive_count)
        contentHeight = CGFloat(value.content_height)
        scrollY = CGFloat(value.scroll_y)
        viewportHeight = CGFloat(value.viewport_height)
        maxScrollY = CGFloat(value.max_scroll_y)
        viewportWidth = CGFloat(value.viewport_width)
    }
}

private struct NativeVisualScenePrimitive {
    let revision: UInt64
    let blockIndex: UInt64
    let sourceRange: NSRange
    let rect: CGRect
    let kind: UInt8

    init(_ value: YuStorageVisualScenePrimitive) {
        revision = value.revision
        blockIndex = value.block_index
        sourceRange = NSRange(
            location: Int(value.source_start_utf16),
            length: Int(value.source_end_utf16 - value.source_start_utf16)
        )
        rect = CGRect(
            x: CGFloat(value.x),
            y: CGFloat(value.y),
            width: CGFloat(value.width),
            height: CGFloat(value.height)
        )
        kind = value.kind
    }
}

private struct NativeVisualImage {
    let revision: UInt64
    let blockIndex: UInt64
    let sourceRange: NSRange
    let labelRange: NSRange
    let destinationRange: NSRange?
    let referenceRange: NSRange?
    let resourceFingerprint: UInt64
    let kind: UInt8

    init(_ value: YuStorageVisualImage) {
        revision = value.revision
        blockIndex = value.block_index
        sourceRange = NSRange(
            location: Int(value.source_start_utf16),
            length: Int(value.source_end_utf16 - value.source_start_utf16)
        )
        labelRange = NSRange(
            location: Int(value.label_start_utf16),
            length: Int(value.label_end_utf16 - value.label_start_utf16)
        )
        destinationRange = NativeVisualImage.optionalRange(
            start: value.destination_start_utf16,
            end: value.destination_end_utf16
        )
        referenceRange = NativeVisualImage.optionalRange(
            start: value.reference_start_utf16,
            end: value.reference_end_utf16
        )
        resourceFingerprint = value.resource_fingerprint
        kind = value.kind
    }

    private static func optionalRange(start: UInt64, end: UInt64) -> NSRange? {
        guard start != UInt64.max, end != UInt64.max, end >= start else {
            return nil
        }
        return NSRange(location: Int(start), length: Int(end - start))
    }
}

private struct NativeVisualRenderPlanSnapshot {
    let revision: UInt64
    let compositionGeneration: UInt64
    let blockRange: Range<UInt64>
    let commandCount: Int
    let uploadCount: Int
    let damageCount: Int
    let contentHeight: CGFloat
    let scrollY: CGFloat
    let viewportHeight: CGFloat
    let maxScrollY: CGFloat
    let viewportWidth: CGFloat

    init(_ value: YuStorageVisualRenderPlanSnapshot) {
        revision = value.revision
        compositionGeneration = value.composition_generation
        blockRange = value.block_start..<value.block_end
        commandCount = Int(value.command_count)
        uploadCount = Int(value.upload_count)
        damageCount = Int(value.damage_count)
        contentHeight = CGFloat(value.content_height)
        scrollY = CGFloat(value.scroll_y)
        viewportHeight = CGFloat(value.viewport_height)
        maxScrollY = CGFloat(value.max_scroll_y)
        viewportWidth = CGFloat(value.viewport_width)
    }
}

private struct NativeVisualSceneGlyphSnapshot {
    let revision: UInt64
    let compositionGeneration: UInt64
    let frameRevision: UInt64
    let surfaceGeneration: UInt64
    let frameSerial: UInt64
    let blockRange: Range<UInt64>
    let glyphCount: Int
    let contentHeight: CGFloat
    let scrollY: CGFloat
    let viewportHeight: CGFloat
    let maxScrollY: CGFloat
    let viewportWidth: CGFloat

    init(_ value: YuStorageVisualSceneGlyphSnapshot) {
        revision = value.revision
        compositionGeneration = value.composition_generation
        frameRevision = value.frame_revision
        surfaceGeneration = value.surface_generation
        frameSerial = value.frame_serial
        blockRange = value.block_start..<value.block_end
        glyphCount = Int(value.glyph_count)
        contentHeight = CGFloat(value.content_height)
        scrollY = CGFloat(value.scroll_y)
        viewportHeight = CGFloat(value.viewport_height)
        maxScrollY = CGFloat(value.max_scroll_y)
        viewportWidth = CGFloat(value.viewport_width)
    }
}

private struct NativeVisualSceneGlyph {
    let revision: UInt64
    let blockIndex: UInt64
    let sourceRange: NSRange
    let page: UInt32
    let atlasRect: CGRect
    let origin: CGPoint
    let bearingX: CGFloat
    let bearingY: CGFloat
    let advanceX: CGFloat
    let bounds: CGRect
    let colorRGBA: UInt32

    init(_ value: YuStorageVisualSceneGlyph) {
        revision = value.revision
        blockIndex = value.block_index
        sourceRange = NSRange(
            location: Int(value.source_start_utf16),
            length: Int(value.source_end_utf16 - value.source_start_utf16)
        )
        page = value.page
        atlasRect = CGRect(
            x: CGFloat(value.atlas_x),
            y: CGFloat(value.atlas_y),
            width: CGFloat(value.atlas_width),
            height: CGFloat(value.atlas_height)
        )
        origin = CGPoint(x: CGFloat(value.origin_x), y: CGFloat(value.origin_y))
        bearingX = CGFloat(value.bearing_x)
        bearingY = CGFloat(value.bearing_y)
        advanceX = CGFloat(value.advance_x)
        bounds = CGRect(
            x: CGFloat(value.bounds_x),
            y: CGFloat(value.bounds_y),
            width: CGFloat(value.bounds_width),
            height: CGFloat(value.bounds_height)
        )
        colorRGBA = value.color_rgba
    }
}

private struct NativeMacosRenderHostSnapshot {
    let revision: UInt64
    let compositionGeneration: UInt64
    let frameRevision: UInt64
    let surfaceGeneration: UInt64
    let frameSerial: UInt64
    let commandCount: Int
    let uploadCount: Int
    let damageCount: Int
    let atlasPageCount: Int
    let atlasGlyphCount: Int
    let atlasBytes: Int
    let contentHeight: CGFloat
    let scrollY: CGFloat
    let viewportHeight: CGFloat
    let maxScrollY: CGFloat
    let viewportWidth: CGFloat
    let published: Bool

    init(_ value: YuStorageMacosRenderHostSnapshot) {
        revision = value.revision
        compositionGeneration = value.composition_generation
        frameRevision = value.frame_revision
        surfaceGeneration = value.surface_generation
        frameSerial = value.frame_serial
        commandCount = Int(value.command_count)
        uploadCount = Int(value.upload_count)
        damageCount = Int(value.damage_count)
        atlasPageCount = Int(value.atlas_page_count)
        atlasGlyphCount = Int(value.atlas_glyph_count)
        atlasBytes = Int(value.atlas_bytes)
        contentHeight = CGFloat(value.content_height)
        scrollY = CGFloat(value.scroll_y)
        viewportHeight = CGFloat(value.viewport_height)
        maxScrollY = CGFloat(value.max_scroll_y)
        viewportWidth = CGFloat(value.viewport_width)
        published = value.published != 0
    }
}

private struct NativeMacosRenderHostSurfaceSnapshot {
    let revision: UInt64
    let compositionGeneration: UInt64
    let surfaceGeneration: UInt64
    let frameSerial: UInt64
    let uploadedPages: Int
    let uploadedImages: Int
    let commandCount: Int
    let damageCount: Int
    let atlasPageCount: Int
    let imageResourceCount: Int
    let imageRequestCount: Int
    let imageFailureCount: Int
    let imageEvictionCount: Int
    let imageAtlasEvictionCount: Int
    let imageCandidateCount: Int
    let imageDuplicateCount: Int
    let imageVisibleCandidateCount: Int
    let imageOverscanCandidateCount: Int
    let imageRetryCount: Int
    let submitted: Bool

    init(_ value: YuStorageMacosRenderHostSurfaceSnapshot) {
        revision = value.revision
        compositionGeneration = value.composition_generation
        surfaceGeneration = value.surface_generation
        frameSerial = value.frame_serial
        uploadedPages = Int(value.uploaded_pages)
        uploadedImages = Int(value.uploaded_images)
        commandCount = Int(value.command_count)
        damageCount = Int(value.damage_count)
        atlasPageCount = Int(value.atlas_page_count)
        imageResourceCount = Int(value.image_resource_count)
        imageRequestCount = Int(value.image_request_count)
        imageFailureCount = Int(value.image_failure_count)
        imageEvictionCount = Int(value.image_eviction_count)
        imageAtlasEvictionCount = Int(value.image_atlas_eviction_count)
        imageCandidateCount = Int(value.image_candidate_count)
        imageDuplicateCount = Int(value.image_duplicate_count)
        imageVisibleCandidateCount = Int(value.image_visible_candidate_count)
        imageOverscanCandidateCount = Int(value.image_overscan_candidate_count)
        imageRetryCount = Int(value.image_retry_count)
        submitted = value.submitted != 0
    }
}

private struct NativeVisualRenderCommand {
    let revision: UInt64
    let blockIndex: UInt64
    let sourceRange: NSRange
    let kind: UInt8
    let page: UInt32
    let atlasRect: CGRect
    let origin: CGPoint
    let bearingX: CGFloat
    let bearingY: CGFloat
    let advanceX: CGFloat
    let bounds: CGRect
    let colorRGBA: UInt32
    let resource: UInt64

    init(_ value: YuStorageVisualRenderCommand) {
        revision = value.revision
        blockIndex = value.block_index
        sourceRange = NSRange(
            location: Int(value.source_start_utf16),
            length: Int(value.source_end_utf16 - value.source_start_utf16)
        )
        kind = value.kind
        page = value.page
        atlasRect = CGRect(
            x: CGFloat(value.atlas_x),
            y: CGFloat(value.atlas_y),
            width: CGFloat(value.atlas_width),
            height: CGFloat(value.atlas_height)
        )
        origin = CGPoint(x: CGFloat(value.origin_x), y: CGFloat(value.origin_y))
        bearingX = CGFloat(value.bearing_x)
        bearingY = CGFloat(value.bearing_y)
        advanceX = CGFloat(value.advance_x)
        bounds = CGRect(
            x: CGFloat(value.bounds_x),
            y: CGFloat(value.bounds_y),
            width: CGFloat(value.bounds_width),
            height: CGFloat(value.bounds_height)
        )
        colorRGBA = value.color_rgba
        resource = value.resource
    }
}

private struct NativeVisualRenderPage {
    let revision: UInt64
    let page: UInt32
    let width: Int
    let height: Int
    let fingerprint: UInt64

    init(_ value: YuStorageVisualRenderPage) {
        revision = value.revision
        page = value.page
        width = Int(value.width)
        height = Int(value.height)
        fingerprint = value.fingerprint
    }
}

private struct NativeVisualRenderDamage {
    let revision: UInt64
    let rect: CGRect

    init(_ value: YuStorageVisualRenderDamage) {
        revision = value.revision
        rect = CGRect(
            x: CGFloat(value.x),
            y: CGFloat(value.y),
            width: CGFloat(value.width),
            height: CGFloat(value.height)
        )
    }
}

private struct NativeVisualViewport {
    let revision: UInt64
    let blockRange: Range<UInt64>
    let contentHeight: CGFloat
    let contentOriginY: CGFloat
    let requestedScrollY: CGFloat
    let viewportHeight: CGFloat
    let maxScrollY: CGFloat

    init(_ snapshot: NativeShapedViewportSnapshot, contentOriginY: CGFloat = 0.0) {
        revision = snapshot.revision
        blockRange = snapshot.blockRange
        contentHeight = snapshot.contentHeight
        self.contentOriginY = contentOriginY
        requestedScrollY = snapshot.scrollY
        viewportHeight = snapshot.viewportHeight
        maxScrollY = snapshot.maxScrollY
    }

    /// Native scroll views clamp their content offset. Keeping the clamp in
    /// this adapter makes document↔viewport conversion deterministic even if
    /// a platform callback briefly reports an out-of-range offset.
    var effectiveScrollY: CGFloat {
        min(max(requestedScrollY, 0.0), maxScrollY)
    }

    func viewportPoint(forDocumentPoint point: NSPoint) -> NSPoint {
        NSPoint(
            x: point.x,
            y: point.y - contentOriginY - effectiveScrollY
        )
    }

    func documentPoint(forViewportPoint point: NSPoint) -> NSPoint {
        NSPoint(
            x: point.x,
            y: point.y + contentOriginY + effectiveScrollY
        )
    }
}

private struct NativeCaretScrollRequest {
    let revision: UInt64
    let sourceUTF16: UInt64
    let blockIndex: UInt64
    let caretPoint: NSPoint
    let caretWidth: CGFloat
    let caretHeight: CGFloat
    let currentScrollY: CGFloat
    let targetScrollY: CGFloat
    let margin: CGFloat
    let needsScroll: Bool

    init(_ value: YuStorageCaretScrollRequest) {
        revision = value.revision
        sourceUTF16 = value.source_utf16
        blockIndex = value.block_index
        caretPoint = NSPoint(x: CGFloat(value.caret_x), y: CGFloat(value.caret_y))
        caretWidth = CGFloat(value.caret_width)
        caretHeight = CGFloat(value.caret_height)
        currentScrollY = CGFloat(value.current_scroll_y)
        targetScrollY = CGFloat(value.target_scroll_y)
        margin = CGFloat(value.margin)
        needsScroll = value.needs_scroll != 0
    }
}

private struct NativeAccessibilitySnapshot {
    let revision: UInt64
    let numberOfCharacters: Int
    let selectedRange: NSRange
    let lineCount: Int
    let affinity: UInt8

    init(_ value: YuStorageAccessibilitySnapshot) {
        revision = value.revision
        numberOfCharacters = Int(value.number_of_characters_utf16)
        selectedRange = NSRange(
            location: Int(value.selection_start_utf16),
            length: Int(value.selection_end_utf16 - value.selection_start_utf16)
        )
        lineCount = Int(value.line_count)
        affinity = value.selection_affinity
    }
}

private struct NativeAccessibilityRange {
    let revision: UInt64
    let range: NSRange

    init(_ value: YuStorageAccessibilityRange) {
        revision = value.revision
        range = NSRange(
            location: Int(value.start_utf16),
            length: Int(value.end_utf16 - value.start_utf16)
        )
    }
}

/// Owned semantic metadata returned by Rust for one Accessibility revision.
/// The host keeps ranges and scalar roles only; node text is fetched through
/// the revision-bound source query when a native Accessibility element needs
/// to speak it.
private struct NativeAccessibilitySemanticNode {
    let revision: UInt64
    let index: UInt32
    let parent: UInt32
    let kind: UInt8
    let flags: UInt8
    let level: UInt8
    let sourceRange: NSRange
    let labelRange: NSRange
    let destinationRange: NSRange?
    let actionBlock: UInt64?

    init(_ value: YuStorageAccessibilityNodeV2) {
        revision = value.revision
        index = value.index
        parent = value.parent
        kind = value.kind
        flags = value.flags
        level = value.level
        sourceRange = NSRange(
            location: Int(value.source_start_utf16),
            length: Int(value.source_end_utf16 - value.source_start_utf16)
        )
        labelRange = NSRange(
            location: Int(value.label_start_utf16),
            length: Int(value.label_end_utf16 - value.label_start_utf16)
        )
        if value.destination_start_utf16 == UInt64.max
            || value.destination_end_utf16 < value.destination_start_utf16 {
            destinationRange = nil
        } else {
            destinationRange = NSRange(
                location: Int(value.destination_start_utf16),
                length: Int(value.destination_end_utf16 - value.destination_start_utf16)
            )
        }
        actionBlock = value.action_block == UInt64.max ? nil : value.action_block
    }
}

private enum SemanticAccessibilityKind: UInt8 {
    case document = 1
    case heading = 2
    case paragraph = 3
    case codeBlock = 4
    case blockQuote = 5
    case listItem = 6
    case taskListItem = 7
    case emphasis = 8
    case strong = 9
    case codeSpan = 10
    case link = 11
    case image = 12
    case autolink = 13
    case referenceLink = 14
    case referenceImage = 15
}

private enum SemanticAccessibilityFlag {
    static let taskDone: UInt8 = 1 << 1
}

/// A lightweight AppKit AX element backed by one Rust semantic node. It owns
/// no Markdown text: labels and range queries always use the node's Revision
/// and ask `StorageBridge` for the current source bytes.
private final class YuAccessibilitySemanticElement: NSObject,
    NSAccessibilityElementProtocol
{
    let node: NativeAccessibilitySemanticNode
    private let bridge: StorageBridge
    weak var parentObject: AnyObject?
    var semanticChildren: [Any] = []
    weak var frameOwner: DocumentTextView?

    init(
        node: NativeAccessibilitySemanticNode,
        bridge: StorageBridge,
        parent: AnyObject?
    ) {
        self.node = node
        self.bridge = bridge
        parentObject = parent
        super.init()
    }

    @objc func accessibilityFrame() -> NSRect {
        frameOwner?.accessibilityFrameForSemanticRange(node.sourceRange) ?? .zero
    }

    @objc func accessibilityParent() -> Any? { parentObject }

    @objc var accessibilityRole: NSAccessibility.Role {
        switch SemanticAccessibilityKind(rawValue: node.kind) {
        case .taskListItem:
            return .checkBox
        case .link, .autolink, .referenceLink:
            return .link
        case .image, .referenceImage:
            return .image
        case .blockQuote, .listItem:
            return .group
        case .document, .heading, .paragraph, .codeBlock, .emphasis, .strong, .codeSpan, .none:
            return .staticText
        }
    }

    @objc var accessibilityRoleDescription: String? {
        switch SemanticAccessibilityKind(rawValue: node.kind) {
        case .heading:
            return "标题（级别 \(node.level)）"
        case .codeBlock, .codeSpan:
            return "代码"
        case .blockQuote:
            return "引用"
        case .listItem:
            return "列表项"
        case .taskListItem:
            return "任务列表项"
        case .emphasis:
            return "强调文本"
        case .strong:
            return "粗体文本"
        case .link, .autolink, .referenceLink:
            return "链接"
        case .image, .referenceImage:
            return "图像"
        case .document, .paragraph, .none:
            return nil
        }
    }

    @objc var accessibilityLabel: String? {
        bridge.copySourceRangeIfAvailable(node.labelRange, revision: node.revision)
    }

    @objc var accessibilityTitle: String? { accessibilityLabel }

    @objc var accessibilityValue: Any? {
        guard SemanticAccessibilityKind(rawValue: node.kind) == .taskListItem else {
            return accessibilityLabel
        }
        return NSNumber(value: node.flags & SemanticAccessibilityFlag.taskDone != 0)
    }

    /// Link destinations are parser-resolved source ranges. The native child
    /// exposes only a Foundation URL value; it never reparses Markdown or
    /// retains a destination string outside the current Revision.
    @objc var accessibilityURL: URL? {
        guard let kind = SemanticAccessibilityKind(rawValue: node.kind),
              kind == .link || kind == .autolink || kind == .referenceLink,
              let destinationRange = node.destinationRange,
              let destination = bridge.copySourceRangeIfAvailable(
                  destinationRange,
                  revision: node.revision
              ),
              !destination.isEmpty else {
            return nil
        }
        if kind == .autolink,
           destination.contains("@"),
           !destination.contains(":") {
            return URL(string: "mailto:\(destination)")
        }
        return URL(string: destination)
    }

    /// VoiceOver can press a task checkbox, but the operation remains a
    /// Revision-bound Rust command. Links deliberately have no press action
    /// yet; opening external content needs a separate product policy.
    @objc func accessibilityPerformPress() -> Bool {
        guard SemanticAccessibilityKind(rawValue: node.kind) == .taskListItem else {
            return false
        }
        return frameOwner?.toggleTaskAccessibilityNode(node) ?? false
    }

    @objc func accessibilityIdentifier() -> String {
        "yu-document-semantic-\(node.revision)-\(node.index)"
    }

    @objc var accessibilityChildren: [Any]? { semanticChildren }

    @objc var accessibilityChildrenInNavigationOrder: [Any]? {
        semanticChildren
    }

    @objc(accessibilityStringForRange:)
    func accessibilityString(for range: NSRange) -> String? {
        guard range.location >= 0,
              range.length >= 0,
              NSMaxRange(range) <= node.sourceRange.length else {
            return nil
        }
        let absolute = NSRange(
            location: node.sourceRange.location + range.location,
            length: range.length
        )
        return bridge.copySourceRangeIfAvailable(absolute, revision: node.revision)
    }

    @objc(accessibilityAttributedStringForRange:)
    func accessibilityAttributedString(for range: NSRange) -> NSAttributedString? {
        guard let text = accessibilityString(for: range) else { return nil }
        return NSAttributedString(string: text)
    }
}

/// VoiceOver asks custom rotors for the next source-backed semantic element.
/// The delegate is intentionally tiny: it never stores text or a second tree,
/// and asks the current DocumentTextView for the live element order.
private final class YuAccessibilityRotorDelegate: NSObject,
    NSAccessibilityCustomRotorItemSearchDelegate
{
    weak var owner: DocumentTextView?
    let kind: SemanticAccessibilityKind

    init(owner: DocumentTextView, kind: SemanticAccessibilityKind) {
        self.owner = owner
        self.kind = kind
        super.init()
    }

    func rotor(
        _ rotor: NSAccessibilityCustomRotor,
        resultFor searchParameters: NSAccessibilityCustomRotor.SearchParameters
    ) -> NSAccessibilityCustomRotor.ItemResult? {
        owner?.accessibilityRotorResult(for: kind, parameters: searchParameters)
    }
}

private struct NativeComposition {
    let revision: UInt64
    let generation: UInt64
    let replacementRange: NSRange
    let selection: NSRange
    let preeditUTF8Length: Int
    let active: Bool

    init(_ value: YuStorageCompositionState) {
        revision = value.revision
        generation = value.generation
        replacementRange = NSRange(
            location: Int(value.replacement_start_utf16),
            length: Int(value.replacement_end_utf16 - value.replacement_start_utf16)
        )
        selection = NSRange(
            location: Int(value.selection_start_utf16),
            length: Int(value.selection_end_utf16 - value.selection_start_utf16)
        )
        preeditUTF8Length = Int(value.preedit_utf8_length)
        active = value.active != 0
    }
}

private struct NativeCompositionProjection {
    let revision: UInt64
    let generation: UInt64
    let replacementRange: NSRange
    let preeditSelection: NSRange
    let visualSelection: NSRange
    let visualReplacementRange: NSRange
    let projectedUTF16Length: Int
    let projectedUTF8Length: Int

    init(_ value: YuStorageCompositionProjection) {
        revision = value.revision
        generation = value.generation
        replacementRange = NSRange(
            location: Int(value.replacement_start_utf16),
            length: Int(value.replacement_end_utf16 - value.replacement_start_utf16)
        )
        preeditSelection = NSRange(
            location: Int(value.preedit_selection_start_utf16),
            length: Int(value.preedit_selection_end_utf16 - value.preedit_selection_start_utf16)
        )
        visualSelection = NSRange(
            location: Int(value.visual_selection_start_utf16),
            length: Int(value.visual_selection_end_utf16 - value.visual_selection_start_utf16)
        )
        visualReplacementRange = NSRange(
            location: Int(value.visual_replacement_start_utf16),
            length: Int(value.visual_replacement_end_utf16 - value.visual_replacement_start_utf16)
        )
        projectedUTF16Length = Int(value.projected_utf16_length)
        projectedUTF8Length = Int(value.projected_utf8_length)
    }
}

private struct NativeCompositionCaret {
    let revision: UInt64
    let generation: UInt64
    let sourceUTF16: UInt64
    let visualUTF16: UInt64
    let roundTripSourceUTF16: UInt64
    let visualSelection: NSRange
    let affinity: UInt8

    init(_ value: YuStorageCompositionCaret) {
        revision = value.revision
        generation = value.generation
        sourceUTF16 = value.source_utf16
        visualUTF16 = value.visual_utf16
        roundTripSourceUTF16 = value.round_trip_source_utf16
        visualSelection = NSRange(
            location: Int(value.visual_selection_start_utf16),
            length: Int(value.visual_selection_end_utf16 - value.visual_selection_start_utf16)
        )
        affinity = value.affinity
    }
}

private struct NativeCompositionShapedCaret {
    let revision: UInt64
    let generation: UInt64
    let sourceUTF16: UInt64
    let blockIndex: UInt64
    let visualUTF16: UInt64
    let roundTripSourceUTF16: UInt64
    let lineIndex: UInt64
    let point: CGPoint
    let size: CGSize
    let visualSelection: NSRange
    let visualReplacement: NSRange
    let affinity: UInt8

    init(_ value: YuStorageCompositionShapedCaret) {
        revision = value.revision
        generation = value.generation
        sourceUTF16 = value.source_utf16
        blockIndex = value.block_index
        visualUTF16 = value.visual_utf16
        roundTripSourceUTF16 = value.round_trip_source_utf16
        lineIndex = value.line_index
        point = CGPoint(x: CGFloat(value.caret_x), y: CGFloat(value.caret_y))
        size = CGSize(width: CGFloat(value.caret_width), height: CGFloat(value.caret_height))
        visualSelection = NSRange(
            location: Int(value.visual_selection_start_utf16),
            length: Int(value.visual_selection_end_utf16 - value.visual_selection_start_utf16)
        )
        visualReplacement = NSRange(
            location: Int(value.visual_replacement_start_utf16),
            length: Int(value.visual_replacement_end_utf16 - value.visual_replacement_start_utf16)
        )
        affinity = value.affinity
    }
}

private struct NativeCompositionProjectionHit {
    let revision: UInt64
    let generation: UInt64
    let sourceUTF16: UInt64
    let blockIndex: UInt64
    let visualUTF16: UInt64
    let roundTripSourceUTF16: UInt64
    let line: UInt64
    let point: CGPoint
    let visualSelection: NSRange
    let visualReplacement: NSRange
    let affinity: UInt8

    init(_ value: YuStorageCompositionProjectionHit) {
        revision = value.revision
        generation = value.generation
        sourceUTF16 = value.source_utf16
        blockIndex = value.block_index
        visualUTF16 = value.visual_utf16
        roundTripSourceUTF16 = value.round_trip_source_utf16
        line = value.line
        point = CGPoint(x: CGFloat(value.x), y: CGFloat(value.y))
        visualSelection = NSRange(
            location: Int(value.visual_selection_start_utf16),
            length: Int(value.visual_selection_end_utf16 - value.visual_selection_start_utf16)
        )
        visualReplacement = NSRange(
            location: Int(value.visual_replacement_start_utf16),
            length: Int(value.visual_replacement_end_utf16 - value.visual_replacement_start_utf16)
        )
        affinity = value.affinity
    }
}

private struct NativeCommandResult {
    let revision: UInt64
    let selection: NSRange
    let affinity: UInt8
    let changed: Bool
    let sourceSync: UInt8
    let oldSourceRange: NSRange?
    let newSourceRange: NSRange?

    init(_ value: YuStorageCommandResult) {
        revision = value.revision
        selection = NSRange(
            location: Int(value.selection_start_utf16),
            length: Int(value.selection_end_utf16 - value.selection_start_utf16)
        )
        affinity = value.affinity
        changed = value.changed != 0
        sourceSync = value.source_sync
        oldSourceRange = sourceSync == 1
            ? NSRange(
                location: Int(value.source_start_utf16),
                length: Int(value.source_old_end_utf16 - value.source_start_utf16)
            )
            : nil
        newSourceRange = sourceSync == 1
            ? NSRange(
                location: Int(value.source_new_start_utf16),
                length: Int(value.source_new_end_utf16 - value.source_new_start_utf16)
            )
            : nil
    }
}

private final class StorageBridge {
    private var handle: OpaquePointer

    init(path: String) throws {
        var created: OpaquePointer?
        let bytes = Array(path.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_storage_session_open(buffer.baseAddress, buffer.count, &created)
        }
        guard status == StorageStatus.ok, let created else {
            throw BridgeError.open(status)
        }
        handle = created
    }

    deinit {
        yu_storage_session_destroy(handle)
    }

    var path: String {
        copyBytes { output, capacity, written in
            yu_storage_session_copy_path(
                handle,
                output,
                capacity,
                written
            )
        }
    }

    var source: String {
        copyBytes { output, capacity, written in
            yu_storage_session_copy_source(
                handle,
                output,
                capacity,
                written
            )
        }
    }

    var copySourceIfAvailable: String? {
        copyBytesIfAvailable { output, capacity, written in
            yu_storage_session_copy_source(
                handle,
                output,
                capacity,
                written
            )
        }
    }

    func projectedSource(revision: UInt64) throws -> String {
        try copyBytesThrowing { output, capacity, written in
            yu_storage_session_projected_source(
                handle,
                revision,
                output,
                capacity,
                written
            )
        }
    }

    func projectionCaret(
        revision: UInt64,
        sourceUTF16: UInt64,
        affinity: UInt8
    ) throws -> NativeProjectionCaret {
        var value = YuStorageProjectionCaret()
        let status = yu_storage_session_projection_caret(
            handle,
            revision,
            sourceUTF16,
            affinity,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeProjectionCaret(value)
    }

    func projectionSelection(
        revision: UInt64,
        sourceRange: NSRange,
        affinity: UInt8
    ) throws -> NativeProjectionSelection {
        var value = YuStorageProjectionSelection()
        let status = yu_storage_session_projection_selection(
            handle,
            revision,
            UInt64(sourceRange.location),
            UInt64(sourceRange.location + sourceRange.length),
            affinity,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeProjectionSelection(value)
    }

    func projectionSourceCaret(
        revision: UInt64,
        visualUTF16: UInt64,
        affinity: UInt8
    ) throws -> NativeProjectionSourceCaret {
        var value = YuStorageProjectionSourceCaret()
        let status = yu_storage_session_projection_source_caret(
            handle,
            revision,
            visualUTF16,
            affinity,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeProjectionSourceCaret(value)
    }

    func projectionSourceSelection(
        revision: UInt64,
        visualRange: NSRange,
        affinity: UInt8
    ) throws -> NativeProjectionSourceSelection {
        var value = YuStorageProjectionSourceSelection()
        let status = yu_storage_session_projection_source_selection(
            handle,
            revision,
            UInt64(visualRange.location),
            UInt64(visualRange.location + visualRange.length),
            affinity,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeProjectionSourceSelection(value)
    }

    func projectionHitTest(
        revision: UInt64,
        point: CGPoint,
        maxWidth: Float = 80.0,
        lineHeight: Float = 1.0,
        defaultAdvance: Float = 1.0
    ) throws -> NativeProjectionHit {
        var value = YuStorageProjectionHit()
        let status = yu_storage_session_projection_hit_test(
            handle,
            revision,
            Float(point.x),
            Float(point.y),
            maxWidth,
            lineHeight,
            defaultAdvance,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeProjectionHit(value)
    }

    func macosProjectionHitTest(
        revision: UInt64,
        point: CGPoint,
        size: Float,
        maxWidth: Float
    ) throws -> NativeProjectionHit {
        var value = YuStorageProjectionHit()
        let status = yu_storage_session_macos_projection_hit_test(
            handle,
            revision,
            Float(point.x),
            Float(point.y),
            size,
            maxWidth,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeProjectionHit(value)
    }

    func projectionBlockCount(revision: UInt64) throws -> Int {
        var count = 0
        let status = yu_storage_session_projection_block_count(handle, revision, &count)
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return count
    }

    func projectedBlock(
        revision: UInt64,
        blockIndex: UInt64
    ) throws -> (NativeProjectionBlock, String) {
        var metadata = YuStorageProjectionBlock()
        var required = 0
        let sizeStatus = yu_storage_session_projected_block(
            handle,
            revision,
            blockIndex,
            &metadata,
            nil,
            0,
            &required
        )
        guard sizeStatus == StorageStatus.ok else {
            throw BridgeError.operation(sizeStatus)
        }
        var bytes = Array(repeating: UInt8(0), count: required)
        var written = required
        let copyStatus = bytes.withUnsafeMutableBufferPointer { buffer in
            yu_storage_session_projected_block(
                handle,
                revision,
                blockIndex,
                &metadata,
                buffer.baseAddress,
                buffer.count,
                &written
            )
        }
        guard copyStatus == StorageStatus.ok, written >= 0, written <= bytes.count else {
            throw BridgeError.operation(copyStatus)
        }
        return (
            NativeProjectionBlock(metadata),
            String(decoding: bytes.prefix(written), as: UTF8.self)
        )
    }

    func projectedTableCells(
        revision: UInt64,
        blockIndex: UInt64
    ) throws -> [YuStorageTableCellRange] {
        var required = 0
        let sizeStatus = yu_storage_session_projected_table_cells(
            handle,
            revision,
            blockIndex,
            nil,
            0,
            &required
        )
        guard sizeStatus == StorageStatus.ok else {
            throw BridgeError.operation(sizeStatus)
        }
        var cells = Array(repeating: YuStorageTableCellRange(), count: required)
        var written = required
        let copyStatus = cells.withUnsafeMutableBufferPointer { buffer in
            yu_storage_session_projected_table_cells(
                handle,
                revision,
                blockIndex,
                buffer.baseAddress,
                buffer.count,
                &written
            )
        }
        guard copyStatus == StorageStatus.ok, written >= 0, written <= cells.count else {
            throw BridgeError.operation(copyStatus)
        }
        return Array(cells.prefix(written))
    }

    func tableLayoutCells(
        revision: UInt64,
        blockIndex: UInt64,
        maxWidth: Float,
        lineHeight: Float,
        defaultAdvance: Float
    ) throws -> [YuStorageTableLayoutCell] {
        var required = 0
        let sizeStatus = yu_storage_session_table_layout_cells(
            handle,
            revision,
            blockIndex,
            maxWidth,
            lineHeight,
            defaultAdvance,
            nil,
            0,
            &required
        )
        guard sizeStatus == StorageStatus.ok else {
            throw BridgeError.operation(sizeStatus)
        }
        var cells = Array(repeating: YuStorageTableLayoutCell(), count: required)
        var written = required
        let copyStatus = cells.withUnsafeMutableBufferPointer { buffer in
            yu_storage_session_table_layout_cells(
                handle,
                revision,
                blockIndex,
                maxWidth,
                lineHeight,
                defaultAdvance,
                buffer.baseAddress,
                buffer.count,
                &written
            )
        }
        guard copyStatus == StorageStatus.ok, written >= 0, written <= cells.count else {
            throw BridgeError.operation(copyStatus)
        }
        return Array(cells.prefix(written))
    }

    func tableLayoutCellsWithResize(
        revision: UInt64,
        blockIndex: UInt64,
        resizeKind: UInt8,
        resizeIndex: UInt64,
        resizeDelta: Float,
        maxWidth: Float,
        lineHeight: Float,
        defaultAdvance: Float
    ) throws -> [YuStorageTableLayoutCell] {
        var required = 0
        let sizeStatus = yu_storage_session_table_layout_cells_with_resize(
            handle,
            revision,
            blockIndex,
            maxWidth,
            lineHeight,
            defaultAdvance,
            resizeKind,
            resizeIndex,
            resizeDelta,
            nil,
            0,
            &required
        )
        guard sizeStatus == StorageStatus.ok else {
            throw BridgeError.operation(sizeStatus)
        }
        var cells = Array(repeating: YuStorageTableLayoutCell(), count: required)
        var written = required
        let copyStatus = cells.withUnsafeMutableBufferPointer { buffer in
            yu_storage_session_table_layout_cells_with_resize(
                handle,
                revision,
                blockIndex,
                maxWidth,
                lineHeight,
                defaultAdvance,
                resizeKind,
                resizeIndex,
                resizeDelta,
                buffer.baseAddress,
                buffer.count,
                &written
            )
        }
        guard copyStatus == StorageStatus.ok, written >= 0, written <= cells.count else {
            throw BridgeError.operation(copyStatus)
        }
        return Array(cells.prefix(written))
    }

    func tableCellHitTest(
        revision: UInt64,
        blockIndex: UInt64,
        point: CGPoint,
        maxWidth: Float,
        lineHeight: Float,
        defaultAdvance: Float
    ) throws -> YuStorageTableCellHit {
        var value = YuStorageTableCellHit()
        let status = yu_storage_session_table_cell_hit_test(
            handle,
            revision,
            blockIndex,
            maxWidth,
            lineHeight,
            defaultAdvance,
            Float(point.x),
            Float(point.y),
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return value
    }

    func tableResizeHitTest(
        revision: UInt64,
        blockIndex: UInt64,
        point: CGPoint,
        tolerance: Float,
        maxWidth: Float,
        lineHeight: Float,
        defaultAdvance: Float
    ) throws -> YuStorageTableResizeHit {
        var value = YuStorageTableResizeHit()
        let status = yu_storage_session_table_resize_hit_test(
            handle,
            revision,
            blockIndex,
            maxWidth,
            lineHeight,
            defaultAdvance,
            Float(point.x),
            Float(point.y),
            tolerance,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return value
    }

    func tableResizeBegin(
        revision: UInt64,
        blockIndex: UInt64,
        point: CGPoint,
        tolerance: Float,
        pointerPosition: Float,
        maxWidth: Float,
        lineHeight: Float,
        defaultAdvance: Float
    ) throws -> YuStorageTableResizeHit {
        var value = YuStorageTableResizeHit()
        let status = yu_storage_session_table_resize_begin(
            handle,
            revision,
            blockIndex,
            maxWidth,
            lineHeight,
            defaultAdvance,
            Float(point.x),
            Float(point.y),
            tolerance,
            pointerPosition,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return value
    }

    func macosTableResizeBegin(
        revision: UInt64,
        blockIndex: UInt64,
        size: Float,
        maxWidth: Float,
        point: CGPoint,
        tolerance: Float,
        pointerPosition: Float
    ) throws -> YuStorageTableResizeHit {
        var value = YuStorageTableResizeHit()
        let status = yu_storage_session_macos_table_resize_begin(
            handle,
            revision,
            blockIndex,
            size,
            maxWidth,
            Float(point.x),
            Float(point.y),
            tolerance,
            pointerPosition,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return value
    }

    func macosTableResizeHitTestAtDocumentPoint(
        revision: UInt64,
        size: Float,
        maxWidth: Float,
        point: CGPoint,
        tolerance: Float
    ) throws -> YuStorageTableResizeHit {
        var value = YuStorageTableResizeHit()
        let status = yu_storage_session_macos_table_resize_hit_test(
            handle,
            revision,
            size,
            maxWidth,
            Float(point.x),
            Float(point.y),
            tolerance,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return value
    }

    func macosTableResizeBeginAtDocumentPoint(
        revision: UInt64,
        size: Float,
        maxWidth: Float,
        point: CGPoint,
        tolerance: Float,
        pointerPosition: Float
    ) throws -> YuStorageTableResizeHit {
        var value = YuStorageTableResizeHit()
        let status = yu_storage_session_macos_table_resize_begin_at_point(
            handle,
            revision,
            size,
            maxWidth,
            Float(point.x),
            Float(point.y),
            tolerance,
            pointerPosition,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return value
    }

    func tableResizeUpdate(
        revision: UInt64,
        pointerPosition: Float
    ) throws -> NativeTableResizeCommit {
        var value = YuStorageTableResizeCommit()
        let status = yu_storage_session_table_resize_update(
            handle,
            revision,
            pointerPosition,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeTableResizeCommit(value)
    }

    func tableResizeFinish(revision: UInt64) throws -> NativeTableResizeCommit {
        var value = YuStorageTableResizeCommit()
        let status = yu_storage_session_table_resize_finish(
            handle,
            revision,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeTableResizeCommit(value)
    }

    func tableResizeCancel(revision: UInt64) throws {
        let status = yu_storage_session_table_resize_cancel(handle, revision)
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
    }

    func blockLayout(
        revision: UInt64,
        blockIndex: UInt64,
        maxWidth: Float,
        lineHeight: Float,
        defaultAdvance: Float
    ) throws -> NativeBlockLayout {
        var value = YuStorageBlockLayout()
        let status = yu_storage_session_block_layout(
            handle,
            revision,
            blockIndex,
            maxWidth,
            lineHeight,
            defaultAdvance,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeBlockLayout(value)
    }

    func macosBlockLayout(
        revision: UInt64,
        blockIndex: UInt64,
        size: Float,
        maxWidth: Float
    ) throws -> NativeBlockLayout {
        var value = YuStorageBlockLayout()
        let status = yu_storage_session_macos_block_layout(
            handle,
            revision,
            blockIndex,
            size,
            maxWidth,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeBlockLayout(value)
    }

    func macosFontMetrics(
        revision: UInt64,
        size: Float,
        maxWidth: Float
    ) throws -> NativeMacosFontMetrics {
        var value = YuStorageMacosFontMetrics()
        let status = yu_storage_session_macos_font_metrics(
            handle,
            revision,
            size,
            maxWidth,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeMacosFontMetrics(value)
    }

    func macosBlockCaret(
        revision: UInt64,
        blockIndex: UInt64,
        sourceUTF16: UInt64,
        affinity: UInt8,
        size: Float,
        maxWidth: Float
    ) throws -> NativeBlockCaret {
        var value = YuStorageBlockCaret()
        let status = yu_storage_session_macos_block_caret(
            handle,
            revision,
            blockIndex,
            sourceUTF16,
            affinity,
            size,
            maxWidth,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeBlockCaret(value)
    }

    func setViewportConfig(
        revision: UInt64,
        maxWidth: Float,
        lineHeight: Float,
        defaultAdvance: Float,
        estimatedBlockHeight: Float,
        overscan: Float
    ) throws {
        let status = yu_storage_session_set_viewport_config(
            handle,
            revision,
            maxWidth,
            lineHeight,
            defaultAdvance,
            estimatedBlockHeight,
            overscan
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
    }

    func macosShapedViewportBlocks(
        revision: UInt64,
        size: Float,
        maxWidth: Float,
        scrollY: Float,
        viewportHeight: Float
    ) throws -> (NativeShapedViewportSnapshot, [NativeShapedViewportBlock]) {
        var snapshot = YuStorageShapedViewportSnapshot()
        var required = 0
        let sizeStatus = yu_storage_session_macos_shaped_viewport_blocks(
            handle,
            revision,
            size,
            maxWidth,
            scrollY,
            viewportHeight,
            &snapshot,
            nil,
            0,
            &required
        )
        guard sizeStatus == StorageStatus.ok else {
            throw BridgeError.operation(sizeStatus)
        }
        var values = Array(repeating: YuStorageShapedViewportBlock(), count: required)
        var written = required
        let fillStatus = values.withUnsafeMutableBufferPointer { buffer in
            yu_storage_session_macos_shaped_viewport_blocks(
                handle,
                revision,
                size,
                maxWidth,
                scrollY,
                viewportHeight,
                &snapshot,
                buffer.baseAddress,
                buffer.count,
                &written
            )
        }
        guard fillStatus == StorageStatus.ok, written == required else {
            throw BridgeError.operation(fillStatus)
        }
        return (
            NativeShapedViewportSnapshot(snapshot),
            values.map(NativeShapedViewportBlock.init)
        )
    }

    func macosVisualDecorations(
        revision: UInt64,
        compositionGeneration: UInt64,
        size: Float,
        maxWidth: Float,
        scrollY: Float,
        viewportHeight: Float
    ) throws -> (
        NativeVisualDecorationSnapshot,
        NativeVisualDecorationCaret,
        [NativeVisualDecorationRect]
    ) {
        var snapshot = YuStorageMacosVisualDecorationSnapshot()
        var caret = YuStorageMacosVisualDecorationCaret()
        var required = 0
        let sizeStatus = yu_storage_session_macos_visual_decorations(
            handle,
            revision,
            compositionGeneration,
            size,
            maxWidth,
            scrollY,
            viewportHeight,
            &snapshot,
            &caret,
            nil,
            0,
            &required
        )
        guard sizeStatus == StorageStatus.ok else {
            throw BridgeError.operation(sizeStatus)
        }
        guard snapshot.revision == revision,
              snapshot.composition_generation == compositionGeneration,
              snapshot.selection_count == UInt64(required),
              required >= 0 else {
            throw BridgeError.operation(StorageStatus.invalidViewport)
        }
        var values = Array(
            repeating: YuStorageMacosVisualDecorationRect(),
            count: required
        )
        var written = required
        let fillStatus = values.withUnsafeMutableBufferPointer { buffer in
            yu_storage_session_macos_visual_decorations(
                handle,
                revision,
                compositionGeneration,
                size,
                maxWidth,
                scrollY,
                viewportHeight,
                &snapshot,
                &caret,
                buffer.baseAddress,
                buffer.count,
                &written
            )
        }
        guard fillStatus == StorageStatus.ok else {
            throw BridgeError.operation(fillStatus)
        }
        guard written == required,
              caret.revision == revision || caret.present == 0,
              values.allSatisfy({
                  $0.revision == revision
                      && $0.width.isFinite && $0.width > 0.0
                      && $0.height.isFinite && $0.height > 0.0
                      && $0.x.isFinite && $0.y.isFinite
              }) else {
            throw BridgeError.operation(StorageStatus.invalidViewport)
        }
        return (
            NativeVisualDecorationSnapshot(snapshot),
            NativeVisualDecorationCaret(caret),
            values.map(NativeVisualDecorationRect.init)
        )
    }

    func macosVisualScene(
        revision: UInt64,
        size: Float,
        maxWidth: Float,
        scrollY: Float,
        viewportHeight: Float
    ) throws -> (NativeVisualSceneSnapshot, [NativeVisualScenePrimitive]) {
        var snapshot = YuStorageVisualSceneSnapshot()
        var required = 0
        let sizeStatus = yu_storage_session_macos_visual_scene(
            handle,
            revision,
            size,
            maxWidth,
            scrollY,
            viewportHeight,
            &snapshot,
            nil,
            0,
            &required
        )
        guard sizeStatus == StorageStatus.ok else {
            throw BridgeError.operation(sizeStatus)
        }
        precondition(snapshot.primitive_count == UInt64(required))
        var values = Array(repeating: YuStorageVisualScenePrimitive(), count: required)
        var written = required
        let fillStatus = values.withUnsafeMutableBufferPointer { buffer in
            yu_storage_session_macos_visual_scene(
                handle,
                revision,
                size,
                maxWidth,
                scrollY,
                viewportHeight,
                &snapshot,
                buffer.baseAddress,
                buffer.count,
                &written
            )
        }
        guard fillStatus == StorageStatus.ok, written == required else {
            throw BridgeError.operation(fillStatus)
        }
        return (
            NativeVisualSceneSnapshot(snapshot),
            values.map(NativeVisualScenePrimitive.init)
        )
    }

    func macosVisualImages(revision: UInt64) throws -> [NativeVisualImage] {
        var required = 0
        let sizeStatus = yu_storage_session_macos_visual_images(
            handle,
            revision,
            nil,
            0,
            &required
        )
        guard sizeStatus == StorageStatus.ok else {
            throw BridgeError.operation(sizeStatus)
        }
        var values = Array(repeating: YuStorageVisualImage(), count: required)
        var written = required
        let fillStatus = values.withUnsafeMutableBufferPointer { buffer in
            yu_storage_session_macos_visual_images(
                handle,
                revision,
                buffer.baseAddress,
                buffer.count,
                &written
            )
        }
        guard fillStatus == StorageStatus.ok, written == required else {
            throw BridgeError.operation(fillStatus)
        }
        return values.map(NativeVisualImage.init)
    }

    func macosVisualSceneGlyphs(
        revision: UInt64,
        size: Float,
        maxWidth: Float,
        scrollY: Float,
        viewportHeight: Float,
        surfaceGeneration: UInt64
    ) throws -> (NativeVisualSceneGlyphSnapshot, [NativeVisualSceneGlyph]) {
        var snapshot = YuStorageVisualSceneGlyphSnapshot()
        var required = 0
        let sizeStatus = yu_storage_session_macos_visual_scene_glyphs(
            handle,
            revision,
            size,
            maxWidth,
            scrollY,
            viewportHeight,
            surfaceGeneration,
            &snapshot,
            nil,
            0,
            &required
        )
        guard sizeStatus == StorageStatus.ok else {
            throw BridgeError.operation(sizeStatus)
        }
        precondition(snapshot.glyph_count == UInt64(required))
        var values = Array(
            repeating: YuStorageVisualSceneGlyph(),
            count: required
        )
        var written = required
        let fillStatus = values.withUnsafeMutableBufferPointer { buffer in
            yu_storage_session_macos_visual_scene_glyphs(
                handle,
                revision,
                size,
                maxWidth,
                scrollY,
                viewportHeight,
                surfaceGeneration,
                &snapshot,
                buffer.baseAddress,
                buffer.count,
                &written
            )
        }
        guard fillStatus == StorageStatus.ok, written == required else {
            throw BridgeError.operation(fillStatus)
        }
        return (
            NativeVisualSceneGlyphSnapshot(snapshot),
            values.map(NativeVisualSceneGlyph.init)
        )
    }

    func macosVisualRenderPlan(
        revision: UInt64,
        size: Float,
        maxWidth: Float,
        scrollY: Float,
        viewportHeight: Float
    ) throws -> (
        NativeVisualRenderPlanSnapshot,
        [NativeVisualRenderCommand],
        [NativeVisualRenderPage],
        [NativeVisualRenderDamage]
    ) {
        var snapshot = YuStorageVisualRenderPlanSnapshot()
        var commandRequired = 0
        var pageRequired = 0
        var damageRequired = 0
        let sizeStatus = yu_storage_session_macos_visual_render_plan(
            handle,
            revision,
            size,
            maxWidth,
            scrollY,
            viewportHeight,
            &snapshot,
            nil,
            0,
            nil,
            0,
            nil,
            0,
            &commandRequired,
            &pageRequired,
            &damageRequired
        )
        guard sizeStatus == StorageStatus.ok else {
            throw BridgeError.operation(sizeStatus)
        }
        precondition(snapshot.command_count == UInt64(commandRequired))
        precondition(snapshot.upload_count == UInt64(pageRequired))
        precondition(snapshot.damage_count == UInt64(damageRequired))
        var commandValues = Array(
            repeating: YuStorageVisualRenderCommand(),
            count: commandRequired
        )
        var pageValues = Array(
            repeating: YuStorageVisualRenderPage(),
            count: pageRequired
        )
        var damageValues = Array(
            repeating: YuStorageVisualRenderDamage(),
            count: damageRequired
        )
        var writtenCommands = commandRequired
        var writtenPages = pageRequired
        var writtenDamage = damageRequired
        let fillStatus = commandValues.withUnsafeMutableBufferPointer { commandBuffer in
            pageValues.withUnsafeMutableBufferPointer { pageBuffer in
                damageValues.withUnsafeMutableBufferPointer { damageBuffer in
                    yu_storage_session_macos_visual_render_plan(
                        handle,
                        revision,
                        size,
                        maxWidth,
                        scrollY,
                        viewportHeight,
                        &snapshot,
                        commandBuffer.baseAddress,
                        commandBuffer.count,
                        pageBuffer.baseAddress,
                        pageBuffer.count,
                        damageBuffer.baseAddress,
                        damageBuffer.count,
                        &writtenCommands,
                        &writtenPages,
                        &writtenDamage
                    )
                }
            }
        }
        guard fillStatus == StorageStatus.ok,
              writtenCommands == commandRequired,
              writtenPages == pageRequired,
              writtenDamage == damageRequired else {
            throw BridgeError.operation(fillStatus)
        }
        return (
            NativeVisualRenderPlanSnapshot(snapshot),
            commandValues.map(NativeVisualRenderCommand.init),
            pageValues.map(NativeVisualRenderPage.init),
            damageValues.map(NativeVisualRenderDamage.init)
        )
    }

    func macosRenderHostFrame(
        revision: UInt64,
        size: Float,
        maxWidth: Float,
        scrollY: Float,
        viewportHeight: Float,
        surfaceGeneration: UInt64
    ) throws -> NativeMacosRenderHostSnapshot {
        var value = YuStorageMacosRenderHostSnapshot()
        let status = yu_storage_session_macos_render_host_frame(
            handle,
            revision,
            size,
            maxWidth,
            scrollY,
            viewportHeight,
            surfaceGeneration,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeMacosRenderHostSnapshot(value)
    }

    func macosRenderHostSurfaceSubmit(
        revision: UInt64,
        size: Float,
        maxWidth: Float,
        scrollY: Float,
        viewportHeight: Float,
        surfaceWidth: Double,
        surfaceHeight: Double,
        scale: Double,
        view: UnsafeMutableRawPointer
    ) throws -> NativeMacosRenderHostSurfaceSnapshot {
        var value = YuStorageMacosRenderHostSurfaceSnapshot()
        let status = yu_storage_session_macos_render_host_surface_submit(
            handle,
            revision,
            size,
            maxWidth,
            scrollY,
            viewportHeight,
            surfaceWidth,
            surfaceHeight,
            scale,
            view,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeMacosRenderHostSurfaceSnapshot(value)
    }

    func macosRenderHostSurfaceDetach() throws {
        let status = yu_storage_session_macos_render_host_surface_detach(handle)
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
    }

    func macosShapedCaretScrollRequest(
        revision: UInt64,
        size: Float,
        maxWidth: Float,
        scrollY: Float,
        viewportHeight: Float,
        margin: Float
    ) throws -> NativeCaretScrollRequest {
        var value = YuStorageCaretScrollRequest()
        let status = yu_storage_session_macos_shaped_caret_scroll_request(
            handle,
            revision,
            size,
            maxWidth,
            scrollY,
            viewportHeight,
            margin,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeCaretScrollRequest(value)
    }

    var state: NativeStorageState {
        var value = YuStorageState()
        let status = yu_storage_session_state(
            handle,
            &value
        )
        precondition(status == StorageStatus.ok, "Rust storage state query failed: \(status)")
        return NativeStorageState(value)
    }

    var selection: NativeSelection {
        var value = YuStorageSelection()
        let status = yu_storage_session_selection(handle, &value)
        precondition(status == StorageStatus.ok, "Rust selection query failed: \(status)")
        return NativeSelection(value)
    }

    var selectionEndpoints: NativeSelectionEndpoints {
        var value = YuStorageSelectionEndpoints()
        let status = yu_storage_session_selection_endpoints(handle, &value)
        precondition(
            status == StorageStatus.ok,
            "Rust selection endpoint query failed: \(status)"
        )
        return NativeSelectionEndpoints(value)
    }

    var accessibilitySnapshot: NativeAccessibilitySnapshot {
        var value = YuStorageAccessibilitySnapshot()
        let status = yu_storage_session_accessibility_snapshot(handle, &value)
        precondition(status == StorageStatus.ok, "Rust accessibility snapshot failed: \(status)")
        return NativeAccessibilitySnapshot(value)
    }

    /// Accessibility callbacks can arrive while a document is being closed,
    /// reloaded, or replaced by an external edit. Those callbacks must not
    /// turn a transient Revision-bound error into a process abort.
    var accessibilitySnapshotIfAvailable: NativeAccessibilitySnapshot? {
        var value = YuStorageAccessibilitySnapshot()
        let status = yu_storage_session_accessibility_snapshot(handle, &value)
        guard status == StorageStatus.ok else { return nil }
        return NativeAccessibilitySnapshot(value)
    }

    /// Returns an owned semantic tree for the current Rust revision. The
    /// current host still exposes one NSTextView Accessibility element; this
    /// query establishes the source-backed child-element contract without
    /// retaining a second document model in AppKit.
    var accessibilitySemanticNodesIfAvailable: [NativeAccessibilitySemanticNode]? {
        let revision = state.revision
        var count = 0
        let countStatus = yu_storage_session_accessibility_semantic_node_count(
            handle,
            revision,
            &count
        )
        guard countStatus == StorageStatus.ok else { return nil }

        var values = Array(repeating: YuStorageAccessibilityNodeV2(), count: count)
        var written = 0
        let status = values.withUnsafeMutableBufferPointer { buffer in
            yu_storage_session_accessibility_semantic_nodes_v2(
                handle,
                revision,
                buffer.baseAddress,
                buffer.count,
                &written
            )
        }
        guard status == StorageStatus.ok, written == count else { return nil }
        return values.map(NativeAccessibilitySemanticNode.init)
    }

    func accessibilityLineRange(
        _ line: Int,
        revision: UInt64
    ) -> NativeAccessibilityRange? {
        guard line >= 0 else { return nil }
        var value = YuStorageAccessibilityRange()
        let status = yu_storage_session_accessibility_line_range(
            handle,
            revision,
            UInt64(line),
            &value
        )
        guard status == StorageStatus.ok else { return nil }
        return NativeAccessibilityRange(value)
    }

    func accessibilityLine(for offset: Int, revision: UInt64) -> Int? {
        guard offset >= 0 else { return nil }
        var line: UInt64 = 0
        let status = yu_storage_session_accessibility_line_for_position(
            handle,
            revision,
            UInt64(offset),
            &line
        )
        guard status == StorageStatus.ok else { return nil }
        return Int(line)
    }

    var composition: NativeComposition {
        var value = YuStorageCompositionState()
        let status = yu_storage_session_composition(handle, &value)
        precondition(status == StorageStatus.ok, "Rust composition query failed: \(status)")
        return NativeComposition(value)
    }

    var compositionIfAvailable: NativeComposition? {
        var value = YuStorageCompositionState()
        let status = yu_storage_session_composition(handle, &value)
        guard status == StorageStatus.ok else { return nil }
        return NativeComposition(value)
    }

    func setSelection(_ range: NSRange, affinity: UInt8 = 1) throws {
        let current = selection
        let status = yu_storage_session_set_selection(
            handle,
            current.revision,
            UInt64(range.location),
            UInt64(range.location + range.length),
            affinity
        )
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    func setSelectionEndpoints(
        anchorUTF16: UInt64,
        focusUTF16: UInt64,
        affinity: UInt8 = 1
    ) throws {
        let current = selection
        let status = yu_storage_session_set_selection_endpoints(
            handle,
            current.revision,
            anchorUTF16,
            focusUTF16,
            affinity
        )
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    func insertText(_ text: String) throws -> NativeCommandResult {
        let current = state.revision
        let bytes = Array(text.utf8)
        var result = YuStorageCommandResult()
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_storage_session_insert_text(
                handle,
                current,
                buffer.baseAddress,
                buffer.count,
                &result
            )
        }
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
        return NativeCommandResult(result)
    }

    func executeCommand(_ command: UInt8, block: UInt64 = 0) throws -> NativeCommandResult {
        var result = YuStorageCommandResult()
        let status = yu_storage_session_execute_command(handle, command, block, &result)
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
        return NativeCommandResult(result)
    }

    func executeShapedVerticalCommand(
        _ command: UInt8,
        size: Float,
        maxWidth: Float
    ) throws -> NativeCommandResult {
        var result = YuStorageCommandResult()
        let status = yu_storage_session_macos_move_vertical(
            handle,
            state.revision,
            command,
            size,
            maxWidth,
            &result
        )
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
        return NativeCommandResult(result)
    }

    func commandAvailable(_ command: UInt8, block: UInt64 = 0) -> Bool {
        var available: UInt8 = 0
        let status = yu_storage_session_command_available(handle, command, block, &available)
        precondition(status == StorageStatus.ok, "Rust command availability query failed: \(status)")
        return available != 0
    }

    func routeKey(kind: UInt8, value: UInt32 = 0, modifiers: UInt8 = 0) throws -> NativeCommandResult {
        var result = YuStorageCommandResult()
        let status = yu_storage_session_route_key(handle, kind, value, modifiers, &result)
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
        return NativeCommandResult(result)
    }

    func beginComposition(replacementRange: NSRange, preedit: String, selection: NSRange) throws {
        let current = state.revision
        let bytes = Array(preedit.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_storage_session_begin_composition(
                handle,
                current,
                UInt64(replacementRange.location),
                UInt64(replacementRange.location + replacementRange.length),
                buffer.baseAddress,
                buffer.count,
                UInt64(selection.location),
                UInt64(selection.location + selection.length)
            )
        }
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    func updateComposition(preedit: String, selection: NSRange) throws {
        let current = composition
        let bytes = Array(preedit.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_storage_session_update_composition(
                handle,
                current.revision,
                current.generation,
                buffer.baseAddress,
                buffer.count,
                UInt64(selection.location),
                UInt64(selection.location + selection.length)
            )
        }
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    func commitComposition(_ text: String) throws {
        let current = composition
        let bytes = Array(text.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            yu_storage_session_commit_composition(
                handle,
                current.revision,
                current.generation,
                buffer.baseAddress,
                buffer.count
            )
        }
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    func cancelComposition() throws {
        let current = composition
        guard current.active else { return }
        let status = yu_storage_session_cancel_composition(
            handle,
            current.revision,
            current.generation
        )
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    func copyComposition(_ value: NativeComposition) -> String {
        guard value.active else { return "" }
        return copyBytes { output, capacity, written in
            yu_storage_session_copy_composition(
                handle,
                value.revision,
                value.generation,
                output,
                capacity,
                written
            )
        }
    }

    func compositionProjection(revision: UInt64) throws -> NativeCompositionProjection {
        var value = YuStorageCompositionProjection()
        let status = yu_storage_session_composition_projection(handle, revision, &value)
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeCompositionProjection(value)
    }

    func copyCompositionProjection(
        revision: UInt64,
        generation: UInt64
    ) throws -> String {
        try copyBytesThrowing { output, capacity, written in
            yu_storage_session_copy_composition_projection(
                handle,
                revision,
                generation,
                output,
                capacity,
                written
            )
        }
    }

    func compositionCaret(
        revision: UInt64,
        generation: UInt64,
        sourceUTF16: UInt64,
        affinity: UInt8
    ) throws -> NativeCompositionCaret {
        var value = YuStorageCompositionCaret()
        let status = yu_storage_session_composition_caret(
            handle,
            revision,
            generation,
            sourceUTF16,
            affinity,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeCompositionCaret(value)
    }

    func macosCompositionShapedCaret(
        revision: UInt64,
        generation: UInt64,
        sourceUTF16: UInt64,
        affinity: UInt8,
        size: Float,
        maxWidth: Float
    ) throws -> NativeCompositionShapedCaret {
        var value = YuStorageCompositionShapedCaret()
        let status = yu_storage_session_macos_composition_shaped_caret(
            handle,
            revision,
            generation,
            sourceUTF16,
            affinity,
            size,
            maxWidth,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeCompositionShapedCaret(value)
    }

    func macosCompositionProjectionHitTest(
        revision: UInt64,
        generation: UInt64,
        point: CGPoint,
        size: Float,
        maxWidth: Float
    ) throws -> NativeCompositionProjectionHit {
        var value = YuStorageCompositionProjectionHit()
        let status = yu_storage_session_macos_composition_projection_hit_test(
            handle,
            revision,
            generation,
            Float(point.x),
            Float(point.y),
            size,
            maxWidth,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeCompositionProjectionHit(value)
    }

    func copySourceRange(_ range: NSRange, revision: UInt64) -> String {
        copyBytes { output, capacity, written in
            yu_storage_session_copy_source_range(
                handle,
                revision,
                UInt64(range.location),
                UInt64(range.location + range.length),
                output,
                capacity,
                written
            )
        }
    }

    func copySourceRangeIfAvailable(_ range: NSRange, revision: UInt64) -> String? {
        copyBytesIfAvailable { output, capacity, written in
            yu_storage_session_copy_source_range(
                handle,
                revision,
                UInt64(range.location),
                UInt64(range.location + range.length),
                output,
                capacity,
                written
            )
        }
    }

    func copySelection() -> String {
        let current = state.revision
        return copyBytes { output, capacity, written in
            yu_storage_session_copy_selection(
                handle,
                current,
                output,
                capacity,
                written
            )
        }
    }

    func copySelectionHTML(revision: UInt64) throws -> String {
        try copyBytesThrowing { output, capacity, written in
            yu_storage_session_copy_selection_html(
                handle,
                revision,
                output,
                capacity,
                written
            )
        }
    }

    /// Runs the stateless, allowlisted Rust HTML importer. A policy rejection
    /// is deliberately represented as `nil`, allowing the native paste path
    /// to use its plain-text fallback without exposing parser internals to
    /// Swift.
    func importHTML(_ html: String) throws -> String? {
        let input = Array(html.utf8)
        var required = 0
        let queryStatus = input.withUnsafeBufferPointer { buffer in
            yu_storage_import_html_fragment(
                buffer.baseAddress,
                buffer.count,
                nil,
                0,
                &required
            )
        }
        if queryStatus == StorageStatus.htmlImportRejected {
            return nil
        }
        guard queryStatus == StorageStatus.ok else {
            throw BridgeError.operation(queryStatus)
        }

        var bytes = Array(repeating: UInt8(0), count: required)
        var written = 0
        let copyStatus = bytes.withUnsafeMutableBufferPointer { buffer in
            input.withUnsafeBufferPointer { inputBuffer in
                yu_storage_import_html_fragment(
                    inputBuffer.baseAddress,
                    inputBuffer.count,
                    buffer.baseAddress,
                    buffer.count,
                    &written
                )
            }
        }
        if copyStatus == StorageStatus.htmlImportRejected {
            return nil
        }
        guard copyStatus == StorageStatus.ok,
              written >= 0,
              written <= bytes.count else {
            throw BridgeError.operation(copyStatus)
        }
        return String(decoding: bytes.prefix(written), as: UTF8.self)
    }

    func save() throws {
        var revision: UInt64 = 0
        var bytes: Int = 0
        var changed: UInt8 = 0
        let status = yu_storage_session_save(
            handle,
            &revision,
            &bytes,
            &changed
        )
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    func reload() throws {
        var revision: UInt64 = 0
        let status = yu_storage_session_reload(
            handle,
            &revision
        )
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    func requestClose() throws -> YuStorageCloseRequest {
        var request = YuStorageCloseRequest()
        let status = yu_storage_session_request_close(
            handle,
            &request
        )
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
        return request
    }

    func cancelClose() throws {
        let status = yu_storage_session_cancel_close(
            handle
        )
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    func saveAndClose() throws {
        let status = yu_storage_session_save_close(
            handle
        )
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    func discardAndClose() throws {
        let status = yu_storage_session_discard_close(
            handle
        )
        guard status == StorageStatus.ok else { throw BridgeError.operation(status) }
    }

    private func copyBytes(
        _ operation: (
            UnsafeMutablePointer<UInt8>?, Int, UnsafeMutablePointer<Int>?
        ) -> Int32
    ) -> String {
        var required = 0
        let sizeStatus = operation(nil, 0, &required)
        precondition(sizeStatus == StorageStatus.ok, "Rust storage length query failed: \(sizeStatus)")
        var bytes = Array(repeating: UInt8(0), count: required)
        let copyStatus = bytes.withUnsafeMutableBufferPointer { buffer in
            operation(buffer.baseAddress, buffer.count, &required)
        }
        precondition(copyStatus == StorageStatus.ok, "Rust storage copy failed: \(copyStatus)")
        return String(decoding: bytes, as: UTF8.self)
    }

    private func copyBytesThrowing(
        _ operation: (
            UnsafeMutablePointer<UInt8>?, Int, UnsafeMutablePointer<Int>?
        ) -> Int32
    ) throws -> String {
        var required = 0
        let sizeStatus = operation(nil, 0, &required)
        guard sizeStatus == StorageStatus.ok else {
            throw BridgeError.operation(sizeStatus)
        }
        var bytes = Array(repeating: UInt8(0), count: required)
        var written = required
        let copyStatus = bytes.withUnsafeMutableBufferPointer { buffer in
            operation(buffer.baseAddress, buffer.count, &written)
        }
        guard copyStatus == StorageStatus.ok, written >= 0, written <= bytes.count else {
            throw BridgeError.operation(copyStatus)
        }
        return String(decoding: bytes.prefix(written), as: UTF8.self)
    }

    private func copyBytesIfAvailable(
        _ operation: (
            UnsafeMutablePointer<UInt8>?, Int, UnsafeMutablePointer<Int>?
        ) -> Int32
    ) -> String? {
        var required = 0
        guard operation(nil, 0, &required) == StorageStatus.ok, required >= 0 else {
            return nil
        }
        var bytes = Array(repeating: UInt8(0), count: required)
        var written = required
        let status = bytes.withUnsafeMutableBufferPointer { buffer in
            operation(buffer.baseAddress, buffer.count, &written)
        }
        guard status == StorageStatus.ok, written >= 0, written <= bytes.count else {
            return nil
        }
        return String(decoding: bytes.prefix(written), as: UTF8.self)
    }
}

/// A disposable TextKit layout for the Rust-owned visual projection. It is
/// created only when the visual pointer adapter is enabled; it never owns a
/// source revision or Markdown semantics.
private final class ProjectionTextKitMirror {
    let revision: UInt64
    let textStorage: NSTextStorage
    let layoutManager: NSLayoutManager
    let textContainer: NSTextContainer

    init(text: String, revision: UInt64, width: CGFloat, font: NSFont) {
        self.revision = revision
        textStorage = NSTextStorage(string: text)
        layoutManager = NSLayoutManager()
        textContainer = NSTextContainer(
            size: NSSize(
                width: max(width, 1.0),
                height: CGFloat.greatestFiniteMagnitude
            )
        )
        textContainer.lineFragmentPadding = 0.0
        layoutManager.addTextContainer(textContainer)
        textStorage.addLayoutManager(layoutManager)
        if textStorage.length > 0 {
            textStorage.addAttribute(
                .font,
                value: font,
                range: NSRange(location: 0, length: textStorage.length)
            )
        }
    }

    var string: String { textStorage.string }

    var utf16Length: Int { textStorage.length }

    func visualUTF16(at point: NSPoint) -> Int {
        guard textStorage.length > 0 else { return 0 }
        var fraction: CGFloat = 0.0
        let glyph = layoutManager.glyphIndex(
            for: point,
            in: textContainer,
            fractionOfDistanceThroughGlyph: &fraction
        )
        let character = layoutManager.characterIndexForGlyph(at: glyph)
        return min(max(character, 0), textStorage.length)
    }

    func point(forVisualUTF16 offset: Int) -> NSPoint {
        guard textStorage.length > 0 else { return .zero }
        let clamped = min(max(offset, 0), textStorage.length)
        let glyph = layoutManager.glyphIndexForCharacter(at: min(clamped, textStorage.length - 1))
        var point = layoutManager.location(forGlyphAt: glyph)
        if clamped >= textStorage.length {
            let rect = layoutManager.boundingRect(
                forGlyphRange: NSRange(location: glyph, length: 1),
                in: textContainer
            )
            point.x = rect.maxX
        }
        return point
    }

    func caretRect(forVisualUTF16 offset: Int) -> NSRect {
        guard textStorage.length > 0 else {
            return NSRect(x: 0.0, y: 0.0, width: 1.0, height: 16.0)
        }
        let clamped = min(max(offset, 0), textStorage.length)
        let character = min(clamped, textStorage.length - 1)
        let glyph = layoutManager.glyphIndexForCharacter(at: character)
        let lineRect = layoutManager.lineFragmentRect(
            forGlyphAt: glyph,
            effectiveRange: nil
        )
        let point = point(forVisualUTF16: clamped)
        let x = clamped >= textStorage.length ? lineRect.maxX : point.x
        return NSRect(
            x: x,
            y: lineRect.minY,
            width: 1.0,
            height: max(lineRect.height, 1.0)
        )
    }

    /// Returns line-fragment rectangles for a projected visual selection.
    /// The rectangles are local to `textContainer`; callers add the native
    /// TextKit container origin when painting in the view coordinate space.
    /// This mirror is disposable, so the caller must validate its Revision
    /// before using the result.
    func selectionRects(forVisualRange range: NSRange) -> [NSRect] {
        guard range.location >= 0,
              range.length > 0,
              NSMaxRange(range) <= textStorage.length else {
            return []
        }
        let glyphRange = layoutManager.glyphRange(
            forCharacterRange: range,
            actualCharacterRange: nil
        )
        guard glyphRange.location != NSNotFound, glyphRange.length > 0 else {
            return []
        }
        var rects: [NSRect] = []
        layoutManager.enumerateEnclosingRects(
            forGlyphRange: glyphRange,
            withinSelectedGlyphRange: glyphRange,
            in: textContainer
        ) { rect, _ in
            guard rect.width.isFinite, rect.height.isFinite,
                  rect.width > 0.0, rect.height > 0.0 else {
                return
            }
            rects.append(rect)
        }
        return rects
    }
}

/// The native source mirror is deliberately a view cache, never a second
/// document model. Rust owns canonical source, revision, selection and
/// composition generation; this TextKit object only projects those values for
/// AppKit's NSTextInputClient callbacks. The visual pointer adapter asks
/// Rust's CoreText-shaped block layout for the visual boundary, then maps that
/// boundary back to canonical source ranges. The disposable TextKit visual
/// mirror remains a geometry and input/IME/accessibility host.
private final class DocumentTextView: NSTextView {
    private enum Command {
        static let deleteBackward: UInt8 = 1
        static let deleteForward: UInt8 = 2
        static let moveLeft: UInt8 = 3
        static let moveRight: UInt8 = 4
        static let insertNewline: UInt8 = 5
        static let indentList: UInt8 = 6
        static let outdentList: UInt8 = 7
        static let undo: UInt8 = 8
        static let redo: UInt8 = 9
        static let toggleTask: UInt8 = 10
        static let moveWordLeft: UInt8 = 11
        static let moveWordRight: UInt8 = 12
        static let moveUp: UInt8 = 13
        static let moveDown: UInt8 = 14
        static let moveUpExtend: UInt8 = 15
        static let moveDownExtend: UInt8 = 16
    }

    /// TextKit is always retained as the native input/IME/Accessibility host,
    /// but it has three deliberately separate paint roles. Keeping this as
    /// one role instead of independently toggling two booleans prevents a
    /// stale projected caret or selection from surviving a surface fallback.
    private enum PresentationRole: Equatable {
        case sourceFallback
        case projectedTextKitOverlay
        case rustSurface
    }

    private let bridge: StorageBridge
    private var canonicalSource: String
    private var canonicalRevision: UInt64
    private var semanticNodes: [NativeAccessibilitySemanticNode] = []
    private var semanticElements: [YuAccessibilitySemanticElement] = []
    private var headingRotorDelegate: YuAccessibilityRotorDelegate!
    private var linkRotorDelegate: YuAccessibilityRotorDelegate!
    private var nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
    private var synchronizingSelection = false
    private var visualMirror: ProjectionTextKitMirror?
    private var visualMirrorEnabled = false
    private var visualCompositionGeneration: UInt64?
    private var visualViewport: NativeVisualViewport?
    private var visualSelectionAnchor: Int?
    private var sourceSelectedTextAttributes: [NSAttributedString.Key: Any]?
    private var presentationRole: PresentationRole = .sourceFallback
    private var externalVisualDecorationsEnabled = false
    private var sourceGlyphsHidden = false
    var onDocumentChange: (() -> Void)?
    var onBeforeCommand: (() -> Void)?
    var onCaretChange: (() -> Void)?
    var onError: ((Error) -> Void)?
    var onTableResizeBegin: ((NSPoint) -> Bool)?
    var onTableResizeUpdate: ((NSPoint) -> Bool)?
    var onTableResizeFinish: (() -> Bool)?
    var onTableResizeCancel: (() -> Bool)?

    init(bridge: StorageBridge) {
        self.bridge = bridge
        canonicalSource = bridge.source
        canonicalRevision = bridge.state.revision
        // NSTextView's frame-only convenience initializer dynamically
        // dispatches to `init(frame:textContainer:)` on subclasses. Because
        // this view owns its bridge and is not storyboard-decoded, construct
        // the TextKit chain explicitly and call the designated initializer.
        // A nil text container can leave a source-backed mirror readable via
        // AX while providing no drawable storage/layout for the native view.
        let textStorage = NSTextStorage()
        let layoutManager = NSLayoutManager()
        let textContainer = NSTextContainer(
            size: NSSize(width: 900, height: CGFloat.greatestFiniteMagnitude)
        )
        layoutManager.addTextContainer(textContainer)
        textStorage.addLayoutManager(layoutManager)
        super.init(frame: .zero, textContainer: textContainer)
        isEditable = true
        isSelectable = true
        isRichText = false
        importsGraphics = false
        allowsUndo = false
        usesFindBar = true
        font = NSFont.systemFont(ofSize: 16)
        textColor = NSColor.textColor
        backgroundColor = NSColor.textBackgroundColor
        setAccessibilityElement(true)
        setAccessibilityRole(.textArea)
        setAccessibilityLabel("Yu Markdown 文档")
        setAccessibilityIdentifier("yu-document-text")
        headingRotorDelegate = YuAccessibilityRotorDelegate(owner: self, kind: .heading)
        linkRotorDelegate = YuAccessibilityRotorDelegate(owner: self, kind: .link)
        minSize = NSSize(width: 0, height: 0)
        maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        isVerticallyResizable = true
        isHorizontallyResizable = false
        autoresizingMask = [.width]
        semanticNodes = bridge.accessibilitySemanticNodesIfAvailable ?? []
        rebuildSemanticAccessibilityTree()
        synchronizeProjection()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    func refreshFromRust() {
        canonicalSource = bridge.source
        canonicalRevision = bridge.state.revision
        semanticNodes = bridge.accessibilitySemanticNodesIfAvailable ?? []
        nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
        synchronizeProjection()
        postAccessibilityRefresh()
    }

    /// Enables the visual pointer adapter. The visible NSTextView remains the
    /// canonical source mirror; this builds a disposable TextKit layout from
    /// Rust projected text for caret/selection/input presentation, while
    /// pointer boundaries come from Rust's CoreText-shaped endpoint.
    func setVisualMirrorEnabled(_ enabled: Bool) throws {
        visualMirrorEnabled = enabled
        visualSelectionAnchor = nil
        if enabled {
            if sourceSelectedTextAttributes == nil {
                sourceSelectedTextAttributes = selectedTextAttributes
            }
            applyVisualSelectionPaint(hidden: true)
            try refreshVisualMirror()
        } else {
            visualMirror = nil
            visualCompositionGeneration = nil
            visualViewport = nil
            if let sourceSelectedTextAttributes {
                selectedTextAttributes = sourceSelectedTextAttributes
            }
            self.sourceSelectedTextAttributes = nil
            setPresentationRole(.sourceFallback)
        }
        needsDisplay = true
    }

    func setVisualMirrorEnabledForSelfCheck(_ enabled: Bool) throws {
        try setVisualMirrorEnabled(enabled)
    }

    /// Moves projected selection/caret painting out of the TextKit view. The
    /// view remains the input/IME/Accessibility owner; disabling this flag is
    /// the safe fallback when the sibling decoration surface is unavailable.
    func setExternalVisualDecorationsEnabled(_ enabled: Bool) {
        setPresentationRole(
            enabled ? .projectedTextKitOverlay : .sourceFallback
        )
    }

    /// Hides only TextKit's source glyph painting after the Rust surface and
    /// its decoration geometry have both been accepted for the same
    /// Revision/generation. TextKit remains the live NSTextInputClient and
    /// Accessibility owner; clearing this flag restores the complete native
    /// fallback without rebuilding the document model.
    func setSourceGlyphsHidden(_ hidden: Bool) {
        setPresentationRole(hidden ? .rustSurface : .sourceFallback)
    }

    /// Applies the production paint contract as one atomic transition. The
    /// Rust surface coordinator owns the authoritative frame check; this view
    /// only changes which pixels TextKit is allowed to contribute.
    func useSourceFallbackPresentation() {
        setPresentationRole(.sourceFallback)
    }

    func useProjectedTextKitOverlayPresentation() {
        setPresentationRole(.projectedTextKitOverlay)
    }

    func useRustSurfacePresentation() {
        setPresentationRole(.rustSurface)
    }

    var sourceGlyphsHiddenForSelfCheck: Bool {
        sourceGlyphsHidden
    }

    var presentationRoleForSelfCheck: String {
        switch presentationRole {
        case .sourceFallback: return "sourceFallback"
        case .projectedTextKitOverlay: return "projectedTextKitOverlay"
        case .rustSurface: return "rustSurface"
        }
    }

    private func setPresentationRole(_ role: PresentationRole) {
        guard presentationRole != role
            || externalVisualDecorationsEnabled != (role != .sourceFallback)
            || sourceGlyphsHidden != (role == .rustSurface) else {
            return
        }
        presentationRole = role
        externalVisualDecorationsEnabled = role != .sourceFallback
        sourceGlyphsHidden = role == .rustSurface
        applyVisualSelectionPaint(hidden: role != .sourceFallback)
        needsDisplay = true
    }

    private func applyVisualSelectionPaint(hidden: Bool) {
        guard visualMirrorEnabled else { return }
        if hidden {
            var attributes = selectedTextAttributes
            attributes[.backgroundColor] = NSColor.clear
            attributes[.foregroundColor] = textColor ?? NSColor.textColor
            selectedTextAttributes = attributes
        } else if let sourceSelectedTextAttributes {
            selectedTextAttributes = sourceSelectedTextAttributes
        }
    }

    func refreshVisualMirrorForDisplay() throws {
        guard visualMirrorEnabled else { return }
        try refreshVisualMirror()
    }

    func visualMirrorPointForSelfCheck(visualUTF16: Int) -> NSPoint? {
        guard visualMirrorEnabled,
              let visualMirror,
              visualMirror.revision == bridge.state.revision,
              visualUTF16 >= 0,
              visualUTF16 <= visualMirror.utf16Length else {
            return nil
        }
        return visualMirror.point(forVisualUTF16: visualUTF16)
    }

    func setVisualViewportForSelfCheck(_ viewport: NativeVisualViewport) {
        guard visualMirrorEnabled,
              viewport.revision == bridge.state.revision else {
            visualViewport = nil
            return
        }
        visualViewport = viewport
    }

    func visualViewportPointForSelfCheck(visualUTF16: Int) -> NSPoint? {
        guard let documentPoint = visualMirrorPointForSelfCheck(visualUTF16: visualUTF16),
              let visualViewport,
              visualViewport.revision == bridge.state.revision else {
            return nil
        }
        return visualViewport.viewportPoint(forDocumentPoint: documentPoint)
    }

    func visualViewportRoundTripForSelfCheck(_ point: NSPoint) -> NSPoint? {
        guard let visualViewport,
              visualViewport.revision == bridge.state.revision else {
            return nil
        }
        return visualViewport.documentPoint(
            forViewportPoint: visualViewport.viewportPoint(forDocumentPoint: point)
        )
    }

    func visualCaretRectForDisplay() -> NSRect? {
        guard visualMirrorEnabled,
              let visualMirror,
              visualMirror.revision == bridge.state.revision else {
            return nil
        }
        let selection = bridge.selection
        let sourceUTF16 = UInt64(selection.range.location + selection.range.length)
        guard let visualUTF16 = visualUTF16ForSource(
            sourceUTF16,
            affinity: selection.affinity,
            mirror: visualMirror
        ) else {
            return nil
        }
        let rect = visualMirror.caretRect(forVisualUTF16: visualUTF16)
        return rect.offsetBy(dx: textContainerOrigin.x, dy: textContainerOrigin.y)
    }

    /// Maps the current Rust-owned source selection into visual TextKit
    /// rectangles. The source NSTextView selection background is cleared when
    /// this adapter is enabled, so these rectangles are the only selection
    /// highlight in the projected surface.
    func visualSelectionRectsForDisplay() -> [NSRect] {
        guard visualMirrorEnabled,
              !bridge.composition.active,
              let visualMirror,
              visualMirror.revision == bridge.state.revision else {
            return []
        }
        let selection = bridge.selection
        guard selection.revision == visualMirror.revision,
              selection.range.location >= 0,
              selection.range.length > 0,
              let projection = try? bridge.projectionSelection(
                  revision: selection.revision,
                  sourceRange: selection.range,
                  affinity: selection.affinity
              ),
              projection.revision == visualMirror.revision else {
            return []
        }
        return visualMirror.selectionRects(forVisualRange: projection.visualRange).map {
            $0.offsetBy(dx: textContainerOrigin.x, dy: textContainerOrigin.y)
        }
    }

    func visualSelectionRectsForSelfCheck() -> [NSRect] {
        visualSelectionRectsForDisplay()
    }

    func visualMirrorStringForSelfCheck() -> String? {
        guard visualMirrorEnabled,
              let visualMirror,
              visualMirror.revision == bridge.state.revision,
              visualCompositionGeneration == currentCompositionGeneration() else {
            return nil
        }
        return visualMirror.string
    }

    @discardableResult
    func applyVisualPointerSelectionForSelfCheck(
        at point: NSPoint,
        extending: Bool = false
    ) -> Bool {
        applyVisualPointerSelection(at: point, extending: extending)
    }

    func visualMarkedRangeForSelfCheck() -> NSRange? {
        guard visualMirrorEnabled,
              bridge.composition.active,
              let visualMirror,
              visualMirror.revision == bridge.state.revision,
              let generation = visualCompositionGeneration,
              let projection = try? bridge.compositionProjection(revision: bridge.state.revision),
              projection.generation == generation,
              projection.visualReplacementRange.location >= 0,
              NSMaxRange(projection.visualReplacementRange) <= visualMirror.utf16Length else {
            return nil
        }
        return projection.visualReplacementRange
    }

    @discardableResult
    func applyVisualSelectionForSelfCheck(_ visualRange: NSRange) -> Bool {
        applyVisualSelection(visualRange)
    }

    private func refreshVisualMirror() throws {
        guard visualMirrorEnabled else {
            visualMirror = nil
            visualCompositionGeneration = nil
            return
        }
        let revision = bridge.state.revision
        let projected: String
        let generation: UInt64?
        if bridge.composition.active {
            let metadata = try bridge.compositionProjection(revision: revision)
            let value = try bridge.copyCompositionProjection(
                revision: metadata.revision,
                generation: metadata.generation
            )
            // A marked-text callback may race a composition update. Do not
            // publish a mirror unless both metadata and copied text belong to
            // the same generation-bound snapshot.
            let current = try bridge.compositionProjection(revision: revision)
            guard current.generation == metadata.generation else {
                throw BridgeError.operation(16)
            }
            projected = value
            generation = metadata.generation
        } else {
            projected = try bridge.projectedSource(revision: revision)
            generation = nil
        }
        let width = max(bounds.width - 2.0 * textContainerOrigin.x, 1.0)
        visualMirror = ProjectionTextKitMirror(
            text: projected,
            revision: revision,
            width: width,
            font: font ?? NSFont.systemFont(ofSize: 16.0)
        )
        visualCompositionGeneration = generation
    }

    private func currentCompositionGeneration() -> UInt64? {
        guard bridge.composition.active else { return nil }
        return try? bridge.compositionProjection(revision: bridge.state.revision).generation
    }

    private func visualPoint(for event: NSEvent) -> NSPoint {
        let local = convert(event.locationInWindow, from: nil)
        return NSPoint(
            x: local.x - textContainerOrigin.x,
            y: local.y - textContainerOrigin.y
        )
    }

    /// Resolves a visual document point through the Rust CoreText-shaped
    /// block layout. TextKit remains the input/IME/accessibility host, but it
    /// must not guess glyph boundaries for production pointer selection.
    private func shapedVisualOffset(
        at point: NSPoint,
        mirror: ProjectionTextKitMirror
    ) -> Int? {
        guard point.x.isFinite,
              point.y.isFinite,
              let (size, width) = visualLayoutMetrics(),
              let hit = try? bridge.macosProjectionHitTest(
                  revision: mirror.revision,
                  point: CGPoint(x: point.x, y: point.y),
                  size: size,
                  maxWidth: width
              ),
              hit.revision == mirror.revision,
              hit.point.x.isFinite,
              hit.point.y.isFinite,
              let visualOffset = Int(exactly: hit.visualUTF16),
              visualOffset >= 0,
              visualOffset <= mirror.utf16Length else {
            return nil
        }
        return visualOffset
    }

    @discardableResult
    private func applyVisualPointerSelection(
        at point: NSPoint,
        extending: Bool
    ) -> Bool {
        guard visualMirrorEnabled,
              !bridge.composition.active,
              let visualMirror,
              visualMirror.revision == bridge.state.revision else {
            return false
        }
        guard let visualOffset = shapedVisualOffset(at: point, mirror: visualMirror) else {
            // The Rust endpoint is deliberately strict about Revision and
            // published viewport metrics. If geometry is stale, return to
            // AppKit's canonical source hit-test instead of selecting an
            // offset from a mismatched visual mirror.
            return false
        }
        if !extending || visualSelectionAnchor == nil {
            if extending {
                let endpoints = bridge.selectionEndpoints
                let sourceUTF16 = endpoints.anchorUTF16
                visualSelectionAnchor = visualUTF16ForSource(
                    sourceUTF16,
                    affinity: endpoints.affinity,
                    mirror: visualMirror
                ) ?? visualOffset
            } else {
                visualSelectionAnchor = visualOffset
            }
        }
        guard let anchor = visualSelectionAnchor else { return false }
        let visualRange = NSRange(
            location: min(anchor, visualOffset),
            length: abs(visualOffset - anchor)
        )
        return applyVisualSelection(
            visualRange,
            anchorIsVisualStart: anchor <= visualOffset
        )
    }

    private func visualUTF16ForSource(
        _ sourceUTF16: UInt64,
        affinity: UInt8,
        mirror: ProjectionTextKitMirror
    ) -> Int? {
        guard let caret = try? bridge.projectionCaret(
            revision: mirror.revision,
            sourceUTF16: sourceUTF16,
            affinity: affinity
        ),
              caret.revision == mirror.revision,
              let visualUTF16 = Int(exactly: caret.visualUTF16),
              visualUTF16 >= 0,
              visualUTF16 <= mirror.utf16Length else {
            return nil
        }
        return visualUTF16
    }

    @discardableResult
    private func applyVisualSelection(
        _ visualRange: NSRange,
        anchorIsVisualStart: Bool? = nil
    ) -> Bool {
        guard visualMirrorEnabled,
              !bridge.composition.active,
              let visualMirror,
              visualMirror.revision == bridge.state.revision,
              visualRange.location >= 0,
              visualRange.length >= 0,
              NSMaxRange(visualRange) <= visualMirror.utf16Length else {
            return false
        }
        do {
            let source = try bridge.projectionSourceSelection(
                revision: visualMirror.revision,
                visualRange: visualRange,
                affinity: 1
            )
            if let anchorIsVisualStart {
                let anchorUTF16 = anchorIsVisualStart
                    ? UInt64(source.sourceRange.location)
                    : UInt64(NSMaxRange(source.sourceRange))
                let focusUTF16 = anchorIsVisualStart
                    ? UInt64(NSMaxRange(source.sourceRange))
                    : UInt64(source.sourceRange.location)
                try bridge.setSelectionEndpoints(
                    anchorUTF16: anchorUTF16,
                    focusUTF16: focusUTF16,
                    affinity: source.affinity
                )
            } else {
                try bridge.setSelection(source.sourceRange, affinity: source.affinity)
            }
            canonicalRevision = bridge.state.revision
            synchronizingSelection = true
            super.setSelectedRange(source.sourceRange)
            synchronizingSelection = false
            postSelectionChanged()
            return true
        } catch {
            synchronizingSelection = false
            // A stale visual point is an expected race with source editing;
            // let the caller fall back to AppKit's source hit-test instead of
            // interrupting typing with a modal error.
            return false
        }
    }

    // These queries deliberately read a fresh Rust snapshot instead of
    // trusting TextKit's disposable projection. TextKit remains the source
    // mirror and AppKit fallback surface; source text, UTF-16 length,
    // selection and logical line ranges remain Revision-bound Rust data.
    override func accessibilityValue() -> String? {
        bridge.copySourceIfAvailable ?? canonicalSource
    }

    override func accessibilityNumberOfCharacters() -> Int {
        bridge.accessibilitySnapshotIfAvailable?.numberOfCharacters
            ?? (canonicalSource as NSString).length
    }

    override func accessibilitySelectedText() -> String? {
        guard let snapshot = bridge.accessibilitySnapshotIfAvailable else { return nil }
        return bridge.copySourceRangeIfAvailable(
            snapshot.selectedRange,
            revision: snapshot.revision
        )
    }

    override func accessibilitySelectedTextRange() -> NSRange {
        bridge.accessibilitySnapshotIfAvailable?.selectedRange ?? selectedRange()
    }

    override func setAccessibilitySelectedTextRange(_ range: NSRange) {
        do {
            if bridge.compositionIfAvailable?.active == true {
                try bridge.cancelComposition()
                nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
            }
            guard let snapshot = bridge.accessibilitySnapshotIfAvailable else { return }
            guard let valid = accessibilitySourceRange(range, snapshot: snapshot) else { return }
            try bridge.setSelection(valid, affinity: snapshot.affinity)
            synchronizeProjection()
            postSelectionChanged()
        } catch {
            onError?(error)
        }
    }

    override func accessibilitySelectedTextRanges() -> [NSValue]? {
        [NSValue(range: accessibilitySelectedTextRange())]
    }

    /// AppKit asks for these children through Objective-C Accessibility
    /// dispatch. The document TextKit element remains the editable source
    /// surface; semantic children are stable owned nodes for VoiceOver
    /// navigation and never become a second text model.
    @objc var accessibilityChildren: [Any]? { semanticElements }

    @objc var accessibilityChildrenInNavigationOrder: [Any]? {
        semanticElements
    }

    /// Heading and Link rotors make the semantic tree discoverable without
    /// requiring VoiceOver to walk every paragraph and inline child. The
    /// delegates remain owned by the document view because AppKit retains
    /// rotor delegates weakly.
    @objc var accessibilityCustomRotors: [NSAccessibilityCustomRotor]? {
        [
            NSAccessibilityCustomRotor(
                rotorType: .heading,
                itemSearchDelegate: headingRotorDelegate
            ),
            NSAccessibilityCustomRotor(
                rotorType: .link,
                itemSearchDelegate: linkRotorDelegate
            ),
        ]
    }

    func accessibilityRotorResult(
        for kind: SemanticAccessibilityKind,
        parameters: NSAccessibilityCustomRotor.SearchParameters
    ) -> NSAccessibilityCustomRotor.ItemResult? {
        let candidates = flattenSemanticElements(semanticElements).filter { element in
            guard let elementKind = SemanticAccessibilityKind(rawValue: element.node.kind) else {
                return false
            }
            switch kind {
            case .heading:
                return elementKind == .heading
            case .link:
                return elementKind == .link
                    || elementKind == .autolink
                    || elementKind == .referenceLink
            default:
                return false
            }
        }.filter { element in
            let filter = parameters.filterString
            guard !filter.isEmpty else { return true }
            return element.accessibilityLabel?.localizedCaseInsensitiveContains(filter) == true
        }
        guard !candidates.isEmpty else { return nil }

        let current = parameters.currentItem?.targetElement as AnyObject?
        let currentIndex = current.flatMap { currentObject in
            candidates.firstIndex { $0 === currentObject }
        }
        let index: Int?
        switch parameters.searchDirection {
        case .next:
            index = currentIndex.map { $0 + 1 } ?? 0
        case .previous:
            index = currentIndex.map { $0 - 1 } ?? (candidates.count - 1)
        @unknown default:
            index = nil
        }
        guard let index, candidates.indices.contains(index) else { return nil }

        let element = candidates[index]
        let result = NSAccessibilityCustomRotor.ItemResult(targetElement: element)
        result.targetRange = element.node.sourceRange
        result.customLabel = element.accessibilityLabel
        return result
    }

    func toggleTaskAccessibilityNode(_ node: NativeAccessibilitySemanticNode) -> Bool {
        guard SemanticAccessibilityKind(rawValue: node.kind) == .taskListItem,
              node.revision == bridge.state.revision,
              let block = node.actionBlock,
              !bridge.composition.active,
              bridge.commandAvailable(Command.toggleTask, block: block) else {
            return false
        }
        do {
            let result = try bridge.executeCommand(Command.toggleTask, block: block)
            guard result.changed else { return false }
            apply(result)
            synchronizeProjection()
            postAccessibilityRefresh()
            onDocumentChange?()
            return true
        } catch {
            onError?(error)
            return false
        }
    }

    override func accessibilityString(for range: NSRange) -> String? {
        guard let snapshot = bridge.accessibilitySnapshotIfAvailable else { return nil }
        guard let valid = accessibilitySourceRange(range, snapshot: snapshot) else { return nil }
        return bridge.copySourceRangeIfAvailable(valid, revision: snapshot.revision)
    }

    override func accessibilityAttributedString(for range: NSRange) -> NSAttributedString? {
        guard let text = accessibilityString(for: range) else { return nil }
        return NSAttributedString(string: text)
    }

    override func accessibilityRange(forLine line: Int) -> NSRange {
        guard let snapshot = bridge.accessibilitySnapshotIfAvailable else {
            return NSRange(location: NSNotFound, length: 0)
        }
        guard line >= 0, line < snapshot.lineCount else {
            return NSRange(location: NSNotFound, length: 0)
        }
        return bridge.accessibilityLineRange(line, revision: snapshot.revision)?.range
            ?? NSRange(location: NSNotFound, length: 0)
    }

    override func accessibilityLine(for index: Int) -> Int {
        guard let snapshot = bridge.accessibilitySnapshotIfAvailable else { return NSNotFound }
        guard index >= 0, index <= snapshot.numberOfCharacters else { return NSNotFound }
        return bridge.accessibilityLine(for: index, revision: snapshot.revision) ?? NSNotFound
    }

    override func accessibilityInsertionPointLineNumber() -> Int {
        accessibilityLine(for: accessibilitySelectedTextRange().location)
    }

    override func accessibilityRange(for index: Int) -> NSRange {
        guard let snapshot = bridge.accessibilitySnapshotIfAvailable else {
            return NSRange(location: NSNotFound, length: 0)
        }
        guard index >= 0, index <= snapshot.numberOfCharacters else {
            return NSRange(location: NSNotFound, length: 0)
        }
        guard index < snapshot.numberOfCharacters else {
            return NSRange(location: index, length: 0)
        }
        let source = (bridge.copySourceIfAvailable ?? canonicalSource) as NSString
        return source.rangeOfComposedCharacterSequence(at: index)
    }

    override func setSelectedRange(_ charRange: NSRange) {
        let range = clampedRange(charRange, length: (string as NSString).length)
        let shouldSync = !synchronizingSelection
        synchronizingSelection = true
        super.setSelectedRange(range)
        synchronizingSelection = false
        guard shouldSync else { return }
        syncNativeSelectionToRust(range)
    }

    /// AppKit uses this plural entry point for mouse clicks, drag selection,
    /// and some TextKit accessibility paths. The Rust editor currently owns a
    /// single selection, so the first native range is the canonical one; the
    /// important part is that mouse selection cannot leave Rust at an older
    /// fixed line while the disposable TextKit mirror moves elsewhere.
    override func setSelectedRanges(
        _ ranges: [NSValue],
        affinity: NSSelectionAffinity,
        stillSelecting: Bool
    ) {
        let shouldSync = !synchronizingSelection
        synchronizingSelection = true
        super.setSelectedRanges(
            ranges,
            affinity: affinity,
            stillSelecting: stillSelecting
        )
        synchronizingSelection = false
        guard shouldSync else { return }
        let range = clampedRange(selectedRange(), length: (string as NSString).length)
        syncNativeSelectionToRust(range)
    }

    /// The visual pointer adapter resolves the click/drag point in the
    /// projected stream, then lets Rust convert that visual range into the
    /// canonical source selection. If the projected mirror is stale or
    /// unavailable, AppKit's source hit-test remains the safe fallback.
    override func mouseDown(with event: NSEvent) {
        if event.buttonNumber == 0,
           onTableResizeBegin?(visualPoint(for: event)) == true {
            visualSelectionAnchor = nil
            return
        }
        if visualMirrorEnabled,
           applyVisualPointerSelection(
               at: visualPoint(for: event),
               extending: event.modifierFlags.contains(.shift)
           ) {
            return
        }
        visualSelectionAnchor = nil
        super.mouseDown(with: event)
    }

    override func mouseDragged(with event: NSEvent) {
        if onTableResizeUpdate?(visualPoint(for: event)) == true {
            return
        }
        if visualSelectionAnchor != nil,
           applyVisualPointerSelection(
               at: visualPoint(for: event),
               extending: true
           ) {
            return
        }
        super.mouseDragged(with: event)
    }

    override func mouseUp(with event: NSEvent) {
        if event.buttonNumber == 0,
           onTableResizeFinish?() == true {
            return
        }
        if visualSelectionAnchor != nil {
            visualSelectionAnchor = nil
            return
        }
        super.mouseUp(with: event)
    }

    /// Source delimiters may be hidden by the Rust projection. Draw the
    /// insertion point at the revision-bound visual caret while retaining the
    /// TextKit view as the input/IME/Accessibility owner.
    override func drawInsertionPoint(
        in rect: NSRect,
        color: NSColor,
        turnedOn: Bool
    ) {
        if sourceGlyphsHidden || externalVisualDecorationsEnabled {
            return
        }
        guard let visualRect = visualCaretRectForDisplay() else {
            super.drawInsertionPoint(in: rect, color: color, turnedOn: turnedOn)
            return
        }
        super.drawInsertionPoint(in: visualRect, color: color, turnedOn: turnedOn)
    }

    override func draw(_ rect: NSRect) {
        guard !sourceGlyphsHidden else { return }
        super.draw(rect)
        guard visualMirrorEnabled, !externalVisualDecorationsEnabled else { return }
        let selectionRects = visualSelectionRectsForDisplay()
        guard !selectionRects.isEmpty else { return }
        NSColor.selectedTextBackgroundColor.withAlphaComponent(0.38).setFill()
        for selectionRect in selectionRects {
            let clipped = selectionRect.intersection(rect)
            guard !clipped.isNull, !clipped.isEmpty else { continue }
            clipped.fill()
        }
    }

    private func syncNativeSelectionToRust(_ range: NSRange) {
        guard !bridge.composition.active else { return }
        do {
            try bridge.setSelection(range)
            canonicalRevision = bridge.state.revision
            postSelectionChanged()
        } catch {
            onError?(error)
        }
    }

    override func insertText(_ insertString: Any, replacementRange: NSRange) {
        let text = stringValue(insertString)
        guard !text.isEmpty || bridge.composition.active else { return }
        do {
            if bridge.composition.active {
                try bridge.commitComposition(text)
                canonicalSource = bridge.source
                canonicalRevision = bridge.state.revision
            } else {
                let target = replacementRange.location == NSNotFound
                    ? bridge.selection.range
                    : replacementRange
                if target != bridge.selection.range {
                    try bridge.setSelection(target)
                }
                let result = try bridge.insertText(text)
                apply(result)
            }
            nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
            synchronizeProjection()
            postAccessibilityRefresh()
            onDocumentChange?()
        } catch {
            onError?(error)
        }
    }

    override func copy(_ sender: Any?) {
        do {
            try finishCompositionForClipboard()
            let revision = bridge.state.revision
            let text = bridge.copySelection()
            guard !text.isEmpty else { return }
            let html = try bridge.copySelectionHTML(revision: revision)
            try publishSourceToPasteboard(text, html: html)
        } catch {
            onError?(error)
        }
    }

    override func cut(_ sender: Any?) {
        do {
            try finishCompositionForClipboard()
            let revision = bridge.state.revision
            let selected = bridge.copySelection()
            guard !selected.isEmpty else { return }
            let html = try bridge.copySelectionHTML(revision: revision)
            try publishSourceToPasteboard(selected, html: html)
            guard bridge.commandAvailable(Command.deleteBackward) else { return }
            apply(try bridge.executeCommand(Command.deleteBackward))
            synchronizeProjection()
            postAccessibilityRefresh()
            onDocumentChange?()
        } catch {
            onError?(error)
        }
    }

    override func paste(_ sender: Any?) {
        do {
            try finishCompositionForClipboard()
            guard let text = try sourceFromPasteboard(), !text.isEmpty else {
                return
            }
            apply(try bridge.insertText(text))
            synchronizeProjection()
            postAccessibilityRefresh()
            onDocumentChange?()
        } catch {
            onError?(error)
        }
    }

    var hasSourceOnPasteboard: Bool {
        let pasteboard = NSPasteboard.general
        return pasteboard.string(forType: .yuMarkdown) != nil
            || pasteboard.string(forType: .string) != nil
            || pasteboard.string(forType: .yuHTML) != nil
    }

    /// Headless self-check hook for the native pasteboard adapter. Production
    /// paste still uses `NSPasteboard.general`; this overload only lets the
    /// command-line check exercise the same priority/fallback logic on a
    /// private pasteboard.
    func sourceFromPasteboardForSelfCheck(_ pasteboard: NSPasteboard) throws -> String? {
        try sourceFromPasteboard(pasteboard)
    }

    override func selectAll(_ sender: Any?) {
        do {
            try finishCompositionForClipboard()
            let length = canonicalSource.utf16.count
            try bridge.setSelection(NSRange(location: 0, length: length))
            synchronizeProjection()
            postSelectionChanged()
        } catch {
            onError?(error)
        }
    }

    override func setMarkedText(
        _ markedText: Any,
        selectedRange: NSRange,
        replacementRange: NSRange
    ) {
        let text = stringValue(markedText)
        do {
            let active = bridge.composition
            let target = active.active
                ? active.replacementRange
                : (replacementRange.location == NSNotFound ? bridge.selection.range : replacementRange)
            if active.active {
                try bridge.updateComposition(preedit: text, selection: selectedRange)
            } else {
                try bridge.beginComposition(
                    replacementRange: target,
                    preedit: text,
                    selection: selectedRange
                )
            }
            let current = bridge.composition
            nativeMarkedRange = NSRange(
                location: current.replacementRange.location,
                length: text.utf16.count
            )
            synchronizeProjection()
            postAccessibilityRefresh()
            onDocumentChange?()
        } catch {
            onError?(error)
        }
    }

    override func unmarkText() {
        // AppKit's unmark is a presentation transition. The Rust overlay must
        // stay alive because some input sources deliver insertText afterwards.
        synchronizeProjection()
        nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
    }

    override func hasMarkedText() -> Bool {
        bridge.composition.active && nativeMarkedRange.location != NSNotFound
    }

    override func markedRange() -> NSRange {
        if let visualRange = visualMarkedRangeForSelfCheck() {
            return visualRange
        }
        return nativeMarkedRange
    }

    override func attributedSubstring(
        forProposedRange proposedRange: NSRange,
        actualRange: NSRangePointer?
    ) -> NSAttributedString? {
        if let visualString = visualMirrorStringForSelfCheck() {
            let length = (visualString as NSString).length
            let range = clampedRange(proposedRange, length: length)
            actualRange?.pointee = range
            guard range.location != NSNotFound else { return nil }
            return NSAttributedString(
                string: (visualString as NSString).substring(with: range)
            )
        }
        let range = clampedRange(proposedRange, length: (string as NSString).length)
        actualRange?.pointee = range
        guard range.location != NSNotFound else { return nil }
        return textStorage?.attributedSubstring(from: range)
    }

    override func validAttributesForMarkedText() -> [NSAttributedString.Key] {
        [.font, .foregroundColor, .underlineStyle]
    }

    override func doCommand(by selector: Selector) {
        let name = NSStringFromSelector(selector)
        if name == "cancel:" || name == "cancelOperation:" {
            if onTableResizeCancel?() == true {
                return
            }
            do {
                if bridge.composition.active {
                    try bridge.cancelComposition()
                    nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
                    synchronizeProjection()
                    postAccessibilityRefresh()
                    onDocumentChange?()
                }
            } catch { onError?(error) }
            return
        }
        if name == "undo:" {
            routeCommand(Command.undo)
            return
        }
        if name == "redo:" {
            routeCommand(Command.redo)
            return
        }

        let command: UInt8?
        switch selector {
        case #selector(NSResponder.deleteBackward(_:)): command = Command.deleteBackward
        case #selector(NSResponder.deleteForward(_:)): command = Command.deleteForward
        case #selector(NSResponder.moveLeft(_:)): command = Command.moveLeft
        case #selector(NSResponder.moveRight(_:)): command = Command.moveRight
        case #selector(NSResponder.moveWordLeft(_:)): command = Command.moveWordLeft
        case #selector(NSResponder.moveWordRight(_:)): command = Command.moveWordRight
        case #selector(NSResponder.moveUp(_:)): command = Command.moveUp
        case #selector(NSResponder.moveDown(_:)): command = Command.moveDown
        case #selector(NSResponder.moveUpAndModifySelection(_:)): command = Command.moveUpExtend
        case #selector(NSResponder.moveDownAndModifySelection(_:)): command = Command.moveDownExtend
        case #selector(NSResponder.insertNewline(_:)): command = Command.insertNewline
        case #selector(NSResponder.insertTab(_:)): command = Command.indentList
        case #selector(NSResponder.insertBacktab(_:)): command = Command.outdentList
        default: command = nil
        }
        guard let command else { return }
        routeCommand(command)
    }

    /// AppKit normally turns Command-Z into an `undo:` selector, but that
    /// path is not guaranteed when TextKit's own undo manager is disabled.
    /// Catch the native key equivalent as a second, explicit bridge to the
    /// Rust history. The menu actions below call the same method, so there
    /// is still only one undo/redo implementation.
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        let modifiers = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        let isCommandZ = modifiers.contains(.command)
            && !modifiers.contains(.option)
            && !modifiers.contains(.control)
            && event.charactersIgnoringModifiers?.lowercased() == "z"
        guard isCommandZ else {
            return super.performKeyEquivalent(with: event)
        }
        let command = modifiers.contains(.shift) ? Command.redo : Command.undo
        return routeCommand(command)
    }

    @discardableResult
    private func routeCommand(_ command: UInt8) -> Bool {
        guard !bridge.composition.active else { return false }
        let isVertical = command == Command.moveUp
            || command == Command.moveDown
            || command == Command.moveUpExtend
            || command == Command.moveDownExtend
        if isVertical {
            // The production coordinator publishes the current CoreText
            // metrics synchronously before Rust resolves a vertical target.
            // Headless/self-check views leave this callback unset and retain
            // the ordinary metrics command path.
            onBeforeCommand?()
        }
        guard bridge.commandAvailable(command) else { return false }
        do {
            let result: NativeCommandResult
            if isVertical, let (size, width) = visualLayoutMetrics() {
                do {
                    result = try bridge.executeShapedVerticalCommand(
                        command,
                        size: size,
                        maxWidth: width
                    )
                } catch BridgeError.operation(let status)
                    where status == StorageStatus.invalidViewport {
                    // A key can arrive before the first surface/layout
                    // publication. Preserve editing availability by falling
                    // back to Rust's ordinary metrics command; the next
                    // command will retry the shaped path after preparation.
                    result = try bridge.executeCommand(command)
                }
            } else {
                result = try bridge.executeCommand(command)
            }
            apply(result)
            synchronizeProjection()
            postAccessibilityRefresh()
            onDocumentChange?()
            return true
        } catch {
            onError?(error)
            return false
        }
    }

    private func visualLayoutMetrics() -> (Float, Float)? {
        guard let font, bounds.width.isFinite, bounds.width > 0.0 else { return nil }
        let width = max(bounds.width - 2.0 * textContainerOrigin.x, 1.0)
        guard width.isFinite, width > 0.0 else { return nil }
        return (Float(max(font.pointSize, 1.0)), Float(width))
    }

    /// Menu actions use these explicit entry points instead of NSTextView's
    /// undo manager. Rust remains the sole owner of history and revision.
    func performUndo() {
        _ = routeCommand(Command.undo)
    }

    func performRedo() {
        _ = routeCommand(Command.redo)
    }

    func canUndo() -> Bool {
        !bridge.composition.active && bridge.commandAvailable(Command.undo)
    }

    func canRedo() -> Bool {
        !bridge.composition.active && bridge.commandAvailable(Command.redo)
    }

    private func finishCompositionForClipboard() throws {
        guard bridge.composition.active else { return }
        try bridge.cancelComposition()
        nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
        synchronizeProjection()
    }

    private func publishSourceToPasteboard(_ source: String, html: String) throws {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        guard pasteboard.setString(source, forType: .string),
              pasteboard.setString(source, forType: .yuMarkdown),
              pasteboard.setString(html, forType: .yuHTML) else {
            throw BridgeError.clipboard
        }
    }

    private func sourceFromPasteboard(_ pasteboard: NSPasteboard = .general) throws -> String? {
        if let markdown = pasteboard.string(forType: .yuMarkdown) {
            return markdown
        }
        if let plain = pasteboard.string(forType: .string) {
            return plain
        }
        guard let html = pasteboard.string(forType: .yuHTML) else {
            return nil
        }
        return try bridge.importHTML(html)
    }

    private func apply(_ result: NativeCommandResult) {
        switch result.sourceSync {
        case 0:
            break
        case 1:
            guard let oldRange = result.oldSourceRange, let newRange = result.newSourceRange else {
                canonicalSource = bridge.source
                break
            }
            let inserted = bridge.copySourceRange(newRange, revision: result.revision)
            let mutable = NSMutableString(string: canonicalSource)
            if oldRange.location >= 0, NSMaxRange(oldRange) <= mutable.length {
                mutable.replaceCharacters(in: oldRange, with: inserted)
                canonicalSource = mutable as String
            } else {
                canonicalSource = bridge.source
            }
        default:
            canonicalSource = bridge.source
        }
        canonicalRevision = result.revision
        _ = result.changed
    }

    private func synchronizeProjection() {
        let active = bridge.composition
        if let visualViewport, visualViewport.revision != bridge.state.revision {
            self.visualViewport = nil
        }
        let projected: String
        let selection: NSRange
        if active.active {
            let preedit = bridge.copyComposition(active)
            let mutable = NSMutableString(string: canonicalSource)
            if active.replacementRange.location >= 0,
               NSMaxRange(active.replacementRange) <= mutable.length {
                mutable.replaceCharacters(in: active.replacementRange, with: preedit)
            }
            projected = mutable as String
            nativeMarkedRange = NSRange(
                location: active.replacementRange.location,
                length: preedit.utf16.count
            )
            selection = NSRange(
                location: active.replacementRange.location + active.selection.location,
                length: active.selection.length
            )
        } else {
            projected = canonicalSource
            nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
            let rustSelection = bridge.selection
            selection = rustSelection.range
        }
        synchronizingSelection = true
        string = projected
        selectedRange = clampedRange(selection, length: (string as NSString).length)
        synchronizingSelection = false
        needsDisplay = true
        if visualMirrorEnabled {
            do {
                try refreshVisualMirror()
            } catch {
                visualMirror = nil
                visualCompositionGeneration = nil
                visualViewport = nil
                // Projection is an enhancement to the source mirror. If a
                // refresh races an edit, keep TextKit interactive and let the
                // next layout/source revision rebuild the disposable mirror.
            }
        }
    }

    private func stringValue(_ value: Any) -> String {
        if let attributed = value as? NSAttributedString { return attributed.string }
        if let string = value as? String { return string }
        return "\(value)"
    }

    private func clampedRange(_ range: NSRange, length: Int) -> NSRange {
        guard range.location != NSNotFound else { return NSRange(location: NSNotFound, length: 0) }
        let location = min(max(range.location, 0), length)
        let maximum = max(0, length - location)
        return NSRange(location: location, length: min(max(range.length, 0), maximum))
    }

    private func accessibilitySourceRange(
        _ range: NSRange,
        snapshot: NativeAccessibilitySnapshot
    ) -> NSRange? {
        guard range.location != NSNotFound,
              range.location >= 0,
              range.length >= 0,
              NSMaxRange(range) <= snapshot.numberOfCharacters else {
            return nil
        }
        let source = (bridge.copySourceIfAvailable ?? canonicalSource) as NSString
        if range.location < source.length,
           source.rangeOfComposedCharacterSequence(at: range.location).location != range.location {
            return nil
        }
        if range.length > 0 {
            let last = NSMaxRange(range) - 1
            if NSMaxRange(source.rangeOfComposedCharacterSequence(at: last)) != NSMaxRange(range) {
                return nil
            }
        }
        return range
    }

    func accessibilityFrameForSemanticRange(_ range: NSRange) -> NSRect {
        guard canonicalRevision == bridge.state.revision,
              !bridge.composition.active,
              range.location >= 0,
              range.length >= 0,
              NSMaxRange(range) <= (string as NSString).length,
              let container = textContainer,
              let layoutManager,
              let window else {
            return .zero
        }
        let glyphRange = layoutManager.glyphRange(
            forCharacterRange: range,
            actualCharacterRange: nil
        )
        guard glyphRange.location != NSNotFound else { return .zero }
        let local = layoutManager
            .boundingRect(forGlyphRange: glyphRange, in: container)
            .offsetBy(dx: textContainerOrigin.x, dy: textContainerOrigin.y)
        return window.convertToScreen(convert(local, to: nil))
    }

    private func rebuildSemanticAccessibilityTree() {
        postDestroyedSemanticElements()
        let nodes = semanticNodes.filter {
            SemanticAccessibilityKind(rawValue: $0.kind) != .document
        }
        var elementsByIndex: [UInt32: YuAccessibilitySemanticElement] = [:]
        for node in nodes {
            let element = YuAccessibilitySemanticElement(
                node: node,
                bridge: bridge,
                parent: self
            )
            element.frameOwner = self
            elementsByIndex[node.index] = element
        }

        var topLevel: [YuAccessibilitySemanticElement] = []
        for node in nodes {
            guard let element = elementsByIndex[node.index] else { continue }
            guard node.parent != UInt32.max,
                  node.parent != 0,
                  let parent = elementsByIndex[node.parent] else {
                element.parentObject = self
                topLevel.append(element)
                continue
            }
            element.parentObject = parent
            parent.semanticChildren.append(element)
        }
        semanticElements = topLevel
    }

    private func postDestroyedSemanticElements() {
        guard !semanticElements.isEmpty else { return }
        for element in flattenSemanticElements(semanticElements) {
            NSAccessibility.post(element: element, notification: .uiElementDestroyed)
        }
    }

    private func flattenSemanticElements(
        _ elements: [YuAccessibilitySemanticElement]
    ) -> [YuAccessibilitySemanticElement] {
        var result: [YuAccessibilitySemanticElement] = []
        result.reserveCapacity(elements.count)
        for element in elements {
            result.append(element)
            let children = element.semanticChildren.compactMap {
                $0 as? YuAccessibilitySemanticElement
            }
            result.append(contentsOf: flattenSemanticElements(children))
        }
        return result
    }

    func postAccessibilityRefresh() {
        semanticNodes = bridge.accessibilitySemanticNodesIfAvailable ?? []
        rebuildSemanticAccessibilityTree()
        NSAccessibility.post(element: self, notification: .valueChanged)
        NSAccessibility.post(element: self, notification: .layoutChanged)
        postSelectionChanged()
    }

    private func postSelectionChanged() {
        NSAccessibility.post(element: self, notification: .selectedTextChanged)
        needsDisplay = true
        onCaretChange?()
    }
}

private enum BridgeError: LocalizedError {
    case open(Int32)
    case operation(Int32)
    case clipboard
    case watcher(Int32)

    var errorDescription: String? {
        switch self {
        case .open(let status): return "无法打开 Markdown 文件（Rust status \(status)）"
        case .operation(let status): return "文档操作失败（Rust status \(status)）"
        case .clipboard: return "无法访问 macOS 剪贴板"
        case .watcher(let status):
            let reason = String(cString: strerror(status))
            return "无法监听文档所在目录（\(reason)）"
        }
    }
}

/// A real product-window surface host. Rust still owns the native layer and
/// GPU resources; this view only reports AppKit window/geometry lifecycle and
/// never becomes the canonical document model.
private final class MacosSurfaceHostView: NSView {
    var onWindowStateChange: ((Bool) -> Void)?
    var onGeometryChange: (() -> Void)?
    private(set) var nativeContentVisible = false

    /// The Rust surface is a visual projection only. Keep AppKit input,
    /// selection and scrolling on the TextKit mirror underneath it.
    func setNativeContentVisible(_ visible: Bool) {
        if nativeContentVisible == visible {
            // NSView defaults to visible before the first successful submit;
            // keep the fallback state deterministic even on the first bind.
            isHidden = !visible
            return
        }
        nativeContentVisible = visible
        isHidden = !visible
    }

    /// Let hit testing continue to the source TextKit view below the overlay.
    override func hitTest(_ point: NSPoint) -> NSView? {
        nil
    }

    override func viewWillMove(toWindow newWindow: NSWindow?) {
        if newWindow == nil {
            setNativeContentVisible(false)
            onWindowStateChange?(false)
        }
        super.viewWillMove(toWindow: newWindow)
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil {
            setNativeContentVisible(false)
        }
        onWindowStateChange?(window != nil)
        onGeometryChange?()
    }

    override func layout() {
        super.layout()
        onGeometryChange?()
    }
}

/// Draws visual selection/caret decorations above the Rust-shaped glyph
/// surface. Geometry is supplied by Rust; TextKit remains the fallback
/// painter while the surface publication is stale or unavailable.
///
/// This view is intentionally transparent to AppKit hit-testing. The source
/// TextKit view remains the owner of keyboard, IME and Accessibility events;
/// this layer only owns the pixels for transient visual decorations. Geometry
/// is supplied by the revision-bound visual mirror and is discarded whenever
/// the mirror is stale or the surface detaches.
private final class MacosVisualDecorationView: NSView {
    private(set) var revision: UInt64?
    private(set) var selectionRects: [NSRect] = []
    private(set) var caretRect: NSRect?
    private(set) var compositionActive = false

    var hasValidFrame: Bool {
        revision != nil && caretRect != nil
    }

    override func hitTest(_ point: NSPoint) -> NSView? {
        nil
    }

    func update(
        revision: UInt64,
        selectionRects: [NSRect],
        caretRect: NSRect?,
        compositionActive: Bool
    ) {
        self.revision = revision
        self.selectionRects = selectionRects.filter(Self.isDrawable)
        self.caretRect = caretRect.flatMap { Self.isDrawable($0) ? $0 : nil }
        self.compositionActive = compositionActive
        needsDisplay = true
    }

    func clear() {
        revision = nil
        selectionRects.removeAll(keepingCapacity: true)
        caretRect = nil
        compositionActive = false
        needsDisplay = true
    }

    override func draw(_ dirtyRect: NSRect) {
        guard revision != nil else { return }

        NSColor.selectedTextBackgroundColor.withAlphaComponent(0.38).setFill()
        for rect in selectionRects {
            let clipped = rect.intersection(dirtyRect)
            guard !clipped.isNull, !clipped.isEmpty else { continue }
            clipped.fill()
        }

        guard let caretRect else { return }
        let clippedCaret = caretRect.intersection(dirtyRect)
        guard !clippedCaret.isNull, !clippedCaret.isEmpty else { return }
        let color = compositionActive
            ? NSColor.controlAccentColor
            : NSColor.textColor
        color.setFill()
        clippedCaret.fill()
    }

    private static func isDrawable(_ rect: NSRect) -> Bool {
        rect.minX.isFinite && rect.minY.isFinite
            && rect.width.isFinite && rect.height.isFinite
            && rect.width > 0.0 && rect.height > 0.0
    }
}

/// The source TextKit mirror is still the native input/IME/Accessibility
/// owner, but its glyph painting can be gated once a matching Rust surface
/// frame and decoration frame are both available.  Keep the gate explicit:
/// a pair of booleans cannot explain why a frame was rejected, and that makes
/// a transient edit/scroll/IME race look like an unexplained visual glitch.
private struct VisualRenderFrameIdentity: Equatable, CustomStringConvertible {
    let revision: UInt64
    let compositionGeneration: UInt64
    let surfaceGeneration: UInt64
    let frameSerial: UInt64

    var description: String {
        "revision=\(revision), composition=\(compositionGeneration), "
            + "surface=\(surfaceGeneration), frame=\(frameSerial)"
    }
}

private struct VisualRenderPublicationIdentity: Equatable {
    let frame: VisualRenderFrameIdentity
    let submitted: Bool

    init(frame: VisualRenderFrameIdentity, submitted: Bool) {
        self.frame = frame
        self.submitted = submitted
    }

    init(_ snapshot: NativeMacosRenderHostSurfaceSnapshot) {
        frame = VisualRenderFrameIdentity(
            revision: snapshot.revision,
            compositionGeneration: snapshot.compositionGeneration,
            surfaceGeneration: snapshot.surfaceGeneration,
            frameSerial: snapshot.frameSerial
        )
        submitted = snapshot.submitted
    }
}

/// Returns the one publication identity that is allowed to hide TextKit's
/// source glyphs. Keeping this predicate pure makes the composition-generation
/// race testable without constructing an AppKit view or a Metal drawable.
private func acceptedVisualRenderFrame(
    revision: UInt64,
    compositionGeneration: UInt64,
    publicationCurrent: Bool,
    publication: VisualRenderPublicationIdentity?,
    decorationRevision: UInt64?,
    decorationHasValidFrame: Bool,
    rustDecorationFrameAccepted: Bool
) -> VisualRenderFrameIdentity? {
    guard publicationCurrent,
          decorationRevision == revision,
          decorationHasValidFrame,
          rustDecorationFrameAccepted,
          let publication,
          publication.submitted,
          publication.frame.revision == revision,
          publication.frame.compositionGeneration == compositionGeneration else {
        return nil
    }
    return publication.frame
}

private enum VisualRenderFallbackReason: String, Equatable, CustomStringConvertible {
    case disabled
    case detached
    case missingGeometry
    case waitingForSurface
    case staleRevision
    case staleComposition
    case decorationUnavailable
    case invalidFrame
    case compositionActive
    case surfaceSubmitFailed
    case visualMirrorUnavailable

    var description: String { rawValue }
}

private enum VisualRenderState: Equatable, CustomStringConvertible {
    case fallback(VisualRenderFallbackReason)
    case active(VisualRenderFrameIdentity)

    var isActive: Bool {
        if case .active = self { return true }
        return false
    }

    var fallbackReason: VisualRenderFallbackReason? {
        guard case .fallback(let reason) = self else { return nil }
        return reason
    }

    var description: String {
        switch self {
        case .fallback(let reason):
            return "fallback(\(reason))"
        case .active(let frame):
            return "active(\(frame))"
        }
    }
}

/// Small deterministic state machine for the source-glyph gate.  This is
/// deliberately independent of AppKit so its transition rules can be tested
/// without creating a window or a Metal drawable.
private struct VisualRenderStateMachine {
    private(set) var state: VisualRenderState = .fallback(.disabled)
    private(set) var transitionSerial: UInt64 = 0

    mutating func enterFallback(_ reason: VisualRenderFallbackReason) {
        let next = VisualRenderState.fallback(reason)
        guard state != next else { return }
        state = next
        transitionSerial &+= 1
    }

    mutating func activate(_ frame: VisualRenderFrameIdentity) {
        let next = VisualRenderState.active(frame)
        guard state != next else { return }
        state = next
        transitionSerial &+= 1
    }

    var diagnosticDescription: String {
        "state=\(state); transitions=\(transitionSerial)"
    }
}

private struct TableResizePointerSession: Equatable {
    let revision: UInt64
    let kind: UInt8
}

/// Keeps the native pointer route explicit and headless-testable. Rust owns
/// the geometry preview; this state only answers whether subsequent mouse
/// events belong to the active divider gesture and when a revision invalidates
/// that route.
private struct TableResizePointerState {
    private(set) var session: TableResizePointerSession?

    var isActive: Bool { session != nil }

    @discardableResult
    mutating func begin(revision: UInt64, kind: UInt8) -> Bool {
        guard kind == YU_STORAGE_TABLE_RESIZE_COLUMN
            || kind == YU_STORAGE_TABLE_RESIZE_ROW else {
            return false
        }
        session = TableResizePointerSession(revision: revision, kind: kind)
        return true
    }

    func acceptsUpdate(revision: UInt64) -> Bool {
        session?.revision == revision
    }

    @discardableResult
    mutating func finish(revision: UInt64) -> Bool {
        guard acceptsUpdate(revision: revision) else { return false }
        session = nil
        return true
    }

    @discardableResult
    mutating func cancel(revision: UInt64) -> Bool {
        guard acceptsUpdate(revision: revision) else { return false }
        session = nil
        return true
    }

    mutating func reset() {
        session = nil
    }
}

/// Coordinates a persistent Rust surface with one product `NSView`.
///
/// The coordinator is deliberately an AppKit lifecycle adapter, not a second
/// editor or renderer. It converts window/layout/scroll/revision changes into
/// the already validated synchronous FFI submit protocol and detaches before
/// the view leaves its window.
private final class MacosSurfaceHostCoordinator {
    private struct MetricsKey: Equatable {
        let revision: UInt64
        let size: Double
        let maxWidth: Double
    }

    private struct Metrics {
        let key: MetricsKey
        let lineHeight: Float
        let defaultAdvance: Float
    }

    private struct SubmitKey: Equatable {
        let revision: UInt64
        let compositionGeneration: UInt64
        let size: Double
        let maxWidth: Double
        let scrollY: Double
        let viewportHeight: Double
        let surfaceWidth: Double
        let surfaceHeight: Double
        let scale: Double
    }

    private let bridge: StorageBridge
    private weak var surfaceView: MacosSurfaceHostView?
    private weak var scrollView: NSScrollView?
    private var fontSize: CGFloat
    private var contentWidth: CGFloat?
    private var metrics: Metrics?
    private var lastSubmitKey: SubmitKey?
    private(set) var lastSnapshot: NativeMacosRenderHostSurfaceSnapshot?
    private var submitScheduled = false
    private var scheduleToken: UInt64 = 0
    private var tableResizePointerState = TableResizePointerState()

    var onError: ((Error) -> Void)?
    var onSurfaceStateChange: (() -> Void)?
    private(set) var isAttached = false

    init(bridge: StorageBridge, fontSize: CGFloat = 16.0) {
        self.bridge = bridge
        self.fontSize = max(fontSize, 1.0)
    }

    func bind(
        surfaceView: MacosSurfaceHostView,
        scrollView: NSScrollView,
        fontSize: CGFloat
    ) {
        if isAttached {
            detach()
        }
        scheduleToken &+= 1
        self.surfaceView = surfaceView
        self.scrollView = scrollView
        self.fontSize = max(fontSize, 1.0)
        contentWidth = nil
        metrics = nil
        lastSubmitKey = nil
        lastSnapshot = nil
        tableResizePointerState.reset()
        isAttached = false
        surfaceView.setNativeContentVisible(false)
    }

    func setFontSize(_ fontSize: CGFloat) {
        let next = max(fontSize, 1.0)
        guard abs(self.fontSize - next) > 0.001 else { return }
        self.fontSize = next
        metrics = nil
        lastSubmitKey = nil
        scheduleSubmit()
    }

    /// Publishes the text content width used by the source TextKit mirror.
    /// The transparent surface may span the full clip viewport, while native
    /// text insets reduce the actual wrapping width. Keeping this value in
    /// the coordinator makes metrics, shaped hit-testing and render layout
    /// share one width contract.
    func setContentWidth(_ width: CGFloat) {
        let next = max(width, 1.0)
        if let current = contentWidth, abs(current - next) <= 0.5 {
            return
        }
        contentWidth = next
        metrics = nil
        lastSubmitKey = nil
        scheduleSubmit()
    }

    /// Returns true only when the Metal surface has accepted the current Rust
    /// revision, transient composition generation and complete submit
    /// geometry. A visible old frame is deliberately insufficient: source
    /// glyphs must remain available until the replacement publication has
    /// reached the native surface after an edit, scroll, resize or scale
    /// change.
    func hasCurrentPublication(revision: UInt64, compositionGeneration: UInt64) -> Bool {
        guard isAttached,
              surfaceView?.nativeContentVisible == true,
              let snapshot = lastSnapshot,
              let currentKey = currentSubmitKey else {
            return false
        }
        return snapshot.submitted
            && snapshot.revision == revision
            && snapshot.compositionGeneration == compositionGeneration
            && currentKey.revision == revision
            && currentKey.compositionGeneration == compositionGeneration
            && lastSubmitKey == currentKey
    }

    private var currentSubmitKey: SubmitKey? {
        guard let surfaceView,
              let scrollView,
              let window = surfaceView.window,
              surfaceView.bounds.width > 0.0,
              surfaceView.bounds.height > 0.0 else {
            return nil
        }
        let viewportBounds = scrollView.contentView.bounds
        return SubmitKey(
            revision: bridge.state.revision,
            compositionGeneration: bridge.composition.generation,
            size: Double(max(fontSize, 1.0)),
            maxWidth: Double(layoutWidth(for: surfaceView)),
            scrollY: Double(max(viewportBounds.origin.y, 0.0)),
            viewportHeight: Double(max(viewportBounds.height, 1.0)),
            surfaceWidth: Double(max(surfaceView.bounds.width, 1.0)),
            surfaceHeight: Double(max(surfaceView.bounds.height, 1.0)),
            scale: Double(max(window.backingScaleFactor, 1.0))
        )
    }

    private func layoutWidth(for surfaceView: MacosSurfaceHostView) -> CGFloat {
        max(contentWidth ?? surfaceView.bounds.width, 1.0)
    }

    func scheduleSubmit() {
        guard !submitScheduled else { return }
        submitScheduled = true
        let token = scheduleToken
        DispatchQueue.main.async { [weak self] in
            guard let self, self.scheduleToken == token else { return }
            self.submitScheduled = false
            do {
                _ = try self.submitNow()
            } catch {
                self.clearTableResizeState()
                self.surfaceView?.setNativeContentVisible(false)
                self.onSurfaceStateChange?()
                self.onError?(error)
            }
        }
    }

    /// Publishes CoreText metrics before a vertical editor command enters the
    /// Rust command path. This keeps the command's shaped line wrapping and
    /// the subsequent caret reveal on one Revision/width contract even when a
    /// key arrives before the next asynchronous surface submit.
    func prepareForEditorCommand() {
        guard let surfaceView,
              surfaceView.bounds.width > 0.0 else {
            return
        }
        let revision = bridge.state.revision
        let size = max(fontSize, 1.0)
        let maxWidth = layoutWidth(for: surfaceView)
        do {
            _ = try ensureMetrics(
                revision: revision,
                size: size,
                maxWidth: maxWidth
            )
        } catch {
            onError?(error)
        }
    }

    /// Attempts to start a CoreText-shaped table divider gesture at a
    /// document-space point. The hit-test is intentionally non-mutating and
    /// is followed by a matching Rust begin call so row gestures can choose
    /// their y-axis pointer coordinate before the preview is created.
    @discardableResult
    func beginTableResize(at point: NSPoint) -> Bool {
        guard !bridge.composition.active,
              point.x.isFinite,
              point.y.isFinite,
              let geometry = visualDecorationGeometry() else {
            return false
        }
        let revision = bridge.state.revision
        let tolerance = Float(max(CGFloat(6.0), fontSize * 0.4))
        do {
            let hit = try bridge.macosTableResizeHitTestAtDocumentPoint(
                revision: revision,
                size: geometry.size,
                maxWidth: geometry.maxWidth,
                point: CGPoint(x: point.x, y: point.y),
                tolerance: tolerance
            )
            // The retained render host currently consumes only column
            // overrides; keep row dividers on the normal selection path until
            // variable-row layout is published end-to-end.
            guard hit.kind == UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN) else {
                return false
            }
            let pointerPosition = Float(point.x)
            let begun = try bridge.macosTableResizeBeginAtDocumentPoint(
                revision: revision,
                size: geometry.size,
                maxWidth: geometry.maxWidth,
                point: CGPoint(x: point.x, y: point.y),
                tolerance: tolerance,
                pointerPosition: pointerPosition
            )
            guard begun.revision == revision,
                  tableResizePointerState.begin(
                      revision: revision,
                      kind: begun.kind
                  ) else {
                return false
            }
            lastSubmitKey = nil
            scheduleSubmit()
            return true
        } catch BridgeError.operation(let status)
            where status == StorageStatus.invalidSelection {
            // A click between table dividers belongs to normal selection.
            return false
        } catch {
            // A stale/temporarily unavailable shaped layout must never make
            // the source TextKit editor stop accepting pointer input.
            tableResizePointerState.reset()
            onError?(error)
            return false
        }
    }

    /// Forwards one drag sample to Rust and invalidates the retained surface
    /// even though the ordinary geometry submit key has not changed.
    @discardableResult
    func updateTableResize(at point: NSPoint) -> Bool {
        let revision = bridge.state.revision
        guard let session = tableResizePointerState.session,
              session.revision == revision,
              point.x.isFinite,
              point.y.isFinite else {
            return false
        }
        let pointerPosition = session.kind == YU_STORAGE_TABLE_RESIZE_COLUMN
            ? Float(point.x)
            : Float(point.y)
        do {
            _ = try bridge.tableResizeUpdate(
                revision: revision,
                pointerPosition: pointerPosition
            )
            lastSubmitKey = nil
            scheduleSubmit()
            return true
        } catch BridgeError.operation(let status)
            where status == StorageStatus.staleRevision
                || status == StorageStatus.tableResizeNotActive {
            tableResizePointerState.reset()
            lastSubmitKey = nil
            return true
        } catch {
            tableResizePointerState.reset()
            onError?(error)
            return true
        }
    }

    /// Finishes the Rust gesture. The final preview remains attached to the
    /// current session until the next source revision or explicit reset, so
    /// the retained frame shows the committed divider immediately.
    @discardableResult
    func finishTableResize() -> Bool {
        let revision = bridge.state.revision
        guard tableResizePointerState.acceptsUpdate(revision: revision) else {
            return false
        }
        do {
            _ = try bridge.tableResizeFinish(revision: revision)
            _ = tableResizePointerState.finish(revision: revision)
            lastSubmitKey = nil
            scheduleSubmit()
        } catch {
            tableResizePointerState.reset()
            onError?(error)
        }
        return true
    }

    /// Cancels only an active pointer gesture. Document edits use
    /// `resetTableResizeAfterDocumentChange()` to also clear a finished
    /// preview that Rust intentionally keeps for the current frame.
    @discardableResult
    func cancelTableResize() -> Bool {
        let revision = bridge.state.revision
        guard tableResizePointerState.acceptsUpdate(revision: revision) else {
            return false
        }
        do {
            try bridge.tableResizeCancel(revision: revision)
            _ = tableResizePointerState.cancel(revision: revision)
            lastSubmitKey = nil
            scheduleSubmit()
        } catch {
            tableResizePointerState.reset()
            onError?(error)
        }
        return true
    }

    /// Clears both an active gesture and the finished preview when the
    /// canonical source revision changes. The FFI call is harmless when no
    /// gesture exists and is revision-bound to avoid clearing a newer edit.
    func resetTableResizeAfterDocumentChange() {
        clearTableResizeState()
    }

    var tableResizeActiveForSelfCheck: Bool {
        tableResizePointerState.isActive
    }

    /// Returns the same revision-bound CoreText metrics and viewport inputs
    /// used by the Rust render host. Visual decorations use this accessor so
    /// their geometry query cannot silently drift to TextKit's independent
    /// wrapping width or line height.
    func visualDecorationGeometry() -> (
        size: Float,
        maxWidth: Float,
        scrollY: Float,
        viewportHeight: Float,
        lineHeight: Float
    )? {
        guard let surfaceView,
              let scrollView,
              surfaceView.bounds.width > 0.0,
              surfaceView.bounds.height > 0.0 else {
            return nil
        }
        let revision = bridge.state.revision
        let size = max(fontSize, 1.0)
        let maxWidth = layoutWidth(for: surfaceView)
        let viewportBounds = scrollView.contentView.bounds
        let viewportHeight = max(viewportBounds.height, 1.0)
        let scrollY = max(viewportBounds.origin.y, 0.0)
        do {
            let metrics = try ensureMetrics(
                revision: revision,
                size: size,
                maxWidth: maxWidth
            )
            return (
                Float(size),
                Float(maxWidth),
                Float(scrollY),
                Float(viewportHeight),
                metrics.lineHeight
            )
        } catch {
            return nil
        }
    }

    /// Reveals the current Rust-owned caret using the same Revision-bound
    /// CoreText/shaped viewport contract as surface submission. This is a
    /// scroll-only adapter: it never asks TextKit for a caret and never lets
    /// AppKit invent document geometry. A stale or unavailable request is
    /// ignored so a transient surface race cannot interrupt editing.
    func revealCaretIfNeeded() {
        guard let surfaceView,
              let scrollView,
              surfaceView.bounds.width > 0.0,
              surfaceView.bounds.height > 0.0 else {
            return
        }
        let revision = bridge.state.revision
        let size = max(fontSize, 1.0)
        let maxWidth = layoutWidth(for: surfaceView)
        let viewportBounds = scrollView.contentView.bounds
        let viewportHeight = max(viewportBounds.height, 1.0)
        let currentScrollY = max(viewportBounds.origin.y, 0.0)
        do {
            let metrics = try ensureMetrics(
                revision: revision,
                size: size,
                maxWidth: maxWidth
            )
            let request = try bridge.macosShapedCaretScrollRequest(
                revision: revision,
                size: Float(size),
                maxWidth: Float(maxWidth),
                scrollY: Float(currentScrollY),
                viewportHeight: Float(viewportHeight),
                margin: max(metrics.lineHeight, 4.0)
            )
            guard request.revision == revision,
                  request.currentScrollY.isFinite,
                  request.targetScrollY.isFinite,
                  request.targetScrollY >= 0.0,
                  request.needsScroll else {
                return
            }
            let nativeMaxScrollY = max(
                (scrollView.documentView?.bounds.height ?? 0.0) - viewportHeight,
                0.0
            )
            let targetScrollY = min(max(request.targetScrollY, 0.0), nativeMaxScrollY)
            guard abs(targetScrollY - currentScrollY) > 0.5 else { return }
            var origin = viewportBounds.origin
            origin.y = targetScrollY
            scrollView.contentView.setBoundsOrigin(origin)
            scrollView.reflectScrolledClipView(scrollView.contentView)
            lastSubmitKey = nil
            scheduleSubmit()
        } catch {
            // Caret reveal is an enhancement to the source TextKit view. The
            // source mirror remains interactive if shaped metrics are stale,
            // unavailable, or temporarily racing a document edit.
        }
    }

    @discardableResult
    func submitNow() throws -> NativeMacosRenderHostSurfaceSnapshot? {
        guard let surfaceView,
              let scrollView,
              let window = surfaceView.window,
              surfaceView.bounds.width > 0.0,
              surfaceView.bounds.height > 0.0 else {
            return nil
        }

        let revision = bridge.state.revision
        let size = max(fontSize, 1.0)
        let surfaceWidth = max(surfaceView.bounds.width, 1.0)
        let surfaceHeight = max(surfaceView.bounds.height, 1.0)
        let viewportBounds = scrollView.contentView.bounds
        let viewportHeight = max(viewportBounds.height, 1.0)
        let scrollY = max(viewportBounds.origin.y, 0.0)
        let maxWidth = layoutWidth(for: surfaceView)
        let scale = max(window.backingScaleFactor, 1.0)
        // Composition updates do not advance the canonical Revision. Include
        // the Rust-owned generation in the submit key so every marked-text
        // update/cancel publishes a fresh transient glyph scene instead of
        // reusing the previous frame by geometry alone.
        let compositionGeneration = bridge.composition.generation
        let key = SubmitKey(
            revision: revision,
            compositionGeneration: compositionGeneration,
            size: Double(size),
            maxWidth: Double(maxWidth),
            scrollY: Double(scrollY),
            viewportHeight: Double(viewportHeight),
            surfaceWidth: Double(surfaceWidth),
            surfaceHeight: Double(surfaceHeight),
            scale: Double(scale)
        )
        if isAttached, key == lastSubmitKey {
            surfaceView.setNativeContentVisible(true)
            return lastSnapshot
        }

        _ = try ensureMetrics(
            revision: revision,
            size: size,
            maxWidth: maxWidth
        )
        let rawView = Unmanaged.passUnretained(surfaceView).toOpaque()
        let snapshot = try bridge.macosRenderHostSurfaceSubmit(
            revision: revision,
            size: Float(size),
            maxWidth: Float(maxWidth),
            scrollY: Float(scrollY),
            viewportHeight: Float(viewportHeight),
            surfaceWidth: Double(surfaceWidth),
            surfaceHeight: Double(surfaceHeight),
            scale: Double(scale),
            view: rawView
        )
        isAttached = true
        lastSubmitKey = key
        lastSnapshot = snapshot
        surfaceView.setNativeContentVisible(true)
        onSurfaceStateChange?()
        return snapshot
    }

    func detach() {
        scheduleToken &+= 1
        submitScheduled = false
        clearTableResizeState()
        if isAttached {
            do {
                try bridge.macosRenderHostSurfaceDetach()
            } catch {
                onError?(error)
            }
        }
        isAttached = false
        lastSubmitKey = nil
        lastSnapshot = nil
        metrics = nil
        surfaceView?.setNativeContentVisible(false)
        onSurfaceStateChange?()
    }

    private func clearTableResizeState() {
        try? bridge.tableResizeCancel(revision: bridge.state.revision)
        tableResizePointerState.reset()
        lastSubmitKey = nil
    }

    private func ensureMetrics(
        revision: UInt64,
        size: CGFloat,
        maxWidth: CGFloat
    ) throws -> Metrics {
        let key = MetricsKey(
            revision: revision,
            size: Double(size),
            maxWidth: Double(maxWidth)
        )
        if let metrics, metrics.key == key {
            return metrics
        }
        let layout = try bridge.macosFontMetrics(
            revision: revision,
            size: Float(size),
            maxWidth: Float(maxWidth)
        )
        guard layout.revision == revision,
              abs(layout.size - size) <= 0.001,
              layout.lineHeight.isFinite,
              layout.lineHeight > 0.0,
              layout.defaultAdvance.isFinite,
              layout.defaultAdvance > 0.0 else {
            throw BridgeError.operation(StorageStatus.invalidViewport)
        }
        let next = Metrics(
            key: key,
            lineHeight: Float(layout.lineHeight),
            defaultAdvance: Float(layout.defaultAdvance)
        )
        try bridge.setViewportConfig(
            revision: revision,
            maxWidth: Float(maxWidth),
            lineHeight: next.lineHeight,
            defaultAdvance: next.defaultAdvance,
            estimatedBlockHeight: next.lineHeight,
            overscan: 0.0
        )
        metrics = next
        return next
    }
}

/// Watches the containing directory rather than the file inode. Rust still
/// owns the file fingerprint and all reload/conflict decisions; this object
/// only turns native vnode notifications into a main-thread callback. Watching
/// the directory keeps atomic-save rename replacement observable.
private final class NativeFileWatcher {
    private let descriptor: Int32
    private let source: DispatchSourceFileSystemObject

    init(directory: URL, handler: @escaping () -> Void) throws {
        let descriptor = open(directory.path, O_EVTONLY)
        guard descriptor >= 0 else { throw BridgeError.watcher(errno) }
        self.descriptor = descriptor
        source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: descriptor,
            eventMask: [.write, .extend, .attrib, .delete, .rename],
            queue: .main
        )
        source.setEventHandler(handler: handler)
        source.setCancelHandler { close(descriptor) }
        source.resume()
    }

    deinit {
        source.cancel()
    }
}

private final class DocumentViewController: NSViewController, NSMenuItemValidation {
    private let bridge: StorageBridge
    private lazy var textView = DocumentTextView(bridge: bridge)
    private let surfaceHostView = MacosSurfaceHostView()
    private let decorationHostView = MacosVisualDecorationView()
    private let surfaceCoordinator: MacosSurfaceHostCoordinator
    private let statusLabel = NSTextField(labelWithString: "")
    private var saveButton: NSButton?
    private var reloadButton: NSButton?
    private var initialState: NativeStorageState
    private var fileWatcher: NativeFileWatcher?
    private var externalCheckWorkItem: DispatchWorkItem?
    private var promptedExternalDisk: DiskState?
    private var surfaceBoundsObserver: NSObjectProtocol?
    private weak var documentScrollView: NSScrollView?
    private var visualPointerAdapterEnabled = false
    private var visualPointerLayoutWidth: CGFloat = -1.0
    private var visualRenderStateMachine = VisualRenderStateMachine()
    /// True only when the current decoration sibling came from the same Rust
    /// shaped frame that may become the primary visual surface. TextKit
    /// projected decorations are a fallback overlay and must never hide the
    /// source mirror or leave a stale Metal frame visible underneath it.
    private var rustDecorationFrameAccepted = false

    init(bridge: StorageBridge) {
        self.bridge = bridge
        self.surfaceCoordinator = MacosSurfaceHostCoordinator(bridge: bridge)
        self.initialState = bridge.state
        super.init(nibName: nil, bundle: nil)
        surfaceCoordinator.onSurfaceStateChange = { [weak self] in
            self?.updateVisualDecorations()
        }
        surfaceCoordinator.onError = { [weak self] error in
            // The source TextKit mirror remains usable when a machine has no
            // Metal drawable; surface lifecycle failure is diagnostic, not a
            // reason to interrupt editing with a modal alert.
            self?.clearVisualDecorations(reason: .surfaceSubmitFailed)
            self?.statusLabel.toolTip = "Native surface inactive: \(error.localizedDescription)"
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    deinit {
        if let surfaceBoundsObserver {
            NotificationCenter.default.removeObserver(surfaceBoundsObserver)
        }
        surfaceCoordinator.detach()
    }

    override func loadView() {
        let root = NSView()
        let scrollView = NSScrollView()
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = false
        scrollView.autohidesScrollers = true
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        surfaceHostView.translatesAutoresizingMaskIntoConstraints = true
        surfaceHostView.autoresizingMask = []
        surfaceHostView.setAccessibilityElement(false)
        documentScrollView = scrollView

        textView.isEditable = true
        textView.isSelectable = true
        textView.usesFindBar = true
        textView.onDocumentChange = { [weak self] in
            guard let self else { return }
            self.initialState = self.bridge.state
            self.surfaceCoordinator.resetTableResizeAfterDocumentChange()
            self.updateStatus()
            self.updateVisualDecorations()
            self.surfaceCoordinator.scheduleSubmit()
        }
        textView.onBeforeCommand = { [weak self] in
            self?.surfaceCoordinator.prepareForEditorCommand()
        }
        textView.onCaretChange = { [weak self] in
            self?.updateVisualDecorations()
            // AppKit may deliver selection changes while TextKit is still
            // inside its event callback. Defer the scroll mutation until the
            // same main-thread turn has finished, while retaining the Rust
            // Revision captured by the coordinator's query.
            DispatchQueue.main.async { [weak self] in
                self?.surfaceCoordinator.revealCaretIfNeeded()
            }
        }
        textView.onError = { [weak self] error in self?.show(error) }
        textView.onTableResizeBegin = { [weak self] point in
            self?.surfaceCoordinator.beginTableResize(at: point) ?? false
        }
        textView.onTableResizeUpdate = { [weak self] point in
            self?.surfaceCoordinator.updateTableResize(at: point) ?? false
        }
        textView.onTableResizeFinish = { [weak self] in
            self?.surfaceCoordinator.finishTableResize() ?? false
        }
        textView.onTableResizeCancel = { [weak self] in
            self?.surfaceCoordinator.cancelTableResize() ?? false
        }
        scrollView.documentView = textView
        // `DocumentTextView` is created before the window has a laid-out
        // content size. Give the scroll view a real initial document frame
        // and let its text container track the viewport width; otherwise an
        // NSTextView created with the designated initializer can retain a
        // zero-sized document view while AX still exposes its source value.
        textView.frame = NSRect(x: 0, y: 0, width: 900, height: 620)
        textView.autoresizingMask = [.width]
        textView.textContainer?.containerSize = NSSize(
            width: scrollView.contentSize.width,
            height: CGFloat.greatestFiniteMagnitude
        )
        textView.textContainer?.widthTracksTextView = true

        surfaceCoordinator.bind(
            surfaceView: surfaceHostView,
            scrollView: scrollView,
            fontSize: textView.font?.pointSize ?? 16.0
        )
        surfaceHostView.onWindowStateChange = { [weak self] attached in
            guard let self else { return }
            if attached {
                self.surfaceCoordinator.scheduleSubmit()
                self.updateVisualDecorations()
            } else {
                self.surfaceCoordinator.detach()
                self.clearVisualDecorations(reason: .detached)
            }
        }
        surfaceHostView.onGeometryChange = { [weak self] in
            self?.surfaceCoordinator.scheduleSubmit()
            self?.updateVisualDecorations()
        }
        scrollView.contentView.postsBoundsChangedNotifications = true
        surfaceBoundsObserver = NotificationCenter.default.addObserver(
            forName: NSView.boundsDidChangeNotification,
            object: scrollView.contentView,
            queue: .main
        ) { [weak self] _ in
            self?.surfaceCoordinator.scheduleSubmit()
            self?.updateVisualDecorations()
        }

        statusLabel.setAccessibilityElement(true)
        statusLabel.setAccessibilityLabel("文档状态")

        let toolbar = NSStackView()
        toolbar.orientation = .horizontal
        toolbar.alignment = .centerY
        toolbar.spacing = 10
        toolbar.translatesAutoresizingMaskIntoConstraints = false

        let saveButton = NSButton(title: "保存", target: self, action: #selector(save))
        let reloadButton = NSButton(title: "重新加载", target: self, action: #selector(reload))
        self.saveButton = saveButton
        self.reloadButton = reloadButton
        toolbar.addArrangedSubview(saveButton)
        toolbar.addArrangedSubview(reloadButton)
        toolbar.addArrangedSubview(statusLabel)

        root.addSubview(toolbar)
        root.addSubview(scrollView)
        // The Rust surface is a visual projection above the TextKit mirror.
        // Its hitTest returns nil, so keyboard, IME, selection and scrolling
        // remain owned by the source view underneath it. The frame is synced
        // to the clip viewport in viewDidLayout, excluding native scrollers.
        root.addSubview(surfaceHostView, positioned: .above, relativeTo: scrollView)
        root.addSubview(decorationHostView, positioned: .above, relativeTo: surfaceHostView)
        NSLayoutConstraint.activate([
            toolbar.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 16),
            toolbar.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -16),
            toolbar.topAnchor.constraint(equalTo: root.topAnchor, constant: 12),
            scrollView.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            scrollView.topAnchor.constraint(equalTo: toolbar.bottomAnchor, constant: 10),
            scrollView.bottomAnchor.constraint(equalTo: root.bottomAnchor),
        ])
        view = root
        startFileWatcher()
        updateStatus()
    }

    override func viewDidLayout() {
        super.viewDidLayout()
        guard let scrollView = documentScrollView else { return }
        let viewportFrame = view.convert(scrollView.contentView.frame, from: scrollView)
        if surfaceHostView.frame != viewportFrame {
            surfaceHostView.frame = viewportFrame
        }
        if decorationHostView.frame != viewportFrame {
            decorationHostView.frame = viewportFrame
        }
        let visualWidth = max(
            textView.bounds.width - 2.0 * textView.textContainerOrigin.x,
            1.0
        )
        surfaceCoordinator.setContentWidth(visualWidth)
        do {
            if !visualPointerAdapterEnabled {
                try textView.setVisualMirrorEnabled(true)
                visualPointerAdapterEnabled = true
                visualPointerLayoutWidth = visualWidth
            } else if abs(visualPointerLayoutWidth - visualWidth) > 0.5 {
                try textView.refreshVisualMirrorForDisplay()
                visualPointerLayoutWidth = visualWidth
            }
        } catch {
            visualPointerAdapterEnabled = false
            visualPointerLayoutWidth = -1.0
            try? textView.setVisualMirrorEnabled(false)
            statusLabel.toolTip = "Visual pointer inactive: \(error.localizedDescription)"
        }
        updateVisualDecorations()
        surfaceCoordinator.scheduleSubmit()
        surfaceCoordinator.revealCaretIfNeeded()
    }

    func refreshFromRust() {
        textView.refreshFromRust()
        surfaceCoordinator.resetTableResizeAfterDocumentChange()
        initialState = bridge.state
        if initialState.disk == .unchanged {
            promptedExternalDisk = nil
        }
        updateStatus()
        updateVisualDecorations()
        surfaceCoordinator.scheduleSubmit()
        surfaceCoordinator.revealCaretIfNeeded()
    }

    func detachSurfaceHost() {
        surfaceCoordinator.detach()
        clearVisualDecorations(reason: .detached)
    }

    private func clearVisualDecorations(
        reason: VisualRenderFallbackReason = .disabled
    ) {
        decorationHostView.clear()
        rustDecorationFrameAccepted = false
        textView.useSourceFallbackPresentation()
        surfaceHostView.setNativeContentVisible(false)
        visualRenderStateMachine.enterFallback(reason)
    }

    private func syncSourceGlyphVisibility(
        useProjectedTextKitFallback: Bool = false
    ) {
        let revision = bridge.state.revision
        let compositionGeneration = bridge.composition.generation
        let publicationCurrent = surfaceCoordinator.hasCurrentPublication(
            revision: revision,
            compositionGeneration: compositionGeneration
        )
        let decorationCurrent = decorationHostView.revision == revision
            && decorationHostView.hasValidFrame
        let activeFrame = acceptedVisualRenderFrame(
            revision: revision,
            compositionGeneration: compositionGeneration,
            publicationCurrent: publicationCurrent,
            publication: surfaceCoordinator.lastSnapshot.map(VisualRenderPublicationIdentity.init),
            decorationRevision: decorationHostView.revision,
            decorationHasValidFrame: decorationHostView.hasValidFrame,
            rustDecorationFrameAccepted: rustDecorationFrameAccepted
        )
        let canHideSourceGlyphs = activeFrame != nil
        if canHideSourceGlyphs {
            textView.useRustSurfacePresentation()
        } else if useProjectedTextKitFallback {
            textView.useProjectedTextKitOverlayPresentation()
        } else {
            textView.useSourceFallbackPresentation()
        }
        // The Rust surface and its Rust-shaped decoration frame are one
        // visual publication. If either side is stale, hide the surface as a
        // unit and let TextKit render the source mirror until both are ready.
        surfaceHostView.setNativeContentVisible(canHideSourceGlyphs)

        if let activeFrame {
            visualRenderStateMachine.activate(activeFrame)
            return
        }

        visualRenderStateMachine.enterFallback(
            visualRenderFallbackReason(
                revision: revision,
                compositionGeneration: compositionGeneration,
                publicationCurrent: publicationCurrent,
                decorationCurrent: decorationCurrent
            )
        )
    }

    private func visualRenderFallbackReason(
        revision: UInt64,
        compositionGeneration: UInt64,
        publicationCurrent: Bool,
        decorationCurrent: Bool
    ) -> VisualRenderFallbackReason {
        guard decorationCurrent else {
            if let decorationRevision = decorationHostView.revision,
               decorationRevision != revision {
                return .staleRevision
            }
            return .decorationUnavailable
        }
        guard publicationCurrent else {
            guard let snapshot = surfaceCoordinator.lastSnapshot else {
                return surfaceCoordinator.isAttached
                    ? .waitingForSurface
                    : .detached
            }
            if snapshot.revision != revision {
                return .staleRevision
            }
            if snapshot.compositionGeneration != compositionGeneration {
                return .staleComposition
            }
            return snapshot.submitted ? .waitingForSurface : .surfaceSubmitFailed
        }
        if bridge.composition.active {
            return .compositionActive
        }
        return .invalidFrame
    }

    /// Publishes Rust/CoreText-shaped decoration geometry into the sibling
    /// overlay. The Rust coordinates are document-space and the only native
    /// transform here is the current scroll offset into the surface sibling's
    /// viewport-local coordinate system. Active composition uses the same
    /// generation-bound Rust geometry as the surface; TextKit's projected
    /// overlay is only a failure fallback.
    private func updateVisualDecorations() {
        guard decorationHostView.superview != nil else {
            clearVisualDecorations(reason: .missingGeometry)
            return
        }
        if let geometry = surfaceCoordinator.visualDecorationGeometry() {
            do {
                let (snapshot, caret, selection) = try bridge.macosVisualDecorations(
                    revision: bridge.state.revision,
                    compositionGeneration: bridge.composition.generation,
                    size: geometry.size,
                    maxWidth: geometry.maxWidth,
                    scrollY: geometry.scrollY,
                    viewportHeight: geometry.viewportHeight
                )
                guard snapshot.revision == bridge.state.revision,
                      snapshot.caretPresent,
                      caret.present,
                      caret.revision == snapshot.revision else {
                    rustDecorationFrameAccepted = false
                    if bridge.composition.active {
                        updateVisualDecorationsFromTextKit()
                    } else {
                        clearVisualDecorations(reason: .decorationUnavailable)
                    }
                    return
                }
                let scrollY = geometry.scrollY
                let localSelection = selection.map {
                    NSRect(
                        x: $0.rect.origin.x,
                        y: $0.rect.origin.y - CGFloat(scrollY),
                        width: $0.rect.width,
                        height: $0.rect.height
                    )
                }
                let localCaret = NSRect(
                    x: caret.rect.origin.x,
                    y: caret.rect.origin.y - CGFloat(scrollY),
                    width: caret.rect.width,
                    height: caret.rect.height
                )
                decorationHostView.update(
                    revision: snapshot.revision,
                    selectionRects: localSelection,
                    caretRect: localCaret,
                    compositionActive: bridge.composition.active
                )
                rustDecorationFrameAccepted = true
                syncSourceGlyphVisibility()
                return
            } catch {
                // A transient TextKit projection is retained only when the
                // generation-bound Rust decoration query cannot provide a
                // drawable frame for active marked text.
                rustDecorationFrameAccepted = false
                if bridge.composition.active {
                    updateVisualDecorationsFromTextKit()
                } else {
                    clearVisualDecorations(reason: .decorationUnavailable)
                }
                return
            }
        }
        if bridge.composition.active {
            updateVisualDecorationsFromTextKit()
        } else {
            clearVisualDecorations(reason: .missingGeometry)
        }
    }

    /// Converts the disposable visual mirror geometry into the sibling
    /// decoration view's local coordinates for active marked text only. A
    /// normal Revision/geometry failure uses the canonical source fallback;
    /// it must not turn TextKit's projected caret into a second renderer.
    private func updateVisualDecorationsFromTextKit() {
        guard bridge.composition.active,
              visualPointerAdapterEnabled,
              decorationHostView.superview != nil,
              let caret = textView.visualCaretRectForDisplay() else {
            clearVisualDecorations(reason: .visualMirrorUnavailable)
            return
        }
        rustDecorationFrameAccepted = false
        let selection = textView.visualSelectionRectsForDisplay().map {
            textView.convert($0, to: decorationHostView)
        }
        let convertedCaret = textView.convert(caret, to: decorationHostView)
        decorationHostView.update(
            revision: bridge.state.revision,
            selectionRects: selection,
            caretRect: convertedCaret,
            compositionActive: bridge.composition.active
        )
        syncSourceGlyphVisibility(useProjectedTextKitFallback: true)
    }

    @objc private func save() {
        do {
            try bridge.save()
            refreshFromRust()
        } catch { show(error) }
    }

    @objc private func reload() {
        do {
            try bridge.reload()
            refreshFromRust()
        } catch { show(error) }
    }

    @objc fileprivate func saveFromMenu(_ sender: Any?) {
        save()
    }

    @objc fileprivate func reloadFromMenu(_ sender: Any?) {
        reload()
    }

    @objc fileprivate func closeFromMenu(_ sender: Any?) {
        view.window?.performClose(sender)
    }

    @objc fileprivate func copyFromMenu(_ sender: Any?) {
        textView.copy(sender)
    }

    @objc fileprivate func cutFromMenu(_ sender: Any?) {
        textView.cut(sender)
    }

    @objc fileprivate func undoFromMenu(_ sender: Any?) {
        textView.performUndo()
    }

    @objc fileprivate func redoFromMenu(_ sender: Any?) {
        textView.performRedo()
    }

    @objc fileprivate func pasteFromMenu(_ sender: Any?) {
        textView.paste(sender)
    }

    @objc fileprivate func selectAllFromMenu(_ sender: Any?) {
        textView.selectAll(sender)
    }

    func focusDocument() {
        _ = view.window?.makeFirstResponder(textView)
    }

    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        let state = bridge.state
        if menuItem.action == #selector(saveFromMenu(_:)) {
            return state.dirty
        }
        if menuItem.action == #selector(reloadFromMenu(_:)) {
            return !state.dirty && state.disk != .unchanged
        }
        if menuItem.action == #selector(undoFromMenu(_:)) {
            return textView.canUndo()
        }
        if menuItem.action == #selector(redoFromMenu(_:)) {
            return textView.canRedo()
        }
        if menuItem.action == #selector(copyFromMenu(_:)) ||
            menuItem.action == #selector(cutFromMenu(_:)) {
            return textView.selectedRange().length > 0
        }
        if menuItem.action == #selector(pasteFromMenu(_:)) {
            return textView.hasSourceOnPasteboard
        }
        if menuItem.action == #selector(selectAllFromMenu(_:)) {
            return textView.string.utf16.count > 0
        }
        return true
    }

    func requestClose() -> Bool {
        do {
            let request = try bridge.requestClose()
            switch request.result {
            case 0:
                return true
            case 1:
                return prompt(request)
            default:
                return false
            }
        } catch {
            show(error)
            return false
        }
    }

    private func prompt(_ request: YuStorageCloseRequest) -> Bool {
        let alert = NSAlert()
        alert.alertStyle = request.close_state >= 3 ? .warning : .informational
        alert.messageText = request.close_state >= 3 ? "文件已被外部修改" : "保存更改？"
        alert.informativeText = request.close_state >= 3
            ? "保存会覆盖外部版本；请选择丢弃本地修改或取消。"
            : "这个 Markdown 文档有未保存更改。"
        alert.addButton(withTitle: request.close_state >= 3 ? "丢弃本地修改" : "保存")
        alert.addButton(withTitle: "取消")
        let response = alert.runModal()
        do {
            if response == .alertFirstButtonReturn {
                if request.close_state >= 3 {
                    try bridge.discardAndClose()
                } else {
                    try bridge.saveAndClose()
                }
                return true
            }
            try bridge.cancelClose()
            return false
        } catch {
            show(error)
            return false
        }
    }

    private func startFileWatcher() {
        guard fileWatcher == nil else { return }
        let directory = URL(fileURLWithPath: bridge.path).deletingLastPathComponent()
        do {
            fileWatcher = try NativeFileWatcher(directory: directory) { [weak self] in
                self?.scheduleExternalStateCheck()
            }
        } catch {
            // The session remains correct without a native notification source;
            // the status/menu state can still be refreshed by explicit actions.
            statusLabel.toolTip = error.localizedDescription
        }
    }

    private func scheduleExternalStateCheck() {
        externalCheckWorkItem?.cancel()
        let workItem = DispatchWorkItem { [weak self] in
            self?.checkExternalState()
        }
        externalCheckWorkItem = workItem
        DispatchQueue.main.asyncAfter(
            deadline: .now() + .milliseconds(150),
            execute: workItem
        )
    }

    private func checkExternalState() {
        let state = bridge.state
        initialState = state
        updateStatus()
        guard state.disk != .unchanged else {
            promptedExternalDisk = nil
            return
        }
        guard promptedExternalDisk != state.disk else { return }
        promptedExternalDisk = state.disk

        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = state.disk == .missing ? "文件已被删除或移动" : "文件已被外部修改"
        if state.dirty {
            alert.informativeText =
                "Rust session 已确认磁盘版本变化。本地有未保存修改，Yu 不会自动覆盖或重载。请保存或关闭窗口处理冲突。"
            alert.addButton(withTitle: "知道了")
        } else {
            alert.informativeText =
                "当前没有本地未保存修改，可以重新加载磁盘上的版本。"
            alert.addButton(withTitle: "重新加载")
            alert.addButton(withTitle: "稍后")
        }
        let response = alert.runModal()
        guard !state.dirty, response == .alertFirstButtonReturn else { return }
        do {
            try bridge.reload()
            refreshFromRust()
        } catch {
            show(error)
        }
    }

    private func updateStatus() {
        let state = bridge.state
        let dirty = state.dirty ? "● 未保存" : "已保存"
        let bom = state.bom ? "UTF-8 BOM" : "UTF-8"
        let status = "\(dirty) · Rev \(state.revision) · \(state.disk.label) · \(bom)"
        statusLabel.stringValue = status
        statusLabel.setAccessibilityValue(status)
        saveButton?.isEnabled = state.dirty
        reloadButton?.isEnabled = !state.dirty && state.disk != .unchanged
    }

    private func show(_ error: Error) {
        let alert = NSAlert(error: error)
        alert.runModal()
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    private var window: NSWindow?
    private var controller: DocumentViewController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let path: String
        if let argument = CommandLine.arguments.dropFirst().first, !argument.hasPrefix("-") {
            path = URL(fileURLWithPath: argument).path
        } else {
            let panel = NSOpenPanel()
            panel.allowedContentTypes = [.text, .plainText]
            panel.canChooseDirectories = false
            guard panel.runModal() == .OK, let url = panel.url else {
                NSApp.terminate(nil)
                return
            }
            path = url.path
        }

        do {
            let bridge = try StorageBridge(path: path)
            let controller = DocumentViewController(bridge: bridge)
            let window = NSWindow(
                contentViewController: controller
            )
            window.setContentSize(NSSize(width: 900, height: 620))
            window.styleMask = [.titled, .closable, .miniaturizable, .resizable]
            window.title = URL(fileURLWithPath: bridge.path).lastPathComponent
            window.center()
            window.delegate = self
            window.isReleasedWhenClosed = false
            window.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            self.controller = controller
            self.window = window
            installMainMenu(for: controller)
            controller.focusDocument()
            print("Yu document host opened path=\(bridge.path) revision=\(bridge.state.revision)")
        } catch {
            let alert = NSAlert(error: error)
            alert.runModal()
            NSApp.terminate(nil)
        }
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        let shouldClose = controller?.requestClose() ?? true
        if shouldClose {
            controller?.detachSurfaceHost()
        }
        return shouldClose
    }

    func windowWillClose(_ notification: Notification) {
        controller?.detachSurfaceHost()
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let controller else { return .terminateNow }
        guard controller.requestClose() else { return .terminateCancel }
        controller.detachSurfaceHost()
        return .terminateNow
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }

    private func installMainMenu(for controller: DocumentViewController) {
        let mainMenu = NSMenu()

        let appMenuItem = NSMenuItem()
        let appMenu = NSMenu(title: "Yu")
        appMenu.addItem(
            withTitle: "关于 Yu",
            action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)),
            keyEquivalent: ""
        )
        appMenu.addItem(.separator())
        let quit = NSMenuItem(
            title: "退出 Yu",
            action: #selector(NSApplication.terminate(_:)),
            keyEquivalent: "q"
        )
        quit.target = NSApp
        appMenu.addItem(quit)
        appMenuItem.submenu = appMenu
        mainMenu.addItem(appMenuItem)

        let fileMenuItem = NSMenuItem()
        let fileMenu = NSMenu(title: "文件")
        let save = NSMenuItem(
            title: "保存",
            action: #selector(DocumentViewController.saveFromMenu(_:)),
            keyEquivalent: "s"
        )
        save.target = controller
        fileMenu.addItem(save)
        let reload = NSMenuItem(
            title: "重新加载",
            action: #selector(DocumentViewController.reloadFromMenu(_:)),
            keyEquivalent: "r"
        )
        reload.target = controller
        fileMenu.addItem(reload)
        fileMenu.addItem(.separator())
        let close = NSMenuItem(
            title: "关闭窗口",
            action: #selector(DocumentViewController.closeFromMenu(_:)),
            keyEquivalent: "w"
        )
        close.target = controller
        fileMenu.addItem(close)
        fileMenuItem.submenu = fileMenu
        mainMenu.addItem(fileMenuItem)

        let editMenuItem = NSMenuItem()
        let editMenu = NSMenu(title: "编辑")
        let editItems: [(String, Selector, String)] = [
            ("撤销", #selector(DocumentViewController.undoFromMenu(_:)), "z"),
            ("重做", #selector(DocumentViewController.redoFromMenu(_:)), "Z"),
            ("剪切", #selector(DocumentViewController.cutFromMenu(_:)), "x"),
            ("复制", #selector(DocumentViewController.copyFromMenu(_:)), "c"),
            ("粘贴", #selector(DocumentViewController.pasteFromMenu(_:)), "v"),
            ("全选", #selector(DocumentViewController.selectAllFromMenu(_:)), "a"),
        ]
        for (title, action, keyEquivalent) in editItems.prefix(2) {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: keyEquivalent)
            item.target = controller
            editMenu.addItem(item)
        }
        editMenu.addItem(.separator())
        for (title, action, keyEquivalent) in editItems.dropFirst(2) {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: keyEquivalent)
            item.target = controller
            editMenu.addItem(item)
        }
        editMenuItem.submenu = editMenu
        mainMenu.addItem(editMenuItem)

        NSApp.mainMenu = mainMenu
    }
}

private func runClipboardSelfCheck(path: String) -> Never {
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

private func runSelectionSelfCheck(path: String) -> Never {
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

private func runUndoSelfCheck(path: String) -> Never {
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

private func runProjectionSelfCheck(path: String) -> Never {
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

private func runProjectionHitTestSelfCheck(path: String) -> Never {
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

private func runShapedProjectionHitTestSelfCheck(path: String) -> Never {
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
        let metrics = try bridge.macosFontMetrics(
            revision: revision,
            size: size,
            maxWidth: pointerWidth
        )
        try bridge.setViewportConfig(
            revision: revision,
            maxWidth: pointerWidth,
            lineHeight: Float(metrics.lineHeight),
            defaultAdvance: Float(metrics.defaultAdvance),
            estimatedBlockHeight: Float(metrics.lineHeight),
            overscan: 0.0
        )
        try textView.setVisualMirrorEnabledForSelfCheck(true)

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

        let visualEnd = (projected as NSString).range(of: "粗体")
        precondition(visualEnd.location != NSNotFound)
        guard let endPoint = textView.visualMirrorPointForSelfCheck(
            visualUTF16: visualEnd.location + visualEnd.length
        ) else {
            preconditionFailure("visual mirror end point is unavailable")
        }
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

private func runVisualMirrorSelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let textView = DocumentTextView(bridge: bridge)
        try textView.setVisualMirrorEnabledForSelfCheck(true)
        let revision = bridge.state.revision
        let projected = try bridge.projectedSource(revision: revision)
        let mirrorStorage = NSTextStorage(string: projected)
        let mirrorLayout = NSLayoutManager()
        let mirrorContainer = NSTextContainer(
            size: NSSize(width: 500.0, height: CGFloat.greatestFiniteMagnitude)
        )
        mirrorContainer.lineFragmentPadding = 0.0
        mirrorLayout.addTextContainer(mirrorContainer)
        mirrorStorage.addLayoutManager(mirrorLayout)
        precondition(mirrorStorage.string == projected)

        let sourceStrong = (bridge.source as NSString).range(of: "**粗体**")
        let visualStrong = (projected as NSString).range(of: "粗体")
        precondition(sourceStrong.location != NSNotFound)
        precondition(visualStrong.location != NSNotFound)
        precondition(
            textView.visualMirrorPointForSelfCheck(visualUTF16: visualStrong.location) != nil
        )
        precondition(textView.visualCaretRectForDisplay() != nil)
        precondition(textView.applyVisualSelectionForSelfCheck(visualStrong))
        precondition(bridge.selection.range == sourceStrong)
        precondition(textView.visualCaretRectForDisplay() != nil)
        let selectionRects = textView.visualSelectionRectsForSelfCheck()
        precondition(!selectionRects.isEmpty)
        precondition(selectionRects.allSatisfy {
            $0.width.isFinite && $0.height.isFinite && $0.width > 0.0 && $0.height > 0.0
        })
        let glyphRange = mirrorLayout.glyphRange(
            forCharacterRange: visualStrong,
            actualCharacterRange: nil
        )
        precondition(glyphRange.length > 0)

        let forward = try bridge.projectionSelection(
            revision: revision,
            sourceRange: sourceStrong,
            affinity: 1
        )
        precondition(forward.visualRange == visualStrong)
        let reverse = try bridge.projectionSourceSelection(
            revision: revision,
            visualRange: visualStrong,
            affinity: 1
        )
        precondition(reverse.revision == revision)
        precondition(reverse.visualRange == visualStrong)
        precondition(reverse.sourceRange == sourceStrong)
        precondition(reverse.roundTripVisualRange == visualStrong)

        let visualCaret = try bridge.projectionSourceCaret(
            revision: revision,
            visualUTF16: UInt64(visualStrong.location),
            affinity: 0
        )
        precondition(visualCaret.revision == revision)
        precondition(visualCaret.sourceUTF16 == UInt64(sourceStrong.location))
        precondition(visualCaret.roundTripVisualUTF16 == UInt64(visualStrong.location))

        _ = try bridge.insertText("x")
        do {
            _ = try bridge.projectionSourceSelection(
                revision: revision,
                visualRange: visualStrong,
                affinity: 1
            )
            preconditionFailure("stale visual mirror mapping unexpectedly succeeded")
        } catch BridgeError.operation(let status) {
            precondition(status == 13)
        }
        // The native mirror remains a disposable old-revision snapshot; it is
        // not silently patched after Rust source changes.
        precondition(mirrorStorage.string == projected)
        print(
            "Yu Visual Mirror self-check: TextKit visual UTF-16 range "
                + "\(visualStrong) ↔ source range \(sourceStrong), selection highlight "
                + "rects=\(selectionRects.count); stale mirror rejected"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Visual Mirror self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

private func runVisualDecorationSelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let textView = DocumentTextView(bridge: bridge)
        let root = NSView(frame: NSRect(x: 0.0, y: 0.0, width: 640.0, height: 480.0))
        let decoration = MacosVisualDecorationView(frame: root.bounds)
        textView.frame = root.bounds
        root.addSubview(textView)
        root.addSubview(decoration)

        try textView.setVisualMirrorEnabledForSelfCheck(true)
        let revision = bridge.state.revision
        let size: Float = 14.0
        let maxWidth: Float = 500.0
        let metrics = try bridge.macosFontMetrics(
            revision: revision,
            size: size,
            maxWidth: maxWidth
        )
        try bridge.setViewportConfig(
            revision: revision,
            maxWidth: maxWidth,
            lineHeight: Float(metrics.lineHeight),
            defaultAdvance: Float(metrics.defaultAdvance),
            estimatedBlockHeight: Float(metrics.lineHeight),
            overscan: 0.0
        )
        let sourceStrong = (bridge.source as NSString).range(of: "**粗体**")
        precondition(sourceStrong.location != NSNotFound)
        precondition(textView.applyVisualSelectionForSelfCheck(
            NSRange(location: sourceStrong.location + 2, length: 2)
        ))
        let (rustSnapshot, rustCaret, rustSelection) = try bridge.macosVisualDecorations(
            revision: revision,
            compositionGeneration: bridge.composition.generation,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 480.0
        )
        precondition(rustSnapshot.revision == revision)
        precondition(rustSnapshot.caretPresent)
        precondition(rustCaret.present)
        precondition(!rustSelection.isEmpty)
        precondition(rustSnapshot.selectionCount == rustSelection.count)

        decoration.update(
            revision: revision,
            selectionRects: rustSelection.map(\.rect),
            caretRect: rustCaret.rect,
            compositionActive: false
        )
        precondition(decoration.hasValidFrame)
        precondition(decoration.revision == revision)
        precondition(decoration.selectionRects.count == rustSelection.count)
        precondition(decoration.caretRect != nil)
        precondition(decoration.hitTest(NSPoint(x: 1.0, y: 1.0)) == nil)

        let compositionRange = NSRange(
            location: sourceStrong.location + 2,
            length: 2
        )
        try bridge.beginComposition(
            replacementRange: compositionRange,
            preedit: "日本🙂",
            selection: NSRange(location: 2, length: 2)
        )
        let composition = bridge.composition
        precondition(composition.active)
        let (compositionSnapshot, compositionCaret, compositionSelection) =
            try bridge.macosVisualDecorations(
                revision: revision,
                compositionGeneration: composition.generation,
                size: size,
                maxWidth: maxWidth,
                scrollY: 0.0,
                viewportHeight: 480.0
            )
        precondition(compositionSnapshot.revision == revision)
        precondition(compositionSnapshot.compositionGeneration == composition.generation)
        precondition(compositionSnapshot.caretPresent)
        precondition(compositionCaret.present)
        precondition(
            compositionCaret.rect.origin.x.isFinite
                && compositionCaret.rect.origin.y.isFinite
        )
        precondition(compositionSnapshot.selectionCount == compositionSelection.count)
        decoration.update(
            revision: revision,
            selectionRects: compositionSelection.map(\.rect),
            caretRect: compositionCaret.rect,
            compositionActive: true
        )
        precondition(decoration.compositionActive)
        precondition(decoration.hasValidFrame)
        try bridge.cancelComposition()
        precondition(!bridge.composition.active)

        textView.useSourceFallbackPresentation()
        precondition(textView.presentationRoleForSelfCheck == "sourceFallback")
        textView.useProjectedTextKitOverlayPresentation()
        precondition(textView.presentationRoleForSelfCheck == "projectedTextKitOverlay")
        textView.useRustSurfacePresentation()
        precondition(textView.presentationRoleForSelfCheck == "rustSurface")
        textView.setSourceGlyphsHidden(true)
        precondition(textView.sourceGlyphsHiddenForSelfCheck)
        precondition(textView.string == bridge.source)
        textView.setSourceGlyphsHidden(false)
        precondition(!textView.sourceGlyphsHiddenForSelfCheck)
        precondition(textView.presentationRoleForSelfCheck == "sourceFallback")

        _ = try bridge.insertText("x")
        do {
            _ = try bridge.macosVisualDecorations(
                revision: revision,
                compositionGeneration: bridge.composition.generation,
                size: size,
                maxWidth: maxWidth,
                scrollY: 0.0,
                viewportHeight: 480.0
            )
            preconditionFailure("stale Rust decoration geometry unexpectedly succeeded")
        } catch BridgeError.operation(let status) {
            precondition(status == 13)
        }
        precondition(textView.visualCaretRectForDisplay() == nil)
        decoration.clear()
        precondition(!decoration.hasValidFrame)
        precondition(decoration.revision == nil)
        print(
            "Yu Visual Decoration self-check: Rust/CoreText-shaped revision-bound "
                + "overlay owns \(rustSelection.count) selection rects and falls back on stale geometry"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Visual Decoration self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

private func runVisualRenderStateSelfCheck() -> Never {
    var machine = VisualRenderStateMachine()
    precondition(machine.state == .fallback(.disabled))
    precondition(machine.transitionSerial == 0)

    machine.enterFallback(.waitingForSurface)
    precondition(machine.state == .fallback(.waitingForSurface))
    let firstTransition = machine.transitionSerial
    machine.enterFallback(.waitingForSurface)
    precondition(machine.transitionSerial == firstTransition)

    machine.enterFallback(.staleRevision)
    precondition(machine.state.fallbackReason == .staleRevision)

    let frame = VisualRenderFrameIdentity(
        revision: 7,
        compositionGeneration: 3,
        surfaceGeneration: 11,
        frameSerial: 19
    )
    machine.activate(frame)
    precondition(machine.state == .active(frame))
    precondition(machine.state.isActive)
    precondition(machine.diagnosticDescription.contains("revision=7"))

    let publication = VisualRenderPublicationIdentity(frame: frame, submitted: true)
    precondition(
        acceptedVisualRenderFrame(
            revision: 7,
            compositionGeneration: 3,
            publicationCurrent: true,
            publication: publication,
            decorationRevision: 7,
            decorationHasValidFrame: true,
            rustDecorationFrameAccepted: true
        ) == frame
    )
    precondition(
        acceptedVisualRenderFrame(
            revision: 7,
            compositionGeneration: 4,
            publicationCurrent: true,
            publication: publication,
            decorationRevision: 7,
            decorationHasValidFrame: true,
            rustDecorationFrameAccepted: true
        ) == nil
    )
    precondition(
        acceptedVisualRenderFrame(
            revision: 7,
            compositionGeneration: 3,
            publicationCurrent: false,
            publication: publication,
            decorationRevision: 7,
            decorationHasValidFrame: true,
            rustDecorationFrameAccepted: true
        ) == nil
    )
    precondition(
        acceptedVisualRenderFrame(
            revision: 7,
            compositionGeneration: 3,
            publicationCurrent: true,
            publication: publication,
            decorationRevision: 8,
            decorationHasValidFrame: true,
            rustDecorationFrameAccepted: true
        ) == nil
    )

    machine.enterFallback(.staleComposition)
    precondition(machine.state == .fallback(.staleComposition))
    precondition(!machine.state.isActive)
    precondition(machine.transitionSerial >= 4)
    print(
        "Yu Visual Render State self-check: explicit active/fallback transitions "
            + "preserve frame identity, generation gate and diagnostics"
    )
    exit(EXIT_SUCCESS)
}

private func runVisualIMESelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let sourceBefore = bridge.source
        let revision = bridge.state.revision
        let shapedSize: Float = 14.0
        let shapedWidth: Float = 500.0
        let metrics = try bridge.macosFontMetrics(
            revision: revision,
            size: shapedSize,
            maxWidth: shapedWidth
        )
        try bridge.setViewportConfig(
            revision: revision,
            maxWidth: shapedWidth,
            lineHeight: Float(metrics.lineHeight),
            defaultAdvance: Float(metrics.defaultAdvance),
            estimatedBlockHeight: Float(metrics.lineHeight),
            overscan: 0.0
        )
        let textView = DocumentTextView(bridge: bridge)
        try textView.setVisualMirrorEnabledForSelfCheck(true)

        let sourceStrong = (sourceBefore as NSString).range(of: "**粗体**")
        precondition(sourceStrong.location != NSNotFound)
        let replacement = NSRange(location: sourceStrong.location + 2, length: 2)
        let visualStrong = try bridge.projectedSource(revision: revision)
        let visualReplacementStart = (visualStrong as NSString).range(of: "粗体").location
        precondition(visualReplacementStart != NSNotFound)

        textView.setMarkedText(
            "日本🙂",
            selectedRange: NSRange(location: 2, length: 2),
            replacementRange: replacement
        )
        let initial = try bridge.compositionProjection(revision: revision)
        let initialProjected = try bridge.copyCompositionProjection(
            revision: initial.revision,
            generation: initial.generation
        )
        precondition(textView.visualMirrorStringForSelfCheck() == initialProjected)
        precondition(initialProjected.contains("日本🙂"))
        precondition(!initialProjected.contains("**粗体**"))
        precondition(
            initial.visualReplacementRange == NSRange(
                location: visualReplacementStart,
                length: "日本🙂".utf16.count
            )
        )
        precondition(textView.visualMarkedRangeForSelfCheck() == initial.visualReplacementRange)
        precondition(textView.markedRange() == initial.visualReplacementRange)
        precondition(textView.hasMarkedText())
        let initialShapedCaret = try bridge.macosCompositionShapedCaret(
            revision: initial.revision,
            generation: initial.generation,
            sourceUTF16: UInt64(replacement.location),
            affinity: 1,
            size: shapedSize,
            maxWidth: shapedWidth
        )
        precondition(initialShapedCaret.revision == revision)
        precondition(initialShapedCaret.generation == initial.generation)
        precondition(initialShapedCaret.sourceUTF16 == UInt64(replacement.location))
        precondition(initialShapedCaret.visualSelection == initial.visualSelection)
        precondition(initialShapedCaret.visualReplacement == initial.visualReplacementRange)
        precondition(initialShapedCaret.point.x.isFinite)
        precondition(initialShapedCaret.point.y.isFinite)
        precondition(initialShapedCaret.size.height > 0.0)
        var actualRange = NSRange(location: NSNotFound, length: 0)
        let marked = textView.attributedSubstring(
            forProposedRange: initial.visualReplacementRange,
            actualRange: &actualRange
        )
        precondition(marked?.string == "日本🙂")
        precondition(actualRange == initial.visualReplacementRange)

        textView.setMarkedText(
            "日本語",
            selectedRange: NSRange(location: 3, length: 0),
            replacementRange: replacement
        )
        let updated = try bridge.compositionProjection(revision: revision)
        let updatedProjected = try bridge.copyCompositionProjection(
            revision: updated.revision,
            generation: updated.generation
        )
        precondition(updated.generation != initial.generation)
        precondition(updatedProjected.contains("日本語"))
        precondition(textView.visualMirrorStringForSelfCheck() == updatedProjected)
        precondition(textView.markedRange() == updated.visualReplacementRange)
        precondition(updated.visualReplacementRange.length == "日本語".utf16.count)
        let updatedShapedCaret = try bridge.macosCompositionShapedCaret(
            revision: updated.revision,
            generation: updated.generation,
            sourceUTF16: UInt64(replacement.location),
            affinity: 1,
            size: shapedSize,
            maxWidth: shapedWidth
        )
        precondition(updatedShapedCaret.generation == updated.generation)
        precondition(updatedShapedCaret.visualSelection == updated.visualSelection)
        precondition(updatedShapedCaret.visualReplacement == updated.visualReplacementRange)
        precondition(updatedShapedCaret.point.x.isFinite)
        precondition(updatedShapedCaret.point.y.isFinite)
        do {
            _ = try bridge.copyCompositionProjection(
                revision: revision,
                generation: initial.generation
            )
            preconditionFailure("stale visual composition unexpectedly succeeded")
        } catch BridgeError.operation(let status) {
            precondition(status == 16)
        }
        do {
            _ = try bridge.macosCompositionShapedCaret(
                revision: revision,
                generation: initial.generation,
                sourceUTF16: UInt64(replacement.location),
                affinity: 1,
                size: shapedSize,
                maxWidth: shapedWidth
            )
            preconditionFailure("stale shaped composition caret unexpectedly succeeded")
        } catch BridgeError.operation(let status) {
            precondition(status == 16)
        }

        try bridge.cancelComposition()
        textView.refreshFromRust()
        precondition(!textView.hasMarkedText())
        precondition(textView.markedRange().location == NSNotFound)
        precondition(bridge.state.revision == revision)
        precondition(bridge.source == sourceBefore)
        let canonicalVisual = try bridge.projectedSource(revision: revision)
        precondition(textView.visualMirrorStringForSelfCheck() == canonicalVisual)
        print(
            "Yu Visual IME self-check: visual preedit/replacement range is "
                + "generation-bound; stale generation rejected and cancel preserved source"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Visual IME self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

private func runCompositionHitTestSelfCheck(path: String) -> Never {
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
        let metrics = try bridge.macosFontMetrics(
            revision: revision,
            size: size,
            maxWidth: maxWidth
        )
        try bridge.setViewportConfig(
            revision: revision,
            maxWidth: maxWidth,
            lineHeight: Float(metrics.lineHeight),
            defaultAdvance: Float(metrics.defaultAdvance),
            estimatedBlockHeight: Float(metrics.lineHeight),
            overscan: 0.0
        )
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

private func runBlockProjectionSelfCheck(path: String) -> Never {
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
            let preview = try bridge.tableResizeUpdate(
                revision: revision,
                pointerPosition: Float(columnDividerX + 1.1)
            )
            precondition(preview.revision == revision)
            precondition(preview.blockIndex == UInt64(tableIndex))
            precondition(preview.kind == YU_STORAGE_TABLE_RESIZE_COLUMN)
            precondition(preview.index == 0)
            precondition(abs(preview.delta - 1.0) < 0.0001)
            let finishedResize = try bridge.tableResizeFinish(revision: revision)
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
            let rowPreview = try bridge.tableResizeUpdate(
                revision: revision,
                pointerPosition: Float(rowDividerY + 0.2)
            )
            precondition(rowPreview.kind == YU_STORAGE_TABLE_RESIZE_ROW)
            try bridge.tableResizeCancel(revision: revision)
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

private func runBlockLayoutSelfCheck(path: String) -> Never {
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

private func runShapedViewportSelfCheck(path: String) -> Never {
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
        try bridge.setViewportConfig(
            revision: revision,
            maxWidth: maxWidth,
            lineHeight: Float(shaped.lineHeight),
            defaultAdvance: Float(shaped.defaultAdvance),
            estimatedBlockHeight: Float(shaped.lineHeight),
            overscan: 0.0
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

private func runShapedVerticalSelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let revision = bridge.state.revision
        let size: Float = 14.0
        let maxWidth: Float = 500.0
        let metrics = try bridge.macosFontMetrics(
            revision: revision,
            size: size,
            maxWidth: maxWidth
        )
        try bridge.setViewportConfig(
            revision: revision,
            maxWidth: maxWidth,
            lineHeight: Float(metrics.lineHeight),
            defaultAdvance: Float(metrics.defaultAdvance),
            estimatedBlockHeight: Float(metrics.lineHeight),
            overscan: 0.0
        )
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

private func runVisualViewportSelfCheck(path: String) -> Never {
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
        try bridge.setViewportConfig(
            revision: revision,
            maxWidth: maxWidth,
            lineHeight: Float(shaped.lineHeight),
            defaultAdvance: Float(shaped.defaultAdvance),
            estimatedBlockHeight: Float(shaped.lineHeight),
            overscan: 0.0
        )

        let viewportHeight = max(shaped.lineHeight * 2.0, 1.0)
        let (fullSnapshot, _) = try bridge.macosShapedViewportBlocks(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 1_000.0
        )
        let expectedMaxScroll = max(fullSnapshot.contentHeight - viewportHeight, 0.0)
        precondition(expectedMaxScroll > 0.0)
        let scrollY = min(shaped.lineHeight, expectedMaxScroll)
        let (snapshot, blocks) = try bridge.macosShapedViewportBlocks(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: Float(scrollY),
            viewportHeight: Float(viewportHeight)
        )
        let viewport = NativeVisualViewport(snapshot)
        precondition(viewport.revision == revision)
        precondition(abs(viewport.requestedScrollY - scrollY) < 0.01)
        precondition(abs(viewport.viewportHeight - viewportHeight) < 0.01)
        precondition(abs(viewport.maxScrollY - expectedMaxScroll) < 0.01)
        precondition(snapshot.blockRange.count == blocks.count)
        precondition(!blocks.isEmpty)
        precondition(blocks.allSatisfy { $0.revision == revision && $0.y.isFinite })

        let firstDocumentPoint = NSPoint(
            x: 12.0,
            y: blocks[0].y + min(1.0, blocks[0].height / 2.0)
        )
        let firstViewportPoint = viewport.viewportPoint(forDocumentPoint: firstDocumentPoint)
        let firstRoundTrip = viewport.documentPoint(forViewportPoint: firstViewportPoint)
        precondition(abs(firstRoundTrip.x - firstDocumentPoint.x) < 0.001)
        precondition(abs(firstRoundTrip.y - firstDocumentPoint.y) < 0.001)
        precondition(
            abs(firstViewportPoint.y - (firstDocumentPoint.y - viewport.effectiveScrollY)) < 0.001
        )

        let textView = DocumentTextView(bridge: bridge)
        try textView.setVisualMirrorEnabledForSelfCheck(true)
        textView.setVisualViewportForSelfCheck(viewport)
        let visualDocumentPoint = try unwrapSelfCheck(
            textView.visualMirrorPointForSelfCheck(visualUTF16: 0)
        )
        let visualViewportPoint = try unwrapSelfCheck(
            textView.visualViewportPointForSelfCheck(visualUTF16: 0)
        )
        precondition(
            abs(
                viewport.documentPoint(forViewportPoint: visualViewportPoint).y
                    - visualDocumentPoint.y
            ) < 0.001
        )
        let textViewRoundTrip = try unwrapSelfCheck(
            textView.visualViewportRoundTripForSelfCheck(visualDocumentPoint)
        )
        precondition(abs(textViewRoundTrip.y - visualDocumentPoint.y) < 0.001)

        let sourceEnd = (bridge.source as NSString).length
        try bridge.setSelection(NSRange(location: sourceEnd, length: 0))
        let request = try bridge.macosShapedCaretScrollRequest(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: Float(scrollY),
            viewportHeight: Float(viewportHeight),
            margin: 0.0
        )
        precondition(request.revision == revision)
        precondition(request.sourceUTF16 == UInt64(sourceEnd))
        precondition(request.caretPoint.y >= 0.0)
        precondition(request.currentScrollY == scrollY)
        precondition(request.targetScrollY >= request.currentScrollY)
        precondition(request.targetScrollY <= viewport.maxScrollY + 0.01)
        precondition(request.needsScroll)

        _ = try bridge.insertText("x")
        precondition(textView.visualViewportPointForSelfCheck(visualUTF16: 0) == nil)
        do {
            _ = try bridge.macosShapedViewportBlocks(
                revision: revision,
                size: size,
                maxWidth: maxWidth,
                scrollY: Float(scrollY),
                viewportHeight: Float(viewportHeight)
            )
            preconditionFailure("stale visual viewport unexpectedly succeeded")
        } catch BridgeError.operation(let status) {
            precondition(status == 13)
        }
        do {
            _ = try bridge.macosShapedCaretScrollRequest(
                revision: revision,
                size: size,
                maxWidth: maxWidth,
                scrollY: Float(scrollY),
                viewportHeight: Float(viewportHeight),
                margin: 0.0
            )
            preconditionFailure("stale caret scroll request unexpectedly succeeded")
        } catch BridgeError.operation(let status) {
            precondition(status == 13)
        }
        print(
            "Yu Visual Viewport self-check: document↔viewport scroll transform and "
                + "shaped caret reveal are Revision-bound"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Visual Viewport self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

private func runVisualSceneSelfCheck(path: String) -> Never {
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
        try bridge.setViewportConfig(
            revision: revision,
            maxWidth: maxWidth,
            lineHeight: Float(shaped.lineHeight),
            defaultAdvance: Float(shaped.defaultAdvance),
            estimatedBlockHeight: Float(shaped.lineHeight),
            overscan: 0.0
        )

        let viewportHeight = max(shaped.lineHeight * 2.0, 1.0)
        let (snapshot, primitives) = try bridge.macosVisualScene(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: Float(viewportHeight)
        )
        precondition(snapshot.revision == revision)
        precondition(snapshot.blockRange.count > 0)
        precondition(snapshot.primitiveCount == primitives.count)
        precondition(primitives.count == snapshot.blockRange.count * 2)
        precondition(abs(snapshot.viewportWidth - CGFloat(maxWidth)) < 0.01)
        precondition(snapshot.contentHeight >= viewportHeight)

        var previousBlock: UInt64?
        var previousY: CGFloat?
        for pair in stride(from: 0, to: primitives.count, by: 2).map({ primitives[$0..<$0 + 2] }) {
            let background = pair[pair.startIndex]
            let text = pair[pair.startIndex + 1]
            precondition(background.kind == YU_STORAGE_SCENE_PRIMITIVE_BACKGROUND)
            precondition(text.kind == YU_STORAGE_SCENE_PRIMITIVE_TEXT_BOUNDS)
            precondition(background.revision == revision && text.revision == revision)
            precondition(background.blockIndex == text.blockIndex)
            precondition(background.sourceRange == text.sourceRange)
            precondition(background.rect.origin.x >= 0.0)
            precondition(background.rect.origin.y >= 0.0)
            precondition(background.rect.width > 0.0)
            precondition(background.rect.height > 0.0)
            precondition(text.rect.origin.y >= background.rect.origin.y)
            precondition(text.rect.maxY <= background.rect.maxY + 0.01)
            precondition(text.rect.width >= 0.0)
            precondition(text.rect.width <= background.rect.width + 0.01)
            precondition(background.rect.maxY <= snapshot.contentHeight + 0.01)
            if let previousBlock {
                precondition(background.blockIndex > previousBlock)
            }
            if let previousY {
                precondition(background.rect.origin.y >= previousY)
            }
            previousBlock = background.blockIndex
            previousY = background.rect.origin.y
        }

        _ = try bridge.insertText("x")
        do {
            _ = try bridge.macosVisualScene(
                revision: revision,
                size: size,
                maxWidth: maxWidth,
                scrollY: 0.0,
                viewportHeight: Float(viewportHeight)
            )
            preconditionFailure("stale visual scene unexpectedly succeeded")
        } catch BridgeError.operation(let status) {
            precondition(status == 13)
        }
        print(
            "Yu Visual Scene self-check: Rust-owned primitive order, geometry, source ranges "
                + "and stale Revision rejection are valid"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Visual Scene self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

private func runVisualSceneGlyphSelfCheck(path: String) -> Never {
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
        try bridge.setViewportConfig(
            revision: revision,
            maxWidth: maxWidth,
            lineHeight: Float(shaped.lineHeight),
            defaultAdvance: Float(shaped.defaultAdvance),
            estimatedBlockHeight: Float(shaped.lineHeight),
            overscan: 0.0
        )

        let (snapshot, glyphs) = try bridge.macosVisualSceneGlyphs(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 1_000.0,
            surfaceGeneration: 0
        )
        precondition(snapshot.revision == revision)
        precondition(snapshot.compositionGeneration == bridge.composition.generation)
        precondition(snapshot.frameRevision == revision)
        precondition(snapshot.glyphCount == glyphs.count)
        precondition(snapshot.glyphCount > 0)
        precondition(snapshot.blockRange.count > 0)
        precondition(abs(snapshot.viewportWidth - CGFloat(maxWidth)) < 0.01)

        var previousBlock: UInt64?
        for glyph in glyphs {
            precondition(glyph.revision == revision)
            precondition(glyph.sourceRange.location >= 0)
            precondition(NSMaxRange(glyph.sourceRange) <= (bridge.source as NSString).length)
            precondition(glyph.origin.x.isFinite && glyph.origin.y.isFinite)
            precondition(glyph.advanceX.isFinite && glyph.advanceX >= 0.0)
            precondition(glyph.bounds.origin.x.isFinite && glyph.bounds.origin.y.isFinite)
            precondition(glyph.bounds.width.isFinite && glyph.bounds.height.isFinite)
            precondition(glyph.bounds.width >= 0.0 && glyph.bounds.height >= 0.0)
            precondition(glyph.atlasRect.width >= 0.0 && glyph.atlasRect.height >= 0.0)
            if let previousBlock {
                precondition(glyph.blockIndex >= previousBlock)
            }
            previousBlock = glyph.blockIndex
        }
        precondition(glyphs.contains { $0.page != YU_STORAGE_RENDER_PAGE_NONE })

        _ = try bridge.insertText("x")
        do {
            _ = try bridge.macosVisualSceneGlyphs(
                revision: revision,
                size: size,
                maxWidth: maxWidth,
                scrollY: 0.0,
                viewportHeight: 1_000.0,
                surfaceGeneration: 0
            )
            preconditionFailure("stale retained scene glyphs unexpectedly succeeded")
        } catch BridgeError.operation(let status) {
            precondition(status == 13)
        }
        print(
            "Yu Visual Scene Glyph self-check: retained glyph primitives, atlas placement, "
                + "source block ranges and stale Revision rejection are valid"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Visual Scene Glyph self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

private func runVisualImageSelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let revision = bridge.state.revision
        let images = try bridge.macosVisualImages(revision: revision)
        precondition(images.count >= 2)
        let inlineKind = UInt8(YU_STORAGE_IMAGE_INLINE)
        let referenceKind = UInt8(YU_STORAGE_IMAGE_REFERENCE)
        var sawInline = false
        var sawReference = false
        for image in images {
            precondition(image.revision == revision)
            precondition(image.sourceRange.location >= 0)
            precondition(NSMaxRange(image.sourceRange) <= (bridge.source as NSString).length)
            precondition(NSMaxRange(image.labelRange) <= (bridge.source as NSString).length)
            precondition(image.resourceFingerprint != 0)
            if let destination = image.destinationRange {
                precondition(NSMaxRange(destination) <= (bridge.source as NSString).length)
            }
            if let reference = image.referenceRange {
                precondition(NSMaxRange(reference) <= (bridge.source as NSString).length)
            }
            switch image.kind {
            case inlineKind:
                sawInline = true
                precondition(image.destinationRange != nil)
                precondition(image.referenceRange == nil)
            case referenceKind:
                sawReference = true
                precondition(image.referenceRange != nil)
                precondition(image.destinationRange != nil)
            default:
                preconditionFailure("unknown image metadata kind")
            }
        }
        precondition(sawInline && sawReference)

        _ = try bridge.insertText("x")
        do {
            _ = try bridge.macosVisualImages(revision: revision)
            preconditionFailure("stale image metadata unexpectedly succeeded")
        } catch BridgeError.operation(let status) {
            precondition(status == StorageStatus.staleRevision)
        }
        print(
            "Yu Visual Image self-check: source ranges, reference resolution, "
                + "resource fingerprints and stale Revision rejection are valid"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Visual Image self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

private func runVisualRenderPlanSelfCheck(path: String) -> Never {
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
        try bridge.setViewportConfig(
            revision: revision,
            maxWidth: maxWidth,
            lineHeight: Float(shaped.lineHeight),
            defaultAdvance: Float(shaped.defaultAdvance),
            estimatedBlockHeight: Float(shaped.lineHeight),
            overscan: 0.0
        )

        let (snapshot, commands, pages, damage) = try bridge.macosVisualRenderPlan(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 1_000.0
        )
        precondition(snapshot.revision == revision)
        precondition(snapshot.compositionGeneration == bridge.composition.generation)
        precondition(snapshot.commandCount == commands.count)
        precondition(snapshot.uploadCount == pages.count)
        precondition(snapshot.damageCount == damage.count)
        precondition(snapshot.commandCount > 0)
        precondition(snapshot.uploadCount > 0)
        precondition(snapshot.damageCount > 0)
        precondition(snapshot.blockRange.count > 0)
        precondition(abs(snapshot.viewportWidth - CGFloat(maxWidth)) < 0.01)

        let expectsImage = bridge.source.contains("![")
        var previousBlock: UInt64?
        var previousSourceEnd: Int = 0
        var sawFill = false
        var sawGlyph = false
        var sawImage = false
        let fillKind = UInt8(YU_STORAGE_RENDER_COMMAND_FILL_RECT)
        let glyphKind = UInt8(YU_STORAGE_RENDER_COMMAND_GLYPH)
        let imageKind = UInt8(YU_STORAGE_RENDER_COMMAND_IMAGE)
        for command in commands {
            precondition(command.revision == revision)
            precondition(command.bounds.origin.x.isFinite)
            precondition(command.bounds.origin.y.isFinite)
            precondition(command.bounds.width.isFinite && command.bounds.height.isFinite)
            precondition(command.bounds.width >= 0.0 && command.bounds.height >= 0.0)
            precondition(command.origin.x.isFinite && command.origin.y.isFinite)
            switch command.kind {
            case fillKind:
                sawFill = true
                precondition(command.page == YU_STORAGE_RENDER_PAGE_NONE)
                precondition(command.atlasRect.width == 0.0 && command.atlasRect.height == 0.0)
                precondition(command.advanceX == 0.0)
            case glyphKind:
                sawGlyph = true
                precondition(command.advanceX.isFinite && command.advanceX >= 0.0)
                precondition(command.atlasRect.width >= 0.0 && command.atlasRect.height >= 0.0)
            case imageKind:
                sawImage = true
                precondition(command.page == YU_STORAGE_RENDER_PAGE_NONE)
                precondition(command.atlasRect.width == 0.0 && command.atlasRect.height == 0.0)
                precondition(command.resource != 0)
                precondition(command.bounds.width > 0.0 && command.bounds.height > 0.0)
            default:
                preconditionFailure("unknown visual render command kind")
            }
            if let previousBlock, command.blockIndex != previousBlock {
                precondition(command.sourceRange.location >= previousSourceEnd)
            }
            previousSourceEnd = max(previousSourceEnd, NSMaxRange(command.sourceRange))
            if let previousBlock {
                precondition(command.blockIndex >= previousBlock)
            }
            previousBlock = command.blockIndex
            if command.page != YU_STORAGE_RENDER_PAGE_NONE {
                precondition(Int(command.page) < pages.count)
            }
        }
        precondition(sawFill)
        precondition(sawGlyph)
        precondition(!expectsImage || sawImage)

        precondition(pages.dropFirst().enumerated().allSatisfy { offset, page in
            page.page > pages[offset].page
        })
        precondition(Set(pages.map(\.page)).count == pages.count)
        precondition(pages.allSatisfy { page in
            page.revision == revision
                && page.width > 0
                && page.height > 0
                && page.fingerprint != 0
        })
        precondition(damage.allSatisfy { item in
            item.revision == revision
                && item.rect.origin.x.isFinite
                && item.rect.origin.y.isFinite
                && item.rect.width.isFinite
                && item.rect.height.isFinite
                && item.rect.width >= 0.0
                && item.rect.height >= 0.0
        })

        let compositionRange = (bridge.source as NSString).range(of: "粗体")
        precondition(compositionRange.location != NSNotFound)
        try bridge.beginComposition(
            replacementRange: compositionRange,
            preedit: "日本🙂",
            selection: NSRange(location: 0, length: 2)
        )
        let initialComposition = bridge.composition
        let (compositionPlan, compositionCommands, _, _) = try bridge.macosVisualRenderPlan(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 1_000.0
        )
        precondition(compositionPlan.revision == revision)
        precondition(compositionPlan.compositionGeneration == initialComposition.generation)
        precondition(compositionPlan.commandCount == compositionCommands.count)
        precondition(compositionPlan.commandCount > 0)
        precondition(compositionCommands.allSatisfy { $0.revision == revision })
        let (compositionDecoration, compositionCaret, compositionSelection) =
            try bridge.macosVisualDecorations(
                revision: revision,
                compositionGeneration: initialComposition.generation,
                size: size,
                maxWidth: maxWidth,
                scrollY: 0.0,
                viewportHeight: 1_000.0
            )
        precondition(compositionDecoration.revision == revision)
        precondition(compositionDecoration.compositionGeneration == initialComposition.generation)
        precondition(compositionDecoration.caretPresent)
        precondition(compositionCaret.present)
        precondition(compositionDecoration.selectionCount == compositionSelection.count)
        precondition(!compositionSelection.isEmpty)

        try bridge.updateComposition(
            preedit: "日本語",
            selection: NSRange(location: 1, length: 2)
        )
        let updatedComposition = bridge.composition
        precondition(updatedComposition.generation != initialComposition.generation)
        let (updatedPlan, _, _, _) = try bridge.macosVisualRenderPlan(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 1_000.0
        )
        precondition(updatedPlan.revision == revision)
        precondition(updatedPlan.compositionGeneration == updatedComposition.generation)
        let (updatedDecoration, updatedCaret, _) = try bridge.macosVisualDecorations(
            revision: revision,
            compositionGeneration: updatedComposition.generation,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 1_000.0
        )
        precondition(updatedDecoration.compositionGeneration == updatedComposition.generation)
        precondition(updatedDecoration.caretPresent && updatedCaret.present)
        try bridge.cancelComposition()
        precondition(!bridge.composition.active)
        precondition(bridge.state.revision == revision)

        _ = try bridge.insertText("x")
        do {
            _ = try bridge.macosVisualRenderPlan(
                revision: revision,
                size: size,
                maxWidth: maxWidth,
                scrollY: 0.0,
                viewportHeight: 1_000.0
            )
            preconditionFailure("stale visual render plan unexpectedly succeeded")
        } catch BridgeError.operation(let status) {
            precondition(status == 13)
        }
        print(
            "Yu Visual Render Plan self-check: block fills, shaped glyph commands, atlas page fingerprints, "
                + "damage, composition generation handoff and stale Revision rejection are valid"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu Visual Render Plan self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

private func runMacosRenderHostSelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let sourceBefore = bridge.source
        let revision = bridge.state.revision
        let size: Float = 14.0
        let maxWidth: Float = 500.0
        let shaped = try bridge.macosBlockLayout(
            revision: revision,
            blockIndex: 2,
            size: size,
            maxWidth: maxWidth
        )
        try bridge.setViewportConfig(
            revision: revision,
            maxWidth: maxWidth,
            lineHeight: Float(shaped.lineHeight),
            defaultAdvance: Float(shaped.defaultAdvance),
            estimatedBlockHeight: Float(shaped.lineHeight),
            overscan: 0.0
        )

        let first = try bridge.macosRenderHostFrame(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 240.0,
            surfaceGeneration: 0
        )
        precondition(first.revision == revision)
        precondition(first.compositionGeneration == bridge.composition.generation)
        precondition(first.frameRevision == revision)
        precondition(first.surfaceGeneration == 0)
        precondition(first.frameSerial == 1)
        precondition(first.published)
        precondition(first.commandCount > 0)
        precondition(first.uploadCount > 0)
        precondition(first.damageCount > 0)
        precondition(first.atlasPageCount > 0)
        precondition(first.atlasGlyphCount > 0)
        precondition(first.atlasBytes > 0)

        let repeated = try bridge.macosRenderHostFrame(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 240.0,
            surfaceGeneration: 0
        )
        precondition(repeated.compositionGeneration == bridge.composition.generation)
        precondition(repeated.frameSerial > first.frameSerial)
        precondition(repeated.atlasPageCount == first.atlasPageCount)
        precondition(repeated.atlasGlyphCount == first.atlasGlyphCount)
        precondition(repeated.uploadCount == 0)

        var tableIndex: UInt64?
        let blockCount = try bridge.projectionBlockCount(revision: revision)
        if blockCount > 0 {
            for index in 0..<blockCount {
                let (block, _) = try bridge.projectedBlock(
                    revision: revision,
                    blockIndex: UInt64(index)
                )
                if block.projectionKind == UInt8(YU_STORAGE_PROJECTION_TABLE) {
                    tableIndex = UInt64(index)
                    break
                }
            }
        }
        if let tableIndex {
            let tableHit = try bridge.macosTableResizeBegin(
                revision: revision,
                blockIndex: tableIndex,
                size: size,
                maxWidth: maxWidth,
                point: CGPoint(x: 0.0, y: 0.5),
                tolerance: maxWidth,
                pointerPosition: 0.0
            )
            precondition(tableHit.kind == YU_STORAGE_TABLE_RESIZE_COLUMN)
            let tablePreview = try bridge.tableResizeUpdate(
                revision: revision,
                pointerPosition: tableHit.position + 1.0
            )
            precondition(tablePreview.revision == revision)
            precondition(tablePreview.blockIndex == tableIndex)
            precondition(tablePreview.kind == YU_STORAGE_TABLE_RESIZE_COLUMN)
            precondition(abs(tablePreview.delta - (tableHit.position + 1.0)) < 0.01)
            let resizedTableFrame = try bridge.macosRenderHostFrame(
                revision: revision,
                size: size,
                maxWidth: maxWidth,
                scrollY: 0.0,
                viewportHeight: 240.0,
                surfaceGeneration: 0
            )
            precondition(resizedTableFrame.frameSerial > repeated.frameSerial)
            let finishedTable = try bridge.tableResizeFinish(revision: revision)
            precondition(finishedTable == tablePreview)
            try bridge.tableResizeCancel(revision: revision)
            let restoredTableFrame = try bridge.macosRenderHostFrame(
                revision: revision,
                size: size,
                maxWidth: maxWidth,
                scrollY: 0.0,
                viewportHeight: 240.0,
                surfaceGeneration: 0
            )
            precondition(restoredTableFrame.frameSerial > resizedTableFrame.frameSerial)
        }

        let strong = (sourceBefore as NSString).range(of: "粗体")
        precondition(strong.location != NSNotFound)
        try bridge.beginComposition(
            replacementRange: strong,
            preedit: "日本🙂",
            selection: NSRange(location: 2, length: 0)
        )
        let composed = try bridge.macosRenderHostFrame(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 240.0,
            surfaceGeneration: 0
        )
        precondition(composed.revision == revision)
        precondition(composed.compositionGeneration == bridge.composition.generation)
        precondition(composed.frameSerial > repeated.frameSerial)
        // The canonical projection contains two visible CJK glyphs here;
        // the transient Japanese/emoji preedit contributes a different shaped
        // glyph sequence to the same persistent Metal publication.
        precondition(composed.commandCount != repeated.commandCount)
        precondition(composed.atlasGlyphCount >= repeated.atlasGlyphCount)
        try bridge.cancelComposition()
        let cancelled = try bridge.macosRenderHostFrame(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 240.0,
            surfaceGeneration: 0
        )
        precondition(cancelled.revision == revision)
        precondition(cancelled.compositionGeneration == bridge.composition.generation)
        precondition(cancelled.frameSerial > composed.frameSerial)
        precondition(cancelled.commandCount == repeated.commandCount)

        let resized = try bridge.macosRenderHostFrame(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 12.0,
            viewportHeight: 180.0,
            surfaceGeneration: 1
        )
        precondition(resized.compositionGeneration == bridge.composition.generation)
        precondition(resized.surfaceGeneration == 1)
        precondition(abs(resized.scrollY - 12.0) < 0.01)
        precondition(abs(resized.viewportHeight - 180.0) < 0.01)
        precondition(resized.frameSerial > repeated.frameSerial)

        _ = try bridge.insertText("!")
        do {
            _ = try bridge.macosRenderHostFrame(
                revision: revision,
                size: size,
                maxWidth: maxWidth,
                scrollY: 0.0,
                viewportHeight: 240.0,
                surfaceGeneration: 1
            )
            preconditionFailure("stale render host frame unexpectedly succeeded")
        } catch BridgeError.operation(let status) {
            precondition(status == 13)
        }

        let next = try bridge.macosRenderHostFrame(
            revision: bridge.state.revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: 240.0,
            surfaceGeneration: 1
        )
        precondition(next.compositionGeneration == bridge.composition.generation)
        precondition(next.revision == bridge.state.revision)
        precondition(next.frameRevision == bridge.state.revision)
        precondition(next.surfaceGeneration == 1)
        precondition(next.frameSerial > resized.frameSerial)
        print(
            "Yu macOS Render Host self-check: persistent CoreText/atlas publication, "
                + "table resize preview lifecycle, viewport resize, surface generation and "
                + "stale Revision rejection are valid"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu macOS Render Host self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

private func installMacosImageSelfCheckFixture(at markdownPath: String) throws -> URL? {
    let markdownURL = URL(fileURLWithPath: markdownPath)
    let imageURL = markdownURL.deletingLastPathComponent()
        .appendingPathComponent("assets", isDirectory: true)
        .appendingPathComponent("yu-logo.png")
    let fileManager = FileManager.default
    guard !fileManager.fileExists(atPath: imageURL.path) else {
        return nil
    }
    try fileManager.createDirectory(
        at: imageURL.deletingLastPathComponent(),
        withIntermediateDirectories: true
    )
    let png = Data([
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82,
        0, 0, 0, 1, 0, 0, 0, 1, 8, 4, 0, 0, 0, 181, 28, 12, 2,
        0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 248, 15,
        0, 1, 5, 1, 1, 39, 24, 227, 102, 0, 0, 0, 0, 73, 69, 78,
        68, 174, 66, 96, 130
    ])
    try png.write(to: imageURL, options: .atomic)
    return imageURL
}

private func removeMacosImageSelfCheckFixture(_ imageURL: URL?) {
    guard let imageURL else { return }
    try? FileManager.default.removeItem(at: imageURL)
    try? FileManager.default.removeItem(at: imageURL.deletingLastPathComponent())
}

private func runMacosRenderHostSurfaceSelfCheck(path: String) -> Never {
    do {
        let imageFixture = try installMacosImageSelfCheckFixture(at: path)
        let bridge = try StorageBridge(path: path)
        let sourceBefore = bridge.source
        let revision = bridge.state.revision
        let size: Float = 14.0
        let maxWidth: Float = 500.0
        let viewportHeight: Float = 240.0
        let shaped = try bridge.macosBlockLayout(
            revision: revision,
            blockIndex: 2,
            size: size,
            maxWidth: maxWidth
        )
        try bridge.setViewportConfig(
            revision: revision,
            maxWidth: maxWidth,
            lineHeight: Float(shaped.lineHeight),
            defaultAdvance: Float(shaped.defaultAdvance),
            estimatedBlockHeight: Float(shaped.lineHeight),
            overscan: 0.0
        )

        let application = NSApplication.shared
        application.setActivationPolicy(.regular)
        let frame = NSRect(
            x: 0.0,
            y: 0.0,
            width: CGFloat(maxWidth),
            height: CGFloat(viewportHeight)
        )
        let window = NSWindow(
            contentRect: frame,
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        let view = NSView(frame: frame)
        window.contentView = view
        window.center()
        window.makeKeyAndOrderFront(nil)
        application.activate(ignoringOtherApps: true)
        window.displayIfNeeded()
        defer {
            try? bridge.macosRenderHostSurfaceDetach()
            try? bridge.macosRenderHostSurfaceDetach()
            window.orderOut(nil)
            window.close()
        }

        let rawView = Unmanaged.passUnretained(view).toOpaque()
        let first = try bridge.macosRenderHostSurfaceSubmit(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: viewportHeight,
            surfaceWidth: Double(maxWidth),
            surfaceHeight: Double(viewportHeight),
            scale: 2.0,
            view: rawView
        )
        precondition(first.revision == revision)
        precondition(first.surfaceGeneration == 0)
        precondition(first.frameSerial > 0)
        precondition(first.uploadedPages > 0)
        precondition(first.commandCount > 0)
        precondition(first.damageCount > 0)
        precondition(first.atlasPageCount > 0)
        precondition(first.submitted)

        let repeated = try bridge.macosRenderHostSurfaceSubmit(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: viewportHeight,
            surfaceWidth: Double(maxWidth),
            surfaceHeight: Double(viewportHeight),
            scale: 2.0,
            view: rawView
        )
        precondition(repeated.compositionGeneration == bridge.composition.generation)
        precondition(repeated.surfaceGeneration == first.surfaceGeneration)
        precondition(repeated.frameSerial > first.frameSerial)
        precondition(repeated.uploadedPages == 0)
        precondition(repeated.submitted)

        var imagePublicationReady = false
        if sourceBefore.contains("![") {
            var imageReady = repeated
            for _ in 0..<20 {
                let candidate = try bridge.macosRenderHostSurfaceSubmit(
                    revision: revision,
                    size: size,
                    maxWidth: maxWidth,
                    scrollY: 0.0,
                    viewportHeight: viewportHeight,
                    surfaceWidth: Double(maxWidth),
                    surfaceHeight: Double(viewportHeight),
                    scale: 2.0,
                    view: rawView
                )
                imageReady = candidate
                if candidate.imageResourceCount > 0 {
                    break
                }
                RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.05))
            }
            precondition(imageReady.imageRequestCount > 0)
            precondition(imageReady.imageCandidateCount >= imageReady.imageRequestCount)
            precondition(
                imageReady.imageVisibleCandidateCount
                    + imageReady.imageOverscanCandidateCount
                    == imageReady.imageCandidateCount
            )
            precondition(imageReady.imageDuplicateCount <= imageReady.imageCandidateCount)
            precondition(imageReady.imageResourceCount > 0)
            precondition(imageReady.imageFailureCount == 0)
            precondition(imageReady.uploadedImages > 0 || repeated.imageResourceCount > 0)

            // Move past the fixture's visible block. The host must drop the
            // now-unreferenced Metal texture instead of retaining every image
            // visited during a long document scroll.
            let offscreen = try bridge.macosRenderHostSurfaceSubmit(
                revision: revision,
                size: size,
                maxWidth: maxWidth,
                scrollY: 10_000.0,
                viewportHeight: viewportHeight,
                surfaceWidth: Double(maxWidth),
                surfaceHeight: Double(viewportHeight),
                scale: 2.0,
                view: rawView
            )
            precondition(offscreen.imageResourceCount == 0)
            precondition(offscreen.imageAtlasEvictionCount > imageReady.imageAtlasEvictionCount)
            imagePublicationReady = true
        }
        // The self-check exits explicitly below, so clean the temporary
        // fixture before that exit instead of relying on Swift defer.
        removeMacosImageSelfCheckFixture(imageFixture)

        let strong = (sourceBefore as NSString).range(of: "粗体")
        precondition(strong.location != NSNotFound)
        try bridge.beginComposition(
            replacementRange: strong,
            preedit: "日本🙂",
            selection: NSRange(location: 2, length: 0)
        )
        let composed = try bridge.macosRenderHostSurfaceSubmit(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: viewportHeight,
            surfaceWidth: Double(maxWidth),
            surfaceHeight: Double(viewportHeight),
            scale: 2.0,
            view: rawView
        )
        precondition(composed.revision == revision)
        precondition(composed.compositionGeneration == bridge.composition.generation)
        precondition(composed.frameSerial > repeated.frameSerial)
        precondition(composed.commandCount != repeated.commandCount)
        precondition(composed.submitted)
        try bridge.cancelComposition()
        let cancelled = try bridge.macosRenderHostSurfaceSubmit(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: viewportHeight,
            surfaceWidth: Double(maxWidth),
            surfaceHeight: Double(viewportHeight),
            scale: 2.0,
            view: rawView
        )
        precondition(cancelled.revision == revision)
        precondition(cancelled.compositionGeneration == bridge.composition.generation)
        precondition(cancelled.frameSerial > composed.frameSerial)
        precondition(cancelled.commandCount == repeated.commandCount)
        precondition(cancelled.submitted)

        let resized = try bridge.macosRenderHostSurfaceSubmit(
            revision: revision,
            size: size,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: viewportHeight,
            surfaceWidth: Double(maxWidth + 20.0),
            surfaceHeight: Double(viewportHeight + 20.0),
            scale: 2.0,
            view: rawView
        )
        precondition(resized.compositionGeneration == bridge.composition.generation)
        precondition(resized.surfaceGeneration == first.surfaceGeneration + 1)
        precondition(resized.frameSerial > repeated.frameSerial)
        precondition(resized.uploadedPages == 0)
        precondition(resized.submitted)

        let largerSize: Float = 16.0
        let largerShaped = try bridge.macosBlockLayout(
            revision: revision,
            blockIndex: 2,
            size: largerSize,
            maxWidth: maxWidth
        )
        try bridge.setViewportConfig(
            revision: revision,
            maxWidth: maxWidth,
            lineHeight: Float(largerShaped.lineHeight),
            defaultAdvance: Float(largerShaped.defaultAdvance),
            estimatedBlockHeight: Float(largerShaped.lineHeight),
            overscan: 0.0
        )
        let resizedFont = try bridge.macosRenderHostSurfaceSubmit(
            revision: revision,
            size: largerSize,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: viewportHeight,
            surfaceWidth: Double(maxWidth + 20.0),
            surfaceHeight: Double(viewportHeight + 20.0),
            scale: 2.0,
            view: rawView
        )
        precondition(resizedFont.surfaceGeneration == resized.surfaceGeneration)
        precondition(resizedFont.frameSerial > resized.frameSerial)
        precondition(resizedFont.submitted)

        _ = try bridge.insertText("!")
        do {
            _ = try bridge.macosRenderHostSurfaceSubmit(
                revision: revision,
                size: largerSize,
                maxWidth: maxWidth,
                scrollY: 0.0,
                viewportHeight: viewportHeight,
                surfaceWidth: Double(maxWidth + 20.0),
                surfaceHeight: Double(viewportHeight + 20.0),
                scale: 2.0,
                view: rawView
            )
            preconditionFailure("stale Metal surface submission unexpectedly succeeded")
        } catch BridgeError.operation(let status) {
            precondition(status == 13)
        }

        let next = try bridge.macosRenderHostSurfaceSubmit(
            revision: bridge.state.revision,
            size: largerSize,
            maxWidth: maxWidth,
            scrollY: 0.0,
            viewportHeight: viewportHeight,
            surfaceWidth: Double(maxWidth + 20.0),
            surfaceHeight: Double(viewportHeight + 20.0),
            scale: 2.0,
            view: rawView
        )
        precondition(next.revision == bridge.state.revision)
        precondition(next.surfaceGeneration == resized.surfaceGeneration)
        precondition(next.frameSerial > resizedFont.frameSerial)
        precondition(next.submitted)
        print(
            "Yu macOS Metal surface self-check: persistent CAMetalLayer attachment, repeated submit, "
                + "resize/generation, atlas reuse and stale Revision rejection are valid"
                + (imagePublicationReady ? "; ImageIO publication reached ready Metal texture" : "")
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu macOS Metal surface self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

private func runMacosRenderHostLifecycleSelfCheck(path: String) -> Never {
    do {
        let bridge = try StorageBridge(path: path)
        let application = NSApplication.shared
        application.setActivationPolicy(.regular)
        let initialFrame = NSRect(x: 0.0, y: 0.0, width: 500.0, height: 240.0)
        let window = NSWindow(
            contentRect: initialFrame,
            styleMask: [.titled, .closable, .resizable],
            backing: .buffered,
            defer: false
        )
        let root = NSView(frame: initialFrame)
        let surfaceView = MacosSurfaceHostView(frame: initialFrame)
        let scrollView = NSScrollView(frame: initialFrame)
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = false
        scrollView.documentView = NSView(
            frame: NSRect(x: 0.0, y: 0.0, width: initialFrame.width, height: 1200.0)
        )
        root.addSubview(surfaceView)
        root.addSubview(scrollView)
        window.contentView = root
        window.center()
        window.makeKeyAndOrderFront(nil)
        application.activate(ignoringOtherApps: true)

        let coordinator = MacosSurfaceHostCoordinator(bridge: bridge, fontSize: 14.0)
        var errors: [String] = []
        coordinator.onError = { error in errors.append(error.localizedDescription) }
        coordinator.bind(
            surfaceView: surfaceView,
            scrollView: scrollView,
            fontSize: 14.0
        )
        precondition(surfaceView.isHidden)
        surfaceView.onWindowStateChange = { attached in
            if attached {
                coordinator.scheduleSubmit()
            } else {
                coordinator.detach()
            }
        }
        surfaceView.onGeometryChange = {
            coordinator.scheduleSubmit()
        }
        scrollView.contentView.postsBoundsChangedNotifications = true
        let observer = NotificationCenter.default.addObserver(
            forName: NSView.boundsDidChangeNotification,
            object: scrollView.contentView,
            queue: .main
        ) { _ in
            coordinator.scheduleSubmit()
        }
        defer {
            NotificationCenter.default.removeObserver(observer)
            coordinator.detach()
            window.orderOut(nil)
            window.close()
        }

        window.displayIfNeeded()
        let first = try unwrapSelfCheck(coordinator.submitNow())
        precondition(first.surfaceGeneration == 0)
        precondition(first.submitted)
        precondition(coordinator.isAttached)
        precondition(surfaceView.nativeContentVisible)
        precondition(surfaceView.hitTest(NSPoint(x: 12.0, y: 12.0)) == nil)
        precondition(
            coordinator.hasCurrentPublication(
                revision: bridge.state.revision,
                compositionGeneration: bridge.composition.generation
            )
        )

        let resizedFrame = NSRect(x: 0.0, y: 0.0, width: 540.0, height: 280.0)
        root.setFrameSize(resizedFrame.size)
        surfaceView.frame = resizedFrame
        scrollView.frame = resizedFrame
        window.setContentSize(resizedFrame.size)
        precondition(
            !coordinator.hasCurrentPublication(
                revision: bridge.state.revision,
                compositionGeneration: bridge.composition.generation
            )
        )
        window.displayIfNeeded()
        let resized = try unwrapSelfCheck(coordinator.submitNow())
        precondition(resized.surfaceGeneration == first.surfaceGeneration + 1)
        precondition(resized.frameSerial > first.frameSerial)
        precondition(resized.submitted)
        precondition(
            coordinator.hasCurrentPublication(
                revision: bridge.state.revision,
                compositionGeneration: bridge.composition.generation
            )
        )

        scrollView.contentView.setBoundsOrigin(NSPoint(x: 0.0, y: 120.0))
        precondition(
            !coordinator.hasCurrentPublication(
                revision: bridge.state.revision,
                compositionGeneration: bridge.composition.generation
            )
        )
        let scrolled = try unwrapSelfCheck(coordinator.submitNow())
        precondition(scrolled.surfaceGeneration == resized.surfaceGeneration)
        precondition(scrolled.frameSerial > resized.frameSerial)
        precondition(scrolled.submitted)
        precondition(
            coordinator.hasCurrentPublication(
                revision: bridge.state.revision,
                compositionGeneration: bridge.composition.generation
            )
        )

        _ = try bridge.insertText("!")
        precondition(
            !coordinator.hasCurrentPublication(
                revision: bridge.state.revision,
                compositionGeneration: bridge.composition.generation
            )
        )
        let edited = try unwrapSelfCheck(coordinator.submitNow())
        precondition(edited.revision == bridge.state.revision)
        precondition(edited.frameSerial > scrolled.frameSerial)
        precondition(edited.submitted)
        precondition(
            coordinator.hasCurrentPublication(
                revision: bridge.state.revision,
                compositionGeneration: bridge.composition.generation
            )
        )

        surfaceView.removeFromSuperview()
        precondition(!coordinator.isAttached)
        precondition(!surfaceView.nativeContentVisible)
        coordinator.detach()
        precondition(errors.isEmpty, errors.joined(separator: "; "))
        print(
            "Yu macOS surface lifecycle self-check: product NSView attach, resize, scroll, "
                + "edit revision and close detach are valid"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu macOS surface lifecycle self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

private func unwrapSelfCheck<T>(_ value: T?) throws -> T {
    guard let value else {
        throw BridgeError.operation(14)
    }
    return value
}

private func runMacosTableResizeCoordinatorSelfCheck(path: String) -> Never {
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
        let metrics = try bridge.macosFontMetrics(
            revision: revision,
            size: size,
            maxWidth: maxWidth
        )
        try bridge.setViewportConfig(
            revision: revision,
            maxWidth: maxWidth,
            lineHeight: Float(metrics.lineHeight),
            defaultAdvance: Float(metrics.defaultAdvance),
            estimatedBlockHeight: Float(metrics.lineHeight),
            overscan: 0.0
        )
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
            frame: NSRect(x: 0.0, y: 0.0, width: 500.0, height: 240.0)
        )
        let scrollView = NSScrollView(
            frame: NSRect(x: 0.0, y: 0.0, width: 500.0, height: 240.0)
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
        precondition(coordinator.beginTableResize(at: dividerPoint))
        precondition(coordinator.tableResizeActiveForSelfCheck)
        precondition(
            coordinator.updateTableResize(
                at: NSPoint(x: dividerPoint.x + 1.0, y: dividerPoint.y)
            )
        )
        precondition(coordinator.finishTableResize())
        precondition(!coordinator.tableResizeActiveForSelfCheck)

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
                + "mouse update/finish/cancel, stale revision reset and headless surface fallback are valid"
        )
        exit(EXIT_SUCCESS)
    } catch {
        fputs("Yu macOS table resize coordinator self-check failed: \(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

private func runCompositionProjectionSelfCheck(path: String) -> Never {
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

private func runAccessibilitySelfCheck(path: String) -> Never {
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

        _ = try bridge.insertText("\n")
        if let staleCandidate = actionChildren.first {
            precondition(staleCandidate.accessibilityLabel == nil)
        }
        textView.refreshFromRust()
        let nextRevision = bridge.state.revision
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

let app = NSApplication.shared
if let flag = CommandLine.arguments.firstIndex(of: "--selection-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runSelectionSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--undo-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runUndoSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--projection-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runProjectionSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--projection-hit-test-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runProjectionHitTestSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--shaped-projection-hit-test-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runShapedProjectionHitTestSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--visual-mirror-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runVisualMirrorSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--visual-decoration-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runVisualDecorationSelfCheck(path: CommandLine.arguments[flag + 1])
}
if CommandLine.arguments.contains("--visual-render-state-self-check") {
    runVisualRenderStateSelfCheck()
}
if let flag = CommandLine.arguments.firstIndex(of: "--visual-ime-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runVisualIMESelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--composition-hit-test-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runCompositionHitTestSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--block-projection-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runBlockProjectionSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--block-layout-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runBlockLayoutSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--shaped-viewport-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runShapedViewportSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--shaped-vertical-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runShapedVerticalSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--visual-viewport-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runVisualViewportSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--visual-scene-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runVisualSceneSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--visual-image-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runVisualImageSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--visual-scene-glyph-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runVisualSceneGlyphSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--visual-render-plan-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runVisualRenderPlanSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--macos-render-host-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runMacosRenderHostSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--macos-render-host-surface-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runMacosRenderHostSurfaceSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--macos-render-host-lifecycle-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runMacosRenderHostLifecycleSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--macos-table-resize-coordinator-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runMacosTableResizeCoordinatorSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--composition-projection-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runCompositionProjectionSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--clipboard-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runClipboardSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--accessibility-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runAccessibilitySelfCheck(path: CommandLine.arguments[flag + 1])
}
let delegate = AppDelegate()
app.setActivationPolicy(.regular)
app.delegate = delegate
app.run()
