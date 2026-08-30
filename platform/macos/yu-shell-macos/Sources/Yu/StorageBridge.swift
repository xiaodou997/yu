import AppKit
import Darwin
import Foundation
import UniformTypeIdentifiers
import YuStorageFFI

// Rust `yu-storage-ffi` 的 Swift 封装：C ABI 调用、错误码映射，以及
// 跨边界结构的 Swift 镜像。这里不做任何决策，只做搬运与类型转换。

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

    init(_ endpoints: NativeSelectionEndpoints) {
        revision = endpoints.revision
        let start = min(endpoints.anchorUTF16, endpoints.focusUTF16)
        let end = max(endpoints.anchorUTF16, endpoints.focusUTF16)
        range = NSRange(location: Int(start), length: Int(end - start))
        affinity = endpoints.affinity
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
    /// VoiceOver 每次增减的列宽步长，由 Rust 按表格行高给出。
    let adjustStep: CGFloat

    init(
        revision: UInt64,
        blockIndex: UInt64,
        kind: UInt8,
        index: UInt64,
        columnCount: UInt64,
        rect: CGRect,
        tableSourceRange: NSRange,
        adjustStep: CGFloat = 8.0
    ) {
        self.revision = revision
        self.blockIndex = blockIndex
        self.kind = kind
        self.index = index
        self.columnCount = columnCount
        self.rect = rect
        self.tableSourceRange = tableSourceRange
        self.adjustStep = adjustStep
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
        adjustStep = CGFloat(value.adjust_step)
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
    let selectionDecorationCount: Int
    let caretDecorationCount: Int
    let resourceRefreshPending: Bool

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
        selectionDecorationCount = Int(value.selection_decoration_count)
        caretDecorationCount = Int(value.caret_decoration_count)
        resourceRefreshPending = value.resource_refresh_pending != 0
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
    let selectionDecorationCount: Int
    let caretDecorationCount: Int
    /// Rust 在提交这一帧时给出的结论：可见范围内还有资源没落定。
    /// 平台据此安排一次有界轮询，不自己判断资源状态。
    let resourceRefreshPending: Bool
    /// 这一帧渲染出来的文档总高度，可滚动范围的唯一依据。
    let contentHeight: CGFloat

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
        selectionDecorationCount = Int(value.selection_decoration_count)
        caretDecorationCount = Int(value.caret_decoration_count)
        resourceRefreshPending = value.resource_refresh_pending != 0
        contentHeight = CGFloat(value.content_height)
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

/// 大纲里的一条标题。与 `NativeAccessibilitySemanticNode` 是并列的两份派生
/// 视图，不是一份套着另一份：语义树是扁平的（每个块都挂在 Document 下），
/// 大纲的全部内容恰恰是标题之间的层级。
///
/// `parent` 指向同一份快照里的 `index`，不是块下标；`UInt32.max` 表示这是
/// 一条根级标题。导航要用的是 `block` 与 `labelRange`。
struct NativeOutlineItem {
    let revision: UInt64
    let index: UInt32
    let parent: UInt32
    let level: UInt8
    let block: UInt64
    let sourceRange: NSRange
    let labelRange: NSRange

    init(_ value: YuStorageOutlineItem) {
        revision = value.revision
        index = value.index
        parent = value.parent
        level = value.level
        block = value.block
        sourceRange = NSRange(
            location: Int(value.source_start_utf16),
            length: Int(value.source_end_utf16 - value.source_start_utf16)
        )
        labelRange = NSRange(
            location: Int(value.label_start_utf16),
            length: Int(value.label_end_utf16 - value.label_start_utf16)
        )
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

    /// 用一个文档坐标点探测或开始一次分隔线拖动。
    /// `pointerPosition` 只对 `.begin` 有意义。
    func macosTableResizeAtDocumentPoint(
        revision: UInt64,
        action: UInt8,
        size: Float,
        maxWidth: Float,
        point: CGPoint,
        tolerance: Float,
        pointerPosition: Float = 0.0
    ) throws -> YuStorageTableResizeHit {
        var value = YuStorageTableResizeHit()
        let status = yu_storage_session_macos_table_resize_at_point(
            handle,
            revision,
            action,
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

    /// 推进一次分隔线拖动。`pointerPosition` 只对 `.update` 有意义。
    @discardableResult
    func tableResizeAction(
        revision: UInt64,
        action: UInt8,
        pointerPosition: Float = 0.0
    ) throws -> NativeTableResizeCommit {
        var value = YuStorageTableResizeCommit()
        let status = yu_storage_session_table_resize_action(
            handle,
            revision,
            action,
            pointerPosition,
            &value
        )
        guard status == StorageStatus.ok else {
            throw BridgeError.operation(status)
        }
        return NativeTableResizeCommit(value)
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

    /// 询问 Rust：按这个几何提交下一帧，是否与已经在屏幕上的那一帧等价。
    ///
    /// 平台只提供 AppKit 才知道的东西——view bounds、clip view 滚动位置、
    /// backing scale。Revision、composition generation、selection 与表格
    /// resize 覆盖全部由 Rust 自己读取，平台不再为了做决策而反复查询状态
    /// （不变量 I3）。
    ///
    /// 查询失败一律按「不是当前帧」处理：多提交一帧只是浪费一次绘制，
    /// 少提交一帧则是画面停在旧内容上，而且没有任何报错。
    func macosFrameIsCurrent(
        size: Float,
        maxWidth: Float,
        scrollY: Float,
        viewportHeight: Float,
        surfaceWidth: Double,
        surfaceHeight: Double,
        scale: Double
    ) -> Bool {
        var geometry = YuStorageFrameGeometry(
            size: size,
            max_width: maxWidth,
            scroll_y: scrollY,
            viewport_height: viewportHeight,
            surface_width: surfaceWidth,
            surface_height: surfaceHeight,
            scale: scale
        )
        var current: UInt8 = 0
        let status = yu_storage_session_macos_frame_is_current(handle, &geometry, &current)
        guard status == StorageStatus.ok else { return false }
        return current != 0
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
        viewportHeight: Float
    ) throws -> NativeCaretScrollRequest {
        var value = YuStorageCaretScrollRequest()
        let status = yu_storage_session_macos_shaped_caret_scroll_request(
            handle,
            revision,
            size,
            maxWidth,
            scrollY,
            viewportHeight,
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

    /// 有序选区。anchor/focus 是完整形式，有序区间由它推导——两者不需要各占
    /// 一个 FFI 入口。
    var selection: NativeSelection {
        NativeSelection(selectionEndpoints)
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
        // count/fill 的长度查询形式：空指针 + 0 容量只回填数量。
        var count = 0
        let countStatus = yu_storage_session_accessibility_semantic_nodes_v2(
            handle,
            revision,
            nil,
            0,
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

    /// 这一版的大纲：文档里全部标题，按文档顺序，带层级。
    ///
    /// 与语义树同一个两遍协议（空指针 + 0 容量只回报条数）。刷新可能与关闭、
    /// 重载或外部改动撞在一起，所以 Revision 失配返回 nil 而不是中止进程——
    /// 面板保留上一版，比让一次刷新杀掉进程好。
    var outlineItemsIfAvailable: [NativeOutlineItem]? {
        let revision = state.revision
        var count = 0
        let countStatus = yu_storage_session_outline_items(
            handle,
            revision,
            nil,
            0,
            &count
        )
        guard countStatus == StorageStatus.ok else { return nil }
        guard count > 0 else { return [] }

        var values = Array(repeating: YuStorageOutlineItem(), count: count)
        var written = 0
        let status = values.withUnsafeMutableBufferPointer { buffer in
            yu_storage_session_outline_items(
                handle,
                revision,
                buffer.baseAddress,
                buffer.count,
                &written
            )
        }
        guard status == StorageStatus.ok, written == count else { return nil }
        return values.map(NativeOutlineItem.init)
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

    /// 正向选区就是 anchor 在前、focus 在后的端点形式。
    func setSelection(_ range: NSRange, affinity: UInt8 = 1) throws {
        try setSelectionEndpoints(
            anchorUTF16: UInt64(range.location),
            focusUTF16: UInt64(range.location + range.length),
            affinity: affinity
        )
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
                UInt8(YU_STORAGE_CLIPBOARD_TEXT),
                output,
                capacity,
                written
            )
        }
    }

    func copySelectionHTML(revision: UInt64) throws -> String {
        try copyBytesThrowing { output, capacity, written in
            yu_storage_session_copy_selection(
                handle,
                revision,
                UInt8(YU_STORAGE_CLIPBOARD_HTML),
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

    /// 结束一次关闭协商。取消 / 保存后关闭 / 丢弃后关闭是同一个协商的三个出口。
    func resolveClose(_ action: UInt8) throws {
        let status = yu_storage_session_close_resolve(handle, action)
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
