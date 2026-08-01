//! Canonical extension and integration operations.

use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;

use super::{
    ChannelCommand, CliContext, CliError, GatewayClient, McpCommand, RegistryCommand, SkillCommand,
    ToolCommand,
};

#[derive(Subcommand, Debug, Clone)]
pub enum ExtensionsCommand {
    /// Activate one exact extension in the running runtime
    Activate(ExtensionActivateArgs),
    #[command(subcommand)]
    Channels(ChannelCommand),
    #[command(subcommand)]
    Tools(ToolCommand),
    #[command(subcommand)]
    Registry(RegistryCommand),
    #[command(subcommand)]
    Mcp(McpCommand),
    #[command(subcommand)]
    Skills(SkillCommand),
}

#[derive(Args, Debug, Clone)]
pub struct ExtensionActivateArgs {
    pub name: String,
    #[arg(long, value_enum)]
    pub kind: Option<ExtensionKindArg>,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionKindArg {
    McpServer,
    WasmTool,
    WasmChannel,
    NativePlugin,
}

pub async fn run_extension_activate(
    args: ExtensionActivateArgs,
    context: &CliContext,
) -> Result<(), CliError> {
    if !crate::skills::validate_skill_name(&args.name) {
        return Err(CliError::usage("invalid extension name"));
    }
    let config = context.config().await?;
    let client = GatewayClient::resolve_from_config(None, None, config)
        .map_err(|error| CliError::operational(error.to_string()))?;
    let value: serde_json::Value = client
        .post_json(
            &format!("/api/extensions/{}/activate", args.name),
            &serde_json::json!({"kind": args.kind}),
        )
        .await
        .map_err(|error| CliError::operational(error.to_string()))?;
    context
        .output()
        .write_record("extensions.activate", &value, |value| {
            value
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("extension activation completed")
                .to_string()
        })?;
    if value.get("success").and_then(serde_json::Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(CliError::operational_reported())
    }
}
