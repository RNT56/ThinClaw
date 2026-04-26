use super::*;
impl Agent {
    /// Execute a tool for chat (without full job context).
    pub(super) async fn execute_chat_tool(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
        job_ctx: &JobContext,
    ) -> Result<String, Error> {
        execute_chat_tool_standalone(
            self.tools(),
            self.safety(),
            tool_name,
            params,
            job_ctx,
            ToolExecutionLane::Chat,
            self.config.main_tool_profile,
        )
        .await
    }
}
