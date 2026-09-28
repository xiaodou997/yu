// Minimal AppKit control experiment. Not a Yu editor or production dependency.
// Use only in an isolated io.github.xiaodou997.yu.* test bundle with the existing
// external event driver. NSTextView owns all text/input behavior unchanged.
import AppKit

let app = NSApplication.shared
app.setActivationPolicy(.regular)
let menu = NSMenu()
let appItem = NSMenuItem(); let appMenu = NSMenu()
let quit = NSMenuItem(title: "退出", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
quit.target = app; appMenu.addItem(quit); appItem.submenu = appMenu; menu.addItem(appItem)
let fileItem = NSMenuItem(title: "文件", action: nil, keyEquivalent: "")
let fileMenu = NSMenu(title: "文件")
fileMenu.addItem(NSMenuItem(title: "对照项目", action: nil, keyEquivalent: ""))
fileItem.submenu = fileMenu; menu.addItem(fileItem); app.mainMenu = menu
let window = NSWindow(contentRect: NSRect(x: 300, y: 300, width: 700, height: 450),
    styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
window.title = "NSTextView 输入法对照"
let text = NSTextView(frame: NSRect(x: 0, y: 0, width: 700, height: 450))
text.isRichText = false; text.font = .systemFont(ofSize: 20)
text.setAccessibilityIdentifier("yu-document-text")
text.autoresizingMask = [.width, .height]
window.contentView = text
window.makeKeyAndOrderFront(nil); window.makeFirstResponder(text)
app.activate(ignoringOtherApps: true); app.run()
