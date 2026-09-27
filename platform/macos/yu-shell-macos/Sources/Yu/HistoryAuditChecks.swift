import Foundation

/// Pure diagnostic contracts; not external event delivery or leak acceptance.
func checkHistoryAuditContracts() throws {
    func require(_ value: Bool, _ message: String) throws {
        if !value { throw NSError(domain: "YuHistoryAuditChecks", code: 1,
            userInfo: [NSLocalizedDescriptionKey: message]) }
    }
    let prefix = "io.github.xiaodou997.yu.resource-soak."
    let suffix = String(repeating: "a", count: 32)
    try require(HistoryEventAudit.authorized(bundle: prefix + suffix, environment: ["YU_HISTORY_AUDIT": "1"]), "isolated opt-in")
    for bundle in [nil, "io.github.xiaodou997.yu", prefix, prefix + suffix + ".other", prefix + String(repeating: "g", count: 32)] {
        try require(!HistoryEventAudit.authorized(bundle: bundle, environment: ["YU_HISTORY_AUDIT": "1"]), "bundle scope")
    }
    for flag in ["", "0", "true"] {
        try require(!HistoryEventAudit.authorized(bundle: prefix + suffix, environment: ["YU_HISTORY_AUDIT": flag]), "explicit opt-in")
    }
    var records: [[String: Any]] = []
    let audit = HistoryEventAudit(limit: 2) { data in
        let prefix = Data("yu-history-audit ".utf8)
        precondition(data.starts(with: prefix) && data.last == 10)
        records.append(try! JSONSerialization.jsonObject(with: data.dropFirst(prefix.count)) as! [String: Any])
    }
    let state = HistoryEventAudit.State(window: 7, firstResponder: true, editable: true,
        composition: false, available: true, revision: 8)
    try require(audit.begin(entry: .direct, command: 1, eventTimestamp: nil, state: state) == nil && records.isEmpty, "non-history excluded")
    let first = audit.begin(entry: .keyDown, command: 8, eventTimestamp: 2, state: state)
    audit.end(ticket: first, revision: 9, handled: true)
    let second = audit.begin(entry: .keyEquivalent, command: 9, eventTimestamp: .nan, state: state)
    audit.end(ticket: second, revision: 8, handled: false)
    for _ in 0..<1000 {
        let ticket = audit.begin(entry: .menuSelector, command: 8, eventTimestamp: nil, state: state)
        audit.end(ticket: ticket, revision: 9, handled: true)
    }
    try require(records.count == 5 && records.last?["phase"] as? String == "truncated", "bounded pairs and one truncation")
    try require(records[0]["ticket"] as? Int == 1 && records[1]["ticket"] as? Int == 1 && records[2]["ticket"] as? Int == 2, "paired identities")
    try require(records[2]["eventTimestamp"] == nil, "nonfinite timestamp excluded")
    let keys: Set<String> = ["schema", "pid", "phase", "ticket", "uptime", "entry", "command", "eventTimestamp", "state", "revisionAfter", "handled"]
    let stateKeys: Set<String> = ["window", "firstResponder", "editable", "composition", "available", "revision"]
    for record in records {
        try require(Set(record.keys).isSubset(of: keys), "no free-text fields")
        if let snapshot = record["state"] as? [String: Any] {
            try require(Set(snapshot.keys) == stateKeys, "minimal state schema")
        }
    }
    print("Yu history audit: isolated opt-in, typed schema, finite timestamps, paired IDs and bounded output passed")
}
