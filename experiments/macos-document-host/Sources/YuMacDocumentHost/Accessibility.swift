import AppKit
import Darwin
import Foundation
import UniformTypeIdentifiers
import YuStorageFFI

// Accessibility 元素。语义节点由 Rust 按 Revision 提供，这里只把它们
// 映射成 AppKit 的 `NSAccessibilityElementProtocol` 对象。

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
