//! Compatibility facade for the extracted WASM tool wrapper.

use async_trait::async_trait;

use crate::context::JobContext;
use crate::safety::{LeakDetector, LeakScanResult};
use crate::tools::execution::HostMediatedToolInvoker;

pub use thinclaw_tools::wasm::wrapper::*;

#[async_trait]
impl thinclaw_tools::wasm::HostToolInvoker for HostMediatedToolInvoker {
    async fn invoke_json(
        &self,
        job_ctx: &JobContext,
        tool_name: &str,
        params_json: &str,
    ) -> Result<String, String> {
        HostMediatedToolInvoker::invoke_json(self, job_ctx, tool_name, params_json)
            .await
            .map_err(|error| error.to_string())
    }
}

/// App adapter that preserves the root safety leak detector behind the tools port.
#[derive(Debug, Default)]
pub struct SafetyLeakScanner;

impl thinclaw_tools::wasm::LeakScanner for SafetyLeakScanner {
    fn scan(&self, content: &str, exact_values: &[String]) -> thinclaw_tools::wasm::LeakScan {
        let detector = LeakDetector::with_exact_values(exact_values);
        convert_leak_scan(detector.scan(content))
    }
}

fn convert_leak_scan(result: LeakScanResult) -> thinclaw_tools::wasm::LeakScan {
    thinclaw_tools::wasm::LeakScan {
        should_block: result.should_block,
        redacted_content: result.redacted_content,
        matches: result
            .matches
            .into_iter()
            .map(|leak_match| thinclaw_tools::wasm::LeakScanMatch {
                pattern_name: leak_match.pattern_name,
                action_taken: leak_match.action.to_string(),
                masked_preview: leak_match.masked_preview,
            })
            .collect(),
    }
}
