import Foundation

/// Civil-day context only: rendering never reads the helper process's clock.
/// Commit the cache only after the FFI consumer accepts the new day.
struct RenderCalendarContext {
    private var interval: DateInterval?
    private var zone: TimeZone?
    private(set) var day: Int32?
    var nextBoundary: Date? { interval?.end }

    mutating func invalidate() {
        interval = nil
        zone = nil
    }

    static func referenceDay(at date: Date, timeZone: TimeZone) -> Int32? {
        guard date.timeIntervalSince1970.isFinite else { return nil }
        var local = Calendar(identifier: .gregorian)
        local.timeZone = timeZone
        let components = local.dateComponents([.era, .year, .month, .day], from: date)
        guard components.era == 1, let year = components.year, (1...9999).contains(year) else { return nil }
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = TimeZone(secondsFromGMT: 0)!
        guard let midnight = utc.date(from: components) else { return nil }
        let value = floor(midnight.timeIntervalSince1970 / 86400)
        guard value >= -719162, value <= 2932896 else { return nil }
        return Int32(value)
    }

    enum Failure: Error { case invalidDate }
    @discardableResult
    mutating func update(at now: Date, timeZone: TimeZone, apply: (Int32) throws -> Void) throws -> Bool {
        if zone == timeZone, let interval, now >= interval.start, now < interval.end { return false }
        guard let nextDay = Self.referenceDay(at: now, timeZone: timeZone) else { throw Failure.invalidDate }
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = timeZone
        guard let next = calendar.dateInterval(of: .day, for: now) else { throw Failure.invalidDate }
        let changed = day != nextDay
        if changed { try apply(nextDay) }
        day = nextDay
        zone = timeZone
        interval = next
        return changed
    }
}

/// One cancellable wall-clock timer per visible surface; no periodic polling.
/// The owner recomputes the civil day after every fire/wake/clock notification.
final class RenderCalendarWakeup {
    private var timer: Timer?
    private var generation: UInt64 = 0
    var scheduledDate: Date? { timer?.fireDate }

    func schedule(boundary: Date, action: @escaping () -> Void) {
        let fireDate = boundary.addingTimeInterval(0.1)
        if timer?.fireDate == fireDate { return }
        cancel()
        let ticket = generation
        let next = Timer(fire: fireDate, interval: 0, repeats: false) { [weak self] _ in
            guard let self, self.generation == ticket else { return }
            self.timer = nil
            action()
        }
        next.tolerance = 0.2
        timer = next
        RunLoop.main.add(next, forMode: .common)
    }

    func cancel() { generation &+= 1; timer?.invalidate(); timer = nil }
    deinit { cancel() }
}
