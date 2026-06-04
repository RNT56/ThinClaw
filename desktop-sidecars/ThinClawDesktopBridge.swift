import AppKit
import ApplicationServices
import EventKit
import Foundation
import Vision

struct BridgeResponse: Codable {
    let ok: Bool
    let result: AnyCodable?
    let error: String?
}

struct AnyCodable: Codable {
    let value: Any

    init(_ value: Any) {
        self.value = value
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if let value = try? container.decode(String.self) {
            self.value = value
        } else if let value = try? container.decode(Bool.self) {
            self.value = value
        } else if let value = try? container.decode(Int.self) {
            self.value = value
        } else if let value = try? container.decode(Double.self) {
            self.value = value
        } else if let value = try? container.decode([String: AnyCodable].self) {
            self.value = value.mapValues(\.value)
        } else if let value = try? container.decode([AnyCodable].self) {
            self.value = value.map(\.value)
        } else {
            self.value = NSNull()
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch value {
        case let value as String:
            try container.encode(value)
        case let value as Bool:
            try container.encode(value)
        case let value as Int:
            try container.encode(value)
        case let value as Double:
            try container.encode(value)
        case let value as [String: Any]:
            try container.encode(value.mapValues(AnyCodable.init))
        case let value as [Any]:
            try container.encode(value.map(AnyCodable.init))
        default:
            try container.encodeNil()
        }
    }
}

func readJSONPayload() -> [String: Any] {
    let data = FileHandle.standardInput.readDataToEndOfFile()
    guard !data.isEmpty else { return [:] }
    guard let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
        return [:]
    }
    return value
}

func emit(_ value: [String: Any]) {
    let data = try! JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys])
    FileHandle.standardOutput.write(data)
}

func ok(_ result: Any) {
    emit(["ok": true, "result": result])
}

func fail(_ error: String) -> Never {
    emit(["ok": false, "error": error])
    exit(1)
}

func commandArg() -> String {
    guard CommandLine.arguments.count >= 2 else {
        fail("missing command")
    }
    return CommandLine.arguments[1]
}

func permissionStatus() -> [String: Any] {
    var calendar: String = "unknown"
    switch EKEventStore.authorizationStatus(for: .event) {
    case .authorized:
        calendar = "authorized"
    case .fullAccess:
        calendar = "authorized"
    case .writeOnly:
        calendar = "write_only"
    case .restricted:
        calendar = "restricted"
    case .denied:
        calendar = "denied"
    case .notDetermined:
        calendar = "not_determined"
    @unknown default:
        calendar = "unknown"
    }

    let accessibility = AXIsProcessTrusted()
    let screenRecording: Any
    if #available(macOS 11.0, *) {
        screenRecording = CGPreflightScreenCaptureAccess()
    } else {
        screenRecording = "unknown"
    }

    return [
        "platform": "macos",
        "accessibility": accessibility,
        "screen_recording": screenRecording,
        "apple_events": "unknown",
        "full_disk_access": "unknown",
        "calendar": calendar,
    ]
}

func runningApps() -> [[String: Any]] {
    NSWorkspace.shared.runningApplications.map { app in
        [
            "name": app.localizedName ?? "",
            "bundle_id": app.bundleIdentifier ?? "",
            "pid": app.processIdentifier,
            "active": app.isActive,
            "hidden": app.isHidden,
        ]
    }
}

func appForBundleID(_ bundleID: String) -> NSRunningApplication? {
    NSRunningApplication.runningApplications(withBundleIdentifier: bundleID).first
}

func axElementValue(_ element: AXUIElement, attribute: String) -> Any? {
    var result: CFTypeRef?
    let error = AXUIElementCopyAttributeValue(element, attribute as CFString, &result)
    guard error == .success else { return nil }
    return result
}

func axChildren(_ element: AXUIElement) -> [AXUIElement] {
    ((axElementValue(element, attribute: kAXChildrenAttribute) as? [Any]) ?? []).compactMap { child in
        guard CFGetTypeID(child as CFTypeRef) == AXUIElementGetTypeID() else { return nil }
        return unsafeBitCast(child, to: AXUIElement.self)
    }
}

@discardableResult
func axElementPerform(_ element: AXUIElement, action: String) -> Bool {
    AXUIElementPerformAction(element, action as CFString) == .success
}

@discardableResult
func axElementSetValue(_ element: AXUIElement, attribute: String, value: Any) -> Bool {
    AXUIElementSetAttributeValue(element, attribute as CFString, value as CFTypeRef) == .success
}

func pointFromAXValue(_ value: Any?) -> CGPoint? {
    guard let value,
          CFGetTypeID(value as CFTypeRef) == AXValueGetTypeID() else { return nil }
    let axValue = unsafeBitCast(value, to: AXValue.self)
    guard AXValueGetType(axValue) == .cgPoint else { return nil }
    var point = CGPoint.zero
    return AXValueGetValue(axValue, .cgPoint, &point) ? point : nil
}

func sizeFromAXValue(_ value: Any?) -> CGSize? {
    guard let value,
          CFGetTypeID(value as CFTypeRef) == AXValueGetTypeID() else { return nil }
    let axValue = unsafeBitCast(value, to: AXValue.self)
    guard AXValueGetType(axValue) == .cgSize else { return nil }
    var size = CGSize.zero
    return AXValueGetValue(axValue, .cgSize, &size) ? size : nil
}

func elementFrame(_ element: AXUIElement) -> CGRect? {
    guard let origin = pointFromAXValue(axElementValue(element, attribute: kAXPositionAttribute)),
          let size = sizeFromAXValue(axElementValue(element, attribute: kAXSizeAttribute)) else {
        return nil
    }
    return CGRect(origin: origin, size: size)
}

func elementCenter(_ element: AXUIElement) -> CGPoint? {
    guard let frame = elementFrame(element) else { return nil }
    return CGPoint(x: frame.midX, y: frame.midY)
}

func resolveElement(bundleID: String?, ref: String?) -> AXUIElement? {
    guard let root = appElement(bundleID: bundleID) else { return nil }
    guard let ref, !ref.isEmpty, ref != "root" else { return root }
    var current = root
    for component in ref.split(separator: "/") {
        if component == "root" { continue }
        guard let index = Int(component) else { return nil }
        let children = axChildren(current)
        guard index >= 0, index < children.count else { return nil }
        current = children[index]
    }
    return current
}

func doubleValue(_ any: Any?) -> Double? {
    if let value = any as? Double { return value }
    if let value = any as? Int { return Double(value) }
    if let value = any as? NSNumber { return value.doubleValue }
    return nil
}

func intValue(_ any: Any?) -> Int? {
    if let value = any as? Int { return value }
    if let value = any as? Double { return Int(value) }
    if let value = any as? NSNumber { return value.intValue }
    return nil
}

func pointFromPayload(_ payload: [String: Any], prefix: String = "") -> CGPoint? {
    let xKey = prefix.isEmpty ? "x" : "\(prefix)_x"
    let yKey = prefix.isEmpty ? "y" : "\(prefix)_y"
    guard let x = doubleValue(payload[xKey]), let y = doubleValue(payload[yKey]) else { return nil }
    return CGPoint(x: x, y: y)
}

func targetPoint(payload: [String: Any], prefix: String = "") -> CGPoint? {
    if let direct = pointFromPayload(payload, prefix: prefix) {
        return direct
    }
    let refKey = prefix.isEmpty ? "target_ref" : "\(prefix)_ref"
    let bundleID = payload["bundle_id"] as? String ?? payload["app_bundle_id"] as? String
    guard let ref = payload[refKey] as? String else { return nil }
    guard let element = resolveElement(bundleID: bundleID, ref: ref) else { return nil }
    return elementCenter(element)
}

@discardableResult
func postMouseClick(at point: CGPoint, double: Bool = false) -> Bool {
    guard let source = CGEventSource(stateID: .hidSystemState) else { return false }
    guard let move = CGEvent(mouseEventSource: source, mouseType: .mouseMoved, mouseCursorPosition: point, mouseButton: .left),
          let down = CGEvent(mouseEventSource: source, mouseType: .leftMouseDown, mouseCursorPosition: point, mouseButton: .left),
          let up = CGEvent(mouseEventSource: source, mouseType: .leftMouseUp, mouseCursorPosition: point, mouseButton: .left) else {
        return false
    }
    move.post(tap: .cghidEventTap)
    down.setIntegerValueField(.mouseEventClickState, value: double ? 2 : 1)
    up.setIntegerValueField(.mouseEventClickState, value: double ? 2 : 1)
    down.post(tap: .cghidEventTap)
    up.post(tap: .cghidEventTap)
    if double {
        down.post(tap: .cghidEventTap)
        up.post(tap: .cghidEventTap)
    }
    return true
}

@discardableResult
func postDrag(from start: CGPoint, to end: CGPoint) -> Bool {
    guard let source = CGEventSource(stateID: .hidSystemState) else { return false }
    guard let down = CGEvent(mouseEventSource: source, mouseType: .leftMouseDown, mouseCursorPosition: start, mouseButton: .left),
          let drag = CGEvent(mouseEventSource: source, mouseType: .leftMouseDragged, mouseCursorPosition: end, mouseButton: .left),
          let up = CGEvent(mouseEventSource: source, mouseType: .leftMouseUp, mouseCursorPosition: end, mouseButton: .left) else {
        return false
    }
    down.post(tap: .cghidEventTap)
    drag.post(tap: .cghidEventTap)
    up.post(tap: .cghidEventTap)
    return true
}

@discardableResult
func postScroll(deltaX: Int32, deltaY: Int32) -> Bool {
    guard let event = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 2, wheel1: deltaY, wheel2: deltaX, wheel3: 0) else {
        return false
    }
    event.post(tap: .cghidEventTap)
    return true
}

func modifierFlags(_ modifiers: [String]) -> CGEventFlags {
    modifiers.reduce(into: CGEventFlags()) { flags, modifier in
        switch modifier.lowercased() {
        case "command", "cmd":
            flags.insert(.maskCommand)
        case "option", "alt":
            flags.insert(.maskAlternate)
        case "shift":
            flags.insert(.maskShift)
        case "control", "ctrl":
            flags.insert(.maskControl)
        default:
            break
        }
    }
}

func keyCode(for key: String) -> CGKeyCode? {
    switch key.lowercased() {
    case "a": return 0
    case "s": return 1
    case "d": return 2
    case "f": return 3
    case "h": return 4
    case "g": return 5
    case "z": return 6
    case "x": return 7
    case "c": return 8
    case "v": return 9
    case "b": return 11
    case "q": return 12
    case "w": return 13
    case "e": return 14
    case "r": return 15
    case "y": return 16
    case "t": return 17
    case "1": return 18
    case "2": return 19
    case "3": return 20
    case "4": return 21
    case "6": return 22
    case "5": return 23
    case "=": return 24
    case "9": return 25
    case "7": return 26
    case "-": return 27
    case "8": return 28
    case "0": return 29
    case "]": return 30
    case "o": return 31
    case "u": return 32
    case "[": return 33
    case "i": return 34
    case "p": return 35
    case "l": return 37
    case "j": return 38
    case "'": return 39
    case "k": return 40
    case ";": return 41
    case "\\": return 42
    case ",": return 43
    case "/": return 44
    case "n": return 45
    case "m": return 46
    case ".": return 47
    case "tab": return 48
    case "space": return 49
    case "`": return 50
    case "delete", "backspace": return 51
    case "enter", "return": return 36
    case "escape", "esc": return 53
    case "left": return 123
    case "right": return 124
    case "down": return 125
    case "up": return 126
    default: return nil
    }
}

@discardableResult
func postKeyPress(key: String, modifiers: [String]) -> Bool {
    let flags = modifierFlags(modifiers)
    if let code = keyCode(for: key) {
        guard let down = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: true),
              let up = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: false) else {
            return false
        }
        down.flags = flags
        up.flags = flags
        down.post(tap: .cghidEventTap)
        up.post(tap: .cghidEventTap)
        return true
    }
    do {
        let usingClause: String
        if modifiers.isEmpty {
            usingClause = ""
        } else {
            let mapped = modifiers.map { modifier -> String in
                switch modifier.lowercased() {
                case "command", "cmd": return "command down"
                case "option", "alt": return "option down"
                case "shift": return "shift down"
                case "control", "ctrl": return "control down"
                default: return ""
                }
            }.filter { !$0.isEmpty }.joined(separator: ", ")
            usingClause = mapped.isEmpty ? "" : " using {\(mapped)}"
        }
        _ = try runAppleScript("tell application \"System Events\" to keystroke \"\(key)\"\(usingClause)")
        return true
    } catch {
        return false
    }
}

func jsonText(_ value: Any) -> String {
    guard JSONSerialization.isValidJSONObject(value),
          let data = try? JSONSerialization.data(withJSONObject: value),
          let text = String(data: data, encoding: .utf8) else {
        return "\(value)"
    }
    return text
}

func frontmostProcessName(bundleID: String?) -> String? {
    if let bundleID, let app = appForBundleID(bundleID) {
        return app.localizedName
    }
    return NSWorkspace.shared.frontmostApplication?.localizedName
}

func menuClickScript(processName: String, menuPath: [String]) -> String? {
    guard menuPath.count >= 2 else { return nil }
    if menuPath.count == 2 {
        return """
        tell application "System Events"
          tell process "\(processName)"
            click menu item "\(menuPath[1])" of menu "\(menuPath[0])" of menu bar item "\(menuPath[0])" of menu bar 1
          end tell
        end tell
        """
    }
    if menuPath.count == 3 {
        return """
        tell application "System Events"
          tell process "\(processName)"
            click menu item "\(menuPath[2])" of menu "\(menuPath[1])" of menu item "\(menuPath[1])" of menu "\(menuPath[0])" of menu bar item "\(menuPath[0])" of menu bar 1
          end tell
        end tell
        """
    }
    return nil
}

func snapshotTree(element: AXUIElement, ref: String = "root", depth: Int = 0, maxDepth: Int = 4) -> [String: Any] {
    var node: [String: Any] = [
        "ref": ref
    ]
    if let role = axElementValue(element, attribute: kAXRoleAttribute) as? String {
        node["role"] = role
    }
    if let title = axElementValue(element, attribute: kAXTitleAttribute) as? String {
        node["title"] = title
    }
    if let desc = axElementValue(element, attribute: kAXDescriptionAttribute) as? String {
        node["description"] = desc
    }
    if let value = axElementValue(element, attribute: kAXValueAttribute) {
        node["value"] = "\(value)"
    }
    guard depth < maxDepth else {
        return node
    }
    let children = (axElementValue(element, attribute: kAXChildrenAttribute) as? [Any]) ?? []
    node["children"] = children.enumerated().compactMap { index, child -> [String: Any]? in
        guard CFGetTypeID(child as CFTypeRef) == AXUIElementGetTypeID() else { return nil }
        return snapshotTree(
            element: unsafeBitCast(child, to: AXUIElement.self),
            ref: "\(ref)/\(index)",
            depth: depth + 1,
            maxDepth: maxDepth
        )
    }
    return node
}

func appElement(bundleID: String?) -> AXUIElement? {
    if let bundleID, let app = appForBundleID(bundleID) {
        return AXUIElementCreateApplication(app.processIdentifier)
    }
    if let app = NSWorkspace.shared.frontmostApplication {
        return AXUIElementCreateApplication(app.processIdentifier)
    }
    return nil
}

func snapshotResult(payload: [String: Any]) -> [String: Any] {
    let bundleID = payload["bundle_id"] as? String ?? payload["app_bundle_id"] as? String
    guard let app = appElement(bundleID: bundleID) else {
        return [
            "session_id": payload["session_id"] as? String ?? "desktop-main-session",
            "app_bundle_id": bundleID as Any,
            "tree": [:],
            "ocr_blocks": [],
            "timestamp": ISO8601DateFormatter().string(from: Date()),
        ]
    }
    return [
        "session_id": payload["session_id"] as? String ?? "desktop-main-session",
        "app_bundle_id": bundleID as Any,
        "tree": snapshotTree(element: app),
        "ocr_blocks": [],
        "timestamp": ISO8601DateFormatter().string(from: Date()),
    ]
}

func captureScreen(payload: [String: Any]) throws -> [String: Any] {
    let path = (payload["path"] as? String)
        ?? (NSTemporaryDirectory() + "/thinclaw-desktop-capture-\(UUID().uuidString).png")
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
    var args = ["-x"]
    if let windowID = payload["window_id"] as? Int {
        args.append(contentsOf: ["-l", "\(windowID)"])
    }
    args.append(path)
    process.arguments = args
    try process.run()
    process.waitUntilExit()
    guard process.terminationStatus == 0 else {
        throw NSError(domain: "bridge", code: 2, userInfo: [NSLocalizedDescriptionKey: "failed to capture screen"])
    }
    return ["path": path]
}

func runOCR(path: String) throws -> [[String: Any]] {
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    let handler = VNImageRequestHandler(url: URL(fileURLWithPath: path))
    try handler.perform([request])
    let results = (request.results ?? []).compactMap { observation -> [String: Any]? in
        guard let candidate = observation.topCandidates(1).first else { return nil }
        return [
            "text": candidate.string,
            "confidence": candidate.confidence,
            "bounds": [
                "x": observation.boundingBox.origin.x,
                "y": observation.boundingBox.origin.y,
                "width": observation.boundingBox.width,
                "height": observation.boundingBox.height,
            ],
        ]
    }
    return results
}

func runAppleScript(_ source: String) throws -> NSAppleEventDescriptor? {
    var error: NSDictionary?
    let script = NSAppleScript(source: source)
    let descriptor = script?.executeAndReturnError(&error)
    if let error {
        throw NSError(domain: "bridge", code: 10, userInfo: [
            NSLocalizedDescriptionKey: error.description
        ])
    }
    return descriptor
}

func appleScriptString(_ raw: String) -> String {
    raw
        .replacingOccurrences(of: "\\", with: "\\\\")
        .replacingOccurrences(of: "\"", with: "\\\"")
        .replacingOccurrences(of: "\n", with: "\\n")
}

func calendarStore() -> EKEventStore {
    EKEventStore()
}

func calendarForTitle(_ title: String, in store: EKEventStore) -> EKCalendar? {
    store.calendars(for: .event).first { $0.title == title }
}

func defaultCalendarSource(_ store: EKEventStore) -> EKSource? {
    if let source = store.defaultCalendarForNewEvents?.source {
        return source
    }
    return store.sources.first {
        $0.sourceType == .local || $0.sourceType == .calDAV || $0.sourceType == .subscribed
    } ?? store.sources.first
}

func calendarPayloadDate(_ payload: [String: Any], key: String) -> Date? {
    guard let raw = payload[key] as? String else { return nil }
    return ISO8601DateFormatter().date(from: raw)
}

func calendarAction(_ action: String, payload: [String: Any]) throws -> Any {
    let store = calendarStore()
    let requestedCalendar = payload["calendar"] as? String ?? payload["calendar_title"] as? String
    let calendars = requestedCalendar.flatMap { title in
        calendarForTitle(title, in: store).map { [$0] }
    } ?? store.calendars(for: .event)
    switch action {
    case "ensure_calendar":
        let title = payload["title"] as? String ?? payload["calendar"] as? String ?? "ThinClaw Canary"
        if let existing = calendarForTitle(title, in: store) {
            return ["id": existing.calendarIdentifier, "title": existing.title, "created": false]
        }
        let calendar = EKCalendar(for: .event, eventStore: store)
        calendar.title = title
        guard let source = defaultCalendarSource(store) else {
            throw NSError(domain: "bridge", code: 23, userInfo: [NSLocalizedDescriptionKey: "no writable calendar source available"])
        }
        calendar.source = source
        try store.saveCalendar(calendar, commit: true)
        return ["id": calendar.calendarIdentifier, "title": title, "created": true]
    case "list":
        let start = calendarPayloadDate(payload, key: "start") ?? Date().addingTimeInterval(-86400)
        let end = calendarPayloadDate(payload, key: "end") ?? Date().addingTimeInterval(7 * 86400)
        let predicate = store.predicateForEvents(withStart: start, end: end, calendars: calendars)
        return store.events(matching: predicate).map { event in
            [
                "id": event.eventIdentifier ?? "",
                "title": event.title ?? "",
                "start": ISO8601DateFormatter().string(from: event.startDate),
                "end": ISO8601DateFormatter().string(from: event.endDate),
                "calendar": event.calendar.title,
                "notes": event.notes as Any,
            ]
        }
    case "find":
        let query = (payload["query"] as? String ?? "").lowercased()
        let predicate = store.predicateForEvents(withStart: Date().addingTimeInterval(-86400), end: Date().addingTimeInterval(30 * 86400), calendars: calendars)
        return store.events(matching: predicate).filter { event in
            event.title.lowercased().contains(query) || (event.notes ?? "").lowercased().contains(query)
        }.map { event in
            [
                "id": event.eventIdentifier ?? "",
                "title": event.title ?? "",
                "start": ISO8601DateFormatter().string(from: event.startDate),
                "end": ISO8601DateFormatter().string(from: event.endDate),
            ]
        }
    case "create":
        let event = EKEvent(eventStore: store)
        event.title = payload["title"] as? String ?? "Untitled Event"
        event.notes = payload["notes"] as? String
        event.startDate = calendarPayloadDate(payload, key: "start") ?? Date()
        event.endDate = calendarPayloadDate(payload, key: "end") ?? event.startDate.addingTimeInterval(3600)
        event.calendar = calendars.first ?? store.defaultCalendarForNewEvents
        try store.save(event, span: .thisEvent)
        return [
            "id": event.eventIdentifier ?? "",
            "title": event.title ?? "",
            "calendar": event.calendar.title,
        ]
    case "update":
        guard let eventID = payload["event_id"] as? String,
              let event = store.event(withIdentifier: eventID) else {
            throw NSError(domain: "bridge", code: 20, userInfo: [NSLocalizedDescriptionKey: "event not found"])
        }
        if let title = payload["title"] as? String { event.title = title }
        if let notes = payload["notes"] as? String { event.notes = notes }
        if let start = calendarPayloadDate(payload, key: "start") { event.startDate = start }
        if let end = calendarPayloadDate(payload, key: "end") { event.endDate = end }
        try store.save(event, span: .thisEvent)
        return ["updated": true, "id": event.eventIdentifier ?? ""]
    case "delete":
        guard let eventID = payload["event_id"] as? String,
              let event = store.event(withIdentifier: eventID) else {
            throw NSError(domain: "bridge", code: 21, userInfo: [NSLocalizedDescriptionKey: "event not found"])
        }
        try store.remove(event, span: .thisEvent)
        return ["deleted": true, "id": eventID]
    default:
        throw NSError(domain: "bridge", code: 22, userInfo: [NSLocalizedDescriptionKey: "unsupported calendar action \(action)"])
    }
}

func numbersAction(_ action: String, payload: [String: Any]) throws -> Any {
    let docPath = payload["path"] as? String
    let exportPath = payload["export_path"] as? String
    let tableName = payload["table"] as? String ?? "Table 1"
    let cell = payload["cell"] as? String ?? "A1"
    let value = payload["value"] as? String ?? ""
    let escapedDocPath = docPath.map(appleScriptString)
    let escapedExportPath = exportPath.map(appleScriptString)
    let escapedTableName = appleScriptString(tableName)

    func tableScript(_ body: String) -> String {
        """
        tell application "Numbers"
          tell front document
            tell active sheet
              tell table "\(escapedTableName)"
                \(body)
              end tell
            end tell
          end tell
        end tell
        """
    }

    switch action {
    case "create_doc":
        guard let docPath, let escapedDocPath else { throw NSError(domain: "bridge", code: 29, userInfo: [NSLocalizedDescriptionKey: "missing path"]) }
        let script = """
        tell application "Numbers"
          activate
          set docRef to make new document
          save docRef in POSIX file "\(escapedDocPath)"
        end tell
        """
        _ = try runAppleScript(script)
        return ["created": true, "path": docPath]
    case "open_doc":
        guard let docPath, let escapedDocPath else { throw NSError(domain: "bridge", code: 30, userInfo: [NSLocalizedDescriptionKey: "missing path"]) }
        _ = try runAppleScript("tell application \"Numbers\" to open POSIX file \"\(escapedDocPath)\"")
        return ["opened": true, "path": docPath]
    case "read_range":
        let script = tableScript("return value of cell \"\(appleScriptString(cell))\"")
        let result = try runAppleScript(script)
        return ["cell": cell, "value": result?.stringValue ?? ""]
    case "write_range":
        let script = tableScript("set value of cell \"\(appleScriptString(cell))\" to \"\(appleScriptString(value))\"")
        _ = try runAppleScript(script)
        return ["written": true, "cell": cell]
    case "set_formula":
        let script = tableScript("set formula of cell \"\(appleScriptString(cell))\" to \"\(appleScriptString(value))\"")
        _ = try runAppleScript(script)
        return ["formula_set": true, "cell": cell]
    case "run_table_action":
        let tableAction = payload["table_action"] as? String ?? ""
        let rowIndex = payload["row_index"] as? Int ?? 0
        let columnIndex = payload["column_index"] as? Int ?? 0
        let rangeRef = payload["range"] as? String ?? ""
        let body: String
        switch tableAction {
        case "add_row_above":
            body = "add row above row \(rowIndex)"
        case "add_row_below":
            body = "add row below row \(rowIndex)"
        case "delete_row":
            body = "delete row \(rowIndex)"
        case "add_column_before":
            body = "add column before column \(columnIndex)"
        case "add_column_after":
            body = "add column after column \(columnIndex)"
        case "delete_column":
            body = "delete column \(columnIndex)"
        case "clear_range":
            body = "set value of every cell of range \"\(appleScriptString(rangeRef))\" to \"\""
        case "sort_column_ascending":
            body = "sort column \(columnIndex) direction ascending"
        case "sort_column_descending":
            body = "sort column \(columnIndex) direction descending"
        default:
            return [
                "success": false,
                "error_code": "unsupported_table_action",
                "table_action": tableAction,
            ]
        }
        _ = try runAppleScript(tableScript(body))
        return [
            "success": true,
            "table_action": tableAction,
            "table": tableName,
        ]
    case "export":
        guard let exportPath, let escapedExportPath else { throw NSError(domain: "bridge", code: 31, userInfo: [NSLocalizedDescriptionKey: "missing export_path"]) }
        _ = try runAppleScript("tell application \"Numbers\" to export front document to POSIX file \"\(escapedExportPath)\" as CSV")
        return ["exported": true, "path": exportPath]
    default:
        throw NSError(domain: "bridge", code: 32, userInfo: [NSLocalizedDescriptionKey: "unsupported numbers action \(action)"])
    }
}

func pagesAction(_ action: String, payload: [String: Any]) throws -> Any {
    let docPath = payload["path"] as? String
    let exportPath = payload["export_path"] as? String
    let text = payload["text"] as? String ?? ""
    let search = payload["search"] as? String ?? ""
    let replacement = payload["replacement"] as? String ?? ""
    let escapedDocPath = docPath.map(appleScriptString)
    let escapedExportPath = exportPath.map(appleScriptString)
    let escapedText = appleScriptString(text)
    let escapedSearch = appleScriptString(search)
    let escapedReplacement = appleScriptString(replacement)

    switch action {
    case "create_doc":
        guard let docPath, let escapedDocPath else { throw NSError(domain: "bridge", code: 39, userInfo: [NSLocalizedDescriptionKey: "missing path"]) }
        let script = """
        tell application "Pages"
          activate
          set docRef to make new document
          save docRef in POSIX file "\(escapedDocPath)"
        end tell
        """
        _ = try runAppleScript(script)
        return ["created": true, "path": docPath]
    case "open_doc":
        guard let docPath, let escapedDocPath else { throw NSError(domain: "bridge", code: 40, userInfo: [NSLocalizedDescriptionKey: "missing path"]) }
        _ = try runAppleScript("tell application \"Pages\" to open POSIX file \"\(escapedDocPath)\"")
        return ["opened": true, "path": docPath]
    case "insert_text":
        _ = try runAppleScript("tell application \"Pages\" to tell front document to tell body text to set it to ((it as string) & \"\(escapedText)\")")
        return ["inserted": true]
    case "replace_text":
        let script = """
        tell application "Pages"
          tell front document
            set currentText to (body text as string)
            set AppleScript's text item delimiters to "\(escapedSearch)"
            set textItems to every text item of currentText
            set AppleScript's text item delimiters to "\(escapedReplacement)"
            set body text to textItems as string
            set AppleScript's text item delimiters to ""
          end tell
        end tell
        """
        _ = try runAppleScript(script)
        return ["replaced": true]
    case "find":
        let script = "tell application \"Pages\" to tell front document to return (body text as string)"
        let result = try runAppleScript(script)?.stringValue ?? ""
        return ["found": result.contains(search), "query": search]
    case "export":
        guard let exportPath, let escapedExportPath else { throw NSError(domain: "bridge", code: 41, userInfo: [NSLocalizedDescriptionKey: "missing export_path"]) }
        _ = try runAppleScript("tell application \"Pages\" to export front document to POSIX file \"\(escapedExportPath)\" as PDF")
        return ["exported": true, "path": exportPath]
    default:
        throw NSError(domain: "bridge", code: 42, userInfo: [NSLocalizedDescriptionKey: "unsupported pages action \(action)"])
    }
}

func handleApps(payload: [String: Any]) {
    let action = payload["action"] as? String ?? "list"
    switch action {
    case "list":
        ok(runningApps())
    case "open":
        if let bundleID = payload["bundle_id"] as? String,
           let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID) {
            let config = NSWorkspace.OpenConfiguration()
            NSWorkspace.shared.openApplication(at: url, configuration: config) { _, error in
                if let error { fail(error.localizedDescription) }
                ok(["opened": true, "bundle_id": bundleID])
            }
            dispatchMain()
        } else if let path = payload["path"] as? String {
            let okValue = NSWorkspace.shared.open(URL(fileURLWithPath: path))
            ok(["opened": okValue, "path": path])
        } else {
            fail("desktop_apps open requires bundle_id or path")
        }
    case "focus":
        guard let bundleID = payload["bundle_id"] as? String,
              let app = appForBundleID(bundleID) else {
            fail("desktop_apps focus requires a running bundle_id")
        }
        let focused = app.activate()
        ok(["focused": focused, "bundle_id": bundleID])
    case "quit":
        guard let bundleID = payload["bundle_id"] as? String,
              let app = appForBundleID(bundleID) else {
            fail("desktop_apps quit requires a running bundle_id")
        }
        ok(["quit": app.terminate(), "bundle_id": bundleID])
    case "windows":
        guard let bundleID = payload["bundle_id"] as? String,
              let app = appForBundleID(bundleID) else {
            fail("desktop_apps windows requires a running bundle_id")
        }
        let axApp = AXUIElementCreateApplication(app.processIdentifier)
        let windows = (axElementValue(axApp, attribute: kAXWindowsAttribute) as? [Any]) ?? []
        ok(windows.enumerated().map { index, item in
            guard CFGetTypeID(item as CFTypeRef) == AXUIElementGetTypeID() else {
                return ["index": index, "title": ""]
            }
            let element = item as! AXUIElement
            return [
                "index": index,
                "title": (axElementValue(element, attribute: kAXTitleAttribute) as? String) ?? "",
            ]
        })
    case "menus":
        ok([])
    default:
        fail("unsupported desktop_apps action \(action)")
    }
}

func handleUI(payload: [String: Any]) {
    let action = payload["action"] as? String ?? "snapshot"
    let bundleID = payload["bundle_id"] as? String ?? payload["app_bundle_id"] as? String
    let targetRef = payload["target_ref"] as? String
    switch action {
    case "snapshot":
        ok(snapshotResult(payload: payload))
    case "keypress":
        let key = payload["key"] as? String ?? ""
        let modifiers = payload["modifiers"] as? [String] ?? []
        ok(["success": postKeyPress(key: key, modifiers: modifiers)])
    case "type_text":
        let text = payload["text"] as? String ?? ""
        do {
            _ = try runAppleScript("tell application \"System Events\" to keystroke \"\(appleScriptString(text))\"")
            ok(["success": true])
        } catch {
            fail(error.localizedDescription)
        }
    case "chord":
        let modifiers = payload["modifiers"] as? [String] ?? []
        if let keys = payload["keys"] as? [String], !keys.isEmpty {
            let success = keys.allSatisfy { postKeyPress(key: $0, modifiers: modifiers) }
            ok(["success": success])
        } else {
            let key = payload["key"] as? String ?? ""
            ok(["success": postKeyPress(key: key, modifiers: modifiers)])
        }
    case "click", "double_click":
        if let element = resolveElement(bundleID: bundleID, ref: targetRef),
           axElementPerform(element, action: kAXPressAction) {
            ok([
                "success": true,
                "method": "ax_press",
                "new_snapshot": snapshotResult(payload: payload),
            ])
        } else if let point = targetPoint(payload: payload) {
            ok([
                "success": postMouseClick(at: point, double: action == "double_click"),
                "method": "mouse_click",
                "point": ["x": point.x, "y": point.y],
                "new_snapshot": snapshotResult(payload: payload),
            ])
        } else {
            ok([
                "success": false,
                "retryable": true,
                "error_code": "missing_target",
                "error_message": "click action requires target_ref or coordinates",
            ])
        }
    case "set_value":
        guard let element = resolveElement(bundleID: bundleID, ref: targetRef) else {
            ok([
                "success": false,
                "retryable": true,
                "error_code": "missing_target",
                "error_message": "set_value requires a resolvable target_ref",
            ])
            return
        }
        let value = payload["value"] ?? payload["text"] ?? ""
        let success = axElementSetValue(element, attribute: kAXValueAttribute, value: "\(value)")
        ok([
            "success": success,
            "new_snapshot": snapshotResult(payload: payload),
        ])
    case "select_menu":
        if let element = resolveElement(bundleID: bundleID, ref: targetRef),
           axElementPerform(element, action: kAXPressAction) {
            ok(["success": true, "method": "ax_press", "new_snapshot": snapshotResult(payload: payload)])
            return
        }
        if let processName = frontmostProcessName(bundleID: bundleID),
           let menuPath = payload["menu_path"] as? [String],
           let script = menuClickScript(processName: processName, menuPath: menuPath) {
            do {
                _ = try runAppleScript(script)
                ok(["success": true, "method": "apple_script_menu", "new_snapshot": snapshotResult(payload: payload)])
            } catch {
                fail(error.localizedDescription)
            }
        } else {
            ok([
                "success": false,
                "retryable": true,
                "error_code": "missing_target",
                "error_message": "select_menu requires target_ref or a menu_path",
            ])
        }
    case "scroll":
        let deltaX = Int32(intValue(payload["delta_x"]) ?? 0)
        let deltaY = Int32(intValue(payload["delta_y"]) ?? intValue(payload["amount"]) ?? -120)
        if let point = targetPoint(payload: payload) {
            _ = postMouseClick(at: point)
        }
        ok([
            "success": postScroll(deltaX: deltaX, deltaY: deltaY),
            "new_snapshot": snapshotResult(payload: payload),
        ])
    case "drag":
        guard let start = targetPoint(payload: payload, prefix: "from") ?? targetPoint(payload: payload),
              let end = targetPoint(payload: payload, prefix: "to") else {
            ok([
                "success": false,
                "retryable": true,
                "error_code": "missing_target",
                "error_message": "drag requires from/to refs or coordinates",
            ])
            return
        }
        ok([
            "success": postDrag(from: start, to: end),
            "from": ["x": start.x, "y": start.y],
            "to": ["x": end.x, "y": end.y],
            "new_snapshot": snapshotResult(payload: payload),
        ])
    case "wait_for":
        let timeoutMS = (payload["timeout_ms"] as? Int) ?? 5000
        let deadline = Date().addingTimeInterval(Double(timeoutMS) / 1000.0)
        let queryText = (payload["query_text"] as? String ?? payload["text"] as? String ?? "").lowercased()
        while Date() < deadline {
            let snapshot = snapshotResult(payload: payload)
            let targetExists = targetRef == nil || resolveElement(bundleID: bundleID, ref: targetRef) != nil
            let queryMatches = queryText.isEmpty || jsonText(snapshot).lowercased().contains(queryText)
            if targetExists && queryMatches {
                ok([
                    "success": true,
                    "new_snapshot": snapshot,
                ])
                return
            }
            usleep(200_000)
        }
        ok([
            "success": false,
            "retryable": true,
            "error_code": "timeout",
            "error_message": "wait_for timed out before the requested UI condition appeared",
            "new_snapshot": snapshotResult(payload: payload),
        ])
    default:
        ok([
            "success": false,
            "retryable": true,
            "error_code": "not_implemented",
            "error_message": "ui action \(action) is not implemented yet in the Swift sidecar"
        ])
    }
}

func handleScreen(payload: [String: Any]) {
    let action = payload["action"] as? String ?? "capture"
    do {
        switch action {
        case "capture", "window_capture":
            ok(try captureScreen(payload: payload))
        case "ocr":
            let path: String
            if let existing = payload["path"] as? String {
                path = existing
            } else {
                path = try captureScreen(payload: payload)["path"] as! String
            }
            ok(["path": path, "ocr_blocks": try runOCR(path: path)])
        case "find_text":
            let query = (payload["query"] as? String ?? "").lowercased()
            let path: String
            if let existing = payload["path"] as? String {
                path = existing
            } else {
                path = try captureScreen(payload: payload)["path"] as! String
            }
            let blocks = try runOCR(path: path).filter {
                (($0["text"] as? String) ?? "").lowercased().contains(query)
            }
            ok(["path": path, "matches": blocks])
        default:
            fail("unsupported desktop_screen action \(action)")
        }
    } catch {
        fail(error.localizedDescription)
    }
}

func handleNative(action: String, payload: [String: Any], fn: (String, [String: Any]) throws -> Any) {
    do {
        ok(try fn(action, payload))
    } catch {
        fail(error.localizedDescription)
    }
}

let payload = readJSONPayload()
switch commandArg() {
case "health":
    ok([
        "ok": true,
        "sidecar": "ThinClawDesktopBridge",
        "platform": "macos",
        "timestamp": ISO8601DateFormatter().string(from: Date()),
    ])
case "permissions":
    ok(permissionStatus())
case "apps":
    handleApps(payload: payload)
case "ui":
    handleUI(payload: payload)
case "screen":
    handleScreen(payload: payload)
case "calendar":
    handleNative(action: payload["action"] as? String ?? "list", payload: payload, fn: calendarAction)
case "numbers":
    handleNative(action: payload["action"] as? String ?? "open_doc", payload: payload, fn: numbersAction)
case "pages":
    handleNative(action: payload["action"] as? String ?? "open_doc", payload: payload, fn: pagesAction)
default:
    fail("unsupported bridge command \(commandArg())")
}
