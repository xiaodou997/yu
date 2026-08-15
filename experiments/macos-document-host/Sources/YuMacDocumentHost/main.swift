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
    static let externalChange: Int32 = 4
    static let unsavedChanges: Int32 = 5
    static let htmlImportRejected: Int32 = 18
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

/// The native source mirror is deliberately a view cache, never a second
/// document model. Rust owns canonical source, revision, selection and
/// composition generation; this TextKit object only projects those values for
/// AppKit's NSTextInputClient callbacks.
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

    private let bridge: StorageBridge
    private var canonicalSource: String
    private var canonicalRevision: UInt64
    private var semanticNodes: [NativeAccessibilitySemanticNode] = []
    private var semanticElements: [YuAccessibilitySemanticElement] = []
    private var headingRotorDelegate: YuAccessibilityRotorDelegate!
    private var linkRotorDelegate: YuAccessibilityRotorDelegate!
    private var nativeMarkedRange = NSRange(location: NSNotFound, length: 0)
    private var synchronizingSelection = false
    var onDocumentChange: (() -> Void)?
    var onError: ((Error) -> Void)?

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

    // These queries deliberately read a fresh Rust snapshot instead of
    // trusting TextKit's disposable projection. TextKit remains responsible
    // for drawing and hit testing; source text, UTF-16 length, selection and
    // logical line ranges remain Revision-bound Rust data.
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
        nativeMarkedRange
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
        guard bridge.commandAvailable(command) else { return false }
        do {
            apply(try bridge.executeCommand(command))
            synchronizeProjection()
            postAccessibilityRefresh()
            onDocumentChange?()
            return true
        } catch {
            onError?(error)
            return false
        }
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
    private let statusLabel = NSTextField(labelWithString: "")
    private var saveButton: NSButton?
    private var reloadButton: NSButton?
    private var initialState: NativeStorageState
    private var fileWatcher: NativeFileWatcher?
    private var externalCheckWorkItem: DispatchWorkItem?
    private var promptedExternalDisk: DiskState?

    init(bridge: StorageBridge) {
        self.bridge = bridge
        self.initialState = bridge.state
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    override func loadView() {
        let root = NSView()
        let scrollView = NSScrollView()
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = false
        scrollView.autohidesScrollers = true
        scrollView.translatesAutoresizingMaskIntoConstraints = false

        textView.isEditable = true
        textView.isSelectable = true
        textView.usesFindBar = true
        textView.onDocumentChange = { [weak self] in
            guard let self else { return }
            self.initialState = self.bridge.state
            self.updateStatus()
        }
        textView.onError = { [weak self] error in self?.show(error) }
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

    func refreshFromRust() {
        textView.refreshFromRust()
        initialState = bridge.state
        if initialState.disk == .unchanged {
            promptedExternalDisk = nil
        }
        updateStatus()
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
        controller?.requestClose() ?? true
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let controller, !controller.requestClose() else { return .terminateNow }
        return .terminateCancel
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
        precondition(kinds.contains(4), "fenced-code block missing")
        precondition(kinds.contains(7), "task-list block missing")
        precondition(visualTexts.contains { $0.contains("粗体") })
        precondition(visualTexts.contains { $0.contains("链接") })
        precondition(visualTexts.contains { $0.contains("任务") })
        precondition(visualTexts.contains { $0.contains("fn main") })
        precondition(visualTexts.allSatisfy { !$0.contains("**粗体**") })
        precondition(visualTexts.allSatisfy { !$0.contains("[链接](https://example.com)") })

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
if let flag = CommandLine.arguments.firstIndex(of: "--block-projection-self-check"),
   CommandLine.arguments.indices.contains(flag + 1) {
    runBlockProjectionSelfCheck(path: CommandLine.arguments[flag + 1])
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
