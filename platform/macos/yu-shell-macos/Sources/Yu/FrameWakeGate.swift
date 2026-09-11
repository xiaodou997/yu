/// Single pending wake-up, independent of the latest content request.
/// Ordinary requests preserve the deadline; immediate requests may promote it.
struct FrameWakeGate {
    private var serial: UInt64 = 0
    private(set) var pending: UInt64?

    mutating func request(immediate: Bool) -> UInt64? {
        if pending != nil && !immediate { return nil }
        serial &+= 1
        pending = serial
        return serial
    }

    mutating func consume(_ ticket: UInt64) -> Bool {
        guard pending == ticket else { return false }
        pending = nil
        return true
    }

    mutating func invalidate() {
        serial &+= 1
        pending = nil
    }
}

/// Resource invalidation is sticky across ordinary scroll requests and is
/// consumed only when a submission actually starts.
struct FrameSubmitIntent {
    private var force = false

    mutating func merge(force: Bool) {
        self.force = self.force || force
    }

    mutating func takeForce() -> Bool {
        let result = force
        force = false
        return result
    }
}
