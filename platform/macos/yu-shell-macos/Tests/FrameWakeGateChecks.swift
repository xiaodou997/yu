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
        print("FrameWakeGate: burst, promotion, duplicate, lifecycle and resource-intent checks passed")
    }
}
