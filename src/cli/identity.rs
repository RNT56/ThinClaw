//! Actor identity registry CLI commands.
//!
//! Manage household actors and their linked delivery endpoints.

use clap::Subcommand;

use crate::db::{Database, IdentityStore};
use crate::identity::{NewActorEndpointRecord, NewActorRecord};

const DEFAULT_PRINCIPAL_ID: &str = "default";

#[derive(Subcommand, Debug, Clone)]
pub enum IdentityCommand {
    /// List actors for a principal
    List {
        /// Principal ID (household) to inspect
        #[arg(long, default_value = DEFAULT_PRINCIPAL_ID)]
        principal: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show one actor and its linked endpoints
    Show {
        /// Actor ID (UUID)
        actor_id: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Create a new actor
    Create {
        /// Display name for the new actor
        display_name: String,

        /// Principal ID (household) that owns this actor
        #[arg(long, default_value = DEFAULT_PRINCIPAL_ID)]
        principal: String,
    },

    /// Link a channel endpoint to an actor
    Link {
        /// Actor ID (UUID)
        actor_id: String,

        /// Channel name (telegram, signal, imessage, whatsapp, ...)
        channel: String,

        /// Stable external user ID for that channel
        external_user_id: String,
    },

    /// Unlink a channel endpoint from whichever actor owns it
    Unlink {
        /// Channel name (telegram, signal, imessage, whatsapp, ...)
        channel: String,

        /// Stable external user ID for that channel
        external_user_id: String,
    },

    /// Rename an actor
    Rename {
        /// Actor ID (UUID)
        actor_id: String,

        /// New display name
        display_name: String,
    },

    /// Set the preferred delivery channel for an actor
    SetPreferredChannel {
        /// Actor ID (UUID)
        actor_id: String,

        /// Channel name
        channel: String,

        /// Stable external user ID for that channel
        external_user_id: String,
    },
}

pub async fn run_identity_command(cmd: IdentityCommand) -> anyhow::Result<()> {
    let db = connect_db().await?;

    match cmd {
        IdentityCommand::List { principal, json } => list_actors(&*db, &principal, json).await,
        IdentityCommand::Show { actor_id, json } => show_actor(&*db, &actor_id, json).await,
        IdentityCommand::Create {
            display_name,
            principal,
        } => create_actor(&*db, &principal, &display_name).await,
        IdentityCommand::Link {
            actor_id,
            channel,
            external_user_id,
        } => link_actor_endpoint(&*db, &actor_id, &channel, &external_user_id).await,
        IdentityCommand::Unlink {
            channel,
            external_user_id,
        } => unlink_actor_endpoint(&*db, &channel, &external_user_id).await,
        IdentityCommand::Rename {
            actor_id,
            display_name,
        } => rename_actor(&*db, &actor_id, &display_name).await,
        IdentityCommand::SetPreferredChannel {
            actor_id,
            channel,
            external_user_id,
        } => set_preferred_channel(&*db, &actor_id, &channel, &external_user_id).await,
    }
}

async fn connect_db() -> anyhow::Result<std::sync::Arc<dyn crate::db::Database>> {
    let config = crate::config::Config::from_env()
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    crate::db::connect_from_config(&config.database)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))
}

async fn list_actors(db: &dyn Database, principal: &str, json: bool) -> anyhow::Result<()> {
    let actors = IdentityStore::list_actors(db, principal)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to list actors: {}", e))?;

    if json {
        println!("{}", serde_json::to_string_pretty(&actors)?);
        return Ok(());
    }

    if actors.is_empty() {
        println!("No actors found for principal '{}'.", principal);
        return Ok(());
    }

    println!(
        "{:<36}  {:<24}  {:<10}  PREFERRED",
        "ACTOR ID", "NAME", "STATUS"
    );
    println!("{}", "-".repeat(100));

    for actor in actors {
        let preferred = actor
            .preferred_delivery_endpoint
            .as_ref()
            .map(|endpoint| format!("{}:{}", endpoint.channel, endpoint.external_user_id))
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{:<36}  {:<24}  {:<10}  {}",
            actor.actor_id,
            truncate(&actor.display_name, 24),
            actor.status.as_str(),
            preferred
        );
    }

    Ok(())
}

async fn show_actor(db: &dyn Database, actor_id: &str, json: bool) -> anyhow::Result<()> {
    let actor = IdentityStore::get_actor(db, actor_id)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to load actor: {}", e))?
        .ok_or_else(|| anyhow::anyhow!("Actor not found: {}", actor_id))?;
    let endpoints = IdentityStore::list_actor_endpoints(db, actor_id)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to list actor endpoints: {}", e))?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "actor": actor,
                "endpoints": endpoints,
            }))?
        );
        return Ok(());
    }

    println!("Actor ID: {}", actor.actor_id);
    println!("Principal: {}", actor.principal_id);
    println!("Name: {}", actor.display_name);
    println!("Status: {}", actor.status.as_str());
    println!(
        "Preferred endpoint: {}",
        actor
            .preferred_delivery_endpoint
            .as_ref()
            .map(|endpoint| format!("{}:{}", endpoint.channel, endpoint.external_user_id))
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "Last active direct endpoint: {}",
        actor
            .last_active_direct_endpoint
            .as_ref()
            .map(|endpoint| format!("{}:{}", endpoint.channel, endpoint.external_user_id))
            .unwrap_or_else(|| "-".to_string())
    );

    if endpoints.is_empty() {
        println!("Endpoints: none");
    } else {
        println!("Endpoints:");
        for endpoint in endpoints {
            println!(
                "  {}:{} ({})",
                endpoint.endpoint.channel,
                endpoint.endpoint.external_user_id,
                endpoint.approval_status.as_str()
            );
        }
    }

    Ok(())
}

async fn create_actor(
    db: &dyn Database,
    principal: &str,
    display_name: &str,
) -> anyhow::Result<()> {
    let actor = db
        .create_actor(&NewActorRecord {
            principal_id: principal.to_string(),
            display_name: display_name.to_string(),
            status: Default::default(),
            preferred_delivery_endpoint: None,
            last_active_direct_endpoint: None,
        })
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create actor: {}", e))?;

    println!(
        "Created actor '{}' ({})",
        actor.display_name, actor.actor_id
    );
    Ok(())
}

async fn link_actor_endpoint(
    db: &dyn Database,
    actor_id: &str,
    channel: &str,
    external_user_id: &str,
) -> anyhow::Result<()> {
    let actor = IdentityStore::get_actor(db, actor_id)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to load actor: {}", e))?
        .ok_or_else(|| anyhow::anyhow!("Actor not found: {}", actor_id))?;

    db.upsert_actor_endpoint(&NewActorEndpointRecord {
        endpoint: crate::identity::ActorEndpointRef::new(channel, external_user_id),
        actor_id: actor.actor_id,
        metadata: serde_json::json!({}),
        approval_status: crate::identity::EndpointApprovalStatus::Approved,
    })
    .await
    .map_err(|e| anyhow::anyhow!("Failed to link endpoint: {}", e))?;

    println!(
        "Linked {}:{} to actor '{}' ({})",
        channel, external_user_id, actor.display_name, actor.actor_id
    );
    Ok(())
}

async fn unlink_actor_endpoint(
    db: &dyn Database,
    channel: &str,
    external_user_id: &str,
) -> anyhow::Result<()> {
    let removed = db
        .unlink_actor_endpoint(channel, external_user_id)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to unlink endpoint: {}", e))?;

    if removed {
        println!("Unlinked {}:{}", channel, external_user_id);
    } else {
        println!(
            "No linked endpoint found for {}:{}",
            channel, external_user_id
        );
    }
    Ok(())
}

async fn rename_actor(db: &dyn Database, actor_id: &str, display_name: &str) -> anyhow::Result<()> {
    IdentityStore::rename_actor(db, actor_id, display_name)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to rename actor: {}", e))?;
    println!("Renamed actor {} to '{}'", actor_id, display_name);
    Ok(())
}

async fn set_preferred_channel(
    db: &dyn Database,
    actor_id: &str,
    channel: &str,
    external_user_id: &str,
) -> anyhow::Result<()> {
    db.set_actor_preferred_endpoint(actor_id, channel, external_user_id)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to set preferred endpoint: {}", e))?;
    println!(
        "Set preferred endpoint for {} to {}:{}",
        actor_id, channel, external_user_id
    );
    Ok(())
}

fn truncate(value: &str, max_len: usize) -> String {
    if value.chars().count() <= max_len {
        return value.to_string();
    }
    let end = value
        .char_indices()
        .map(|(idx, _)| idx)
        .take_while(|&idx| idx < max_len.saturating_sub(1))
        .last()
        .unwrap_or(0);
    format!("{}…", &value[..end])
}
