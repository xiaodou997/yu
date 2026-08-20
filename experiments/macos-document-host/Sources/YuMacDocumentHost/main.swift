import AppKit
import Darwin
import UniformTypeIdentifiers
import YuStorageFFI

extension NSPasteboard.PasteboardType {
    /// The de-facto Markdown pasteboard UTI used by macOS Markdown editors.
    /// The payload is always the canonical source selected in Rust, never the
    /// TextKit projection (which may contain a transient IME overlay).
    static let yuMarkdown = NSPasteboard.PasteboardType("net.daringfireball.markdown")
    /// Semantic HTML generated from the same Rust-owned source selection.
    static let yuHTML = NSPasteboard.PasteboardType(UTType.html.identifier)
}

enum StorageStatus {
    static let ok: Int32 = 0
    static let staleRevision: Int32 = 13
    static let externalChange: Int32 = 4
    static let unsavedChanges: Int32 = 5
    static let htmlImportRejected: Int32 = 18
    static let invalidSelection: Int32 = 14
    static let invalidViewport: Int32 = 20
    static let tableResizeNotActive: Int32 = 22
}

enum DiskState: UInt8 {
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

struct NativeStorageState {
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

    /// A deliberately conservative state used only when the native host is
    /// losing access to an already-open Rust session.  Treating the document
    /// as dirty/changed disables destructive menu actions and keeps the
    /// source TextKit fallback alive; it is safer than aborting the process
    /// from an FFI status query.
    init(unavailable: Void) {
        revision = 0
        savedRevision = 0
        dirty = true
        disk = .changed
        bom = false
        closeState = 0
    }
}

struct NativeSelection {
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

struct NativeSelectionEndpoints {
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

struct NativeProjectionCaret {
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

struct NativeProjectionSelection {
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

struct NativeProjectionSourceCaret {
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

struct NativeProjectionSourceSelection {
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

struct NativeProjectionHit {
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

struct NativeTaskCheckboxHit: Equatable {
    let revision: UInt64
    let blockIndex: UInt64
    let markerRange: NSRange
    let bounds: CGRect

    init(_ value: YuStorageTaskCheckboxHit) {
        revision = value.revision
        blockIndex = value.block_index
        markerRange = NSRange(
            location: Int(value.marker_start_utf16),
            length: Int(value.marker_end_utf16 - value.marker_start_utf16)
        )
        bounds = CGRect(
            x: CGFloat(value.x),
            y: CGFloat(value.y),
            width: CGFloat(value.width),
            height: CGFloat(value.height)
        )
    }
}

struct NativeProjectionBlock {
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

struct NativeTableResizeCommit: Equatable {
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

/// Read-only, Revision-bound metadata for one visible table column divider.
/// The descriptor is suitable for an ephemeral native Accessibility element,
/// but it does not own a resize session or any Markdown source.
struct NativeTableResizeAccessibilityDivider: Equatable {
    let revision: UInt64
    let blockIndex: UInt64
    let kind: UInt8
    let index: UInt64
    let columnCount: UInt64
    let rect: CGRect
    let tableSourceRange: NSRange

    init(
        revision: UInt64,
        blockIndex: UInt64,
        kind: UInt8,
        index: UInt64,
        columnCount: UInt64,
        rect: CGRect,
        tableSourceRange: NSRange
    ) {
        self.revision = revision
        self.blockIndex = blockIndex
        self.kind = kind
        self.index = index
        self.columnCount = columnCount
        self.rect = rect
        self.tableSourceRange = tableSourceRange
    }

    init(_ value: YuStorageTableResizeAccessibilityDivider) {
        revision = value.revision
        blockIndex = value.block_index
        kind = value.kind
        index = value.index
        columnCount = value.column_count
        rect = CGRect(
            x: CGFloat(value.x),
            y: CGFloat(value.y),
            width: CGFloat(value.width),
            height: CGFloat(value.height)
        )
        tableSourceRange = NSRange(
            location: Int(value.table_source_start_utf16),
            length: Int(value.table_source_end_utf16 - value.table_source_start_utf16)
        )
    }
}

struct NativeBlockLayout {
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

struct NativeMacosFontMetrics {
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

struct NativeBlockCaret {
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

struct NativeShapedViewportBlock {
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

struct NativeShapedViewportSnapshot {
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






struct NativeVisualImage {
    let revision: UInt64
    let blockIndex: UInt64
    let sourceRange: NSRange
    let labelRange: NSRange
    let destinationRange: NSRange?
    let referenceRange: NSRange?
    let resourceFingerprint: UInt64
    let kind: UInt8
    let resourceStatus: UInt8

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
        resourceStatus = value.resource_status
    }

    private static func optionalRange(start: UInt64, end: UInt64) -> NSRange? {
        guard start != UInt64.max, end != UInt64.max, end >= start else {
            return nil
        }
        return NSRange(location: Int(start), length: Int(end - start))
    }
}

struct NativeVisualRenderPlanSnapshot {
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
    let embeddedCommandCount: Int
    let embeddedUploadCount: Int
    let embeddedUploadBytes: Int

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
        embeddedCommandCount = Int(value.embedded_command_count)
        embeddedUploadCount = Int(value.embedded_upload_count)
        embeddedUploadBytes = Int(value.embedded_upload_bytes)
    }
}



let visualRenderCommandFillRectBit =
    UInt64(1) << UInt64(YU_STORAGE_RENDER_COMMAND_FILL_RECT)
let visualRenderCommandGlyphBit =
    UInt64(1) << UInt64(YU_STORAGE_RENDER_COMMAND_GLYPH)
let visualRenderCommandImageBit =
    UInt64(1) << UInt64(YU_STORAGE_RENDER_COMMAND_IMAGE)
let visualRenderCommandEmbeddedSvgBit =
    UInt64(1) << UInt64(YU_STORAGE_RENDER_COMMAND_EMBEDDED_SVG)
let visualRenderCommandTaskCheckboxBit =
    UInt64(1) << UInt64(YU_STORAGE_RENDER_COMMAND_TASK_CHECKBOX)







/// Normalizes the separate image/embedded C status domains before applying a
/// shared coverage policy. Raw numeric equality between those domains is not
/// part of the native host contract.
enum RetainedResourceCoverageState {
    case unknown
    case pending
    case ready
    case failed
    case unsupported
    case invalid
}

func imageResourceCoverageState(_ status: UInt8) -> RetainedResourceCoverageState {
    switch status {
    case UInt8(YU_STORAGE_IMAGE_RESOURCE_UNKNOWN): return .unknown
    case UInt8(YU_STORAGE_IMAGE_RESOURCE_PENDING): return .pending
    case UInt8(YU_STORAGE_IMAGE_RESOURCE_READY): return .ready
    case UInt8(YU_STORAGE_IMAGE_RESOURCE_FAILED): return .failed
    default: return .invalid
    }
}

func embeddedResourceCoverageState(_ status: UInt8) -> RetainedResourceCoverageState {
    switch status {
    case UInt8(YU_STORAGE_EMBEDDED_RESOURCE_UNKNOWN): return .unknown
    case UInt8(YU_STORAGE_EMBEDDED_RESOURCE_PENDING): return .pending
    case UInt8(YU_STORAGE_EMBEDDED_RESOURCE_READY): return .ready
    case UInt8(YU_STORAGE_EMBEDDED_RESOURCE_FAILED): return .failed
    case UInt8(YU_STORAGE_EMBEDDED_RESOURCE_UNSUPPORTED): return .unsupported
    default: return .invalid
    }
}

/// Proves whether a resource is covered by the current retained publication.
/// Ready resources use their texture; pending/failed images use the scene's
/// placeholder, while pending/failed/unsupported embedded blocks keep their
/// projected source glyphs. An unknown image with no stable identity also
/// keeps the retained projected alt label. An unclassified non-zero identity
/// is fail-closed because it may represent a renderer state unknown to this
/// host.
/// 资源未就绪或指纹失效时安排一次刷新。
///
/// 这里只回答「要不要再取一次」。coverage 判断已随 TextKit fallback 一同
/// 删除（不变量 I5）：没有第二条渲染路径可以回退。
func retainedResourceNeedsRefresh(
    state: RetainedResourceCoverageState,
    resourceFingerprint: UInt64
) -> Bool {
    switch state {
    case .ready, .unsupported:
        return false
    case .pending, .failed, .invalid:
        return true
    case .unknown:
        return resourceFingerprint != 0
    }
}

struct NativeVisualEmbeddedResource {
    let revision: UInt64
    let blockIndex: UInt64
    let sourceRange: NSRange
    let infoRange: NSRange
    let contentRange: NSRange
    let resourceFingerprint: UInt64
    let kind: UInt8
    let resourceStatus: UInt8

    init(_ value: YuStorageVisualEmbeddedResource) {
        revision = value.revision
        blockIndex = value.block_index
        sourceRange = NSRange(
            location: Int(value.source_start_utf16),
            length: Int(value.source_end_utf16 - value.source_start_utf16)
        )
        infoRange = NSRange(
            location: Int(value.info_start_utf16),
            length: Int(value.info_end_utf16 - value.info_start_utf16)
        )
        contentRange = NSRange(
            location: Int(value.content_start_utf16),
            length: Int(value.content_end_utf16 - value.content_start_utf16)
        )
        resourceFingerprint = value.resource_fingerprint
        kind = value.kind
        resourceStatus = value.resource_status
    }
}

struct NativeMacosRenderHostSnapshot {
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
    let commandKindMask: UInt64
    let blockKindMask: UInt64
    let selectionDecorationCount: Int
    let caretDecorationCount: Int

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
        commandKindMask = value.command_kind_mask
        blockKindMask = value.block_kind_mask
        selectionDecorationCount = Int(value.selection_decoration_count)
        caretDecorationCount = Int(value.caret_decoration_count)
    }
}

struct NativeMacosRenderHostSurfaceSnapshot {
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
    let commandKindMask: UInt64
    let blockKindMask: UInt64
    let selectionDecorationCount: Int
    let caretDecorationCount: Int

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
        commandKindMask = value.command_kind_mask
        blockKindMask = value.block_kind_mask
        selectionDecorationCount = Int(value.selection_decoration_count)
        caretDecorationCount = Int(value.caret_decoration_count)
    }
}

struct NativeVisualRenderCommand {
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
    let embeddedGeneration: UInt64
    let embeddedKind: UInt8
    let embeddedWidth: Int
    let embeddedHeight: Int

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
        embeddedGeneration = value.embedded_generation
        embeddedKind = value.embedded_kind
        embeddedWidth = Int(value.embedded_width)
        embeddedHeight = Int(value.embedded_height)
    }
}

struct NativeVisualRenderPage {
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

struct NativeVisualRenderDamage {
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

struct NativeVisualViewport {
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

struct NativeCaretScrollRequest {
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

struct NativeAccessibilitySnapshot {
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

struct NativeAccessibilityRange {
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
struct NativeAccessibilitySemanticNode {
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

enum SemanticAccessibilityKind: UInt8 {
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

enum SemanticAccessibilityFlag {
    static let taskDone: UInt8 = 1 << 1
}

/// A lightweight AppKit AX element backed by one Rust semantic node. It owns
/// no Markdown text: labels and range queries always use the node's Revision
/// and ask `StorageBridge` for the current source bytes.
final class YuAccessibilitySemanticElement: NSObject,
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

/// A Revision-bound native Accessibility splitter for one visible Markdown
/// table column divider. The element is deliberately ephemeral: it owns only
/// scalar geometry and source provenance, while all resize actions go back
/// through `DocumentTextView` to the Rust session-only preview.
final class YuAccessibilityTableResizeElement: NSObject,
    NSAccessibilityElementProtocol,
    NSAccessibilityStepper
{
    let descriptor: NativeTableResizeAccessibilityDivider
    weak var parentObject: AnyObject?
    weak var frameOwner: DocumentTextView?

    init(
        descriptor: NativeTableResizeAccessibilityDivider,
        parent: AnyObject?,
        owner: DocumentTextView
    ) {
        self.descriptor = descriptor
        parentObject = parent
        frameOwner = owner
        super.init()
    }

    @objc func accessibilityFrame() -> NSRect {
        frameOwner?.accessibilityFrameForTableResizeDescriptor(descriptor) ?? .zero
    }

    @objc func accessibilityParent() -> Any? { parentObject }

    @objc var accessibilityRole: NSAccessibility.Role { .splitter }

    @objc var accessibilityRoleDescription: String? { "表格列分隔线" }

    @objc(accessibilityLabel) func accessibilityLabel() -> String? {
        let left = descriptor.index + 1
        let right = left + 1
        return "表格第 \(left) 列与第 \(right) 列之间的分隔线"
    }

    @objc var accessibilityTitle: String? { accessibilityLabel() }

    /// The value is intentionally human-readable rather than a second source
    /// model. It gives VoiceOver context while the effective x coordinate
    /// remains owned by the Revision-bound Rust layout query.
    @objc(accessibilityValue) func accessibilityValue() -> Any? {
        "第 \(descriptor.index + 1) / \(descriptor.columnCount) 列分隔线"
    }

    @objc func accessibilityPerformIncrement() -> Bool {
        frameOwner?.performTableResizeAccessibilityAction(descriptor, direction: 1)
            ?? false
    }

    @objc func accessibilityPerformDecrement() -> Bool {
        frameOwner?.performTableResizeAccessibilityAction(descriptor, direction: -1)
            ?? false
    }

    @objc func accessibilityIdentifier() -> String {
        "yu-table-divider-\(descriptor.revision)-\(descriptor.blockIndex)-\(descriptor.index)"
    }
}

/// VoiceOver asks custom rotors for the next source-backed semantic element.
/// The delegate is intentionally tiny: it never stores text or a second tree,
/// and asks the current DocumentTextView for the live element order.
final class YuAccessibilityRotorDelegate: NSObject,
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

struct NativeComposition {
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

struct NativeCompositionProjection {
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

struct NativeCompositionCaret {
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

struct NativeCompositionShapedCaret {
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

struct NativeCompositionProjectionHit {
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

struct NativeCommandResult {
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

final class StorageBridge {
    private var handle: OpaquePointer
    private let openedPath: String
    private var cachedSource: String
    private var cachedState: NativeStorageState

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
        openedPath = path
        cachedSource = ""
        cachedState = NativeStorageState(unavailable: ())

        // Validate the two snapshots needed to construct the native source
        // mirror before returning from init.  A malformed path/file or an ABI
        // mismatch now becomes a normal launch error instead of a later
        // `precondition` abort while AppKit is laying out the first window.
        do {
            cachedSource = try copyBytesThrowing { output, capacity, written in
                yu_storage_session_copy_source(
                    created,
                    output,
                    capacity,
                    written
                )
            }
            cachedState = try readState()
        } catch {
            yu_storage_session_destroy(created)
            throw error
        }
    }

    deinit {
        yu_storage_session_destroy(handle)
    }

    var path: String {
        copyBytesIfAvailable { output, capacity, written in
            yu_storage_session_copy_path(
                handle,
                output,
                capacity,
                written
            )
        } ?? openedPath
    }

    var source: String {
        if let current = copyBytesIfAvailable({ output, capacity, written in
            yu_storage_session_copy_source(
                handle,
                output,
                capacity,
                written
            )
        }) {
            cachedSource = current
        }
        return cachedSource
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

    func macosTaskCheckboxHitTest(
        revision: UInt64,
        point: CGPoint
    ) throws -> NativeTaskCheckboxHit {
        var value = YuStorageTaskCheckboxHit()
        let status = yu_storage_session_macos_task_checkbox_hit_test(
            handle,
            revision,
            Float(point.x),
            Float(point.y),
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeTaskCheckboxHit(value)
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

    /// 不需要调用方指定 block 的 caret 几何查询。平台不解析 Markdown，
    /// 无法知道某个 source offset 属于哪个 block（不变量 I1）。
    func macosSourceCaret(
        revision: UInt64,
        sourceUTF16: UInt64,
        affinity: UInt8,
        size: Float,
        maxWidth: Float
    ) throws -> NativeBlockCaret {
        var value = YuStorageBlockCaret()
        let status = yu_storage_session_macos_source_caret(
            handle,
            revision,
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

    func macosTableResizeAccessibilityDividers(
        revision: UInt64,
        size: Float,
        maxWidth: Float,
        scrollY: Float,
        viewportHeight: Float
    ) throws -> [NativeTableResizeAccessibilityDivider] {
        var required = 0
        let sizeStatus = yu_storage_session_macos_table_resize_accessibility_dividers(
            handle,
            revision,
            size,
            maxWidth,
            scrollY,
            viewportHeight,
            nil,
            0,
            &required
        )
        guard sizeStatus == StorageStatus.ok else {
            throw BridgeError.operation(sizeStatus)
        }
        var values = Array(
            repeating: YuStorageTableResizeAccessibilityDivider(),
            count: required
        )
        var written = required
        let fillStatus = values.withUnsafeMutableBufferPointer { buffer in
            yu_storage_session_macos_table_resize_accessibility_dividers(
                handle,
                revision,
                size,
                maxWidth,
                scrollY,
                viewportHeight,
                buffer.baseAddress,
                buffer.count,
                &written
            )
        }
        guard fillStatus == StorageStatus.ok, written == required else {
            throw BridgeError.operation(fillStatus)
        }
        return values.map(NativeTableResizeAccessibilityDivider.init)
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

    func macosVisualEmbeddedResources(
        revision: UInt64
    ) throws -> [NativeVisualEmbeddedResource] {
        var required = 0
        let sizeStatus = yu_storage_session_macos_visual_embedded_resources(
            handle,
            revision,
            nil,
            0,
            &required
        )
        guard sizeStatus == StorageStatus.ok else {
            throw BridgeError.operation(sizeStatus)
        }
        var values = Array(repeating: YuStorageVisualEmbeddedResource(), count: required)
        var written = required
        let fillStatus = values.withUnsafeMutableBufferPointer { buffer in
            yu_storage_session_macos_visual_embedded_resources(
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
        return values.map(NativeVisualEmbeddedResource.init)
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
        if let current = try? readState() {
            cachedState = current
        }
        return cachedState
    }

    private func readState() throws -> NativeStorageState {
        var value = YuStorageState()
        let status = yu_storage_session_state(handle, &value)
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
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
        // Availability is a capability query, not a reason to abort the
        // host. During a close/reload race the session can reject it; the
        // native editor should simply leave the command disabled and retain
        // the source fallback.
        guard status == StorageStatus.ok else { return false }
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
        guard sizeStatus == StorageStatus.ok, required >= 0 else { return "" }
        var bytes = Array(repeating: UInt8(0), count: required)
        let copyStatus = bytes.withUnsafeMutableBufferPointer { buffer in
            operation(buffer.baseAddress, buffer.count, &required)
        }
        guard copyStatus == StorageStatus.ok,
              required >= 0,
              required <= bytes.count else {
            return ""
        }
        return String(decoding: bytes.prefix(required), as: UTF8.self)
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


/// The native source mirror is deliberately a view cache, never a second
/// document model. Rust owns canonical source, revision, selection and
/// composition generation; this TextKit object only projects those values for
/// AppKit's NSTextInputClient callbacks. The visual pointer adapter asks
/// Rust's CoreText-shaped block layout for the visual boundary, then maps that
/// boundary back to canonical source ranges. The disposable TextKit visual
/// mirror remains a geometry and input/IME/accessibility host.
final class DocumentTextView: NSTextView {
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


    private let bridge: StorageBridge
    private var canonicalSource: String
    private var canonicalRevision: UInt64
    private var semanticNodes: [NativeAccessibilitySemanticNode] = []
    private var semanticElements: [YuAccessibilitySemanticElement] = []
    private var tableResizeAccessibilityDescriptors:
        [NativeTableResizeAccessibilityDivider] = []
    private var tableResizeAccessibilityElements:
        [YuAccessibilityTableResizeElement] = []
    private var headingRotorDelegate: YuAccessibilityRotorDelegate!
    private var linkRotorDelegate: YuAccessibilityRotorDelegate!
    private var nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
    private var synchronizingSelection = false
    private var visualCompositionGeneration: UInt64?
    private var visualViewport: NativeVisualViewport?
    private var visualSelectionAnchor: Int?
    private var tableResizeTrackingArea: NSTrackingArea?
    private var tableResizeCursorActive = false
    private var taskCheckboxPointerConsumed = false
    var onDocumentChange: (() -> Void)?
    var onBeforeCommand: (() -> Void)?
    var onCaretChange: (() -> Void)?
    var onError: ((Error) -> Void)?
    var onTableResizeHover: ((NSPoint) -> Bool)?
    var onTaskCheckboxPress: ((NSPoint) -> Bool)?
    var onTableResizeBegin: ((NSPoint) -> Bool)?
    var onTableResizeUpdate: ((NSPoint) -> Bool)?
    var onTableResizeFinish: (() -> Bool)?
    var onTableResizeCancel: (() -> Bool)?
    var tableResizeAccessibilityProvider:
        (() -> [NativeTableResizeAccessibilityDivider])?
    var tableResizeAccessibilityFrameProvider:
        ((NativeTableResizeAccessibilityDivider) -> NSRect)?
    var onTableResizeAccessibilityAction:
        ((NativeTableResizeAccessibilityDivider, Int) -> Bool)?

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

    override func updateTrackingAreas() {
        if let tableResizeTrackingArea {
            removeTrackingArea(tableResizeTrackingArea)
        }
        let area = NSTrackingArea(
            rect: bounds,
            options: [
                .mouseEnteredAndExited,
                .mouseMoved,
                .activeInKeyWindow,
                .inVisibleRect
            ],
            owner: self,
            userInfo: nil
        )
        tableResizeTrackingArea = area
        addTrackingArea(area)
        super.updateTrackingAreas()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        window?.acceptsMouseMovedEvents = true
        if window == nil {
            setTableResizeCursor(active: false)
        }
    }

    override func mouseMoved(with event: NSEvent) {
        let divider = onTableResizeHover?(visualPoint(for: event)) ?? false
        setTableResizeCursor(active: divider)
        super.mouseMoved(with: event)
    }

    override func mouseExited(with event: NSEvent) {
        setTableResizeCursor(active: false)
        super.mouseExited(with: event)
    }

    func refreshFromRust() {
        canonicalSource = bridge.source
        canonicalRevision = bridge.state.revision
        semanticNodes = bridge.accessibilitySemanticNodesIfAvailable ?? []
        nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
        synchronizeProjection()
        postAccessibilityRefresh()
    }





















    @discardableResult
    func applyVisualPointerSelectionForSelfCheck(
        at point: NSPoint,
        extending: Bool = false
    ) -> Bool {
        applyVisualPointerSelection(at: point, extending: extending)
    }


    @discardableResult
    func applyVisualSelectionForSelfCheck(_ visualRange: NSRange) -> Bool {
        applyVisualSelection(visualRange)
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

    private func setTableResizeCursor(active: Bool) {
        let next = active && window != nil
        guard tableResizeCursorActive != next else { return }
        tableResizeCursorActive = next
        (next ? NSCursor.resizeLeftRight : NSCursor.arrow).set()
    }

    /// Resolves a visual document point through the Rust CoreText-shaped
    /// block layout. TextKit remains the input/IME/accessibility host, but it
    /// must not guess glyph boundaries for production pointer selection.
    /// 命中测试完全由 Rust layout 完成。此处不再用 TextKit 布局出的
    /// visual 长度做上界校验——那等于用第二套布局系统验证第一套，
    /// 而第二套布局系统本身就是要消除的对象（不变量 I5、E1）。
    /// Rust 返回的 visualUTF16 已绑定同一 Revision，越界由 Rust 侧拒绝。
    private func shapedVisualOffset(at point: NSPoint) -> Int? {
        guard point.x.isFinite,
              point.y.isFinite,
              let (size, width) = visualLayoutMetrics(),
              let hit = try? bridge.macosProjectionHitTest(
                  revision: bridge.state.revision,
                  point: CGPoint(x: point.x, y: point.y),
                  size: size,
                  maxWidth: width
              ),
              hit.revision == bridge.state.revision,
              hit.point.x.isFinite,
              hit.point.y.isFinite,
              let visualOffset = Int(exactly: hit.visualUTF16),
              visualOffset >= 0 else {
            return nil
        }
        return visualOffset
    }

    @discardableResult
    private func applyVisualPointerSelection(
        at point: NSPoint,
        extending: Bool
    ) -> Bool {
        guard !bridge.composition.active else { return false }
        guard let visualOffset = shapedVisualOffset(at: point) else {
            // Rust 端对 Revision 与已发布的 viewport metrics 有意严格。
            // 几何过期时放弃本次指针选区，等下一帧重试。
            return false
        }
        if !extending || visualSelectionAnchor == nil {
            if extending {
                let endpoints = bridge.selectionEndpoints
                let sourceUTF16 = endpoints.anchorUTF16
                visualSelectionAnchor = visualUTF16ForSource(
                    sourceUTF16,
                    affinity: endpoints.affinity
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
        affinity: UInt8
    ) -> Int? {
        let revision = bridge.state.revision
        guard let caret = try? bridge.projectionCaret(
            revision: revision,
            sourceUTF16: sourceUTF16,
            affinity: affinity
        ),
              caret.revision == revision,
              let visualUTF16 = Int(exactly: caret.visualUTF16),
              visualUTF16 >= 0 else {
            return nil
        }
        return visualUTF16
    }

    @discardableResult
    private func applyVisualSelection(
        _ visualRange: NSRange,
        anchorIsVisualStart: Bool? = nil
    ) -> Bool {
        guard !bridge.composition.active,
              visualRange.location >= 0,
              visualRange.length >= 0 else {
            return false
        }
        do {
            let source = try bridge.projectionSourceSelection(
                revision: bridge.state.revision,
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
    /// surface; semantic children and visible table splitters are stable,
    /// Revision-bound elements and never become a second text model.
    @objc var accessibilityChildren: [Any]? {
        semanticElements.map { $0 as Any } + tableResizeAccessibilityElements
            .map { $0 as Any }
    }

    @objc var accessibilityChildrenInNavigationOrder: [Any]? {
        semanticElements.map { $0 as Any } + tableResizeAccessibilityElements
            .map { $0 as Any }
    }

    /// Expose the same elements through AppKit's dedicated splitter
    /// attribute. VoiceOver may discover them either as document children or
    /// through this role-specific collection.
    @objc var accessibilitySplitters: [Any]? {
        tableResizeAccessibilityElements.map { $0 as Any }
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
              let block = node.actionBlock else {
            return false
        }
        return toggleTask(block: block, revision: node.revision)
    }

    private func toggleTask(block: UInt64, revision: UInt64) -> Bool {
        guard revision == bridge.state.revision,
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

    func toggleTaskPointerHit(_ hit: NativeTaskCheckboxHit) -> Bool {
        toggleTask(block: hit.blockIndex, revision: hit.revision)
    }

    @discardableResult
    func pressTaskCheckboxForSelfCheck(at point: NSPoint) -> Bool {
        onTaskCheckboxPress?(point) ?? false
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
           event.clickCount == 1,
           !event.modifierFlags.contains(.shift),
           onTaskCheckboxPress?(visualPoint(for: event)) == true {
            visualSelectionAnchor = nil
            taskCheckboxPointerConsumed = true
            return
        }
        taskCheckboxPointerConsumed = false
        if event.buttonNumber == 0,
           onTableResizeBegin?(visualPoint(for: event)) == true {
            visualSelectionAnchor = nil
            setTableResizeCursor(active: true)
            return
        }
        if applyVisualPointerSelection(
            at: visualPoint(for: event),
            extending: event.modifierFlags.contains(.shift)
        ) {
            return
        }
        visualSelectionAnchor = nil
        super.mouseDown(with: event)
    }

    override func mouseDragged(with event: NSEvent) {
        if taskCheckboxPointerConsumed {
            return
        }
        if onTableResizeUpdate?(visualPoint(for: event)) == true {
            setTableResizeCursor(active: true)
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
        if taskCheckboxPointerConsumed {
            taskCheckboxPointerConsumed = false
            return
        }
        if event.buttonNumber == 0,
           onTableResizeFinish?() == true {
            setTableResizeCursor(active: false)
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
    /// caret 由 Rust retained decoration 绘制。TextKit 不贡献像素。
    override func drawInsertionPoint(
        in rect: NSRect,
        color: NSColor,
        turnedOn: Bool
    ) {}

    /// Rust surface 是唯一渲染路径（不变量 I5）。本视图仍然是
    /// `NSTextInputClient` 与 Accessibility 的宿主，但不绘制任何像素：
    /// 没有 fallback 路径，也就不需要判断「该不该画」。
    override func draw(_ rect: NSRect) {}

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

    /// Runs the production copy payload against a private pasteboard so the
    /// workflow smoke test never changes the user's global clipboard.
    func copyToPasteboardForSelfCheck(_ pasteboard: NSPasteboard) throws {
        try finishCompositionForClipboard()
        let revision = bridge.state.revision
        let text = bridge.copySelection()
        guard !text.isEmpty else { return }
        let html = try bridge.copySelectionHTML(revision: revision)
        try publishSourceToPasteboard(text, html: html, to: pasteboard)
    }

    /// Runs the production paste transaction against a private pasteboard.
    /// The same Rust selection/insert path is used; AppKit notifications are
    /// intentionally omitted because this is a headless check.
    func pasteFromPasteboardForSelfCheck(_ pasteboard: NSPasteboard) throws {
        try finishCompositionForClipboard()
        guard let text = try sourceFromPasteboard(pasteboard), !text.isEmpty else {
            return
        }
        apply(try bridge.insertText(text))
        synchronizeProjection()
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
        return nativeMarkedRange
    }

    override func attributedSubstring(
        forProposedRange proposedRange: NSRange,
        actualRange: NSRangePointer?
    ) -> NSAttributedString? {
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
                setTableResizeCursor(active: false)
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

    /// IME 候选窗定位。
    ///
    /// 必须用 Rust 几何：NSTextView 的默认实现基于 TextKit 布局，而 TextKit
    /// 排的是 canonical source，屏幕上显示的却是 Rust 的投影结果（未聚焦的
    /// Markdown 语法被隐藏），两者的字符位置并不对应。沿用默认实现会让候选窗
    /// 偏离真实插入点，违反不变量 H3（OS 查询的 caret rect 必须与当前编辑
    /// 状态一致）。
    ///
    /// 几何不可用时回退到默认实现：候选窗位置略偏，好过不显示。
    override func firstRect(
        forCharacterRange range: NSRange,
        actualRange: NSRangePointer?
    ) -> NSRect {
        guard let caretRect = rustCaretRect(forSourceUTF16: range.location) else {
            return super.firstRect(forCharacterRange: range, actualRange: actualRange)
        }
        actualRange?.pointee = NSRange(location: range.location, length: 0)
        let viewRect = NSRect(
            x: caretRect.origin.x + textContainerOrigin.x,
            y: caretRect.origin.y + textContainerOrigin.y,
            width: max(caretRect.width, 1.0),
            height: caretRect.height
        )
        let windowRect = convert(viewRect, to: nil)
        return window?.convertToScreen(windowRect) ?? windowRect
    }

    /// 当前 Revision 下某个 source offset 的 caret 矩形，document-space。
    /// composition 期间使用同一 generation 绑定的 transient 布局，
    /// 否则 preedit 的候选窗会落在提交前的旧位置。
    private func rustCaretRect(forSourceUTF16 sourceUTF16: Int) -> NSRect? {
        guard sourceUTF16 >= 0,
              let (size, width) = visualLayoutMetrics() else {
            return nil
        }
        let revision = bridge.state.revision
        let offset = UInt64(sourceUTF16)
        if bridge.composition.active {
            guard let caret = try? bridge.macosCompositionShapedCaret(
                revision: revision,
                generation: bridge.composition.generation,
                sourceUTF16: offset,
                affinity: 0,
                size: size,
                maxWidth: width
            ), caret.revision == revision else {
                return nil
            }
            return NSRect(origin: caret.point, size: caret.size)
        }
        guard let caret = try? bridge.macosSourceCaret(
            revision: revision,
            sourceUTF16: offset,
            affinity: 0,
            size: size,
            maxWidth: width
        ), caret.revision == revision else {
            return nil
        }
        return NSRect(
            origin: caret.point,
            size: CGSize(width: 1.0, height: caret.height)
        )
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

    private func publishSourceToPasteboard(
        _ source: String,
        html: String,
        to pasteboard: NSPasteboard = .general
    ) throws {
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

    func accessibilityFrameForTableResizeDescriptor(
        _ descriptor: NativeTableResizeAccessibilityDivider
    ) -> NSRect {
        guard descriptor.revision == bridge.state.revision,
              !bridge.composition.active else {
            return .zero
        }
        return tableResizeAccessibilityFrameProvider?(descriptor) ?? .zero
    }

    @discardableResult
    func performTableResizeAccessibilityAction(
        _ descriptor: NativeTableResizeAccessibilityDivider,
        direction: Int
    ) -> Bool {
        guard descriptor.revision == bridge.state.revision,
              descriptor.kind == UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN),
              descriptor.columnCount >= 2,
              descriptor.index < descriptor.columnCount - 1,
              direction == 1 || direction == -1,
              !bridge.composition.active else {
            return false
        }
        let changed = onTableResizeAccessibilityAction?(descriptor, direction) ?? false
        if changed {
            refreshTableResizeAccessibility(postNotification: true)
        }
        return changed
    }

    /// Rebuilds only the visible table splitter children. The provider owns
    /// no AppKit objects; it returns fresh scalar descriptors from the
    /// coordinator, and stale Revision descriptors are discarded before they
    /// become discoverable by VoiceOver.
    func refreshTableResizeAccessibility(postNotification: Bool = false) {
        let descriptors = (tableResizeAccessibilityProvider?() ?? []).filter {
            $0.revision == bridge.state.revision
                && $0.kind == UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN)
                && $0.columnCount >= 2
                && $0.index < $0.columnCount - 1
                && $0.rect.origin.x.isFinite
                && $0.rect.origin.y.isFinite
                && $0.rect.width.isFinite
                && $0.rect.height.isFinite
                && $0.rect.width > 0.0
                && $0.rect.height > 0.0
        }
        guard descriptors != tableResizeAccessibilityDescriptors else {
            if postNotification {
                NSAccessibility.post(element: self, notification: .layoutChanged)
            }
            return
        }
        postDestroyedTableResizeAccessibilityElements()
        tableResizeAccessibilityDescriptors = descriptors
        tableResizeAccessibilityElements = descriptors.map {
            YuAccessibilityTableResizeElement(
                descriptor: $0,
                parent: self,
                owner: self
            )
        }
        if postNotification {
            NSAccessibility.post(element: self, notification: .layoutChanged)
        }
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
        rebuildTableResizeAccessibilityTree()
    }

    private func postDestroyedSemanticElements() {
        for element in flattenSemanticElements(semanticElements) {
            NSAccessibility.post(element: element, notification: .uiElementDestroyed)
        }
        postDestroyedTableResizeAccessibilityElements()
    }

    private func rebuildTableResizeAccessibilityTree() {
        let descriptors = (tableResizeAccessibilityProvider?() ?? []).filter {
            $0.revision == bridge.state.revision
                && $0.kind == UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN)
                && $0.columnCount >= 2
                && $0.index < $0.columnCount - 1
                && $0.rect.origin.x.isFinite
                && $0.rect.origin.y.isFinite
                && $0.rect.width.isFinite
                && $0.rect.height.isFinite
                && $0.rect.width > 0.0
                && $0.rect.height > 0.0
        }
        tableResizeAccessibilityDescriptors = descriptors
        tableResizeAccessibilityElements = descriptors.map {
            YuAccessibilityTableResizeElement(
                descriptor: $0,
                parent: self,
                owner: self
            )
        }
    }

    private func postDestroyedTableResizeAccessibilityElements() {
        for element in tableResizeAccessibilityElements {
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

enum BridgeError: LocalizedError {
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
final class MacosSurfaceHostView: NSView {
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









struct TableResizePointerSession: Equatable {
    let revision: UInt64
    let kind: UInt8
}

/// Keeps the native pointer route explicit and headless-testable. Rust owns
/// the geometry preview; this state only answers whether subsequent mouse
/// events belong to the active divider gesture and when a revision invalidates
/// that route.
struct TableResizePointerState {
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
final class MacosSurfaceHostCoordinator {
    private static let maxImageRefreshAttempts = 40
    private static let imageRefreshDelay: DispatchTimeInterval = .milliseconds(50)

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
    private var imageRefreshTask: DispatchWorkItem?
    private var imageRefreshAttempts = 0
    private var imageRefreshNeeded = false
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
        cancelImageResourceRefresh()
        scheduleToken &+= 1
        self.surfaceView = surfaceView
        self.scrollView = scrollView
        self.fontSize = max(fontSize, 1.0)
        contentWidth = nil
        metrics = nil
        lastSubmitKey = nil
        lastSnapshot = nil
        imageRefreshNeeded = false
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
        imageRefreshAttempts = 0
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
                self.imageRefreshNeeded = false
                self.cancelImageResourceRefresh()
                self.surfaceView?.setNativeContentVisible(false)
                self.onSurfaceStateChange?()
                self.onError?(error)
            }
        }
    }

    /// Selection/caret movement does not advance the canonical source
    /// Revision, but it does change retained editor-decoration geometry.
    /// Revoke the current visual publication before scheduling its replacement
    /// so equal primitive counts cannot make an old caret look current.
    func invalidateEditorDecorationPublication() {
        lastSubmitKey = nil
        lastSnapshot = nil
    }

    private func cancelImageResourceRefresh() {
        imageRefreshTask?.cancel()
        imageRefreshTask = nil
        imageRefreshAttempts = 0
    }

    /// Polls only while the current viewport has an image resource that is
    /// pending, failed, or otherwise unproven. The Rust worker is deliberately
    /// asynchronous and has no callback into AppKit, so a short bounded poll
    /// is the smallest safe bridge. Every attempt invalidates the submit key
    /// to force Rust to drain worker results and republish the frame.
    private func scheduleImageResourceRefresh() {
        guard imageRefreshNeeded,
              isAttached,
              imageRefreshTask == nil,
              imageRefreshAttempts < Self.maxImageRefreshAttempts else {
            return
        }
        let token = scheduleToken
        imageRefreshAttempts += 1
        let task = DispatchWorkItem { [weak self] in
            guard let self,
                  self.scheduleToken == token,
                  self.isAttached else {
                return
            }
            self.imageRefreshTask = nil
            self.lastSubmitKey = nil
            do {
                _ = try self.submitNow()
            } catch {
                self.imageRefreshNeeded = false
                self.cancelImageResourceRefresh()
                self.surfaceView?.setNativeContentVisible(false)
                self.onSurfaceStateChange?()
                self.onError?(error)
            }
        }
        imageRefreshTask = task
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.imageRefreshDelay, execute: task)
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

    /// Resolves the current document-space point against the same shaped
    /// table geometry used by the pointer begin path. Hover is intentionally
    /// read-only: it never opens a Rust resize session and silently falls
    /// back to the normal arrow when metrics or the Revision are stale.
    func tableResizeHover(at point: NSPoint) -> Bool {
        if let session = tableResizePointerState.session,
           session.revision == bridge.state.revision {
            return session.kind == YU_STORAGE_TABLE_RESIZE_COLUMN
        }
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
            return hit.kind == UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN)
        } catch {
            return false
        }
    }

    /// Resolves a primary click against the exact Rust scene publication that
    /// is currently visible. The coordinator returns metadata only; canonical
    /// mutation stays in `DocumentTextView`'s existing command path.
    func taskCheckboxHit(at point: NSPoint) -> NativeTaskCheckboxHit? {
        let revision = bridge.state.revision
        guard !bridge.composition.active,
              point.x.isFinite,
              point.y.isFinite,
              hasCurrentPublication(
                  revision: revision,
                  compositionGeneration: bridge.composition.generation
              ) else {
            return nil
        }
        do {
            let hit = try bridge.macosTaskCheckboxHitTest(
                revision: revision,
                point: CGPoint(x: point.x, y: point.y)
            )
            guard hit.revision == revision,
                  hit.bounds.width > 0.0,
                  hit.bounds.height > 0.0 else {
                return nil
            }
            return hit
        } catch BridgeError.operation(let status)
            where status == StorageStatus.invalidSelection
                || status == StorageStatus.staleRevision {
            return nil
        } catch {
            // A missing/stale retained publication is an enhancement miss.
            // Preserve AppKit's ordinary source selection without surfacing a
            // modal error for a pointer query.
            return nil
        }
    }

    /// Returns the visible, read-only divider descriptors from the same
    /// document-space CoreText layout used by hover and begin. Callers project
    /// these into ephemeral native Accessibility elements; the descriptors
    /// never retain a Rust layout or open a resize gesture.
    func tableResizeAccessibilityDividers() -> [NativeTableResizeAccessibilityDivider] {
        guard !bridge.composition.active,
              let geometry = visualDecorationGeometry() else {
            return []
        }
        do {
            return try bridge.macosTableResizeAccessibilityDividers(
                revision: bridge.state.revision,
                size: geometry.size,
                maxWidth: geometry.maxWidth,
                scrollY: geometry.scrollY,
                viewportHeight: geometry.viewportHeight
            )
        } catch {
            return []
        }
    }

    /// Converts one document-space divider descriptor into a screen-space AX
    /// frame. The conversion is intentionally performed at query time so a
    /// scroll, window move, or surface detach cannot leave an element holding
    /// stale AppKit coordinates.
    func tableResizeAccessibilityFrame(
        for descriptor: NativeTableResizeAccessibilityDivider
    ) -> NSRect {
        guard descriptor.revision == bridge.state.revision,
              !bridge.composition.active,
              let surfaceView,
              let window = surfaceView.window,
              let geometry = visualDecorationGeometry() else {
            return .zero
        }
        let local = descriptor.rect.offsetBy(
            dx: 0.0,
            dy: -CGFloat(geometry.scrollY)
        )
        return window.convertToScreen(surfaceView.convert(local, to: nil))
    }

    /// Performs one VoiceOver increment/decrement as a Rust-owned transient
    /// resize. The effective divider descriptor is queried again after the
    /// action, so repeated actions accumulate through the session override
    /// without changing Markdown source or creating an editor transaction.
    @discardableResult
    func adjustTableResizeAccessibility(
        _ descriptor: NativeTableResizeAccessibilityDivider,
        direction: Int
    ) -> Bool {
        guard descriptor.revision == bridge.state.revision,
              descriptor.kind == UInt8(YU_STORAGE_TABLE_RESIZE_COLUMN),
              descriptor.columnCount >= 2,
              descriptor.index < descriptor.columnCount - 1,
              (direction == 1 || direction == -1),
              !bridge.composition.active,
              !tableResizePointerState.isActive,
              let geometry = visualDecorationGeometry() else {
            return false
        }
        let step = max(
            CGFloat(8.0),
            min(CGFloat(16.0), CGFloat(geometry.lineHeight) * 0.5)
        )
        let dividerPoint = NSPoint(
            x: descriptor.rect.midX,
            y: descriptor.rect.midY
        )
        guard beginTableResize(at: dividerPoint) else { return false }
        let updatedPoint = NSPoint(
            x: dividerPoint.x + CGFloat(direction) * step,
            y: dividerPoint.y
        )
        guard updateTableResize(at: updatedPoint),
              finishTableResize() else {
            _ = cancelTableResize()
            return false
        }
        return true
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
        var finished = false
        do {
            _ = try bridge.tableResizeFinish(revision: revision)
            _ = tableResizePointerState.finish(revision: revision)
            lastSubmitKey = nil
            scheduleSubmit()
            finished = true
        } catch {
            tableResizePointerState.reset()
            onError?(error)
        }
        return finished
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

    /// 扫描当前 viewport 的图片与嵌入资源，决定是否需要安排一次刷新。
    ///
    /// 这里不再做 retained coverage 判断：Rust surface 是唯一渲染路径，
    /// 资源未就绪时由 Rust 绘制 placeholder，不存在回退 TextKit 的分支
    /// （不变量 I5）。资源状态只影响「要不要再取一次」，不影响「谁来画」。
    private func updateResourceRefreshState(
        snapshot: NativeMacosRenderHostSurfaceSnapshot,
        revision: UInt64,
        size: Float,
        maxWidth: Float,
        scrollY: Float,
        viewportHeight: Float
    ) {
        imageRefreshNeeded = false
        guard snapshot.commandCount > 0 else { return }
        do {
            let (viewport, blocks) = try bridge.macosShapedViewportBlocks(
                revision: revision,
                size: size,
                maxWidth: maxWidth,
                scrollY: scrollY,
                viewportHeight: viewportHeight
            )
            guard viewport.revision == revision else { return }
            let visibleBlockIndexes = Set(blocks.map(\.blockIndex))
            let images = try bridge.macosVisualImages(revision: revision)
            for image in images where visibleBlockIndexes.contains(image.blockIndex) {
                if retainedResourceNeedsRefresh(
                    state: imageResourceCoverageState(image.resourceStatus),
                    resourceFingerprint: image.resourceFingerprint
                ) {
                    imageRefreshNeeded = true
                }
            }
            guard blocks.contains(where: {
                $0.kind == UInt8(YU_STORAGE_PROJECTION_BLOCK_FENCED_CODE)
            }) else { return }
            let embeddedResources = try bridge.macosVisualEmbeddedResources(
                revision: revision
            )
            for resource in embeddedResources
                where visibleBlockIndexes.contains(resource.blockIndex) {
                if retainedResourceNeedsRefresh(
                    state: embeddedResourceCoverageState(resource.resourceStatus),
                    resourceFingerprint: resource.resourceFingerprint
                ) {
                    imageRefreshNeeded = true
                }
            }
        } catch {
            // 资源查询失败不影响绘制；下一帧会重试。
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
            // A same-key submit can be reached after the controller has
            // rejected an empty plan. Do not briefly re-show that blank
            // surface; source TextKit remains the canonical visible fallback
            // until a publication with actual draw commands exists.
            // Rust surface 是唯一渲染路径：attach 之后一直可见，
            // 内容由 retained frame 决定，不由 coverage 决定（不变量 I5）。
            surfaceView.setNativeContentVisible(true)
            scheduleImageResourceRefresh()
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
        updateResourceRefreshState(
            snapshot: snapshot,
            revision: revision,
            size: Float(size),
            maxWidth: Float(maxWidth),
            scrollY: Float(scrollY),
            viewportHeight: Float(viewportHeight)
        )
        surfaceView.setNativeContentVisible(true)
        onSurfaceStateChange?()
        if imageRefreshNeeded {
            scheduleImageResourceRefresh()
        } else {
            cancelImageResourceRefresh()
        }
        return snapshot
    }

    func detach() {
        scheduleToken &+= 1
        submitScheduled = false
        cancelImageResourceRefresh()
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
        imageRefreshNeeded = false
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
final class NativeFileWatcher {
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

final class DocumentViewController: NSViewController, NSMenuItemValidation {
    private let bridge: StorageBridge
    private lazy var textView = DocumentTextView(bridge: bridge)
    private let surfaceHostView = MacosSurfaceHostView()
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
    /// The source TextKit mirror must have one complete layout pass before
    /// optional Rust projection/surface work is allowed to run. This keeps
    /// opening a document on the primary editing path independent from the
    /// enhancement layer's first drawable/metrics publication.
    private var visualEnhancementsReady = false
    /// True only when the current decoration sibling came from the same Rust
    /// shaped frame that may become the primary visual surface. TextKit
    /// projected decorations are a fallback overlay and must never hide the
    /// source mirror or leave a stale Metal frame visible underneath it.

    init(bridge: StorageBridge) {
        self.bridge = bridge
        self.surfaceCoordinator = MacosSurfaceHostCoordinator(bridge: bridge)
        self.initialState = bridge.state
        super.init(nibName: nil, bundle: nil)
        surfaceCoordinator.onSurfaceStateChange = { [weak self] in
            self?.textView.refreshTableResizeAccessibility()
            self?.syncSourceGlyphVisibility()
        }
        surfaceCoordinator.onError = { [weak self] error in
            // The source TextKit mirror remains usable when a machine has no
            // Metal drawable; surface lifecycle failure is diagnostic, not a
            // reason to interrupt editing with a modal alert.
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
            self.textView.refreshTableResizeAccessibility()
            self.updateStatus()
            self.syncSourceGlyphVisibility()
            self.scheduleVisualSubmit()
        }
        textView.onBeforeCommand = { [weak self] in
            self?.surfaceCoordinator.prepareForEditorCommand()
        }
        textView.onCaretChange = { [weak self] in
            guard let self else { return }
            self.surfaceCoordinator.invalidateEditorDecorationPublication()
            self.scheduleVisualSubmit()
            // AppKit may deliver selection changes while TextKit is still
            // inside its event callback. Defer the scroll mutation until the
            // same main-thread turn has finished, while retaining the Rust
            // Revision captured by the coordinator's query.
            DispatchQueue.main.async { [weak self] in
                guard let self, self.visualEnhancementsReady else { return }
                self.surfaceCoordinator.revealCaretIfNeeded()
            }
        }
        textView.onError = { [weak self] error in self?.show(error) }
        textView.onTableResizeHover = { [weak self] point in
            self?.surfaceCoordinator.tableResizeHover(at: point) ?? false
        }
        textView.onTaskCheckboxPress = { [weak self] point in
            guard let self,
                  let hit = self.surfaceCoordinator.taskCheckboxHit(at: point) else {
                return false
            }
            return self.textView.toggleTaskPointerHit(hit)
        }
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
        textView.tableResizeAccessibilityProvider = { [weak self] in
            self?.surfaceCoordinator.tableResizeAccessibilityDividers() ?? []
        }
        textView.tableResizeAccessibilityFrameProvider = { [weak self] descriptor in
            self?.surfaceCoordinator.tableResizeAccessibilityFrame(for: descriptor)
                ?? .zero
        }
        textView.onTableResizeAccessibilityAction = { [weak self] descriptor, direction in
            guard let self else { return false }
            return self.surfaceCoordinator.adjustTableResizeAccessibility(
                descriptor,
                direction: direction
            )
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
        textView.refreshTableResizeAccessibility()
        surfaceHostView.onWindowStateChange = { [weak self] attached in
            guard let self else { return }
            if attached {
                if self.visualEnhancementsReady {
                    self.surfaceCoordinator.scheduleSubmit()
                    self.syncSourceGlyphVisibility()
                } else {
                }
                self.textView.refreshTableResizeAccessibility()
            } else {
                self.surfaceCoordinator.detach()
                self.textView.refreshTableResizeAccessibility()
            }
        }
        surfaceHostView.onGeometryChange = { [weak self] in
            self?.scheduleVisualSubmit()
            self?.syncSourceGlyphVisibility()
            self?.textView.refreshTableResizeAccessibility()
        }
        scrollView.contentView.postsBoundsChangedNotifications = true
        surfaceBoundsObserver = NotificationCenter.default.addObserver(
            forName: NSView.boundsDidChangeNotification,
            object: scrollView.contentView,
            queue: .main
        ) { [weak self] _ in
            self?.scheduleVisualSubmit()
            self?.syncSourceGlyphVisibility()
            self?.textView.refreshTableResizeAccessibility()
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
        guard visualEnhancementsReady else {
            // Keep the native source mirror fully visible during the first
            // layout. The enhancement layer is enabled from viewDidAppear,
            // after AppKit has a real window/clip geometry to report.
            return
        }
        let visualWidth = max(
            textView.bounds.width - 2.0 * textView.textContainerOrigin.x,
            1.0
        )
        surfaceCoordinator.setContentWidth(visualWidth)
        textView.refreshTableResizeAccessibility()
        // 指针命中测试直接走 Rust layout，不需要预先建立任何 TextKit 镜像，
        // 因而也没有「适配器未就绪」这个状态。
        syncSourceGlyphVisibility()
        textView.refreshTableResizeAccessibility()
        surfaceCoordinator.scheduleSubmit()
        surfaceCoordinator.revealCaretIfNeeded()
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        guard !visualEnhancementsReady else { return }
        visualEnhancementsReady = true
        // Defer the first optional projection submit by one main-thread turn
        // so source TextKit focus/IME setup has completed before any native
        // surface callback can run.
        DispatchQueue.main.async { [weak self] in
            guard let self, self.view.window != nil else { return }
            self.view.needsLayout = true
            self.scheduleVisualSubmit()
        }
    }

    func refreshFromRust() {
        textView.refreshFromRust()
        surfaceCoordinator.resetTableResizeAfterDocumentChange()
        textView.refreshTableResizeAccessibility()
        initialState = bridge.state
        if initialState.disk == .unchanged {
            promptedExternalDisk = nil
        }
        updateStatus()
        syncSourceGlyphVisibility()
        textView.refreshTableResizeAccessibility()
        scheduleVisualSubmit()
        if visualEnhancementsReady {
            surfaceCoordinator.revealCaretIfNeeded()
        }
    }

    func detachSurfaceHost() {
        surfaceCoordinator.detach()
    }


    /// Rust surface 是唯一渲染路径（不变量 I5）。TextKit 永不绘制像素，
    /// 因此这里没有 gate、没有 fallback reason、没有 coverage 判断：
    /// surface 一旦 attach 就保持可见，未支持的语法由 Rust 按源码文本绘制。
    private func syncSourceGlyphVisibility() {
        surfaceHostView.setNativeContentVisible(true)
    }



    private func scheduleVisualSubmit() {
        guard visualEnhancementsReady else { return }
        surfaceCoordinator.scheduleSubmit()
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
    private var launchSelfCheck = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        let path: String
        launchSelfCheck = CommandLine.arguments.contains("--launch-window-self-check")
        if let argument = CommandLine.arguments.dropFirst().first(where: { !$0.hasPrefix("-") }) {
            path = URL(fileURLWithPath: argument).path
        } else {
            let panel = NSOpenPanel()
            // `.md` is not consistently classified as `UTType.text` by
            // Finder (especially for files created by scripts). Include the
            // Markdown declaration explicitly while retaining ordinary text
            // files in the first-stage host.
            var contentTypes: [UTType] = [.text, .plainText]
            if let markdown = UTType(filenameExtension: "md") {
                contentTypes.append(markdown)
            }
            panel.allowedContentTypes = contentTypes
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
            if launchSelfCheck {
                // Give AppKit one complete appearance/layout turn. This is a
                // real window smoke test: source fallback must become visible
                // before optional Rust surface work is allowed to run.
                DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(500)) {
                    guard window.isVisible else {
                        fputs("Yu launch self-check failed: window is not visible\n", stderr)
                        exit(EXIT_FAILURE)
                    }
                    print("Yu launch self-check: window appeared and remained stable")
                    NSApp.terminate(nil)
                }
            }
        } catch {
            if launchSelfCheck {
                fputs("Yu launch self-check failed: \(error)\n", stderr)
                exit(EXIT_FAILURE)
            }
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

let app = NSApplication.shared
if let flag = CommandLine.arguments.firstIndex(of: "--selection-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runSelectionSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--undo-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runUndoSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--document-workflow-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runDocumentWorkflowSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--document-interaction-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runDocumentInteractionSelfCheck(path: CommandLine.arguments[flag + 1])
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
if let flag = CommandLine.arguments.firstIndex(of: "--macos-table-resize-coordinator-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runMacosTableResizeCoordinatorSelfCheck(path: CommandLine.arguments[flag + 1])
}
if let flag = CommandLine.arguments.firstIndex(of: "--macos-task-checkbox-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runMacosTaskCheckboxSelfCheck(path: CommandLine.arguments[flag + 1])
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
