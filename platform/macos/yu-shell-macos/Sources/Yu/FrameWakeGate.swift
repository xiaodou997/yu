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

/// Only an explicit loss of the latest drawable requests recovery. A persistent
/// compositor failure must not turn an idle document into an unbounded loop.
/// Ordinary content/geometry/lifecycle requests start a fresh recovery budget.
struct FramePresentationRecoveryBudget {
    static let limit = 8
    private(set) var attempts = 0
    private(set) var pending = false

    mutating func request(attached: Bool, visible: Bool, sameSurface: Bool) -> Bool {
        guard attached, visible, sameSurface, !pending, attempts < Self.limit else { return false }
        attempts += 1
        pending = true
        return true
    }

    mutating func consume() -> Bool {
        guard pending else { return false }
        pending = false
        return true
    }

    mutating func reset() { attempts = 0; pending = false }
}
