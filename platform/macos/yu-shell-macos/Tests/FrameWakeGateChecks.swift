// Standalone: compile with Sources/Yu/FrameWakeGate.swift. No AppKit/FFI needed.
@main
enum FrameWakeGateChecks {
    static func main() {
        var gate = FrameWakeGate()
        let first = gate.request(immediate: false)!
        for _ in 0..<1_000 {
            precondition(gate.request(immediate: false) == nil)
        }
        precondition(gate.consume(first), "burst must not starve the original wake-up")
        precondition(!gate.consume(first), "wake-up must run only once")
        let delayed = gate.request(immediate: false)!
        let urgent = gate.request(immediate: true)!
        precondition(!gate.consume(delayed), "cancelled wake-up must be rejected")
        precondition(gate.consume(urgent))
        let detached = gate.request(immediate: false)!
        gate.invalidate()
        let rebound = gate.request(immediate: false)!
        precondition(!gate.consume(detached), "old lifecycle must not consume new wake-up")
        precondition(gate.consume(rebound))
        var intent = FrameSubmitIntent()
        intent.merge(force: true)
        for _ in 0..<1_000 { intent.merge(force: false) }
        precondition(intent.takeForce(), "scroll must not erase resource invalidation")
        precondition(!intent.takeForce(), "refresh is consumed exactly once")
        intent.merge(force: true)
        intent = FrameSubmitIntent()
        precondition(!intent.takeForce(), "lifecycle reset must clear pending refresh")
        var recovery = FramePresentationRecoveryBudget()
        precondition(!recovery.request(attached: false, visible: true, sameSurface: true))
        precondition(!recovery.request(attached: true, visible: false, sameSurface: true))
        precondition(!recovery.request(attached: true, visible: true, sameSurface: false))
        precondition(recovery.attempts == 0, "unrelated or hidden surfaces must not consume a retry")
        for _ in 0..<FramePresentationRecoveryBudget.limit {
            precondition(recovery.request(attached: true, visible: true, sameSurface: true))
            precondition(!recovery.request(attached: true, visible: true, sameSurface: true), "coalesce until a display tick")
            precondition(recovery.consume())
            precondition(!recovery.consume(), "one retry per display tick")
        }
        for _ in 0..<1_000 {
            precondition(!recovery.request(attached: true, visible: true, sameSurface: true))
        }
        precondition(recovery.attempts == FramePresentationRecoveryBudget.limit)
        recovery.reset()
        precondition(recovery.request(attached: true, visible: true, sameSurface: true))
        precondition(recovery.attempts == 1, "a new request/lifecycle restores the bounded budget")
        recovery.reset()
        precondition(!recovery.consume(), "reset rejects an old queued display tick")
        print("Presentation recovery: hidden/foreign/detached rejection, bounded retries and reset passed")
        print("FrameWakeGate: burst, promotion, duplicate, lifecycle and resource-intent checks passed")
    }
}
