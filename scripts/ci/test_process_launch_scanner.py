import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("check-process-launches.py")
SPEC = importlib.util.spec_from_file_location("process_launch_scanner", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
SCANNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SCANNER)


class ProcessLaunchScannerTests(unittest.TestCase):
    def test_test_module_range_ignores_braces_in_literals_and_comments(self):
        source = r'''
#[cfg(test)]
mod tests {
    const FIXTURE: &str = r#"int main(void) { return 0; }"#;
    // A comment brace must not terminate the module: }
    fn helper() { std::process::Command::new("git"); }
}

fn production() {
    crate::std_process_command!("crate.production.std.1", "git");
}
'''
        ranges = SCANNER.test_only_ranges(source)
        self.assertEqual(len(ranges), 1)
        self.assertTrue(SCANNER.in_ranges(source.index("Command::new"), ranges))
        self.assertFalse(SCANNER.in_ranges(source.index("std_process_command"), ranges))

    def test_launch_pattern_accepts_external_and_internal_macro_paths(self):
        source = '''
thinclaw_platform::tokio_process_command!("root.launch.tokio.1", "git");
crate::std_process_command!("platform.launch.std.1", "git");
'''
        matches = list(SCANNER.LAUNCH.finditer(source))
        self.assertEqual(
            [(match.group("kind"), match.group("id")) for match in matches],
            [
                ("tokio", "root.launch.tokio.1"),
                ("std", "platform.launch.std.1"),
            ],
        )


if __name__ == "__main__":
    unittest.main()
