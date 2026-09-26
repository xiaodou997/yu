import Foundation

@main
struct RenderCalendarChecks {
    static func main() throws {
        let parser = ISO8601DateFormatter()
        func date(_ text: String) -> Date { parser.date(from: text)! }
        let utc = TimeZone(secondsFromGMT: 0)!
        let la = TimeZone(identifier: "America/Los_Angeles")!
        let perth = TimeZone(identifier: "Australia/Perth")!
        let jan = date("2026-01-01T00:00:00Z")
        let initial = RenderCalendarContext.referenceDay(at: jan, timeZone: utc)!
        var passed = 0

        // Ordinary midnight and missed notifications (no input is necessary).
        do {
            var context = RenderCalendarContext()
            var calls: [Int32] = []
            precondition(try! context.update(at: jan, timeZone: utc) { calls.append($0) })
            precondition(!(try! context.update(at: jan.addingTimeInterval(86399), timeZone: utc) { calls.append($0) }))
            precondition(try! context.update(at: jan.addingTimeInterval(86400), timeZone: utc) { calls.append($0) })
            precondition(calls == [initial, initial + 1])
            passed += 1
        }
        // Sleep through several dates, then rollback; no +1-day assumption.
        do {
            var context = RenderCalendarContext()
            var calls: [Int32] = []
            try context.update(at: jan, timeZone: utc) { calls.append($0) }
            context.invalidate()
            try context.update(at: jan.addingTimeInterval(86400 * 4 + 120), timeZone: utc) { calls.append($0) }
            try context.update(at: jan.addingTimeInterval(-86400), timeZone: utc) { calls.append($0) }
            precondition(calls == [initial, initial + 4, initial - 1])
            passed += 1
        }
        // Zone changes use local civil date, including one-day backward moves.
        do {
            var context = RenderCalendarContext()
            var calls: [Int32] = []
            try context.update(at: jan, timeZone: utc) { calls.append($0) }
            try context.update(at: jan, timeZone: la) { calls.append($0) }
            try context.update(at: jan, timeZone: perth) { calls.append($0) }
            context.invalidate()
            precondition(!(try! context.update(at: jan, timeZone: perth) { calls.append($0) }))
            precondition(calls == [initial, initial - 1, initial])
            passed += 1
        }
        // Actual local-day lengths are 23 and 25 hours, not fixed 86400 seconds.
        do {
            for (start, hours) in [("2026-03-08T08:00:00Z", 23), ("2026-11-01T07:00:00Z", 25)] {
                var context = RenderCalendarContext()
                let now = date(start)
                var calls = 0
                try context.update(at: now, timeZone: la) { _ in calls += 1 }
                precondition(context.nextBoundary!.timeIntervalSince(now) == Double(hours * 3600))
                precondition(!(try! context.update(at: now.addingTimeInterval(Double(hours * 3600 - 1)), timeZone: la) { _ in calls += 1 }))
                precondition(try! context.update(at: now.addingTimeInterval(Double(hours * 3600)), timeZone: la) { _ in calls += 1 })
                precondition(calls == 2)
            }
            passed += 1
        }
        // A rejected FFI publication must be retried, never cached as success.
        do {
            enum Rejected: Error { case expected }
            var context = RenderCalendarContext()
            try context.update(at: jan, timeZone: utc) { _ in }
            let boundary = context.nextBoundary
            do {
                try context.update(at: jan.addingTimeInterval(86400), timeZone: utc) { _ in throw Rejected.expected }
                preconditionFailure("publication must fail")
            } catch Rejected.expected { }
            precondition(context.day == initial && context.nextBoundary == boundary)
            var retry = 0
            precondition(try! context.update(at: jan.addingTimeInterval(86400), timeZone: utc) { _ in retry += 1 })
            precondition(retry == 1)
            passed += 1
        }
        // Invalid input does not corrupt the last published day.
        do {
            var context = RenderCalendarContext()
            try context.update(at: jan, timeZone: utc) { _ in }
            do {
                try context.update(at: Date(timeIntervalSince1970: .nan), timeZone: utc) { _ in preconditionFailure("invalid date published") }
                preconditionFailure("invalid date accepted")
            } catch RenderCalendarContext.Failure.invalidDate { }
            precondition(context.day == initial)
            passed += 1
        }
        // Main-runloop timer is one-shot, cancellable, replaceable, and weak-owned.
        do {
            var fired = 0
            var wake: RenderCalendarWakeup? = RenderCalendarWakeup()
            weak let weakWake = wake
            let later = Date().addingTimeInterval(3600)
            wake!.schedule(boundary: later) { fired += 100 }
            precondition(wake!.scheduledDate == later.addingTimeInterval(0.1))
            wake!.cancel()
            precondition(wake!.scheduledDate == nil)
            wake!.schedule(boundary: Date().addingTimeInterval(-1)) { fired += 1 }
            let end = Date().addingTimeInterval(2)
            while fired == 0 && Date() < end { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
            precondition(fired == 1 && wake!.scheduledDate == nil)
            wake!.schedule(boundary: later) { fired += 100 }
            wake = nil
            precondition(weakWake == nil && fired == 1)
            passed += 1
        }
        print("Render calendar checks: \(passed) passed; midnight, sleep/rollback, zones, DST, retry, invalid date, timer cleanup. Not real-window acceptance.")
    }
}
