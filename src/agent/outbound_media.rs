//! Compatibility adapter for agent-owned outbound media extraction.

use thinclaw_media::MediaContent;
use thinclaw_tools_core::ToolArtifact;

pub(crate) async fn attachments_from_tool_result(
    tool_name: &str,
    result_json: &serde_json::Value,
    artifacts: &[ToolArtifact],
) -> Vec<MediaContent> {
    thinclaw_agent::outbound_media::attachments_from_tool_result(tool_name, result_json, artifacts)
        .await
}

pub(crate) fn dedupe_extend(target: &mut Vec<MediaContent>, incoming: Vec<MediaContent>) {
    thinclaw_agent::outbound_media::dedupe_extend(target, incoming);
}
