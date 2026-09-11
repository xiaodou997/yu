import CoreVideo
import Foundation

/// Main-thread wake-up pacer for live presentation. CVDisplayLink invokes its
/// callback on a private thread; the callback only hops to the main queue and
/// never touches AppKit, Rust FFI, or Metal directly.
final class DisplayLinkPacer {
    private var link: CVDisplayLink?
    private let callback: () -> Void
    private(set) var isRunning = false

    init(callback: @escaping () -> Void) {
        self.callback = callback
    }

    func start() {
        guard link == nil else { return }
        var created: CVDisplayLink?
        guard CVDisplayLinkCreateWithActiveCGDisplays(&created) == kCVReturnSuccess,
              let created else { return }
        let context = UnsafeMutableRawPointer(Unmanaged.passUnretained(self).toOpaque())
        let status = CVDisplayLinkSetOutputCallback(created, { _, _, _, _, _, context in
            guard let context else { return kCVReturnSuccess }
            let pacer = Unmanaged<DisplayLinkPacer>.fromOpaque(context).takeUnretainedValue()
            DispatchQueue.main.async { [weak pacer] in pacer?.callback() }
            return kCVReturnSuccess
        }, context)
        guard status == kCVReturnSuccess, CVDisplayLinkStart(created) == kCVReturnSuccess else {
            return
        }
        link = created
        isRunning = true
    }

    func stop() {
        guard let link else { return }
        CVDisplayLinkStop(link)
        self.link = nil
        isRunning = false
    }

    deinit { stop() }
}
