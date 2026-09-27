import Foundation
import Darwin

/// Opt-in diagnostics for isolated test processes only. No source, selected text,
/// paths, typed characters or input-source identifiers enter this schema.
final class HistoryEventAudit {
    enum Entry: String, Codable {
        case keyEquivalent, keyDown, textSystem, menuSelector, direct
    }
    struct State: Codable {
        let window: Int
        let firstResponder: Bool
        let editable: Bool
        let composition: Bool
        let available: Bool
        let revision: UInt64
    }
    private struct Record: Encodable {
        let schema = 1
        let pid = getpid()
        let phase: String
        let ticket: UInt64
        let uptime: Double
        let entry: Entry?
        let command: UInt8?
        let eventTimestamp: Double?
        let state: State?
        let revisionAfter: UInt64?
        let handled: Bool?
    }
    static func authorized(bundle: String?, environment: [String: String]) -> Bool {
        guard environment["YU_HISTORY_AUDIT"] == "1", let bundle else { return false }
        return ["io.github.xiaodou997.yu.resource-soak.", "io.github.xiaodou997.yu.embedded-check."].contains { prefix in
            guard bundle.hasPrefix(prefix) else { return false }
            let suffix = bundle.dropFirst(prefix.count)
            return suffix.utf8.count == 32 && suffix.utf8.allSatisfy {
                (48...57).contains($0) || (97...102).contains($0)
            }
        }
    }
    static let shared: HistoryEventAudit? = {
        guard authorized(bundle: Bundle.main.bundleIdentifier, environment: ProcessInfo.processInfo.environment) else { return nil }
        return HistoryEventAudit { data in
            // One bounded record at a time; failed diagnostic writes cannot
            // change editing semantics. The reader requires complete pairs.
            data.withUnsafeBytes { bytes in
                guard let base = bytes.baseAddress else { return }
                _ = fwrite(base, 1, bytes.count, stderr)
            }
            fflush(stderr)
        }
    }()
    private let limit: Int
    private let sink: (Data) -> Void
    private let encoder = JSONEncoder()
    private var issued: UInt64 = 0
    private var exhausted = false

    init(limit: Int = 4096, sink: @escaping (Data) -> Void) {
        precondition((1...4096).contains(limit))
        self.limit = limit
        self.sink = sink
    }
    func begin(entry: Entry, command: UInt8, eventTimestamp: Double?, state: State) -> UInt64? {
        precondition(Thread.isMainThread)
        guard command == 8 || command == 9 else { return nil }
        guard issued < UInt64(limit) else {
            if !exhausted {
                exhausted = true
                emit(Record(phase: "truncated", ticket: issued, uptime: ProcessInfo.processInfo.systemUptime,
                    entry: nil, command: nil, eventTimestamp: nil, state: nil, revisionAfter: nil, handled: nil))
            }
            return nil
        }
        issued += 1
        let timestamp = eventTimestamp.flatMap { $0.isFinite && $0 >= 0 ? $0 : nil }
        emit(Record(phase: "begin", ticket: issued, uptime: ProcessInfo.processInfo.systemUptime,
            entry: entry, command: command, eventTimestamp: timestamp, state: state, revisionAfter: nil, handled: nil))
        return issued
    }
    func end(ticket: UInt64?, revision: UInt64, handled: Bool) {
        precondition(Thread.isMainThread)
        guard let ticket else { return }
        emit(Record(phase: "end", ticket: ticket, uptime: ProcessInfo.processInfo.systemUptime,
            entry: nil, command: nil, eventTimestamp: nil, state: nil, revisionAfter: revision, handled: handled))
    }
    private func emit(_ record: Record) {
        guard let json = try? encoder.encode(record) else { return }
        var line = Data("yu-history-audit ".utf8)
        line.append(json)
        line.append(10)
        sink(line)
    }
}
