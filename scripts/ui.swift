// Lets an agent see and drive the running editor: screenshots of its windows and real
// mouse and keyboard input. The terminal it runs from needs the Screen Recording and
// Accessibility permissions (System Settings → Privacy & Security).
//
//   swift scripts/ui.swift check                      are the permissions granted?
//   swift scripts/ui.swift windows                    list the editor's windows
//   swift scripts/ui.swift shot <dir>                 save <dir>/window-<n>.png for each window
//   swift scripts/ui.swift click <n> <x> <y>          also: rclick, dclick, move
//   swift scripts/ui.swift drag <n> <x> <y> <n> <x> <y>
//   swift scripts/ui.swift scroll <n> <x> <y> <lines> negative scrolls down
//   swift scripts/ui.swift key <n> cmd+p              keys: a-z 0-9 enter esc tab space up down left right delete
//   swift scripts/ui.swift type <n> <text>
//
// <n> is the number of a window as listed by `windows`. Coordinates are in points from the
// top left corner of that window, title bar included; one pixel of a screenshot is one point.

import AppKit
import ApplicationServices
import CoreGraphics

struct Window {
    let id: Int
    let pid: pid_t
    let title: String
    let bounds: CGRect
}

func fail(_ message: String) -> Never {
    FileHandle.standardError.write((message + "\n").data(using: .utf8)!)
    exit(1)
}

/// The editor's windows, oldest first, so that their numbers stay the same while windows are added.
func windows() -> [Window] {
    let info = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
    return info.compactMap { window in
        guard window[kCGWindowOwnerName as String] as? String == "editor",
              window[kCGWindowLayer as String] as? Int == 0,
              let id = window[kCGWindowNumber as String] as? Int,
              let pid = window[kCGWindowOwnerPID as String] as? Int,
              let bounds = CGRect(dictionaryRepresentation: window[kCGWindowBounds as String] as! CFDictionary)
        else { return nil }
        return Window(id: id, pid: pid_t(pid), title: window[kCGWindowName as String] as? String ?? "", bounds: bounds)
    }.sorted { $0.id < $1.id }
}

func window(_ number: String) -> Window {
    let all = windows()
    guard let index = Int(number), all.indices.contains(index) else { fail("no window \(number); the editor has \(all.count)") }
    return all[index]
}

func run(_ tool: String, _ arguments: [String]) {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: tool)
    process.arguments = arguments
    try! process.run()
    process.waitUntilExit()
    if process.terminationStatus != 0 { fail("\(tool) failed") }
}

func pause(_ seconds: Double = 0.03) {
    Thread.sleep(forTimeInterval: seconds)
}

/// Brings the window's application to the front, so that it receives the input.
func activate(_ window: Window) {
    NSRunningApplication(processIdentifier: window.pid)?.activate(options: [])
    pause(0.3)
}

func point(_ window: Window, _ x: String, _ y: String) -> CGPoint {
    guard let x = Double(x), let y = Double(y) else { fail("coordinates must be numbers") }
    return CGPoint(x: window.bounds.minX + x, y: window.bounds.minY + y)
}

func mouse(_ type: CGEventType, _ at: CGPoint, button: CGMouseButton = .left, clicks: Int64 = 1) {
    let event = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: at, mouseButton: button)!
    event.setIntegerValueField(.mouseEventClickState, value: clicks)
    event.post(tap: .cghidEventTap)
    pause()
}

func click(_ at: CGPoint, button: CGMouseButton = .left, clicks: Int64 = 1) {
    let (down, up): (CGEventType, CGEventType) = button == .left ? (.leftMouseDown, .leftMouseUp) : (.rightMouseDown, .rightMouseUp)
    mouse(.mouseMoved, at)
    for count in 1...clicks {
        mouse(down, at, button: button, clicks: count)
        mouse(up, at, button: button, clicks: count)
    }
}

/// Drags slowly enough for the application to follow along, also from one window into another.
func drag(from: CGPoint, to: CGPoint) {
    let steps = 30
    mouse(.mouseMoved, from)
    mouse(.leftMouseDown, from)
    for step in 1...steps {
        let t = Double(step) / Double(steps)
        mouse(.leftMouseDragged, CGPoint(x: from.x + (to.x - from.x) * t, y: from.y + (to.y - from.y) * t))
    }
    pause(0.2)
    mouse(.leftMouseUp, to)
}

let keyCodes: [String: CGKeyCode] = [
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9, "b": 11, "q": 12, "w": 13, "e": 14, "r": 15,
    "y": 16, "t": 17, "1": 18, "2": 19, "3": 20, "4": 21, "6": 22, "5": 23, "9": 25, "7": 26, "8": 28, "0": 29, "o": 31, "u": 32,
    "i": 34, "p": 35, "l": 37, "j": 38, "k": 40, "n": 45, "m": 46,
    "enter": 36, "tab": 48, "space": 49, "delete": 51, "esc": 53, "left": 123, "right": 124, "down": 125, "up": 126,
]

let modifierFlags: [String: CGEventFlags] = ["cmd": .maskCommand, "ctrl": .maskControl, "alt": .maskAlternate, "shift": .maskShift]

/// Presses a key combination such as "cmd+shift+p".
func key(_ combination: String) {
    var parts = combination.lowercased().split(separator: "+").map(String.init)
    guard let name = parts.popLast(), let code = keyCodes[name] else { fail("unknown key in \(combination)") }
    var flags: CGEventFlags = []
    for modifier in parts {
        guard let flag = modifierFlags[modifier] else { fail("unknown modifier \(modifier)") }
        flags.insert(flag)
    }
    for down in [true, false] {
        let event = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: down)!
        event.flags = flags
        event.post(tap: .cghidEventTap)
        pause()
    }
}

func type(_ text: String) {
    for character in text {
        let units = Array(String(character).utf16)
        for down in [true, false] {
            let event = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: down)!
            event.flags = []
            event.keyboardSetUnicodeString(stringLength: units.count, unicodeString: units)
            event.post(tap: .cghidEventTap)
            pause()
        }
    }
}

let arguments = Array(CommandLine.arguments.dropFirst())
let command = arguments.first ?? ""
let needsInput = !["check", "windows", "shot"].contains(command)
if needsInput && !AXIsProcessTrusted() {
    fail("no Accessibility permission: input would be ignored. Enable the terminal under System Settings → Privacy & Security → Accessibility")
}

switch (command, arguments.count - 1) {
case ("check", 0):
    print("screen recording: \(CGPreflightScreenCaptureAccess() ? "granted" : "missing")")
    print("accessibility: \(AXIsProcessTrusted() ? "granted" : "missing")")
case ("windows", 0):
    for (number, window) in windows().enumerated() {
        let b = window.bounds
        print("\(number): \(Int(b.width))x\(Int(b.height)) at \(Int(b.minX)),\(Int(b.minY)) \(window.title)")
    }
case ("shot", 1):
    if !CGPreflightScreenCaptureAccess() {
        fail("no Screen Recording permission. Enable the terminal under System Settings → Privacy & Security → Screen & System Audio Recording and restart it")
    }
    try! FileManager.default.createDirectory(atPath: arguments[1], withIntermediateDirectories: true)
    for (number, window) in windows().enumerated() {
        let path = "\(arguments[1])/window-\(number).png"
        run("/usr/sbin/screencapture", ["-x", "-o", "-l", String(window.id), path])
        // scaled down to one pixel per point, so that positions in the image can be used as coordinates
        run("/usr/bin/sips", ["-z", String(Int(window.bounds.height)), String(Int(window.bounds.width)), path, "--out", path])
        print(path)
    }
case ("click", 3), ("rclick", 3), ("dclick", 3), ("move", 3):
    let target = window(arguments[1])
    activate(target)
    let at = point(target, arguments[2], arguments[3])
    switch command {
    case "click": click(at)
    case "rclick": click(at, button: .right)
    case "dclick": click(at, clicks: 2)
    default: mouse(.mouseMoved, at)
    }
case ("drag", 6):
    let (from, to) = (window(arguments[1]), window(arguments[4]))
    activate(from)
    drag(from: point(from, arguments[2], arguments[3]), to: point(to, arguments[5], arguments[6]))
case ("scroll", 4):
    let target = window(arguments[1])
    activate(target)
    mouse(.mouseMoved, point(target, arguments[2], arguments[3]))
    guard let lines = Int32(arguments[4]) else { fail("lines must be a number") }
    CGEvent(scrollWheelEvent2Source: nil, units: .line, wheelCount: 1, wheel1: lines, wheel2: 0, wheel3: 0)!.post(tap: .cghidEventTap)
case ("key", 2):
    activate(window(arguments[1]))
    key(arguments[2])
case ("type", 2):
    activate(window(arguments[1]))
    type(arguments[2])
default:
    fail("usage: see the top of scripts/ui.swift")
}
pause(0.2)
