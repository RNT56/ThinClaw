#!/usr/bin/env python3
import importlib.util
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


MODULE_PATH = Path(__file__).with_name("thinclaw_desktop_bridge.py")
SPEC = importlib.util.spec_from_file_location("thinclaw_desktop_bridge", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot load {MODULE_PATH}")
BRIDGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BRIDGE)


class DesktopBridgeCommandBoundaryTests(unittest.TestCase):
    def test_quit_passes_bundle_id_as_one_pkill_argument(self):
        commands = []

        def record_run(command, **kwargs):
            commands.append((command, kwargs))
            return SimpleNamespace(returncode=0, stdout="", stderr="")

        hostile_bundle_id = "victim'; touch /tmp/thinclaw-command-injection; #"
        with (
            mock.patch.object(BRIDGE, "load_state", return_value={}),
            mock.patch.object(BRIDGE, "run", side_effect=record_run),
        ):
            result = BRIDGE.invoke_apps({"action": "quit", "bundle_id": hostile_bundle_id})

        self.assertEqual(result, {"quit": True, "bundle_id": hostile_bundle_id})
        self.assertEqual(
            commands,
            [(["pkill", "-f", "--", hostile_bundle_id], {"check": False})],
        )

    def test_process_listing_does_not_invoke_a_shell(self):
        with (
            mock.patch.object(BRIDGE, "load_state", return_value={}),
            mock.patch.object(BRIDGE, "window_listing", return_value=[]),
            mock.patch.object(
                BRIDGE,
                "run",
                return_value=SimpleNamespace(returncode=0, stdout="42 thinclaw\n", stderr=""),
            ) as run,
        ):
            self.assertEqual(BRIDGE.invoke_apps({"action": "list"})[0]["pid"], 42)

        run.assert_called_once_with(["ps", "-eo", "pid=,comm="])

    def test_dotool_rejects_command_language_line_breaks(self):
        with (
            mock.patch.object(BRIDGE, "keyboard_backend", return_value="dotool"),
            mock.patch.object(BRIDGE, "run_with_input") as run_with_input,
        ):
            for unsafe_text in ("hello\nkey ctrl+l", "hello\rkey ctrl+l"):
                with self.subTest(text=unsafe_text):
                    with self.assertRaisesRegex(RuntimeError, "multiline"):
                        BRIDGE.send_text(unsafe_text)
            for unsafe_keys in ("ctrl+l\ntype injected", "ctrl+l\rtype injected"):
                with self.subTest(keys=unsafe_keys):
                    with self.assertRaisesRegex(RuntimeError, "line break"):
                        BRIDGE.send_keys(unsafe_keys)

        run_with_input.assert_not_called()

    def test_dotool_preserves_single_line_inputs(self):
        with (
            mock.patch.object(BRIDGE, "keyboard_backend", return_value="dotool"),
            mock.patch.object(BRIDGE, "run_with_input") as run_with_input,
        ):
            self.assertEqual(BRIDGE.send_text("hello"), "dotool")
            self.assertEqual(BRIDGE.send_keys("ctrl+l"), "dotool")

        self.assertEqual(
            run_with_input.call_args_list,
            [
                mock.call(["dotool"], "type hello\n"),
                mock.call(["dotool"], "key ctrl+l\n"),
            ],
        )


if __name__ == "__main__":
    unittest.main()
