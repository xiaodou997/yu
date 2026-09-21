import Foundation

/// Converts the shared layout caret into a native clip-view origin.
/// The first lines clamp at the document start; trailing viewport padding
/// permits the final line to reach the same writing position.
enum NativeTypewriterGeometry {
    static func scrollTarget(caretY: CGFloat, caretHeight: CGFloat, top: CGFloat,
                             viewportHeight: CGFloat, maximumScroll: CGFloat) -> CGFloat {
        min(max(caretY + top + caretHeight / 2 - viewportHeight / 2, 0), max(maximumScroll, 0))
    }
}
