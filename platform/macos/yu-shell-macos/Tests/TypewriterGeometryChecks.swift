import Foundation

@main
struct TypewriterGeometryChecks {
    static func main() {
        // Shared caret includes line metrics, while theme top is a single
        // document-space translation. It must never be applied twice.
        let target = NativeTypewriterGeometry.scrollTarget(caretY: 1000, caretHeight: 26,
            top: 30, viewportHeight: 600, maximumScroll: 2000)
        precondition(target == 743)
        precondition(1000 + 30 + 13 - target == 300)
        precondition(NativeTypewriterGeometry.scrollTarget(caretY: 0, caretHeight: 26,
            top: 30, viewportHeight: 600, maximumScroll: 2000) == 0)
        precondition(NativeTypewriterGeometry.scrollTarget(caretY: 3000, caretHeight: 52,
            top: 30, viewportHeight: 400, maximumScroll: 2800) == 2800)
        precondition(NativeTypewriterGeometry.scrollTarget(caretY: 40, caretHeight: 26,
            top: 30, viewportHeight: 800, maximumScroll: 0) == 0)
        print("Typewriter scroll geometry: centered line, theme origin, document boundaries passed")
    }
}
