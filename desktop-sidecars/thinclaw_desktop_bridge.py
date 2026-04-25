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


def run_with_input(cmd, text, check=True):
    result = subprocess.run(
        cmd,
        input=text,
        check=False,
        text=True,
        capture_output=True,
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


def desktop_tokens():
    raw = os.environ.get("XDG_CURRENT_DESKTOP") or ""
    return [part.strip().lower() for part in raw.replace(";", ":").split(":") if part.strip()]


def session_type():
    return (os.environ.get("XDG_SESSION_TYPE") or "").strip().lower()


def is_wayland_session():
    return bool(os.environ.get("WAYLAND_DISPLAY")) or session_type() == "wayland"


def input_backends():
    backends = []
    if os.environ.get("DISPLAY") and command_exists("xdotool"):
        backends.append("xdotool")
    if command_exists("ydotool"):
        backends.append("ydotool")
    if command_exists("dotool"):
        backends.append("dotool")
    if command_exists("wtype"):
        backends.append("wtype")
    return backends


def pointer_backend():
    backends = input_backends()
    for candidate in ("xdotool", "ydotool", "dotool"):
        if candidate in backends:
            return candidate
    return None


def keyboard_backend(require_combo=False):
    backends = input_backends()
    for candidate in ("xdotool", "dotool", "ydotool", "wtype"):
        if candidate == "wtype" and require_combo:
            continue
        if candidate in backends:
            return candidate
    return None


def window_backends():
    backends = []
    if command_exists("wmctrl") and os.environ.get("DISPLAY"):
        backends.append("wmctrl")
    if pyatspi_available():
        backends.append("atspi")
    return backends


def menu_backends():
    return ["atspi"] if pyatspi_available() else []


def linux_desktop_capabilities():
    desktops = desktop_tokens()
    known_desktop = any(
        item in desktops
        for item in ("gnome", "kde", "plasma", "xfce", "lxqt", "mate", "cinnamon", "unity", "budgie", "sway")
    )
    session_supported = session_type() in ("", "x11", "wayland")
    has_display = bool(os.environ.get("DISPLAY") or os.environ.get("WAYLAND_DISPLAY"))
    return {
        "session_type": session_type() or "unknown",
        "xdg_current_desktop": os.environ.get("XDG_CURRENT_DESKTOP"),
        "display": os.environ.get("DISPLAY"),
        "wayland_display": os.environ.get("WAYLAND_DISPLAY"),
        "desktops": desktops,
        "known_desktop": known_desktop,
        "supported_desktop": has_display and session_supported,
        "input_backends": input_backends(),
        "pointer_backend": pointer_backend(),
        "keyboard_backend": keyboard_backend(),
        "window_backends": window_backends(),
        "menu_backends": menu_backends(),
        "accessibility": pyatspi_available(),
    }


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
        return atspi_window_listing()
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
                "provider": "wmctrl",
            }
        )
    return windows or atspi_window_listing()


def focus_window(window_id=None, bundle_id=None):
    if not command_exists("wmctrl"):
        return {"focused": False, "provider": "none", "reason": "wmctrl is unavailable"}
    if window_id:
        run(["wmctrl", "-ia", window_id])
        return {"focused": True, "window_id": window_id}
    if bundle_id:
        run(["wmctrl", "-xa", bundle_id], check=False)
        return {"focused": True, "bundle_id": bundle_id}
    raise RuntimeError("desktop_apps focus requires window_id or bundle_id")


def find_atspi_node_by_ref(target_ref):
    if not target_ref or not str(target_ref).startswith("atspi:") or not pyatspi_available():
        return None
    _, _, raw = str(target_ref).partition(":")
    parts = [part for part in raw.split(":") if part]
    try:
        for node in iter_atspi(atspi_desktop()):
            name = safe_name(node)
            if parts and name and name.replace(":", "_") == parts[-1]:
                return node
    except Exception:
        return None
    return None


def perform_accessible_default_action(node):
    try:
        action = node.queryAction()
        if action.nActions > 0:
            action.doAction(0)
            return True
    except Exception:
        pass
    try:
        node.queryComponent().grabFocus()
        return True
    except Exception:
        return False


def focus_target(target):
    if target.get("window_id"):
        return focus_window(window_id=target["window_id"])
    target_ref = target.get("target_ref")
    node = find_atspi_node_by_ref(target_ref)
    if node is not None and perform_accessible_default_action(node):
        return {"focused": True, "target_ref": target_ref, "provider": "atspi"}
    if target.get("bundle_id"):
        return focus_window(bundle_id=target.get("bundle_id"))
    return {"focused": False, "target_ref": target_ref, "provider": "none"}


def pick_window(payload):
    direct = direct_target_from_payload(payload)
    if direct:
        return direct
    target_ref = payload.get("target_ref")
    if isinstance(target_ref, str) and target_ref.startswith("window:"):
        window_id = target_ref.split(":", 1)[1]
        for window in window_listing():
            if str(window.get("window_id", "")).lower() == window_id.lower():
                return window
    window_id = payload.get("window_id")
    if window_id:
        for window in window_listing():
            if str(window.get("window_id", "")).lower() == str(window_id).lower():
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


def direct_target_from_payload(payload):
    if payload.get("x") is not None and payload.get("y") is not None:
        x = int(payload.get("x"))
        y = int(payload.get("y"))
        return {
            "bounds": {"x": x, "y": y, "width": 1, "height": 1},
            "target_ref": f"point:{x},{y}",
            "provider": "coordinates",
        }
    bounds = payload.get("bounds")
    if isinstance(bounds, dict):
        return {
            "bounds": {
                "x": int(bounds.get("x", 0)),
                "y": int(bounds.get("y", 0)),
                "width": int(bounds.get("width", 1)),
                "height": int(bounds.get("height", 1)),
            },
            "target_ref": "bounds",
            "provider": "coordinates",
        }
    return None


def pyatspi_available():
    try:
        import pyatspi  # noqa: F401

        return True
    except Exception:
        return False


def atspi_desktop():
    import pyatspi

    return pyatspi.Registry.getDesktop(0)


def safe_role_name(node):
    try:
        return str(node.getRoleName())
    except Exception:
        return ""


def safe_name(node):
    try:
        return node.name or ""
    except Exception:
        return ""


def safe_bounds(node):
    try:
        component = node.queryComponent()
        extents = component.getExtents(0)
        return {
            "x": int(extents.x),
            "y": int(extents.y),
            "width": int(extents.width),
            "height": int(extents.height),
        }
    except Exception:
        return {}


def safe_target_ref(prefix, *parts):
    clean = ":".join(str(part).replace(":", "_") for part in parts if str(part))
    return f"{prefix}:{clean}" if clean else prefix


def iter_atspi(node, depth=0, max_depth=8, budget=None):
    if budget is None:
        budget = {"remaining": 1500}
    if budget["remaining"] <= 0 or depth > max_depth:
        return
    budget["remaining"] -= 1
    yield node
    try:
        children = list(node)
    except Exception:
        children = []
    for child in children:
        yield from iter_atspi(child, depth + 1, max_depth, budget)


def build_atspi_tree():
    desktop = atspi_desktop()
    children = []
    for app in desktop:
        child_nodes = []
        for window in app:
            child_nodes.append(
                {
                    "role": safe_role_name(window),
                    "name": safe_name(window),
                    "target_ref": safe_target_ref("atspi", safe_name(app), safe_name(window)),
                    "bounds": safe_bounds(window),
                    "children": [],
                }
            )
        children.append(
            {
                "role": "application",
                "name": safe_name(app),
                "target_ref": safe_target_ref("app", safe_name(app)),
                "children": child_nodes,
            }
        )
    return {"role": "desktop", "name": "desktop", "children": children}


def atspi_window_listing():
    if not pyatspi_available():
        return []
    windows = []
    try:
        desktop = atspi_desktop()
        for app in desktop:
            app_name = safe_name(app)
            for window in app:
                role = safe_role_name(window).lower()
                if "window" not in role and "frame" not in role and "dialog" not in role:
                    continue
                name = safe_name(window)
                windows.append(
                    {
                        "window_id": None,
                        "desktop": "atspi",
                        "pid": 0,
                        "bounds": safe_bounds(window),
                        "bundle_id": app_name,
                        "name": app_name,
                        "title": name,
                        "target_ref": safe_target_ref("atspi", app_name, name),
                        "provider": "atspi",
                    }
                )
    except Exception:
        return []
    return windows


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


def menu_role(role):
    return "menu" in str(role).lower()


def collect_menu_item(node, depth=0):
    item = {
        "role": safe_role_name(node),
        "name": safe_name(node),
        "target_ref": safe_target_ref("atspi", safe_role_name(node), safe_name(node)),
        "bounds": safe_bounds(node),
        "children": [],
    }
    if depth >= 8:
        return item
    try:
        for child in node:
            if menu_role(safe_role_name(child)) or safe_name(child):
                item["children"].append(collect_menu_item(child, depth + 1))
    except Exception:
        pass
    return item


def atspi_menu_listing(payload):
    if not pyatspi_available():
        return [
            {
                "available": False,
                "provider": "atspi",
                "reason": "pyatspi is unavailable; install python3-pyatspi and enable AT-SPI accessibility",
            }
        ]
    requested = str(payload.get("bundle_id") or payload.get("app") or payload.get("name") or "").lower()
    menus = []
    try:
        for app in atspi_desktop():
            app_name = safe_name(app)
            if requested and requested not in app_name.lower():
                continue
            app_entry = {"app": app_name, "provider": "atspi", "menus": []}
            for node in iter_atspi(app):
                role = safe_role_name(node).lower()
                if role in {"menu bar", "menubar"} or "menu bar" in role:
                    app_entry["menus"].append(collect_menu_item(node))
            if app_entry["menus"]:
                menus.append(app_entry)
    except Exception as exc:
        return [{"available": False, "provider": "atspi", "reason": str(exc)}]
    return menus or [{"available": False, "provider": "atspi", "reason": "no accessible menus found"}]


def find_menu_accessible(label, requested_app=None):
    if not pyatspi_available():
        return None
    label_norm = str(label).strip().lower()
    app_norm = str(requested_app or "").strip().lower()
    if not label_norm:
        return None
    try:
        for app in atspi_desktop():
            if app_norm and app_norm not in safe_name(app).lower():
                continue
            for node in iter_atspi(app):
                if not menu_role(safe_role_name(node)):
                    continue
                if safe_name(node).strip().lower() == label_norm:
                    return node
    except Exception:
        return None
    return None


def select_menu_atspi(path, payload):
    if not pyatspi_available() or not path:
        return None
    app = payload.get("bundle_id") or payload.get("app") or payload.get("name")
    activated = []
    for label in path:
        node = find_menu_accessible(label, app)
        if node is None:
            return None
        if not perform_accessible_default_action(node):
            return None
        activated.append(label)
        time.sleep(0.15)
    return {
        "success": True,
        "menu_path": path,
        "provider": "atspi",
        "introspection_used": True,
        "activated": activated,
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
            return focus_target(target)
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
        return atspi_menu_listing(payload)
    raise RuntimeError(f"unsupported desktop_apps action {action}")


def xdotool_mouse(window, clicks=1, button="1"):
    x, y = target_center(window)
    run(["xdotool", "mousemove", str(x), str(y)], capture=False)
    run(["xdotool", "click", "--repeat", str(clicks), button], capture=False)


def pointer_click(window, clicks=1, button="1"):
    backend = pointer_backend()
    if backend is None:
        raise RuntimeError(
            "Linux pointer actions require xdotool on X11, or ydotool/dotool for Wayland/KDE/general desktops"
        )
    x, y = target_center(window)
    if backend == "xdotool":
        xdotool_mouse(window, clicks=clicks, button=button)
        return backend
    if backend == "ydotool":
        run(["ydotool", "mousemove", "--absolute", str(x), str(y)], capture=False)
        code = {"1": "0xC0", "2": "0xC2", "3": "0xC1"}.get(str(button), "0xC0")
        for _ in range(clicks):
            run(["ydotool", "click", code], capture=False)
        return backend
    script = f"mouseto {x} {y}\n"
    for _ in range(clicks):
        script += "button left\n"
    run_with_input(["dotool"], script)
    return backend


def send_text(text):
    backend = keyboard_backend()
    if backend is None:
        raise RuntimeError("Linux text input requires xdotool, dotool, ydotool, or wtype")
    if backend == "xdotool":
        run(["xdotool", "type", "--delay", "1", text], capture=False)
    elif backend == "wtype":
        run(["wtype", text], capture=False)
    elif backend == "ydotool":
        run(["ydotool", "type", text], capture=False)
    else:
        run_with_input(["dotool"], f"type {text}\n")
    return backend


def send_keys(keys):
    backend = keyboard_backend(require_combo=("+" in str(keys)))
    if backend is None:
        raise RuntimeError("Linux key input requires xdotool, dotool, ydotool, or wtype")
    keys = str(keys).replace("cmd", "super").replace("command", "super")
    if backend == "xdotool":
        run(["xdotool", "key", keys], capture=False)
    elif backend == "wtype":
        run(["wtype", "-k", keys], capture=False)
    elif backend == "ydotool":
        run(["ydotool", "key", keys], capture=False)
    else:
        run_with_input(["dotool"], f"key {keys}\n")
    return backend


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
        focus_target(target)
        provider = pointer_click(target)
        return {"success": True, "target_ref": target["target_ref"], "provider": provider}
    if action == "double_click":
        target = pick_window(payload)
        if not target:
            raise RuntimeError("desktop_ui double_click could not resolve a target")
        focus_target(target)
        provider = pointer_click(target, clicks=2)
        return {"success": True, "target_ref": target["target_ref"], "provider": provider}
    if action == "type_text":
        target = pick_window(payload)
        if target:
            focus_target(target)
        provider = send_text(str(payload.get("text", "")))
        return {"success": True, "provider": provider}
    if action == "set_value":
        target = pick_window(payload)
        if target:
            focus_target(target)
        send_keys("ctrl+a")
        send_keys("BackSpace")
        provider = send_text(str(payload.get("value") or payload.get("text") or ""))
        return {"success": True, "provider": provider}
    if action == "keypress":
        provider = send_keys(str(payload.get("key", "")))
        return {"success": True, "provider": provider}
    if action == "chord":
        modifiers = payload.get("modifiers") or []
        key = payload.get("key") or ""
        combo = "+".join([*modifiers, key]).replace("cmd", "super").replace("command", "super")
        provider = send_keys(combo)
        return {"success": True, "provider": provider}
    if action == "select_menu":
        path = payload.get("menu_path") or payload.get("path") or payload.get("value") or []
        if isinstance(path, str):
            path = [part.strip() for part in path.split(">") if part.strip()]
        atspi_result = select_menu_atspi(path, payload)
        if atspi_result is not None:
            return atspi_result
        for label in path:
            send_text(str(label))
            send_keys("Return")
        return {
            "success": True,
            "menu_path": path,
            "provider": "keyboard_fallback",
            "introspection_used": False,
        }
    if action == "scroll":
        backend = pointer_backend()
        if backend is None:
            raise RuntimeError("Linux scroll actions require xdotool, ydotool, or dotool")
        amount = int(payload.get("amount", 1))
        button = "4" if amount > 0 else "5"
        if backend == "xdotool":
            for _ in range(abs(amount)):
                run(["xdotool", "click", button], capture=False)
        elif backend == "ydotool":
            code = "0xC3" if amount > 0 else "0xC4"
            for _ in range(abs(amount)):
                run(["ydotool", "click", code], capture=False)
        else:
            direction = "wheelup" if amount > 0 else "wheeldown"
            run_with_input(["dotool"], "".join(f"{direction}\n" for _ in range(abs(amount))))
        return {"success": True, "amount": amount, "provider": backend}
    if action == "drag":
        target = pick_window(payload)
        destination = payload.get("destination") or {}
        backend = pointer_backend()
        if backend is None:
            raise RuntimeError("Linux drag actions require xdotool, ydotool, or dotool")
        if not target:
            raise RuntimeError("desktop_ui drag could not resolve a source target")
        start_x, start_y = target_center(target)
        end_x = int(destination.get("x", start_x))
        end_y = int(destination.get("y", start_y))
        if backend == "xdotool":
            run(["xdotool", "mousemove", str(start_x), str(start_y)], capture=False)
            run(["xdotool", "mousedown", "1"], capture=False)
            run(["xdotool", "mousemove", "--sync", str(end_x), str(end_y)], capture=False)
            run(["xdotool", "mouseup", "1"], capture=False)
        elif backend == "ydotool":
            run(["ydotool", "mousemove", "--absolute", str(start_x), str(start_y)], capture=False)
            run(["ydotool", "click", "0x40"], capture=False)
            run(["ydotool", "mousemove", "--absolute", str(end_x), str(end_y)], capture=False)
            run(["ydotool", "click", "0x80"], capture=False)
        else:
            run_with_input(["dotool"], f"mouseto {start_x} {start_y}\nbuttondown left\nmouseto {end_x} {end_y}\nbuttonup left\n")
        return {"success": True, "provider": backend}
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
    raise RuntimeError(f"unsupported desktop_ui action {action}")


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
        "desktop_capabilities": linux_desktop_capabilities(),
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
                "desktop_capabilities": linux_desktop_capabilities(),
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
                "input_backends": input_backends(),
                "window_backends": window_backends(),
                "menu_backends": menu_backends(),
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
