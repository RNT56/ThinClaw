#!/usr/bin/env python3
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import zipfile
from pathlib import Path
from xml.etree import ElementTree as ET


OOO_OFFICE_NS = "urn:oasis:names:tc:opendocument:xmlns:office:1.0"
OOO_TABLE_NS = "urn:oasis:names:tc:opendocument:xmlns:table:1.0"
OOO_TEXT_NS = "urn:oasis:names:tc:opendocument:xmlns:text:1.0"
OOO_MANIFEST_NS = "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"

ET.register_namespace("office", OOO_OFFICE_NS)
ET.register_namespace("table", OOO_TABLE_NS)
ET.register_namespace("text", OOO_TEXT_NS)
ET.register_namespace("manifest", OOO_MANIFEST_NS)


def read_payload():
    raw = sys.stdin.read()
    if not raw.strip():
        return {}
    return json.loads(raw)


def emit_ok(result):
    print(json.dumps({"ok": True, "result": result}, sort_keys=True))


def emit_error(message):
    print(json.dumps({"ok": False, "error": str(message)}, sort_keys=True))
    sys.exit(1)


def run(cmd, check=True, capture=True):
    result = subprocess.run(
        cmd,
        check=False,
        text=True,
        capture_output=capture,
    )
    if check and result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip() or f"command failed: {cmd}")
    return result


def command_exists(name):
    return shutil.which(name) is not None


def state_dir():
    base = os.environ.get("THINCLAW_HOME")
    if base:
        root = Path(base)
    else:
        root = Path.home() / ".thinclaw"
    path = root / "autonomy"
    path.mkdir(parents=True, exist_ok=True)
    return path


def state_path():
    return state_dir() / "linux-sidecar-state.json"


def load_state():
    path = state_path()
    if not path.exists():
        return {
            "numbers_current_doc": None,
            "pages_current_doc": None,
            "generic_ui_provider": None,
            "calendars": {},
        }
    try:
        return json.loads(path.read_text())
    except Exception:
        return {
            "numbers_current_doc": None,
            "pages_current_doc": None,
            "generic_ui_provider": None,
            "calendars": {},
        }


def save_state(state):
    state_path().write_text(json.dumps(state, sort_keys=True, indent=2))


def generic_ui_provider(state=None):
    if state and state.get("generic_ui_provider"):
        return state["generic_ui_provider"]
    if command_exists("gedit"):
        return "gedit"
    if command_exists("xdg-text-editor"):
        return "xdg-text-editor"
    return "gedit"


def now_iso():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def capture_screen(payload):
    path = payload.get("path") or str(
        Path(tempfile.gettempdir()) / f"thinclaw-screen-{int(time.time() * 1000)}.png"
    )
    if command_exists("gnome-screenshot"):
        run(["gnome-screenshot", "-f", path])
    elif command_exists("scrot"):
        run(["scrot", path])
    elif command_exists("import"):
        run(["import", "-window", "root", path])
    else:
        raise RuntimeError("no supported Linux screenshot command found")
    return {"path": path}


def ocr_blocks(path):
    if not command_exists("tesseract"):
        return []
    result = run(["tesseract", path, "stdout"], check=False)
    text = (result.stdout or "").strip()
    if not text:
        return []
    return [
        {
            "text": text,
            "confidence": None,
            "bounds": {"x": 0, "y": 0, "width": 1, "height": 1},
        }
    ]


def window_listing():
    if not command_exists("wmctrl"):
        return []
    result = run(["wmctrl", "-lpGx"], check=False)
    windows = []
    for line in (result.stdout or "").splitlines():
        parts = line.split(None, 8)
        if len(parts) < 9:
            continue
        window_id, desktop, pid, x, y, width, height, wm_class, title = parts
        class_parts = wm_class.split(".", 1)
        app_name = class_parts[0] if class_parts else wm_class
        windows.append(
            {
                "window_id": window_id,
                "desktop": desktop,
                "pid": int(pid),
                "bounds": {
                    "x": int(x),
                    "y": int(y),
                    "width": int(width),
                    "height": int(height),
                },
                "bundle_id": wm_class,
                "name": app_name,
                "title": title,
                "target_ref": f"window:{window_id}",
            }
        )
    return windows


def focus_window(window_id=None, bundle_id=None):
    if not command_exists("wmctrl"):
        raise RuntimeError("wmctrl is required for Linux app focus")
    if window_id:
        run(["wmctrl", "-ia", window_id])
        return {"focused": True, "window_id": window_id}
    if bundle_id:
        run(["wmctrl", "-xa", bundle_id], check=False)
        return {"focused": True, "bundle_id": bundle_id}
    raise RuntimeError("desktop_apps focus requires window_id or bundle_id")


def pick_window(payload):
    target_ref = payload.get("target_ref")
    if isinstance(target_ref, str) and target_ref.startswith("window:"):
        window_id = target_ref.split(":", 1)[1]
        for window in window_listing():
            if window["window_id"].lower() == window_id.lower():
                return window
    window_id = payload.get("window_id")
    if window_id:
        for window in window_listing():
            if window["window_id"].lower() == str(window_id).lower():
                return window
    bundle_id = payload.get("bundle_id")
    if bundle_id:
        candidates = [
            window
            for window in window_listing()
            if bundle_id.lower() in window["bundle_id"].lower()
            or bundle_id.lower() in window["name"].lower()
            or bundle_id.lower() in window["title"].lower()
        ]
        if candidates:
            return candidates[0]
    return None


def target_center(target):
    bounds = target.get("bounds", {})
    x = int(bounds.get("x", 0) + max(bounds.get("width", 1), 1) / 2)
    y = int(bounds.get("y", 0) + max(bounds.get("height", 1), 1) / 2)
    return x, y


def pyatspi_available():
    try:
        import pyatspi  # noqa: F401

        return True
    except Exception:
        return False


def build_atspi_tree():
    import pyatspi

    desktop = pyatspi.Registry.getDesktop(0)
    children = []
    for app in desktop:
        child_nodes = []
        for window in app:
            child_nodes.append(
                {
                    "role": str(window.getRoleName()),
                    "name": window.name or "",
                    "target_ref": f"atspi:{app.name}:{window.name}",
                    "children": [],
                }
            )
        children.append(
            {
                "role": "application",
                "name": app.name or "",
                "target_ref": f"app:{app.name}",
                "children": child_nodes,
            }
        )
    return {"role": "desktop", "name": "desktop", "children": children}


def build_window_tree():
    windows = window_listing()
    return {
        "role": "desktop",
        "name": "desktop",
        "children": [
            {
                "role": "window",
                "name": window["title"],
                "target_ref": window["target_ref"],
                "bundle_id": window["bundle_id"],
                "bounds": window["bounds"],
                "children": [],
            }
            for window in windows
        ],
    }


def invoke_apps(payload):
    state = load_state()
    action = payload.get("action", "list")
    if action == "list":
        windows = window_listing()
        if windows:
            by_pid = {}
            for window in windows:
                by_pid.setdefault(window["pid"], window)
            return [
                {
                    "name": window["name"],
                    "bundle_id": window["bundle_id"],
                    "pid": window["pid"],
                    "active": True,
                    "hidden": False,
                    "window_id": window["window_id"],
                    "target_ref": window["target_ref"],
                }
                for window in by_pid.values()
            ]
        result = run(["sh", "-lc", "ps -eo pid=,comm="])
        apps = []
        for line in result.stdout.splitlines():
            parts = line.strip().split(None, 1)
            if len(parts) != 2:
                continue
            pid, name = parts
            apps.append(
                {
                    "name": name,
                    "bundle_id": name,
                    "pid": int(pid),
                    "active": False,
                    "hidden": False,
                }
            )
        return apps
    if action == "open":
        path = payload.get("path")
        if not path:
            raise RuntimeError("desktop_apps open requires path")
        launcher = None
        if str(path).lower().endswith(".txt"):
            launcher = generic_ui_provider(state)
        if launcher == "gedit" and command_exists("gedit"):
            run(["gedit", path], capture=False)
        elif launcher == "xdg-text-editor" and command_exists("xdg-text-editor"):
            run(["xdg-text-editor", path], capture=False)
        else:
            opener = shutil.which("xdg-open") or shutil.which("gio")
            if opener is None:
                raise RuntimeError("xdg-open or gio is required")
            run([opener, path], capture=False)
        state["generic_ui_provider"] = generic_ui_provider(state)
        save_state(state)
        return {"opened": True, "path": path, "provider": state["generic_ui_provider"]}
    if action == "focus":
        target = pick_window(payload)
        if target:
            return focus_window(window_id=target["window_id"])
        bundle_id = payload.get("bundle_id")
        return focus_window(bundle_id=bundle_id)
    if action == "quit":
        bundle_id = payload.get("bundle_id")
        if not bundle_id:
            raise RuntimeError("desktop_apps quit requires bundle_id")
        run(["sh", "-lc", f"pkill -f '{bundle_id}'"], check=False)
        return {"quit": True, "bundle_id": bundle_id}
    if action == "windows":
        bundle_id = payload.get("bundle_id")
        windows = window_listing()
        if bundle_id:
            windows = [w for w in windows if bundle_id.lower() in w["bundle_id"].lower()]
        return windows
    if action == "menus":
        return []
    raise RuntimeError(f"unsupported desktop_apps action {action}")


def ensure_xdotool():
    if not command_exists("xdotool"):
        raise RuntimeError("xdotool is required for Linux desktop_ui actions")


def xdotool_mouse(window, clicks=1, button="1"):
    ensure_xdotool()
    x, y = target_center(window)
    run(["xdotool", "mousemove", str(x), str(y)], capture=False)
    run(["xdotool", "click", "--repeat", str(clicks), button], capture=False)


def send_text(text):
    ensure_xdotool()
    run(["xdotool", "type", "--delay", "1", text], capture=False)


def send_keys(keys):
    ensure_xdotool()
    run(["xdotool", "key", keys], capture=False)


def invoke_ui(payload):
    action = payload.get("action", "snapshot")
    if action == "snapshot":
        tree = build_atspi_tree() if pyatspi_available() else build_window_tree()
        return {
            "session_id": payload.get("session_id", "desktop-main-session"),
            "tree": tree,
            "timestamp": now_iso(),
        }
    if action == "click":
        target = pick_window(payload)
        if not target:
            raise RuntimeError("desktop_ui click could not resolve a target")
        focus_window(window_id=target["window_id"])
        xdotool_mouse(target)
        return {"success": True, "target_ref": target["target_ref"]}
    if action == "double_click":
        target = pick_window(payload)
        if not target:
            raise RuntimeError("desktop_ui double_click could not resolve a target")
        focus_window(window_id=target["window_id"])
        xdotool_mouse(target, clicks=2)
        return {"success": True, "target_ref": target["target_ref"]}
    if action == "type_text":
        target = pick_window(payload)
        if target:
            focus_window(window_id=target["window_id"])
        send_text(str(payload.get("text", "")))
        return {"success": True}
    if action == "set_value":
        target = pick_window(payload)
        if target:
            focus_window(window_id=target["window_id"])
        send_keys("ctrl+a")
        send_keys("BackSpace")
        send_text(str(payload.get("value") or payload.get("text") or ""))
        return {"success": True}
    if action == "keypress":
        send_keys(str(payload.get("key", "")))
        return {"success": True}
    if action == "chord":
        modifiers = payload.get("modifiers") or []
        key = payload.get("key") or ""
        combo = "+".join([*modifiers, key]).replace("cmd", "super").replace("command", "super")
        send_keys(combo)
        return {"success": True}
    if action == "select_menu":
        path = payload.get("menu_path") or payload.get("path") or payload.get("value") or []
        if isinstance(path, str):
            path = [part.strip() for part in path.split(">") if part.strip()]
        for label in path:
            send_text(str(label))
            send_keys("Return")
        return {"success": True, "menu_path": path}
    if action == "scroll":
        ensure_xdotool()
        amount = int(payload.get("amount", 1))
        button = "4" if amount > 0 else "5"
        for _ in range(abs(amount)):
            run(["xdotool", "click", button], capture=False)
        return {"success": True, "amount": amount}
    if action == "drag":
        target = pick_window(payload)
        destination = payload.get("destination") or {}
        ensure_xdotool()
        if not target:
            raise RuntimeError("desktop_ui drag could not resolve a source target")
        start_x, start_y = target_center(target)
        end_x = int(destination.get("x", start_x))
        end_y = int(destination.get("y", start_y))
        run(["xdotool", "mousemove", str(start_x), str(start_y)], capture=False)
        run(["xdotool", "mousedown", "1"], capture=False)
        run(["xdotool", "mousemove", "--sync", str(end_x), str(end_y)], capture=False)
        run(["xdotool", "mouseup", "1"], capture=False)
        return {"success": True}
    if action == "wait_for":
        timeout_ms = int(payload.get("timeout_ms", 250))
        deadline = time.time() + max(timeout_ms, 50) / 1000.0
        while time.time() < deadline:
            target = pick_window(payload)
            if target:
                return {"success": True, "target_ref": target["target_ref"]}
            time.sleep(0.1)
        return {
            "success": False,
            "retryable": True,
            "error_code": "target_not_found",
            "error_message": "desktop_ui wait_for timed out before the requested target appeared",
        }
    return {
        "success": False,
        "retryable": True,
        "error_code": "not_implemented",
        "error_message": f"ui action {action} is not implemented yet in the Linux sidecar",
    }


def invoke_screen(payload):
    action = payload.get("action", "capture")
    if action in {"capture", "window_capture"}:
        return capture_screen(payload)
    if action == "ocr":
        path = payload.get("path") or capture_screen(payload)["path"]
        return {"path": path, "ocr_blocks": ocr_blocks(path)}
    if action == "find_text":
        path = payload.get("path") or capture_screen(payload)["path"]
        query = str(payload.get("query", "")).lower()
        matches = [block for block in ocr_blocks(path) if query in block["text"].lower()]
        return {"path": path, "matches": matches}
    raise RuntimeError(f"unsupported desktop_screen action {action}")


def ensure_directory_for(path):
    Path(path).parent.mkdir(parents=True, exist_ok=True)


def coord_from_cell(cell):
    cell = cell.upper()
    letters = ""
    digits = ""
    for ch in cell:
        if ch.isalpha():
            letters += ch
        elif ch.isdigit():
            digits += ch
    if not letters or not digits:
        raise RuntimeError(f"invalid cell reference {cell}")
    col = 0
    for ch in letters:
        col = col * 26 + (ord(ch) - 64)
    return int(digits), col


def cell_from_coord(row, col):
    name = ""
    while col > 0:
        col, remainder = divmod(col - 1, 26)
        name = chr(65 + remainder) + name
    return f"{name}{row}"


def default_sheet():
    return {
        "table_name": "Table 1",
        "cells": {
            "A1": {"value": "Column1"},
            "B1": {"value": "Column2"},
        }
    }


def max_sheet_bounds(sheet):
    max_row = 1
    max_col = 1
    for cell in sheet["cells"].keys():
        row, col = coord_from_cell(cell)
        max_row = max(max_row, row)
        max_col = max(max_col, col)
    return max_row, max_col


def shift_rows(sheet, start_row, delta):
    updated = {}
    for cell, data in list(sheet["cells"].items()):
        row, col = coord_from_cell(cell)
        if row >= start_row:
            row += delta
        updated[cell_from_coord(row, col)] = data
    sheet["cells"] = updated


def shift_columns(sheet, start_col, delta):
    updated = {}
    for cell, data in list(sheet["cells"].items()):
        row, col = coord_from_cell(cell)
        if col >= start_col:
            col += delta
        updated[cell_from_coord(row, col)] = data
    sheet["cells"] = updated


def normalize_formula(value):
    if value.startswith("of:="):
        return value
    if value.startswith("="):
        return "of:" + value
    return value


def parse_cell_range(value):
    if ":" not in value:
        start = coord_from_cell(value)
        return start, start
    left, right = value.split(":", 1)
    return coord_from_cell(left), coord_from_cell(right)


def write_ods(path, sheet):
    ensure_directory_for(path)
    content = ET.Element(f"{{{OOO_OFFICE_NS}}}document-content", {"{urn:oasis:names:tc:opendocument:xmlns:office:1.0}version": "1.2"})
    body = ET.SubElement(content, f"{{{OOO_OFFICE_NS}}}body")
    spreadsheet = ET.SubElement(body, f"{{{OOO_OFFICE_NS}}}spreadsheet")
    table = ET.SubElement(
        spreadsheet,
        f"{{{OOO_TABLE_NS}}}table",
        {f"{{{OOO_TABLE_NS}}}name": sheet.get("table_name", "Table 1")},
    )
    max_row, max_col = max_sheet_bounds(sheet)
    for row in range(1, max_row + 1):
        row_el = ET.SubElement(table, f"{{{OOO_TABLE_NS}}}table-row")
        for col in range(1, max_col + 1):
            cell_key = cell_from_coord(row, col)
            cell_data = sheet["cells"].get(cell_key, {})
            attrs = {}
            value = cell_data.get("value", "")
            formula = cell_data.get("formula")
            if formula:
                attrs[f"{{{OOO_TABLE_NS}}}formula"] = normalize_formula(formula)
            attrs[f"{{{OOO_OFFICE_NS}}}value-type"] = "string"
            cell_el = ET.SubElement(row_el, f"{{{OOO_TABLE_NS}}}table-cell", attrs)
            text_p = ET.SubElement(cell_el, f"{{{OOO_TEXT_NS}}}p")
            text_p.text = str(value)
    content_xml = ET.tostring(content, encoding="utf-8", xml_declaration=True)
    manifest = ET.Element(f"{{{OOO_MANIFEST_NS}}}manifest", {"{urn:oasis:names:tc:opendocument:xmlns:manifest:1.0}version": "1.2"})
    for full_path, media_type in [
        ("/", "application/vnd.oasis.opendocument.spreadsheet"),
        ("content.xml", "text/xml"),
    ]:
        ET.SubElement(
            manifest,
            f"{{{OOO_MANIFEST_NS}}}file-entry",
            {
                f"{{{OOO_MANIFEST_NS}}}full-path": full_path,
                f"{{{OOO_MANIFEST_NS}}}media-type": media_type,
            },
        )
    manifest_xml = ET.tostring(manifest, encoding="utf-8", xml_declaration=True)
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("mimetype", "application/vnd.oasis.opendocument.spreadsheet", compress_type=zipfile.ZIP_STORED)
        archive.writestr("content.xml", content_xml)
        archive.writestr("META-INF/manifest.xml", manifest_xml)


def load_ods(path):
    if not Path(path).exists():
        return default_sheet()
    try:
        with zipfile.ZipFile(path) as archive:
            raw = archive.read("content.xml")
    except Exception:
        return default_sheet()
    root = ET.fromstring(raw)
    table = root.find(f".//{{{OOO_TABLE_NS}}}table")
    sheet = {"table_name": "Table 1", "cells": {}}
    if table is None:
        return sheet
    sheet["table_name"] = table.attrib.get(f"{{{OOO_TABLE_NS}}}name", "Table 1")
    for row_index, row_el in enumerate(table.findall(f"{{{OOO_TABLE_NS}}}table-row"), start=1):
        col_index = 1
        for cell_el in row_el.findall(f"{{{OOO_TABLE_NS}}}table-cell"):
            texts = []
            for p in cell_el.findall(f"{{{OOO_TEXT_NS}}}p"):
                texts.append(p.text or "")
            data = {"value": "\n".join(texts)}
            formula = cell_el.attrib.get(f"{{{OOO_TABLE_NS}}}formula")
            if formula:
                data["formula"] = formula
            if data["value"] or formula:
                sheet["cells"][cell_from_coord(row_index, col_index)] = data
            col_index += 1
    return sheet


def csv_export(path, sheet):
    ensure_directory_for(path)
    max_row, max_col = max_sheet_bounds(sheet)
    lines = []
    for row in range(1, max_row + 1):
        values = []
        for col in range(1, max_col + 1):
            values.append(str(sheet["cells"].get(cell_from_coord(row, col), {}).get("value", "")))
        lines.append(",".join(values))
    Path(path).write_text("\n".join(lines))


def resolve_numbers_path(payload, state):
    path = payload.get("path") or state.get("numbers_current_doc")
    if not path:
        raise RuntimeError("desktop_numbers_native requires path or a previously opened document")
    state["numbers_current_doc"] = path
    save_state(state)
    return path


def invoke_numbers(payload):
    state = load_state()
    action = payload.get("action", "open_doc")
    path = payload.get("path")
    if action == "create_doc":
        if not path:
            raise RuntimeError("desktop_numbers_native.create_doc requires path")
        sheet = default_sheet()
        write_ods(path, sheet)
        state["numbers_current_doc"] = path
        save_state(state)
        return {"created": True, "path": path, "table": sheet["table_name"]}
    path = resolve_numbers_path(payload, state)
    sheet = load_ods(path)
    if action == "open_doc":
        write_ods(path, sheet)
        return {"opened": True, "path": path}
    if action == "read_range":
        cell = str(payload.get("cell", "A1")).upper()
        return {"cell": cell, "value": sheet["cells"].get(cell, {}).get("value", "")}
    if action == "write_range":
        cell = str(payload.get("cell", "A1")).upper()
        sheet["cells"][cell] = {"value": str(payload.get("value", ""))}
        write_ods(path, sheet)
        return {"written": True, "cell": cell, "table": sheet["table_name"]}
    if action == "set_formula":
        cell = str(payload.get("cell", "A1")).upper()
        existing = sheet["cells"].get(cell, {})
        existing["formula"] = str(payload.get("value", ""))
        if not existing.get("value"):
            existing["value"] = existing["formula"]
        sheet["cells"][cell] = existing
        write_ods(path, sheet)
        return {"formula_set": True, "cell": cell}
    if action == "run_table_action":
        table_action = str(payload.get("table_action", ""))
        row_index = int(payload.get("row_index", 0))
        column_index = int(payload.get("column_index", 0))
        if table_action == "add_row_above":
            shift_rows(sheet, row_index, 1)
        elif table_action == "add_row_below":
            shift_rows(sheet, row_index + 1, 1)
        elif table_action == "delete_row":
            updated = {}
            for cell, data in list(sheet["cells"].items()):
                row, col = coord_from_cell(cell)
                if row == row_index:
                    continue
                if row > row_index:
                    row -= 1
                updated[cell_from_coord(row, col)] = data
            sheet["cells"] = updated
        elif table_action == "add_column_before":
            shift_columns(sheet, column_index, 1)
        elif table_action == "add_column_after":
            shift_columns(sheet, column_index + 1, 1)
        elif table_action == "delete_column":
            updated = {}
            for cell, data in list(sheet["cells"].items()):
                row, col = coord_from_cell(cell)
                if col == column_index:
                    continue
                if col > column_index:
                    col -= 1
                updated[cell_from_coord(row, col)] = data
            sheet["cells"] = updated
        elif table_action == "clear_range":
            start, end = parse_cell_range(str(payload.get("range", "A1")))
            for row in range(min(start[0], end[0]), max(start[0], end[0]) + 1):
                for col in range(min(start[1], end[1]), max(start[1], end[1]) + 1):
                    sheet["cells"].pop(cell_from_coord(row, col), None)
        elif table_action in {"sort_column_ascending", "sort_column_descending"}:
            max_row, _ = max_sheet_bounds(sheet)
            rows = []
            for row in range(1, max_row + 1):
                row_values = {}
                for cell, data in sheet["cells"].items():
                    cell_row, cell_col = coord_from_cell(cell)
                    if cell_row == row:
                        row_values[cell_col] = dict(data)
                rows.append(row_values)
            descending = table_action == "sort_column_descending"
            header = rows[:1]
            body = rows[1:]
            body.sort(
                key=lambda row_values: str(row_values.get(column_index, {}).get("value", "")),
                reverse=descending,
            )
            sheet["cells"] = {}
            for row_index_1, row_values in enumerate(header + body, start=1):
                for col_index_1, data in row_values.items():
                    sheet["cells"][cell_from_coord(row_index_1, col_index_1)] = data
        else:
            return {
                "success": False,
                "error_code": "unsupported_table_action",
                "table_action": table_action,
            }
        write_ods(path, sheet)
        return {"success": True, "table_action": table_action, "table": sheet["table_name"]}
    if action == "export":
        export_path = payload.get("export_path")
        if not export_path:
            raise RuntimeError("desktop_numbers_native.export requires export_path")
        csv_export(export_path, sheet)
        return {"exported": True, "path": export_path}
    raise RuntimeError(f"unsupported numbers action {action}")


def default_document():
    return {"paragraphs": [""]}


def write_odt(path, document):
    ensure_directory_for(path)
    content = ET.Element(f"{{{OOO_OFFICE_NS}}}document-content", {"{urn:oasis:names:tc:opendocument:xmlns:office:1.0}version": "1.2"})
    body = ET.SubElement(content, f"{{{OOO_OFFICE_NS}}}body")
    text_body = ET.SubElement(body, f"{{{OOO_OFFICE_NS}}}text")
    for paragraph in document.get("paragraphs", [""]):
        p = ET.SubElement(text_body, f"{{{OOO_TEXT_NS}}}p")
        p.text = paragraph
    content_xml = ET.tostring(content, encoding="utf-8", xml_declaration=True)
    manifest = ET.Element(f"{{{OOO_MANIFEST_NS}}}manifest", {"{urn:oasis:names:tc:opendocument:xmlns:manifest:1.0}version": "1.2"})
    for full_path, media_type in [
        ("/", "application/vnd.oasis.opendocument.text"),
        ("content.xml", "text/xml"),
    ]:
        ET.SubElement(
            manifest,
            f"{{{OOO_MANIFEST_NS}}}file-entry",
            {
                f"{{{OOO_MANIFEST_NS}}}full-path": full_path,
                f"{{{OOO_MANIFEST_NS}}}media-type": media_type,
            },
        )
    manifest_xml = ET.tostring(manifest, encoding="utf-8", xml_declaration=True)
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("mimetype", "application/vnd.oasis.opendocument.text", compress_type=zipfile.ZIP_STORED)
        archive.writestr("content.xml", content_xml)
        archive.writestr("META-INF/manifest.xml", manifest_xml)


def load_odt(path):
    if not Path(path).exists():
        return default_document()
    try:
        with zipfile.ZipFile(path) as archive:
            raw = archive.read("content.xml")
    except Exception:
        return default_document()
    root = ET.fromstring(raw)
    paragraphs = []
    for p in root.findall(f".//{{{OOO_TEXT_NS}}}p"):
        paragraphs.append("".join(p.itertext()))
    return {"paragraphs": paragraphs or [""]}


def resolve_pages_path(payload, state):
    path = payload.get("path") or state.get("pages_current_doc")
    if not path:
        raise RuntimeError("desktop_pages_native requires path or a previously opened document")
    state["pages_current_doc"] = path
    save_state(state)
    return path


def export_document(path, export_path):
    ensure_directory_for(export_path)
    if command_exists("libreoffice"):
        outdir = str(Path(export_path).parent)
        run(["libreoffice", "--headless", "--convert-to", "pdf", "--outdir", outdir, path], check=False)
        converted = Path(outdir) / f"{Path(path).stem}.pdf"
        if converted.exists():
            if str(converted) != str(export_path):
                shutil.move(str(converted), export_path)
            return
    text = "\n".join(load_odt(path).get("paragraphs", []))
    Path(export_path).write_text(text)


def invoke_pages(payload):
    state = load_state()
    action = payload.get("action", "open_doc")
    path = payload.get("path")
    if action == "create_doc":
        if not path:
            raise RuntimeError("desktop_pages_native.create_doc requires path")
        write_odt(path, default_document())
        state["pages_current_doc"] = path
        save_state(state)
        return {"created": True, "path": path}
    path = resolve_pages_path(payload, state)
    document = load_odt(path)
    if action == "open_doc":
        write_odt(path, document)
        return {"opened": True, "path": path}
    if action == "insert_text":
        text = str(payload.get("text", ""))
        paragraphs = document.get("paragraphs", [""])
        if paragraphs and paragraphs[-1]:
            paragraphs[-1] = paragraphs[-1] + text
        else:
            paragraphs[-1:] = [text]
        document["paragraphs"] = paragraphs
        write_odt(path, document)
        return {"inserted": True}
    if action == "replace_text":
        search = str(payload.get("search", ""))
        replacement = str(payload.get("replacement", ""))
        document["paragraphs"] = [paragraph.replace(search, replacement) for paragraph in document.get("paragraphs", [])]
        write_odt(path, document)
        return {"replaced": True}
    if action == "find":
        search = str(payload.get("search", ""))
        content = "\n".join(document.get("paragraphs", []))
        return {"found": search in content, "query": search}
    if action == "export":
        export_path = payload.get("export_path")
        if not export_path:
            raise RuntimeError("desktop_pages_native.export requires export_path")
        export_document(path, export_path)
        return {"exported": True, "path": export_path}
    raise RuntimeError(f"unsupported pages action {action}")


def calendar_store(state):
    return state.setdefault("calendars", {})


def invoke_calendar(payload):
    state = load_state()
    action = payload.get("action", "list")
    calendar_name = str(payload.get("calendar") or payload.get("title") or "ThinClaw Canary")
    calendars = calendar_store(state)
    if action == "ensure_calendar":
        calendars.setdefault(calendar_name, [])
        save_state(state)
        return {"id": calendar_name, "title": calendar_name, "created": True}
    if action == "list":
        return calendars.get(calendar_name, [])
    if action == "find":
        query = str(payload.get("query", "")).lower()
        return [
            item
            for item in calendars.get(calendar_name, [])
            if query in item.get("title", "").lower() or query in item.get("notes", "").lower()
        ]
    if action == "create":
        calendars.setdefault(calendar_name, [])
        event = {
            "id": f"evt-{int(time.time() * 1000)}",
            "title": str(payload.get("title", "Untitled Event")),
            "start": str(payload.get("start", now_iso())),
            "end": str(payload.get("end", now_iso())),
            "notes": str(payload.get("notes", "")),
            "calendar": calendar_name,
        }
        calendars[calendar_name].append(event)
        save_state(state)
        return event
    if action == "update":
        event_id = str(payload.get("event_id", ""))
        for entries in calendars.values():
            for event in entries:
                if event["id"] == event_id:
                    if payload.get("title") is not None:
                        event["title"] = str(payload.get("title"))
                    if payload.get("notes") is not None:
                        event["notes"] = str(payload.get("notes"))
                    if payload.get("start") is not None:
                        event["start"] = str(payload.get("start"))
                    if payload.get("end") is not None:
                        event["end"] = str(payload.get("end"))
                    save_state(state)
                    return {"updated": True, "id": event_id}
        raise RuntimeError("event not found")
    if action == "delete":
        event_id = str(payload.get("event_id", ""))
        for name, entries in calendars.items():
            for index, event in enumerate(entries):
                if event["id"] == event_id:
                    del entries[index]
                    save_state(state)
                    return {"deleted": True, "id": event_id, "calendar": name}
        raise RuntimeError("event not found")
    raise RuntimeError(f"unsupported calendar action {action}")


def main():
    command = sys.argv[1] if len(sys.argv) > 1 else None
    payload = read_payload()
    state = load_state()
    providers = {
        "calendar": "evolution",
        "numbers": "libreoffice_calc",
        "pages": "libreoffice_writer",
        "generic_ui": generic_ui_provider(state),
    }

    if command == "health":
        emit_ok(
            {
                "ok": True,
                "sidecar": "ThinClawDesktopBridge",
                "platform": "linux",
                "bridge_backend": "linux_python",
                "providers": providers,
                "display": os.environ.get("DISPLAY"),
                "dbus_session_bus_address": os.environ.get("DBUS_SESSION_BUS_ADDRESS"),
                "at_spi_bus_address": os.environ.get("AT_SPI_BUS_ADDRESS"),
                "xdg_current_desktop": os.environ.get("XDG_CURRENT_DESKTOP"),
                "xdg_session_type": os.environ.get("XDG_SESSION_TYPE"),
                "timestamp": now_iso(),
            }
        )
        return
    if command == "permissions":
        emit_ok(
            {
                "platform": "linux",
                "accessibility": bool(os.environ.get("AT_SPI_BUS_ADDRESS") or pyatspi_available()),
                "screen_recording": command_exists("gnome-screenshot")
                or command_exists("scrot")
                or command_exists("import"),
                "calendar": "available" if command_exists("evolution") or command_exists("gdbus") else "missing",
                "ocr": "available" if command_exists("tesseract") else "missing",
                "xdotool": "available" if command_exists("xdotool") else "missing",
                "wmctrl": "available" if command_exists("wmctrl") else "missing",
                "generic_ui": providers["generic_ui"],
            }
        )
        return

    try:
        if command == "apps":
            emit_ok(invoke_apps(payload))
        elif command == "ui":
            emit_ok(invoke_ui(payload))
        elif command == "screen":
            emit_ok(invoke_screen(payload))
        elif command == "calendar":
            emit_ok(invoke_calendar(payload))
        elif command == "numbers":
            emit_ok(invoke_numbers(payload))
        elif command == "pages":
            emit_ok(invoke_pages(payload))
        else:
            emit_error(f"unsupported command {command}")
    except Exception as exc:
        emit_error(exc)


if __name__ == "__main__":
    main()
