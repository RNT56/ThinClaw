//! MCP server management CLI commands.
//!
//! Commands for adding, removing, authenticating, and testing MCP servers.

use std::io::Write;
use std::sync::Arc;

use clap::Subcommand;

use crate::config::Config;
use crate::db::Database;
#[cfg(feature = "postgres")]
use crate::secrets::PostgresSecretsStore;
use crate::secrets::{SecretsCrypto, SecretsStore};
use crate::terminal_branding::TerminalBranding;
use crate::tools::mcp::{
    McpClient, McpServerConfig, McpSessionManager, OAuthConfig, PromptContent,
    auth::{authorize_mcp_server, is_authenticated},
    config::{self, McpConfigStore, McpServersFile},
};

#[derive(Subcommand, Debug, Clone)]
pub enum McpCommand {
    /// Manage MCP server registration, activation, and auth.
    #[command(subcommand)]
    Server(McpServerCommand),

    /// Browse MCP resources from a server.
    #[command(subcommand)]
    Resource(McpResourceCommand),

    /// Browse MCP prompts from a server.
    #[command(subcommand)]
    Prompt(McpPromptCommand),

    /// Inspect and manage roots grants for a server.
    #[command(subcommand)]
    Root(McpRootCommand),

    /// Inspect and change MCP logging levels.
    #[command(subcommand)]
    Log(McpLogCommand),
}

#[derive(Subcommand, Debug, Clone)]
pub enum McpServerCommand {
    /// Add an MCP server
    Add {
        /// Server name (e.g., "notion", "filesystem")
        name: String,

        /// Server URL (e.g., "https://mcp.notion.com"). Required for HTTP transport.
        #[arg(required_unless_present = "command")]
        url: Option<String>,

        /// Command to run for stdio transport (e.g., "npx", "uvx", "python")
        #[arg(long)]
        command: Option<String>,

        /// Arguments for the stdio command (comma-separated or repeated)
        #[arg(long, value_delimiter = ',')]
        args: Option<Vec<String>>,

        /// Environment variables for stdio server (KEY=VALUE, comma-separated)
        #[arg(long, value_delimiter = ',')]
        env: Option<Vec<String>>,

        /// OAuth client ID (if authentication is required)
        #[arg(long)]
        client_id: Option<String>,

        /// OAuth authorization URL (optional, can be discovered)
        #[arg(long)]
        auth_url: Option<String>,

        /// OAuth token URL (optional, can be discovered)
        #[arg(long)]
        token_url: Option<String>,

        /// Scopes to request (comma-separated)
        #[arg(long)]
        scopes: Option<String>,

        /// Server description
        #[arg(long)]
        description: Option<String>,
    },

    /// Remove an MCP server
    Remove {
        /// Server name to remove
        name: String,
    },

    /// List configured MCP servers
    List {
        /// Show detailed information
        #[arg(short, long)]
        verbose: bool,
    },

    /// Show a single MCP server configuration
    Show {
        /// Server name
        name: String,
    },

    /// Authenticate with an MCP server (OAuth flow)
    Auth {
        /// Server name to authenticate
        name: String,

        /// User ID for storing the token (default: "default")
        #[arg(short, long, default_value = "default")]
        user: String,
    },

    /// Test connection to an MCP server
    Test {
        /// Server name to test
        name: String,

        /// User ID for authentication (default: "default")
        #[arg(short, long, default_value = "default")]
        user: String,
    },

    /// Enable or disable an MCP server
    Toggle {
        /// Server name
        name: String,

        /// Enable the server
        #[arg(long, conflicts_with = "disable")]
        enable: bool,

        /// Disable the server
        #[arg(long, conflicts_with = "enable")]
        disable: bool,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum McpResourceCommand {
    /// List MCP resources exposed by a server
    List {
        /// Server name
        name: String,

        /// User ID for authentication (default: "default")
        #[arg(short, long, default_value = "default")]
        user: String,
    },

    /// Read a specific MCP resource
    Read {
        /// Server name
        name: String,

        /// Resource URI
        uri: String,

        /// User ID for authentication (default: "default")
        #[arg(short, long, default_value = "default")]
        user: String,
    },

    /// List MCP resource templates exposed by a server
    Templates {
        /// Server name
        name: String,

        /// User ID for authentication (default: "default")
        #[arg(short, long, default_value = "default")]
        user: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum McpPromptCommand {
    /// List MCP prompts exposed by a server
    List {
        /// Server name
        name: String,

        /// User ID for authentication (default: "default")
        #[arg(short, long, default_value = "default")]
        user: String,
    },

    /// Fetch a prompt from an MCP server
    Get {
        /// Server name
        name: String,

        /// Prompt name
        prompt: String,

        /// Prompt arguments as repeated KEY=VALUE pairs
        #[arg(long = "arg", value_delimiter = ',')]
        args: Vec<String>,

        /// User ID for authentication (default: "default")
        #[arg(short, long, default_value = "default")]
        user: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum McpRootCommand {
    /// List roots granted to a server
    List {
        /// Server name
        name: String,
    },

    /// Grant a filesystem root to a server
    Grant {
        /// Server name
        name: String,

        /// Root path or URI
        root: String,
    },

    /// Revoke a filesystem root from a server
    Revoke {
        /// Server name
        name: String,

        /// Root path or URI
        root: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum McpLogCommand {
    /// Show the configured log level for a server
    Show {
        /// Server name
        name: String,
    },

    /// Set the log level for a server and notify the server if reachable
    Set {
        /// Server name
        name: String,

        /// Log level: debug | info | warning | error
        level: String,

        /// User ID for authentication (default: "default")
        #[arg(short, long, default_value = "default")]
        user: String,
    },
}

/// Run an MCP command.
pub async fn run_mcp_command(cmd: McpCommand) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    match cmd {
        McpCommand::Server(server_cmd) => match server_cmd {
            McpServerCommand::Add {
                name,
                url,
                command,
                args,
                env,
                client_id,
                auth_url,
                token_url,
                scopes,
                description,
            } => {
                branding.print_banner("MCP", Some("Register a model context server"));
                add_server(
                    name,
                    url,
                    command,
                    args,
                    env,
                    client_id,
                    auth_url,
                    token_url,
                    scopes,
                    description,
                )
                .await
            }
            McpServerCommand::Remove { name } => {
                branding.print_banner("MCP", Some("Remove a model context server"));
                remove_server(name).await
            }
            McpServerCommand::List { verbose } => {
                branding.print_banner("MCP", Some("Inspect configured servers"));
                list_servers(verbose).await
            }
            McpServerCommand::Show { name } => {
                branding.print_banner("MCP", Some("Inspect a model context server"));
                show_server(name).await
            }
            McpServerCommand::Auth { name, user } => auth_server(name, user).await,
            McpServerCommand::Test { name, user } => {
                branding.print_banner("MCP", Some("Test a model context server"));
                test_server(name, user).await
            }
            McpServerCommand::Toggle {
                name,
                enable,
                disable,
            } => {
                branding.print_banner("MCP", Some("Enable or disable a server"));
                toggle_server(name, enable, disable).await
            }
        },
        McpCommand::Resource(resource_cmd) => match resource_cmd {
            McpResourceCommand::List { name, user } => {
                branding.print_banner("MCP", Some("List MCP resources"));
                list_server_resources(name, user).await
            }
            McpResourceCommand::Read { name, uri, user } => {
                branding.print_banner("MCP", Some("Read an MCP resource"));
                read_server_resource(name, uri, user).await
            }
            McpResourceCommand::Templates { name, user } => {
                branding.print_banner("MCP", Some("List MCP resource templates"));
                list_server_resource_templates(name, user).await
            }
        },
        McpCommand::Prompt(prompt_cmd) => match prompt_cmd {
            McpPromptCommand::List { name, user } => {
                branding.print_banner("MCP", Some("List MCP prompts"));
                list_server_prompts(name, user).await
            }
            McpPromptCommand::Get {
                name,
                prompt,
                args,
                user,
            } => {
                branding.print_banner("MCP", Some("Fetch an MCP prompt"));
                get_server_prompt(name, prompt, args, user).await
            }
        },
        McpCommand::Root(root_cmd) => match root_cmd {
            McpRootCommand::List { name } => {
                branding.print_banner("MCP", Some("List MCP roots"));
                list_server_roots(name).await
            }
            McpRootCommand::Grant { name, root } => {
                branding.print_banner("MCP", Some("Grant an MCP root"));
                grant_server_root(name, root).await
            }
            McpRootCommand::Revoke { name, root } => {
                branding.print_banner("MCP", Some("Revoke an MCP root"));
                revoke_server_root(name, root).await
            }
        },
        McpCommand::Log(log_cmd) => match log_cmd {
            McpLogCommand::Show { name } => {
                branding.print_banner("MCP", Some("Inspect MCP logging"));
                show_server_log_level(name).await
            }
            McpLogCommand::Set { name, level, user } => {
                branding.print_banner("MCP", Some("Set MCP logging"));
                set_server_log_level(name, level, user).await
            }
        },
    }
}

/// Add a new MCP server.
#[allow(clippy::too_many_arguments)]
async fn add_server(
    name: String,
    url: Option<String>,
    command: Option<String>,
    args: Option<Vec<String>>,
    env: Option<Vec<String>>,
    client_id: Option<String>,
    auth_url: Option<String>,
    token_url: Option<String>,
    scopes: Option<String>,
    description: Option<String>,
) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    use crate::tools::mcp::config::McpTransport;

    let is_stdio = command.is_some();

    let mut config = if is_stdio {
        // Stdio transport: command is required, url is ignored
        let cmd = command.expect("guarded by is_stdio = command.is_some()");
        let cmd_args = args.unwrap_or_default();
        let mut cfg = McpServerConfig::new_stdio(&name, &cmd, cmd_args);

        // Parse env vars: KEY=VALUE
        if let Some(env_vars) = env {
            let mut env_map = std::collections::BTreeMap::new();
            for kv in env_vars {
                if let Some((k, v)) = kv.split_once('=') {
                    env_map.insert(k.to_string(), v.to_string());
                } else {
                    anyhow::bail!("Invalid env var format '{}'. Expected KEY=VALUE", kv);
                }
            }
            cfg = cfg.with_env(env_map);
        }

        cfg
    } else {
        // HTTP transport: url is required
        let url = url.ok_or_else(|| anyhow::anyhow!("URL is required for HTTP MCP servers"))?;
        McpServerConfig::new(&name, &url)
    };

    if let Some(desc) = description {
        config = config.with_description(desc);
    }

    // Track if auth is required
    let requires_auth = client_id.is_some();

    // Set up OAuth if client_id is provided (HTTP servers only)
    if let Some(client_id) = client_id {
        if is_stdio {
            anyhow::bail!("OAuth is not supported for stdio MCP servers");
        }

        let mut oauth = OAuthConfig::new(client_id);

        if let (Some(auth), Some(token)) = (auth_url, token_url) {
            oauth = oauth.with_endpoints(auth, token);
        }

        if let Some(scopes_str) = scopes {
            let scope_list: Vec<String> = scopes_str
                .split(',')
                .map(|s| s.trim().to_string())
                .collect();
            oauth = oauth.with_scopes(scope_list);
        }

        config = config.with_oauth(oauth);
    }

    // Validate
    config.validate()?;

    // Save (DB if available, else disk)
    let db = connect_db().await;
    let mut servers = load_servers(db.as_deref()).await?;
    servers.upsert(config.clone());
    save_servers(db.as_deref(), &servers).await?;

    println!(
        "  {}",
        branding.good(format!("Added MCP server '{}'", name))
    );
    match config.transport {
        McpTransport::Stdio => {
            println!(
                "    Transport: stdio ({})",
                config.command.as_deref().unwrap_or("?")
            );
            if !config.args.is_empty() {
                println!("    Args: {}", config.args.join(" "));
            }
        }
        McpTransport::Http => {
            println!("    URL: {}", config.url);
        }
    }

    if requires_auth {
        println!();
        println!(
            "  {}",
            branding.muted(format!("Run `thinclaw mcp auth {}` to authenticate.", name))
        );
    }

    println!();

    Ok(())
}

/// Remove an MCP server.
async fn remove_server(name: String) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let db = connect_db().await;
    let mut servers = load_servers(db.as_deref()).await?;
    if !servers.remove(&name) {
        anyhow::bail!("Server '{}' not found", name);
    }
    save_servers(db.as_deref(), &servers).await?;

    println!(
        "  {}",
        branding.good(format!("Removed MCP server '{}'", name))
    );
    println!();

    Ok(())
}

/// List configured MCP servers.
async fn list_servers(verbose: bool) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let db = connect_db().await;
    let servers = load_servers(db.as_deref()).await?;

    if servers.servers.is_empty() {
        println!("{}", branding.warn("No MCP servers configured."));
        println!();
        println!("{}", branding.body_bold("Add a server with:"));
        println!("    thinclaw mcp add <name> <url>");
        println!("    thinclaw mcp add <name> --command <cmd> --args <args>");
        println!();
        println!("  Examples:");
        println!("    thinclaw mcp add notion https://mcp.notion.com");
        println!(
            "    thinclaw mcp add fs --command npx --args '-y,@modelcontextprotocol/server-filesystem,/tmp'"
        );
        println!();
        return Ok(());
    }

    println!("{}", branding.body_bold("Configured MCP servers:"));
    println!();

    for server in &servers.servers {
        let status = if server.enabled { "●" } else { "○" };
        let transport_label = if server.is_stdio() {
            format!(" [stdio: {}]", server.command.as_deref().unwrap_or("?"))
        } else if server.requires_auth() {
            " (auth required)".to_string()
        } else {
            String::new()
        };

        if verbose {
            println!("  {} {}{}", status, server.name, transport_label);
            if server.is_stdio() {
                println!(
                    "      Command: {}",
                    server.command.as_deref().unwrap_or("?")
                );
                if !server.args.is_empty() {
                    println!("      Args: {}", server.args.join(" "));
                }
                if !server.env.is_empty() {
                    for (k, v) in &server.env {
                        println!("      Env: {}={}", k, v);
                    }
                }
            } else {
                println!("      URL: {}", server.url);
            }
            if let Some(ref desc) = server.description {
                println!("      Description: {}", desc);
            }
            if let Some(ref oauth) = server.oauth {
                println!("      OAuth Client ID: {}", oauth.client_id);
                if !oauth.scopes.is_empty() {
                    println!("      Scopes: {}", oauth.scopes.join(", "));
                }
            }
            println!();
        } else {
            let desc = if server.is_stdio() {
                format!("stdio:{}", server.command.as_deref().unwrap_or("?"))
            } else {
                server.url.clone()
            };
            println!("  {} {} - {}{}", status, server.name, desc, transport_label);
        }
    }

    if !verbose {
        println!();
        println!("{}", branding.muted("Use --verbose for more details."));
    }

    println!();

    Ok(())
}

async fn show_server(name: String) -> anyhow::Result<()> {
    let db = connect_db().await;
    let servers = load_servers(db.as_deref()).await?;
    let server = servers
        .get(&name)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not found", name))?;

    println!("  Name: {}", server.name);
    println!("  Display: {}", server.display_label());
    println!(
        "  Transport: {}",
        if server.is_stdio() { "stdio" } else { "http" }
    );
    if server.is_stdio() {
        println!(
            "  Command: {}",
            server.command.as_deref().unwrap_or("<missing>")
        );
        if !server.args.is_empty() {
            println!("  Args: {}", server.args.join(" "));
        }
    } else {
        println!("  URL: {}", server.url);
    }
    println!("  Enabled: {}", if server.enabled { "yes" } else { "no" });
    println!(
        "  Requires auth: {}",
        if server.requires_auth() { "yes" } else { "no" }
    );
    println!(
        "  Log level: {}",
        format!("{:?}", server.logging_level).to_ascii_lowercase()
    );
    println!("  Tool namespace: {}", server.tool_namespace());
    if !server.roots_grants.is_empty() {
        println!("  Roots:");
        for root in &server.roots_grants {
            println!("    - {}", root);
        }
    }
    if let Some(description) = server.description {
        println!("  Description: {}", description);
    }
    if let Some(oauth) = server.oauth {
        println!("  OAuth client ID: {}", oauth.client_id);
        if let Some(resource) = oauth.resource {
            println!("  OAuth resource: {}", resource);
        }
        if !oauth.scopes.is_empty() {
            println!("  OAuth scopes: {}", oauth.scopes.join(", "));
        }
    }
    println!();

    Ok(())
}

async fn build_client(server: &McpServerConfig, user_id: &str) -> anyhow::Result<McpClient> {
    let config_store = Some(McpConfigStore::new(connect_db().await, user_id.to_string()));
    if server.is_stdio() {
        return McpClient::new_stdio_with_store(server, config_store).map_err(Into::into);
    }

    let session_manager = Arc::new(McpSessionManager::new());
    match get_secrets_store().await {
        Ok(secrets) if is_authenticated(server, &secrets, user_id).await => {
            Ok(McpClient::new_authenticated_with_store(
                server.clone(),
                session_manager,
                secrets,
                user_id,
                config_store,
            ))
        }
        Ok(_) | Err(_) => Ok(McpClient::new_configured_with_store(
            server.clone(),
            config_store,
        )),
    }
}

async fn load_server(name: &str) -> anyhow::Result<McpServerConfig> {
    let db = connect_db().await;
    let servers = load_servers(db.as_deref()).await?;
    servers
        .get(name)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not found", name))
}

fn parse_arg_pairs(args: Vec<String>) -> anyhow::Result<Option<serde_json::Value>> {
    if args.is_empty() {
        return Ok(None);
    }

    let mut payload = serde_json::Map::new();
    for entry in args {
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("Invalid argument '{}'. Expected KEY=VALUE", entry))?;
        payload.insert(
            key.trim().to_string(),
            serde_json::Value::String(value.trim().to_string()),
        );
    }
    Ok(Some(serde_json::Value::Object(payload)))
}

async fn list_server_resources(name: String, user_id: String) -> anyhow::Result<()> {
    let server = load_server(&name).await?;
    let client = build_client(&server, &user_id).await?;
    let resources = client.list_resources().await?;

    if resources.is_empty() {
        println!("  No resources exposed by '{}'.", name);
        println!();
        return Ok(());
    }

    println!("  Resources from '{}':", name);
    for resource in resources {
        println!("    - {} ({})", resource.name, resource.uri);
        if let Some(description) = resource.description {
            println!("      {}", description);
        }
    }
    println!();
    Ok(())
}

async fn read_server_resource(name: String, uri: String, user_id: String) -> anyhow::Result<()> {
    let server = load_server(&name).await?;
    let client = build_client(&server, &user_id).await?;
    let result = client.read_resource(&uri).await?;

    println!("  Resource: {}", uri);
    for content in result.contents {
        match content {
            crate::tools::mcp::McpResourceContents::Text {
                uri,
                mime_type,
                text,
            } => {
                println!(
                    "  --- {} ({}) ---",
                    uri,
                    mime_type.unwrap_or_else(|| "text/plain".to_string())
                );
                println!("{}", text);
            }
            crate::tools::mcp::McpResourceContents::Blob {
                uri,
                mime_type,
                blob,
            } => {
                println!(
                    "  --- {} ({}) ---",
                    uri,
                    mime_type.unwrap_or_else(|| "application/octet-stream".to_string())
                );
                println!("{}", blob);
            }
        }
    }
    println!();
    Ok(())
}

async fn list_server_resource_templates(name: String, user_id: String) -> anyhow::Result<()> {
    let server = load_server(&name).await?;
    let client = build_client(&server, &user_id).await?;
    let templates = client.list_resource_templates().await?;

    if templates.is_empty() {
        println!("  No resource templates exposed by '{}'.", name);
        println!();
        return Ok(());
    }

    println!("  Resource templates from '{}':", name);
    for template in templates {
        println!("    - {} ({})", template.name, template.uri_template);
        if let Some(description) = template.description {
            println!("      {}", description);
        }
    }
    println!();
    Ok(())
}

async fn list_server_prompts(name: String, user_id: String) -> anyhow::Result<()> {
    let server = load_server(&name).await?;
    let client = build_client(&server, &user_id).await?;
    let prompts = client.list_prompts().await?;

    if prompts.is_empty() {
        println!("  No prompts exposed by '{}'.", name);
        println!();
        return Ok(());
    }

    println!("  Prompts from '{}':", name);
    for prompt in prompts {
        let args = if prompt.arguments.is_empty() {
            "no args".to_string()
        } else {
            prompt
                .arguments
                .iter()
                .map(|arg| format!("{}{}", arg.name, if arg.required { "*" } else { "" }))
                .collect::<Vec<_>>()
                .join(", ")
        };
        println!("    - {} ({})", prompt.name, args);
        if let Some(description) = prompt.description {
            println!("      {}", description);
        }
    }
    println!();
    Ok(())
}

async fn get_server_prompt(
    name: String,
    prompt: String,
    args: Vec<String>,
    user_id: String,
) -> anyhow::Result<()> {
    let server = load_server(&name).await?;
    let client = build_client(&server, &user_id).await?;
    let prompt_result = client.get_prompt(&prompt, parse_arg_pairs(args)?).await?;

    if let Some(description) = prompt_result.description {
        println!("  {}", description);
        println!();
    }

    for message in prompt_result.messages {
        println!("  [{}]", message.role);
        match message.content {
            PromptContent::Text(text) => println!("{}", text),
            PromptContent::Block(block) => println!("{}", serde_json::to_string_pretty(&block)?),
            PromptContent::Blocks(blocks) => {
                println!("{}", serde_json::to_string_pretty(&blocks)?)
            }
        }
        println!();
    }

    Ok(())
}

async fn list_server_roots(name: String) -> anyhow::Result<()> {
    let server = load_server(&name).await?;
    if server.roots_grants.is_empty() {
        println!("  No roots granted to '{}'.", name);
    } else {
        println!("  Roots granted to '{}':", name);
        for root in server.roots_grants {
            println!("    - {}", root);
        }
    }
    println!();
    Ok(())
}

async fn grant_server_root(name: String, root: String) -> anyhow::Result<()> {
    let db = connect_db().await;
    let mut servers = load_servers(db.as_deref()).await?;
    let server = servers
        .get_mut(&name)
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not found", name))?;
    if !server.roots_grants.iter().any(|existing| existing == &root) {
        server.roots_grants.push(root.clone());
    }
    save_servers(db.as_deref(), &servers).await?;
    println!("  Granted root '{}' to '{}'.", root, name);
    println!();
    Ok(())
}

async fn revoke_server_root(name: String, root: String) -> anyhow::Result<()> {
    let db = connect_db().await;
    let mut servers = load_servers(db.as_deref()).await?;
    let server = servers
        .get_mut(&name)
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not found", name))?;
    let before = server.roots_grants.len();
    server.roots_grants.retain(|existing| existing != &root);
    if before == server.roots_grants.len() {
        anyhow::bail!("Root '{}' was not granted to '{}'", root, name);
    }
    save_servers(db.as_deref(), &servers).await?;
    println!("  Revoked root '{}' from '{}'.", root, name);
    println!();
    Ok(())
}

async fn show_server_log_level(name: String) -> anyhow::Result<()> {
    let server = load_server(&name).await?;
    println!(
        "  Configured log level for '{}': {}",
        name,
        format!("{:?}", server.logging_level).to_ascii_lowercase()
    );
    println!();
    Ok(())
}

async fn set_server_log_level(name: String, level: String, user_id: String) -> anyhow::Result<()> {
    let parsed_level = match level.trim().to_ascii_lowercase().as_str() {
        "debug" => crate::tools::mcp::McpLoggingLevel::Debug,
        "info" => crate::tools::mcp::McpLoggingLevel::Info,
        "warn" | "warning" => crate::tools::mcp::McpLoggingLevel::Warning,
        "error" => crate::tools::mcp::McpLoggingLevel::Error,
        other => anyhow::bail!("Unsupported log level '{}'", other),
    };

    let db = connect_db().await;
    let mut servers = load_servers(db.as_deref()).await?;
    let server = servers
        .get_mut(&name)
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not found", name))?;
    server.logging_level = parsed_level;
    let updated_server = server.clone();
    save_servers(db.as_deref(), &servers).await?;

    let client = build_client(&updated_server, &user_id).await?;
    let _ = client.set_logging_level(parsed_level).await;

    println!(
        "  Set log level for '{}' to {}.",
        name,
        level.to_ascii_lowercase()
    );
    println!();
    Ok(())
}

/// Authenticate with an MCP server.
async fn auth_server(name: String, user_id: String) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    // Get server config
    let db = connect_db().await;
    let servers = load_servers(db.as_deref()).await?;
    let server = servers
        .get(&name)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not found", name))?;

    // Stdio servers don't use OAuth
    if server.is_stdio() {
        branding.print_banner("MCP", Some("Authenticate a model context server"));
        println!(
            "{}",
            branding.warn(format!(
                "Server '{}' uses stdio transport and does not require authentication.",
                name
            ))
        );
        println!();
        return Ok(());
    }

    // Initialize secrets store
    let secrets = get_secrets_store().await?;

    // Check if already authenticated
    if is_authenticated(&server, &secrets, &user_id).await {
        branding.print_banner("MCP", Some("Authenticate a model context server"));
        println!(
            "{}",
            branding.good(format!("Server '{}' is already authenticated.", name))
        );
        println!();
        print!("  Re-authenticate? [y/N]: ");
        std::io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            return Ok(());
        }
        println!();
    }

    branding.print_banner("MCP", Some(&format!("Authenticate {}", name)));

    // Perform OAuth flow (supports both pre-configured OAuth and DCR)
    match authorize_mcp_server(&server, &secrets, &user_id).await {
        Ok(_token) => {
            println!();
            println!(
                "  {}",
                branding.good(format!("Successfully authenticated with '{}'!", name))
            );
            println!();
            println!(
                "  {}",
                branding.body("You can now use tools from this server.")
            );
            println!();
        }
        Err(crate::tools::mcp::auth::AuthError::NotSupported) => {
            println!();
            println!(
                "  {}",
                branding.bad("Server does not support OAuth authentication.")
            );
            println!();
            println!(
                "  {}",
                branding.body("The server may require a different authentication method.")
            );
            println!(
                "  {}",
                branding.body("You may need to configure OAuth manually:")
            );
            println!();
            println!("    thinclaw mcp remove {}", name);
            println!(
                "    thinclaw mcp add {} {} --client-id YOUR_CLIENT_ID",
                name, server.url
            );
            println!();
        }
        Err(e) => {
            println!();
            println!(
                "  {}",
                branding.bad(format!("Authentication failed: {}", e))
            );
            println!();
            return Err(e.into());
        }
    }

    Ok(())
}

/// Test connection to an MCP server.
async fn test_server(name: String, user_id: String) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    // Get server config
    let db = connect_db().await;
    let servers = load_servers(db.as_deref()).await?;
    let server = servers
        .get(&name)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not found", name))?;

    println!(
        "  {}",
        branding.accent(format!("Testing connection to '{}'...", name))
    );

    // Create client — use from_config for automatic transport dispatch
    if server.is_stdio() {
        let config_store = Some(McpConfigStore::new(db.clone(), user_id.clone()));
        // Stdio: spawn the process directly
        let client = match McpClient::new_stdio_with_store(&server, config_store) {
            Ok(c) => c,
            Err(e) => {
                println!(
                    "  {}",
                    branding.bad(format!("Failed to spawn stdio server: {}", e))
                );
                println!();
                return Ok(());
            }
        };

        // Test connection
        match client.test_connection().await {
            Ok(()) => {
                println!("  {}", branding.good("Connection successful!"));
                println!();
                print_tools(&client).await;
            }
            Err(e) => {
                println!("  {}", branding.bad(format!("Connection failed: {}", e)));
            }
        }
    } else {
        // HTTP: existing logic with auth handling
        let session_manager = Arc::new(McpSessionManager::new());
        let secrets = get_secrets_store().await?;
        let has_tokens = is_authenticated(&server, &secrets, &user_id).await;
        let config_store = Some(McpConfigStore::new(db.clone(), user_id.clone()));

        let client = if has_tokens {
            McpClient::new_authenticated_with_store(
                server.clone(),
                session_manager,
                secrets,
                user_id,
                config_store,
            )
        } else if server.requires_auth() {
            println!();
            println!(
                "  {}",
                branding.bad(format!(
                    "Not authenticated. Run `thinclaw mcp auth {}` first.",
                    name
                ))
            );
            println!();
            return Ok(());
        } else {
            McpClient::new_configured_with_store(server.clone(), config_store)
        };

        // Test connection
        match client.test_connection().await {
            Ok(()) => {
                println!("  {}", branding.good("Connection successful!"));
                println!();
                print_tools(&client).await;
            }
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("401") || err_str.contains("requires authentication") {
                    if has_tokens {
                        println!(
                            "  {}",
                            branding.bad(
                                "Authentication failed (token may be expired). Try re-authenticating:"
                            )
                        );
                        println!("    thinclaw mcp auth {}", name);
                    } else {
                        println!("  {}", branding.bad("Server requires authentication."));
                        println!();
                        println!(
                            "  {}",
                            branding.muted(format!(
                                "Run `thinclaw mcp auth {}` to authenticate.",
                                name
                            ))
                        );
                    }
                } else {
                    println!("  {}", branding.bad(format!("Connection failed: {}", e)));
                }
            }
        }
    }

    println!();

    Ok(())
}

/// Print the list of tools from an MCP server (shared helper).
async fn print_tools(client: &McpClient) {
    match client.list_tools().await {
        Ok(tools) => {
            println!("  Available tools ({}):", tools.len());
            for tool in tools {
                let approval = if tool.requires_approval() {
                    " [approval required]"
                } else {
                    ""
                };
                println!("    • {}{}", tool.name, approval);
                if !tool.description.is_empty() {
                    let desc = if tool.description.chars().count() > 60 {
                        let byte_offset = tool
                            .description
                            .char_indices()
                            .nth(57)
                            .map(|(i, _)| i)
                            .unwrap_or(tool.description.len());
                        format!("{}...", &tool.description[..byte_offset])
                    } else {
                        tool.description.clone()
                    };
                    println!("      {}", desc);
                }
            }
        }
        Err(e) => {
            println!("  ✗ Failed to list tools: {}", e);
        }
    }
}

/// Toggle server enabled/disabled state.
async fn toggle_server(name: String, enable: bool, disable: bool) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let db = connect_db().await;
    let mut servers = load_servers(db.as_deref()).await?;

    let server = servers
        .get_mut(&name)
        .ok_or_else(|| anyhow::anyhow!("Server '{}' not found", name))?;

    let new_state = if enable {
        true
    } else if disable {
        false
    } else {
        !server.enabled // Toggle if neither specified
    };

    server.enabled = new_state;
    save_servers(db.as_deref(), &servers).await?;

    let status = if new_state { "enabled" } else { "disabled" };
    println!();
    println!(
        "  {}",
        branding.good(format!("Server '{}' is now {}.", name, status))
    );
    println!();

    Ok(())
}

const DEFAULT_USER_ID: &str = "default";

/// Try to connect to the database (backend-agnostic).
async fn connect_db() -> Option<Arc<dyn Database>> {
    let config = Config::from_env().await.ok()?;
    crate::db::connect_from_config(&config.database).await.ok()
}

/// Load MCP servers (DB if available, else disk).
async fn load_servers(db: Option<&dyn Database>) -> Result<McpServersFile, config::ConfigError> {
    if let Some(db) = db {
        config::load_mcp_servers_from_db(db, DEFAULT_USER_ID).await
    } else {
        config::load_mcp_servers().await
    }
}

/// Save MCP servers (DB if available, else disk).
async fn save_servers(
    db: Option<&dyn Database>,
    servers: &McpServersFile,
) -> Result<(), config::ConfigError> {
    if let Some(db) = db {
        config::save_mcp_servers_to_db(db, DEFAULT_USER_ID, servers).await
    } else {
        config::save_mcp_servers(servers).await
    }
}

/// Initialize and return the secrets store.
async fn get_secrets_store() -> anyhow::Result<Arc<dyn SecretsStore + Send + Sync>> {
    let config = Config::from_env().await?;

    let master_key = config.secrets.master_key().ok_or_else(|| {
        anyhow::anyhow!(
            "SECRETS_MASTER_KEY not set. Run 'thinclaw onboard' first or set it in .env"
        )
    })?;

    let crypto = SecretsCrypto::new(master_key.clone())?;

    #[cfg(feature = "postgres")]
    {
        let store = crate::history::Store::new(&config.database).await?;
        store.run_migrations().await?;
        Ok(Arc::new(PostgresSecretsStore::new(
            store.pool(),
            Arc::new(crypto),
        )))
    }

    #[cfg(all(feature = "libsql", not(feature = "postgres")))]
    {
        use crate::db::Database as _;
        use crate::db::libsql::LibSqlBackend;
        use secrecy::ExposeSecret as _;

        let default_path = crate::config::default_libsql_path();
        let db_path = config
            .database
            .libsql_path
            .as_deref()
            .unwrap_or(&default_path);

        let backend = if let Some(ref url) = config.database.libsql_url {
            let token = config.database.libsql_auth_token.as_ref().ok_or_else(|| {
                anyhow::anyhow!("LIBSQL_AUTH_TOKEN is required when LIBSQL_URL is set")
            })?;
            LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret())
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?
        } else {
            LibSqlBackend::new_local(db_path)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?
        };
        backend
            .run_migrations()
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        Ok(Arc::new(crate::secrets::LibSqlSecretsStore::new(
            backend.shared_db(),
            Arc::new(crypto),
        )))
    }

    #[cfg(not(any(feature = "postgres", feature = "libsql")))]
    {
        let _ = crypto;
        anyhow::bail!(
            "No database backend available for secrets. Enable 'postgres' or 'libsql' feature."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_command_parsing() {
        // Just verify the command structure is valid
        use clap::CommandFactory;

        // Create a dummy parent command to test subcommand parsing
        #[derive(clap::Parser)]
        struct TestCli {
            #[command(subcommand)]
            cmd: McpCommand,
        }

        TestCli::command().debug_assert();
    }
}
