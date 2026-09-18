// External macOS event/AX driver. Mutations require the explicitly targeted app
// to be frontmost; never type into whichever unrelated application has focus.
import AppKit
import ApplicationServices
import Carbon

var failureCleanup: (() -> Void)?
func fail(_ message: String) -> Never { failureCleanup?(); failureCleanup = nil; fputs(message + "\n", stderr); exit(1) }
func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
    return value
}
func describe(_ value: CFTypeRef?) -> Any {
    guard let value else { return NSNull() }
    if CFGetTypeID(value) == AXValueGetTypeID() {
        let ax = unsafeBitCast(value, to: AXValue.self)
        switch AXValueGetType(ax) {
        case .cgPoint:
            var p = CGPoint.zero; AXValueGetValue(ax, .cgPoint, &p); return ["x":p.x,"y":p.y]
        case .cgSize:
            var p = CGSize.zero; AXValueGetValue(ax, .cgSize, &p); return ["width":p.width,"height":p.height]
        case .cgRect:
            var p = CGRect.zero; AXValueGetValue(ax, .cgRect, &p); return ["x":p.minX,"y":p.minY,"width":p.width,"height":p.height]
        case .cfRange:
            var p = CFRange(); AXValueGetValue(ax, .cfRange, &p); return ["location":p.location,"length":p.length]
        default: return "AXValue"
        }
    }
    if let string = value as? String { return string }
    if let number = value as? NSNumber { return number }
    return String(describing:value)
}
func sourceID(_ source: TISInputSource) -> String {
    guard let pointer = TISGetInputSourceProperty(source,kTISPropertyInputSourceID) else { return "" }
    return Unmanaged<CFString>.fromOpaque(pointer).takeUnretainedValue() as String
}
func json(_ value: Any) {
    guard let data = try? JSONSerialization.data(withJSONObject:value,options:[.prettyPrinted,.sortedKeys]) else { fail("JSON encoding failed") }
    print(String(data:data,encoding:.utf8)!)
}
let args = Array(CommandLine.arguments.dropFirst())
if args == ["--preflight"] {
    let session = CGSessionCopyCurrentDictionary() as? [String: Any]
    let locked = session == nil || (session?["CGSSessionScreenIsLocked"] as? Bool == true)
    let console = session?["kCGSSessionOnConsoleKey"] as? Bool == true
    let trusted = AXIsProcessTrusted()
    let posting = CGPreflightPostEventAccess()
    let capture = CGPreflightScreenCaptureAccess()
    let ready = !locked && console && trusted && posting && capture
    json(["ready": ready, "screen_locked": locked, "on_console": console,
        "accessibility_trusted": trusted, "event_posting_allowed": posting,
        "screen_capture_allowed": capture])
    exit(ready ? 0 : 2)
}
guard args.count >= 2, let pid = Int32(args[0]), let app = NSRunningApplication(processIdentifier:pid) else { fail("Usage: native-event-driver PID snapshot|key|source|click|drag|select|bounds [arguments]") }
guard AXIsProcessTrusted() else { fail("Accessibility access is required") }
let application = AXUIElementCreateApplication(pid)
var visited = 0
func findEditor(_ element: AXUIElement, depth: Int = 0) -> AXUIElement? {
    guard depth < 18, visited < 2000 else { return nil }; visited += 1
    if attribute(element,"AXRole") as? String == "AXTextArea" { return element }
    for child in attribute(element,"AXChildren") as? [AXUIElement] ?? [] {
        if let result = findEditor(child,depth:depth+1) { return result }
    }
    return nil
}
let action = args[1]
if !["snapshot","bounds"].contains(action) {
    guard app.bundleIdentifier?.hasPrefix("io.github.xiaodou997.yu.") == true else { fail("Target is not an isolated Yu check bundle") }
    if NSWorkspace.shared.frontmostApplication?.processIdentifier != pid {
        app.activate(options:[])
        let deadline = Date().addingTimeInterval(3)
        repeat { RunLoop.current.run(until: Date().addingTimeInterval(0.05)) }
        while NSWorkspace.shared.frontmostApplication?.processIdentifier != pid && Date() < deadline
        RunLoop.current.run(until: Date().addingTimeInterval(0.4))
    }
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else { fail("Target app did not become frontmost") }
}
guard let editor = findEditor(application) else { fail("No native document AXTextArea found") }
func flags(_ raw: String) -> CGEventFlags {
    var result: CGEventFlags = []
    for name in raw.split(separator:"+") {
        switch name { case "cmd":result.insert(.maskCommand); case "shift":result.insert(.maskShift); case "alt":result.insert(.maskAlternate); case "ctrl":result.insert(.maskControl); default:break }
    }
    return result
}
func post(_ event: CGEvent?) {
    // Command-Q can terminate the target on key-down. Deliver only its key-up
    // to that explicit PID rather than the newly frontmost application.
    if let event, event.type == .keyUp,
       NSWorkspace.shared.frontmostApplication?.processIdentifier != pid || app.isTerminated {
        event.postToPid(pid)
        return
    }
    guard CGPreflightPostEventAccess(), NSWorkspace.shared.frontmostApplication?.processIdentifier == pid, (attribute(application,"AXFrontmost") as? Bool) == true, let event else { fail("Cannot safely post event to foreground target") }
    event.post(tap:.cghidEventTap)
    RunLoop.current.run(until: Date().addingTimeInterval(0.08))
}
switch action {
case "snapshot", "activate":
    var result: [String:Any] = ["pid":pid,"input_source":sourceID(TISCopyCurrentKeyboardInputSource().takeRetainedValue())]
    for key in ["AXRole","AXValue","AXSelectedText","AXSelectedTextRange","AXPosition","AXSize","AXFocused"] { result[key] = describe(attribute(editor,key)) }
    if let focused = attribute(application,"AXFocusedUIElement") { let element = unsafeBitCast(focused,to:AXUIElement.self); result["focused_role"] = describe(attribute(element,"AXRole")); result["focused_description"] = describe(attribute(element,"AXDescription")); result["focused_value"] = describe(attribute(element,"AXValue")); result["focused_selected_range"] = describe(attribute(element,"AXSelectedTextRange")) }
    result["children"] = (attribute(editor,"AXChildren") as? [AXUIElement] ?? []).map { child in
        Dictionary(uniqueKeysWithValues:["AXRole","AXDescription","AXValue","AXPosition","AXSize"].map { ($0,describe(attribute(child,$0))) })
    }
    result["windows"] = (CGWindowListCopyWindowInfo([.optionOnScreenOnly,.excludeDesktopElements],kCGNullWindowID) as? [[String:Any]] ?? []).filter { ($0[kCGWindowOwnerPID as String] as? Int32) == pid || ($0[kCGWindowOwnerName as String] as? String)?.contains("Input") == true }
    json(result)
case "paste-text", "copy-read", "cut-read", "copy-paste":
    let board = NSPasteboard.general
    let saved: [NSPasteboardItem] = (board.pasteboardItems ?? []).map { item in
        let copy = NSPasteboardItem()
        for type in item.types { if let data = item.data(forType:type) { copy.setData(data,forType:type) } }
        return copy
    }
    failureCleanup = { board.clearContents(); if !saved.isEmpty { board.writeObjects(saved) } }
    defer { failureCleanup?(); failureCleanup = nil }
    if action == "paste-text" {
        guard args.count == 3 else { fail("paste-text requires text") }
        board.clearContents(); board.setString(args[2],forType:.string)
    }
    let beforeCopy = board.changeCount
    let code: CGKeyCode = action == "paste-text" ? 9 : (action == "cut-read" ? 7 : 8)
    for down in [true,false] { let event=CGEvent(keyboardEventSource:nil,virtualKey:code,keyDown:down); event?.flags = down ? .maskCommand : []; post(event) }
    RunLoop.current.run(until:Date().addingTimeInterval(0.3))
    if action != "paste-text" && board.changeCount == beforeCopy { fail("Copy/cut did not update the test clipboard") }
    let copied: [String:Any] = ["text":board.string(forType:.string) ?? "", "types":board.types?.map { $0.rawValue } ?? []]
    if action == "copy-paste" {
        guard args.count == 4, let x=Double(args[2]), let y=Double(args[3]) else { fail("copy-paste requires target screen x y") }
        for type:CGEventType in [.mouseMoved,.leftMouseDown,.leftMouseUp] { post(CGEvent(mouseEventSource:nil,mouseType:type,mouseCursorPosition:CGPoint(x:x,y:y),mouseButton:.left)) }
        for down in [true,false] { let event=CGEvent(keyboardEventSource:nil,virtualKey:9,keyDown:down); event?.flags = down ? .maskCommand : []; post(event) }
        RunLoop.current.run(until:Date().addingTimeInterval(0.3))
    }
    if action != "paste-text" { json(copied) }
case "capture":
    guard args.count == 3 else { fail("capture requires output PNG prefix") }
    RunLoop.current.run(until:Date().addingTimeInterval(0.4))
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else { fail("Capture target lost focus") }
    let windows = (CGWindowListCopyWindowInfo([.optionOnScreenOnly,.excludeDesktopElements],kCGNullWindowID) as? [[String:Any]] ?? []).filter { ($0[kCGWindowOwnerPID as String] as? Int32) == pid }
    guard windows.contains(where:{ ($0[kCGWindowLayer as String] as? Int) == 0 && (($0[kCGWindowBounds as String] as? [String:Double])?["Width"] ?? 0) >= 400 }) else { fail("Capture target is a thumbnail or hidden") }
    for window in windows {
        guard let id=window[kCGWindowNumber as String] as? Int else { continue }
        let process=Process(); process.executableURL=URL(fileURLWithPath:"/usr/sbin/screencapture")
        process.arguments=["-x","-o","-l\(id)",args[2]+"-\(id).png"]
        try process.run(); process.waitUntilExit()
        guard process.terminationStatus == 0 else { fail("Window capture failed") }
    }
    json(["windows":windows])
case "keys":
    guard args.count > 2 else { fail("keys requires hardware keycodes") }
    for argument in args.dropFirst(2) {
        guard let code = UInt16(argument) else { fail("Invalid hardware keycode") }
        for down in [true,false] { let event=CGEvent(keyboardEventSource:nil,virtualKey:code,keyDown:down); event?.flags=[]; post(event) }
    }
case "key":
    guard args.count >= 3, let code = UInt16(args[2]) else { fail("key requires hardware keycode and optional cmd+shift+alt+ctrl") }
    let modifiers = args.count > 3 ? flags(args[3]) : []
    for down in [true,false] { let event = CGEvent(keyboardEventSource:nil,virtualKey:code,keyDown:down); event?.flags = down ? modifiers : []; post(event) }
case "source":
    guard args.count == 3 else { fail("source requires input-source ID") }
    let sources = TISCreateInputSourceList(nil,false).takeRetainedValue() as! [TISInputSource]
    guard let source = sources.first(where:{ sourceID($0) == args[2] }) else { fail("Cannot find input source") }
    // TIS documents selecting the current source as a no-op. A newly focused
    // client's input context may still be processing activation; deliver an
    // actual source transition and allow the distributed notification to run.
    if args[2] == "com.apple.inputmethod.SCIM.ITABC",
       let latin = sources.first(where:{ sourceID($0) == "com.apple.keylayout.ABC" }) {
        guard TISSelectInputSource(latin) == noErr else { fail("Cannot prepare input source") }
        RunLoop.current.run(until:Date().addingTimeInterval(0.35))
    }
    guard TISSelectInputSource(source) == noErr else { fail("Cannot select input source") }
    RunLoop.current.run(until:Date().addingTimeInterval(0.35))
    guard sourceID(TISCopyCurrentKeyboardInputSource().takeRetainedValue()) == args[2] else { fail("Input source selection did not settle") }
case "select", "bounds":
    guard args.count == 4, let location = Int(args[2]), let length = Int(args[3]) else { fail("range requires UTF-16 location length") }
    var range = CFRange(location:location,length:length)
    let value = AXValueCreate(.cfRange,&range)!
    if action == "select" {
        guard AXUIElementSetAttributeValue(editor,"AXSelectedTextRange" as CFString,value) == .success else { fail("Cannot set selection") }
    } else {
        var output:CFTypeRef?
        guard AXUIElementCopyParameterizedAttributeValue(editor,"AXBoundsForRange" as CFString,value,&output) == .success else { fail("Cannot read bounds") }
        json(["bounds":describe(output)])
    }
case "text":
    guard args.count == 3 else { fail("text requires Unicode text") }
    let units = Array(args[2].utf16)
    for down in [true,false] {
        let event = CGEvent(keyboardEventSource:nil,virtualKey:0,keyDown:down)
        units.withUnsafeBufferPointer { buffer in event?.keyboardSetUnicodeString(stringLength:buffer.count,unicodeString:buffer.baseAddress) }
        event?.flags=[]
        post(event)
    }
case "scroll":
    guard args.count == 3, let delta = Int32(args[2]) else { fail("scroll requires vertical pixels") }
    post(CGEvent(scrollWheelEvent2Source:nil,units:.pixel,wheelCount:1,wheel1:delta,wheel2:0,wheel3:0))
case "resize":
    guard args.count == 4, let width = Double(args[2]), let height = Double(args[3]), let windows = attribute(application,"AXWindows") as? [AXUIElement], let window = windows.first else { fail("resize requires width height") }
    var size = CGSize(width:width,height:height)
    guard AXUIElementSetAttributeValue(window,"AXSize" as CFString,AXValueCreate(.cgSize,&size)!) == .success else { fail("Cannot resize window") }
case "click", "double-click", "triple-click", "word-drag", "drag", "drag-cancel":
    guard args.count >= 4, let x = Double(args[2]), let y = Double(args[3]) else { fail("mouse requires screen x y") }
    let start = CGPoint(x:x,y:y)
    let isDrag = action.hasPrefix("drag") || action == "word-drag"
    let modifiers = args.count > (isDrag ? 6 : 4) ? flags(args.last!) : []
    var clickCount = 1
    func mouse(_ type:CGEventType,_ point:CGPoint) { let event=CGEvent(mouseEventSource:nil,mouseType:type,mouseCursorPosition:point,mouseButton:.left); event?.flags=type == .leftMouseUp ? [] : modifiers; event?.setIntegerValueField(.mouseEventClickState,value:Int64(clickCount)); post(event) }
    let clicks = action == "triple-click" ? 3 : (["double-click", "word-drag"].contains(action) ? 2 : 1)
    mouse(.mouseMoved,start)
    for count in 1...clicks {
        clickCount = count
        mouse(.leftMouseDown,start)
        if count < clicks { mouse(.leftMouseUp,start) }
    }
    if isDrag {
        guard args.count >= 6, let endX = Double(args[4]), let endY = Double(args[5]) else { fail("drag requires x y endX endY") }
        for step in 1...16 { let t = Double(step)/16; mouse(.leftMouseDragged,CGPoint(x:x+(endX-x)*t,y:y+(endY-y)*t)) }
        if action == "drag-cancel" {
            for down in [true,false] { let event=CGEvent(keyboardEventSource:nil,virtualKey:53,keyDown:down); event?.flags=[]; post(event) }
        }
        mouse(.leftMouseUp,CGPoint(x:endX,y:endY))
    } else { mouse(.leftMouseUp,start) }
default: fail("Unknown action")
}
