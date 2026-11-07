// Posts synthetic keys and drags so an agent can prove a Mac check, in a window of
// time agreed with the user. Reads one command per line from the script it is given.
//
//   LENS_DRIVER_WINDOW=agreed driver --front "Attn Lens" script.txt
//
// It refuses to start without LENS_DRIVER_WINDOW=agreed or without Accessibility,
// which posting events and watching for real input both need. It stops at once, and
// releases everything it holds, when:
//   - the frontmost normal window does not belong to an app named by --front, checked
//     before each posted key, click and scroll (exit 3);
//   - real input arrives from the user during the run (exit 5);
//   - an `expect` does not come true (exit 4).
//
// Commands:
//   down|up KEY            hold or release a modifier: fn ctrl shift cmd opt
//   tap KEY                press and release a key
//   chord cmd+opt x [ms]   hold the modifiers, tap a key, hold it for ms (default 40)
//   move X Y | press X Y | release | click X Y | drag X0 Y0 X1 Y1 STEPS STEP_MS
//   scroll DY [PHASE]
//   wait MS
//   dock NAME              presses the app's Dock icon through Accessibility
//   lens JSON              sends a request to Lens's automation socket, prints the reply
//   expect TEXT            polls {"cmd":"state"} until the reply contains TEXT
//   time-window OWNER      after the last chord's key went down, reports how long until
//                          OWNER had a new window on screen
//   mark NAME              logs a named time
// Every command logs its time in nanoseconds on the host clock Lens uses.
import AppKit
import ApplicationServices
import CoreGraphics
import Foundation

setvbuf(stdout, nil, _IOLBF, 0)
var timebase = mach_timebase_info_data_t()
mach_timebase_info(&timebase)
func now() -> UInt64 { mach_absolute_time() * UInt64(timebase.numer) / UInt64(timebase.denom) }
func log(_ text: String) { print("\(now()) \(text)") }

let posted: Int64 = 0x4c454e53  // Marks the driver's own events, so real input stands out.
let source = CGEventSource(stateID: .hidSystemState)!
source.userData = posted

var arguments = Array(CommandLine.arguments.dropFirst())
var allowedFront: [String] = []
while arguments.first == "--front", arguments.count >= 2 {
    allowedFront.append(arguments[1])
    arguments.removeFirst(2)
}
guard ProcessInfo.processInfo.environment["LENS_DRIVER_WINDOW"] == "agreed" else {
    print("refusing: set LENS_DRIVER_WINDOW=agreed only inside a window agreed with the user")
    exit(2)
}
guard !allowedFront.isEmpty, let scriptPath = arguments.first else {
    print("usage: LENS_DRIVER_WINDOW=agreed driver --front APP [--front APP] script.txt")
    exit(2)
}
guard AXIsProcessTrusted() else {
    print("refusing: this process has no Accessibility permission")
    exit(2)
}

let modifiers: [String: (CGKeyCode, CGEventFlags)] = [
    "ctrl": (0x3B, .maskControl), "fn": (0x3F, .maskSecondaryFn), "shift": (0x38, .maskShift),
    "cmd": (0x37, .maskCommand), "opt": (0x3A, .maskAlternate),
]
let keys: [String: CGKeyCode] = [
    "x": 7, "c": 8, "a": 0, "b": 11, "h": 4, "s": 1, "i": 34, "2": 19,
    "return": 36, "esc": 53, "tab": 48, "space": 49, "backtick": 50,
]
var held: CGEventFlags = []
var heldNames = Set<String>()
var buttonDown = false
var pointer = CGPoint.zero
var lastChordKeyDown: UInt64 = 0

func post(_ event: CGEvent) {
    event.setIntegerValueField(.eventSourceUserData, value: posted)
    event.post(tap: .cghidEventTap)
}
func modifier(_ name: String, _ down: Bool) {
    guard let (code, flag) = modifiers[name] else { fail("unknown modifier \(name)", 2) }
    if down {
        held.insert(flag)
        heldNames.insert(name)
    } else {
        held.remove(flag)
        heldNames.remove(name)
    }
    let event = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: down)!
    event.type = .flagsChanged
    event.flags = held
    post(event)
    log("posted \(name) \(down ? "down" : "up")")
}
func key(_ name: String, _ down: Bool) {
    guard let code = keys[name] else { fail("unknown key \(name)", 2) }
    let event = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: down)!
    event.flags = held
    post(event)
    log("posted key \(name) \(down ? "down" : "up")")
}
func mouse(_ type: CGEventType, _ point: CGPoint) {
    let event = CGEvent(
        mouseEventSource: source, mouseType: type, mouseCursorPosition: point, mouseButton: .left)!
    event.flags = held
    post(event)
    pointer = point
}
func pause(_ ms: Double) { if ms > 0 { usleep(UInt32(ms * 1000)) } }
func release() {
    if buttonDown { mouse(.leftMouseUp, pointer) }
    for name in heldNames { modifier(name, false) }
}
func fail(_ reason: String, _ code: Int32) -> Never {
    log("ABORT \(reason)")
    release()
    exit(code)
}
signal(SIGINT) { _ in release(); exit(1) }
signal(SIGTERM) { _ in release(); exit(1) }

// Anything the driver did not post came from the user.
let watched: [CGEventType] = [
    .keyDown, .keyUp, .flagsChanged, .leftMouseDown, .leftMouseUp, .rightMouseDown, .rightMouseUp,
    .mouseMoved, .leftMouseDragged, .rightMouseDragged, .scrollWheel,
]
let mask = watched.reduce(CGEventMask(0)) { $0 | (CGEventMask(1) << $1.rawValue) }
guard
    let tap = CGEvent.tapCreate(
        tap: .cgSessionEventTap, place: .headInsertEventTap, options: .listenOnly,
        eventsOfInterest: mask,
        callback: { _, type, event, _ in
            if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
                fail("the real-input watch was disabled", 5)
            }
            if event.getIntegerValueField(.eventSourceUserData) != posted {
                fail("real input arrived: event type \(type.rawValue)", 5)
            }
            return Unmanaged.passUnretained(event)
        }, userInfo: nil)
else {
    print("refusing: could not watch for real input")
    exit(2)
}
let watchThread = Thread {
    CFRunLoopAddSource(
        CFRunLoopGetCurrent(), CFMachPortCreateRunLoopSource(nil, tap, 0), .commonModes)
    CGEvent.tapEnable(tap: tap, enable: true)
    CFRunLoopRun()
}
watchThread.start()

func windows(owner: String? = nil) -> [[String: Any]] {
    let list =
        CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
        as? [[String: Any]] ?? []
    guard let owner else { return list }
    return list.filter { $0[kCGWindowOwnerName as String] as? String == owner }
}
func requireFront() {
    let front = windows().first { ($0[kCGWindowLayer as String] as? Int) == 0 }
    let owner = front?[kCGWindowOwnerName as String] as? String ?? "?"
    if !allowedFront.contains(owner) { fail("front window belongs to \(owner)", 3) }
}

func socketPath() -> String {
    ProcessInfo.processInfo.environment["ATTN_LENS_SOCKET"]
        ?? NSHomeDirectory() + "/Library/Caches/dev.attn.lens/automation/lens.sock"
}
func ask(_ line: String) -> String {
    let descriptor = socket(AF_UNIX, SOCK_STREAM, 0)
    defer { close(descriptor) }
    var address = sockaddr_un()
    address.sun_family = sa_family_t(AF_UNIX)
    let path = Array(socketPath().utf8)
    guard path.count < MemoryLayout.size(ofValue: address.sun_path) else {
        fail("socket path too long: \(socketPath())", 2)
    }
    withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: path) }
    let connected = withUnsafePointer(to: &address) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
            connect(descriptor, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
        }
    }
    guard connected == 0 else { fail("cannot reach Lens at \(socketPath())", 4) }
    _ = (line + "\n").withCString { write(descriptor, $0, strlen($0)) }
    var reply = [UInt8]()
    var byte: UInt8 = 0
    while read(descriptor, &byte, 1) == 1, byte != 10 { reply.append(byte) }
    return String(decoding: reply, as: UTF8.self)
}
func pressDockIcon(_ name: String) {
    guard
        let dock = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.dock")
            .first?.processIdentifier
    else { fail("the Dock is not running", 4) }
    let app = AXUIElementCreateApplication(dock)
    var children: AnyObject?
    AXUIElementCopyAttributeValue(app, kAXChildrenAttribute as CFString, &children)
    for list in children as? [AXUIElement] ?? [] {
        var items: AnyObject?
        AXUIElementCopyAttributeValue(list, kAXChildrenAttribute as CFString, &items)
        for item in items as? [AXUIElement] ?? [] {
            var title: AnyObject?
            AXUIElementCopyAttributeValue(item, kAXTitleAttribute as CFString, &title)
            if title as? String == name {
                let result = AXUIElementPerformAction(item, kAXPressAction as CFString)
                guard result == .success else { fail("pressing \(name) gave \(result.rawValue)", 4) }
                log("pressed Dock icon \(name)")
                return
            }
        }
    }
    fail("no Dock icon named \(name)", 4)
}
func run(_ line: String) {
    let parts = line.split(separator: " ").map(String.init)
    guard let command = parts.first, !command.hasPrefix("#") else { return }
    func rest(_ from: Int) -> String {
        line.split(separator: " ", maxSplits: from, omittingEmptySubsequences: true).dropFirst(from)
            .joined(separator: " ")
    }
    func number(_ index: Int) -> Double {
        guard index < parts.count, let value = Double(parts[index]) else {
            fail("bad number in: \(line)", 2)
        }
        return value
    }
    if ["down", "tap", "chord", "press", "click", "drag", "scroll"].contains(command) {
        requireFront()
    }
    switch command {
    case "down": modifier(parts[1], true)
    case "up": modifier(parts[1], false)
    case "tap":
        key(parts[1], true)
        pause(30)
        key(parts[1], false)
    case "chord":
        let names = parts[1].split(separator: "+").map(String.init)
        names.forEach { modifier($0, true) }
        pause(20)
        lastChordKeyDown = now()
        key(parts[2], true)
        pause(parts.count > 3 ? number(3) : 40)
        key(parts[2], false)
        pause(20)
        names.reversed().forEach { modifier($0, false) }
    case "move":
        mouse(.mouseMoved, CGPoint(x: number(1), y: number(2)))
        log("posted move")
    case "press":
        let point = CGPoint(x: number(1), y: number(2))
        mouse(.mouseMoved, point)
        pause(5)
        buttonDown = true
        mouse(.leftMouseDown, point)
        log("posted press")
    case "release":
        mouse(.leftMouseUp, pointer)
        buttonDown = false
        log("posted release")
    case "click":
        let point = CGPoint(x: number(1), y: number(2))
        mouse(.mouseMoved, point)
        pause(10)
        mouse(.leftMouseDown, point)
        pause(30)
        mouse(.leftMouseUp, point)
        log("posted click")
    case "drag":
        let (x0, y0, x1, y1) = (number(1), number(2), number(3), number(4))
        let steps = Int(number(5))
        for step in 1...steps {
            let f = Double(step) / Double(steps)
            mouse(.leftMouseDragged, CGPoint(x: x0 + (x1 - x0) * f, y: y0 + (y1 - y0) * f))
            pause(number(6))
        }
        log("posted drag")
    case "scroll":
        let event = CGEvent(
            scrollWheelEvent2Source: source, units: .pixel, wheelCount: 1, wheel1: Int32(number(1)),
            wheel2: 0, wheel3: 0)!
        event.setIntegerValueField(.scrollWheelEventIsContinuous, value: 1)
        if parts.count > 2 { event.setIntegerValueField(.scrollWheelEventScrollPhase, value: Int64(number(2))) }
        post(event)
        log("posted scroll")
    case "wait": pause(number(1))
    case "dock": pressDockIcon(rest(1))
    case "lens":
        let request = rest(1)
        log("lens \(request) -> \(ask(request))")
    case "expect":
        let text = rest(1)
        // 200 × 25 ms: past the slowest still the spike measured (189 ms) many times over.
        for _ in 0..<200 {
            let reply = ask("{\"cmd\":\"state\"}")
            if reply.contains(text) {
                log("expect ok: \(text)")
                return
            }
            pause(25)
        }
        fail("expected \(text) in \(ask("{\"cmd\":\"state\"}"))", 4)
    case "time-window":
        let owner = rest(1)
        let before = Set(windows(owner: owner).compactMap { $0[kCGWindowNumber as String] as? Int })
        // Polling every 2 ms, as the spike's baselines did. 5 s is far past its 841 ms worst case.
        let deadline = now() + 5_000_000_000
        while now() < deadline {
            let current = windows(owner: owner).compactMap { $0[kCGWindowNumber as String] as? Int }
            if current.contains(where: { !before.contains($0) }) {
                log("time-window \(owner) \(Double(now() - lastChordKeyDown) / 1e6) ms after key down")
                return
            }
            usleep(2000)
        }
        fail("no new window for \(owner)", 4)
    case "mark": log("mark \(rest(1))")
    default: fail("unknown command: \(line)", 2)
    }
}

guard let script = try? String(contentsOfFile: scriptPath, encoding: .utf8) else {
    fail("cannot read \(scriptPath)", 2)
}
log("driver start")
for line in script.split(separator: "\n") { run(String(line)) }
release()
log("driver end")
