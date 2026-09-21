import AppKit

/// Checks bounded source windows around visible text and the editing position.
/// Only Rust publishes diagnostics; this adapter owns cancellable OS requests.
final class NativeSpellingCoordinator {
    private let bridge: StorageBridge
    private let service = NativeSpellingService()
    private let changed: () -> Void
    private(set) var publishedDiagnosticCount = 0
    private var generation: UInt64 = 0
    private var pending: DispatchWorkItem?
    private var sourceSnapshot: NSString?
    private var identity: Identity?
    private struct Identity: Equatable {
        let revision: UInt64
        let composition: UInt64
        let enabled: Bool
        let chunks: [Int]
    }

    init(bridge: StorageBridge, changed: @escaping () -> Void) {
        self.bridge = bridge
        self.changed = changed
    }
    deinit { pending?.cancel() }

    func update(source: String, visible: NSRange?, focus: Int, enabled: Bool) {
        let text = source as NSString
        let revision = bridge.revision
        let composition = bridge.composition
        let chunkSize = 4096
        var chunks: [Int] = []
        func include(_ range: NSRange) {
            guard text.length > 0 else { return }
            let first = min(max(range.location, 0), text.length - 1) / chunkSize
            let last = min(max(NSMaxRange(range) - 1, 0), text.length - 1) / chunkSize
            for index in first...max(first, last) where !chunks.contains(index) { chunks.append(index) }
        }
        // Active text first, followed by the current reading viewport. Distant
        // regions are not bridged into an enormous whole-document request.
        include(NSRange(location: min(max(focus, 0), text.length), length: 0))
        if let visible { include(visible) }
        let next = Identity(revision: revision, composition: composition.generation, enabled: enabled, chunks: chunks)
        guard next != identity else { return }
        identity = next
        sourceSnapshot = text
        generation &+= 1
        let ticket = generation
        pending?.cancel()
        guard !composition.active else { sourceSnapshot = nil; return }
        guard enabled, !chunks.isEmpty else {
            sourceSnapshot = nil
            if (try? bridge.setSpellingDiagnostics([], revision: revision)) != nil { publishedDiagnosticCount = 0; changed() }
            return
        }
        let task = DispatchWorkItem { [weak self] in
            guard let self, self.current(ticket, revision: revision), let source = self.sourceSnapshot else { return }
            self.check(source: source, chunks: chunks, index: 0, collected: [], revision: revision, ticket: ticket)
        }
        pending = task
        DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(300), execute: task)
    }

    private func current(_ ticket: UInt64, revision: UInt64) -> Bool {
        generation == ticket && bridge.revision == revision && !bridge.composition.active
    }

    private func check(source: NSString, chunks: [Int], index: Int, collected: [NSRange], revision: UInt64, ticket: UInt64) {
        guard current(ticket, revision: revision), index < chunks.count else { return }
        // Overlap protects ordinary words crossing a request boundary. A word
        // clipped at the outer edge is never published as a partial diagnostic.
        let start = max(0, chunks[index] * 4096 - 256)
        let end = min(source.length, (chunks[index] + 1) * 4096 + 256)
        let region = source.rangeOfComposedCharacterSequences(for: NSRange(location: start, length: end - start))
        do {
            let allowed = try bridge.spellingRanges(in: region, revision: revision).map {
                NSRange(location: $0.location - region.location, length: $0.length)
            }
            let fragment = source.substring(with: region)
            let sourceLength = source.length
            service.requestDiagnostics(source: fragment, allowed: allowed, isCancelled: { [weak self] in
                self?.current(ticket, revision: revision) != true
            }) { [weak self] ranges in
                guard let self, self.current(ticket, revision: revision) else { return }
                let absolute = ranges.filter {
                    ($0.location > 0 || region.location == 0) && (NSMaxRange($0) < region.length || NSMaxRange(region) == sourceLength)
                }.map { NSRange(location: region.location + $0.location, length: $0.length) }
                let combined = Array(Set(collected + absolute)).sorted { $0.location < $1.location }
                // The native publisher has a fixed diagnostic budget. Keep
                // visible/active work responsive instead of growing unbounded.
                let diagnostics = Array(combined.prefix(4096))
                do {
                    try self.bridge.setSpellingDiagnostics(diagnostics, revision: revision)
                    self.publishedDiagnosticCount = diagnostics.count
                    self.changed()
                    if index + 1 < chunks.count {
                        DispatchQueue.main.async { [weak self] in
                            guard let self, self.current(ticket, revision: revision), let source = self.sourceSnapshot else { return }
                            self.check(source: source, chunks: chunks, index: index + 1, collected: diagnostics, revision: revision, ticket: ticket)
                        }
                    }
                } catch { /* A replaced document/IME request will schedule fresh work. */ }
            }
        } catch { /* Revision-bound queries never publish partial stale state. */ }
    }
}
