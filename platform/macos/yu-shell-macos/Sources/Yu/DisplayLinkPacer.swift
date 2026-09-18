import AppKit
import QuartzCore

/// A view-bound display link follows the window's display and refresh rate.
/// It only runs during live scrolling; no background polling or thread hops.
final class DisplayLinkPacer: NSObject {
    private var link: CADisplayLink?
    private weak var view: NSView?
    private let callback: () -> Void
    private let traceCycles: Bool
    private(set) var isRunning = false

    init(traceCycles: Bool = false, callback: @escaping () -> Void) {
        self.traceCycles = traceCycles
        self.callback = callback
        super.init()
    }

    func start(view: NSView) {
        guard link == nil, view.window != nil else { return }
        self.view = view
        let created = view.displayLink(target: self, selector: #selector(tick(_:)))
        created.add(to: .main, forMode: .common)
        link = created
        isRunning = true
    }

    @objc private func tick(_ sender: CADisplayLink) {
        guard let window = view?.window,
              window.occlusionState.contains(.visible), !window.isMiniaturized else {
            stop()
            return
        }
        if traceCycles {
            print("yu-render-metric event=display_cycle time_s=\(CACurrentMediaTime()) timestamp_s=\(sender.timestamp) target_s=\(sender.targetTimestamp)")
        }
        callback()
    }

    func stop() {
        link?.invalidate()
        link = nil
        view = nil
        isRunning = false
    }

    deinit { stop() }
}
