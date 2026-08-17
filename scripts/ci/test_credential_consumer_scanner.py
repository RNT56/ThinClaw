import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("check-credential-consumers.py")
SPEC = importlib.util.spec_from_file_location("credential_consumer_scanner", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
SCANNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SCANNER)


class CredentialConsumerScannerTests(unittest.TestCase):
    def test_sensitive_markers_match_exact_and_compound_public_fields(self):
        source = """
pub api_key: SecretString,
pub token: String,
pub private_key: Option<String>,
pub webhook_secret: SecretString,
pub(crate) credential_source_id: CredentialSourceId,
pub ordinary_value: String,
"""

        self.assertEqual(
            [match.group("field") for match in SCANNER.PUBLIC_FIELD.finditer(source)],
            [
                "api_key",
                "token",
                "private_key",
                "webhook_secret",
                "credential_source_id",
            ],
        )

    def test_private_and_test_only_fields_are_not_public_candidates(self):
        source = """
api_key: SecretString,
pub fn token() -> String { String::new() }
"""

        self.assertEqual(list(SCANNER.PUBLIC_FIELD.finditer(source)), [])

    def test_exact_locators_and_protocol_metadata_are_not_treated_as_raw_values(self):
        locator = SCANNER.disposition("src/example.rs", "secret_name", "String")
        token_url = SCANNER.disposition("src/example.rs", "token_url", "String")
        api_key = SCANNER.disposition("src/example.rs", "api_key", "SecretString")

        self.assertEqual(locator[0], "source_bound")
        self.assertEqual(token_url[0], "non_secret_semantic")
        self.assertEqual(api_key[0], "ephemeral_internal")


if __name__ == "__main__":
    unittest.main()
