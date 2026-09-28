// Test-only Preview driver. Effects require an exact open test-file URL and PID.
// It cannot type into other documents, alter preferences, or touch the clipboard.
import AppKit
import ApplicationServices
let args = Array(CommandLine.arguments.dropFirst())
func fail(_ message: String) -> Never { fputs(message + "\n", stderr); exit(1) }
guard args.count >= 3, let pid = Int32(args[0]), let app = NSRunningApplication(processIdentifier: pid), app.bundleIdentifier == "com.apple.Preview" else { fail("Expected explicit Preview PID and test PDF path") }
let expected = URL(fileURLWithPath: args[1]).standardizedFileURL
let root = URL(fileURLWithPath: FileManager.default.currentDirectoryPath).appendingPathComponent("artifacts").path + "/"
guard expected.path.hasPrefix(root), FileManager.default.fileExists(atPath: expected.path), AXIsProcessTrusted() else { fail("Expected an existing artifact PDF and AX permission") }
let application = AXUIElementCreateApplication(pid)
func attr(_ e: AXUIElement, _ name: String) -> CFTypeRef? { var v: CFTypeRef?; return AXUIElementCopyAttributeValue(e, name as CFString, &v) == .success ? v : nil }
let windows = attr(application, "AXWindows") as? [AXUIElement] ?? []
guard let window = windows.first(where: { w in
    guard let value = attr(w, "AXDocument") as? String, let url = URL(string: value) else { return false }
    return url.standardizedFileURL.path == expected.path
}) else { fail("Exact test PDF is not open in Preview") }
func json(_ value: Any) { let data = try! JSONSerialization.data(withJSONObject: value, options: [.sortedKeys, .prettyPrinted]); print(String(decoding: data, as: UTF8.self)) }
func describe(_ v: CFTypeRef?) -> Any {
    guard let v else { return NSNull() }
    if CFGetTypeID(v) == AXValueGetTypeID() {
        let value = unsafeBitCast(v, to: AXValue.self)
        if AXValueGetType(value) == .cgPoint { var p = CGPoint.zero; AXValueGetValue(value, .cgPoint, &p); return ["x":p.x,"y":p.y] }
        if AXValueGetType(value) == .cgSize { var s = CGSize.zero; AXValueGetValue(value, .cgSize, &s); return ["width":s.width,"height":s.height] }
    }
    if let s = v as? String { return String(s.prefix(5000)) }; if let n = v as? NSNumber { return n }; return NSNull()
}
func pump(_ seconds: Double = 0.15) { RunLoop.current.run(until: Date().addingTimeInterval(seconds)) }
func ready() {
    app.activate(options: []); AXUIElementPerformAction(window, "AXRaise" as CFString); pump(0.25)
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid,
          let front = attr(application, "AXFocusedWindow"), CFEqual(front, window), CGPreflightPostEventAccess() else { fail("Expected PDF window lost focus") }
}
func post(_ event: CGEvent?) {
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid, let event else { fail("Preview lost foreground") }
    event.post(tap: .cghidEventTap); pump(0.07)
}
let action = args[2]
if action == "snapshot" {
    var controls: [[String: Any]] = [], visited = 0
    func walk(_ e: AXUIElement, _ depth: Int) {
        guard depth < 22, visited < 3000 else { return }; visited += 1
        var row: [String: Any] = [:]
        for name in ["AXRole", "AXIdentifier", "AXTitle", "AXValue", "AXSelectedText", "AXDescription", "AXPosition", "AXSize", "AXEnabled"] { row[name] = describe(attr(e, name)) }
        controls.append(row)
        for c in attr(e, "AXChildren") as? [AXUIElement] ?? [] { walk(c, depth + 1) }
    }
    walk(window, 0); json(["document": expected.path, "controls": controls]); exit(0)
}
ready()
switch action {
case "key":
    guard args.count >= 4, let key = UInt16(args[3]), [0,3,13,24,27,36,53,115,119,121,116].contains(key) else { fail("Key not admitted for reader checks") }
    let flags: CGEventFlags = args.count > 4 && args[4] == "cmd" ? .maskCommand : []
    for down in [true, false] { let e = CGEvent(keyboardEventSource:nil,virtualKey:key,keyDown:down); e?.flags = down ? flags : []; post(e) }
case "text":
    guard args.count == 4, args[3].utf16.count <= 200 else { fail("Short search text required") }
    let units = Array(args[3].utf16)
    for down in [true,false] { let e = CGEvent(keyboardEventSource:nil,virtualKey:0,keyDown:down); units.withUnsafeBufferPointer { e?.keyboardSetUnicodeString(stringLength:$0.count,unicodeString:$0.baseAddress) }; post(e) }
case "click", "drag":
    guard args.count >= 5, let x = Double(args[3]), let y = Double(args[4]), x.isFinite, y.isFinite else { fail("Point required") }
    let point = CGPoint(x:x,y:y)
    guard let pv = attr(window,"AXPosition"), let sv = attr(window,"AXSize") else { fail("Window geometry unavailable") }
    var origin = CGPoint.zero, size = CGSize.zero
    AXValueGetValue(unsafeBitCast(pv,to:AXValue.self),.cgPoint,&origin); AXValueGetValue(unsafeBitCast(sv,to:AXValue.self),.cgSize,&size)
    let rect = CGRect(origin:origin,size:size)
    guard rect.contains(point) else { fail("Point outside the test document window") }
    func mouse(_ type: CGEventType, _ p: CGPoint) { post(CGEvent(mouseEventSource:nil,mouseType:type,mouseCursorPosition:p,mouseButton:.left)) }
    mouse(.mouseMoved,point); mouse(.leftMouseDown,point)
    var end = point
    if action == "drag" {
        guard args.count == 7, let endX=Double(args[5]), let endY=Double(args[6]), rect.contains(CGPoint(x:endX,y:endY)) else { fail("Drag endpoint outside test window") }
        end=CGPoint(x:endX,y:endY)
        for i in 1...12 { let t=Double(i)/12; mouse(.leftMouseDragged,CGPoint(x:x+(endX-x)*t,y:y+(endY-y)*t)) }
    }
    mouse(.leftMouseUp,end)
case "capture":
    guard args.count == 4, args[3].hasPrefix(root) else { fail("Artifact screenshot path required") }
    let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly,.excludeDesktopElements],kCGNullWindowID) as? [[String:Any]] ?? []
    guard let match = windows.first(where: { ($0[kCGWindowOwnerPID as String] as? Int32) == pid && ($0[kCGWindowLayer as String] as? Int) == 0 }), let number=match[kCGWindowNumber as String] as? Int else { fail("Test window unavailable") }
    let p=Process();p.executableURL=URL(fileURLWithPath:"/usr/sbin/screencapture");p.arguments=["-x","-o","-l\(number)",args[3]];try p.run();p.waitUntilExit();guard p.terminationStatus==0 else {fail("Capture failed")}
default: fail("Unknown reader action")
}
