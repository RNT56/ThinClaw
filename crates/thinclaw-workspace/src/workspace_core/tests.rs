//! Unit tests for the workspace core.
//!
//! Covers path normalization, AGENTS.md instruction extraction, and personality
//! pack seed fallback. The `#[cfg(any())]` tests are retained verbatim from the
//! pre-decomposition module (they exercise DB-backed seeding/redaction flows but
//! are gated off) so the migration history stays faithful.

use super::prompt_text::extract_essential_instructions;
use super::seed::personality_pack_content;
use super::{normalize_directory, normalize_path};

#[cfg(any())]
use super::Workspace;
#[cfg(any())]
use super::soul::read_home_soul;
#[cfg(any())]
use crate::document::paths;
#[cfg(any())]
use thinclaw_identity::{ConversationKind, ResolvedIdentity};
#[cfg(any())]
use uuid::Uuid;

/// Tests that manipulate the process-global `THINCLAW_HOME` environment
/// variable must hold this mutex to prevent races under parallel `cargo test`.
#[cfg(any())]
static THINCLAW_HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(any())]
fn test_identity(actor_id: &str) -> ResolvedIdentity {
    ResolvedIdentity {
        principal_id: actor_id.to_string(),
        actor_id: actor_id.to_string(),
        conversation_scope_id: Uuid::new_v4(),
        conversation_kind: ConversationKind::Direct,
        raw_sender_id: actor_id.to_string(),
        stable_external_conversation_key: format!("principal:{actor_id}"),
    }
}

#[test]
fn test_normalize_path() {
    assert_eq!(normalize_path("foo/bar"), "foo/bar");
    assert_eq!(normalize_path("/foo/bar/"), "foo/bar");
    assert_eq!(normalize_path("foo//bar"), "foo/bar");
    assert_eq!(normalize_path("  /foo/  "), "foo");
    assert_eq!(normalize_path("README.md"), "README.md");
}

#[test]
fn test_normalize_directory() {
    assert_eq!(normalize_directory("foo/bar/"), "foo/bar");
    assert_eq!(normalize_directory("foo/bar"), "foo/bar");
    assert_eq!(normalize_directory("/"), "");
    assert_eq!(normalize_directory(""), "");
}

#[test]
fn extract_essential_instructions_includes_expanded_operational_sections() {
    let agents = r#"
## Session Startup
Read SOUL.md first.

## External vs Internal
Ask first before external actions.

## Tools
Use SKILL.md for tool guidance.

## 💓 Heartbeats - Be Proactive!
Do proactive maintenance.

## Make It Yours
Experiment freely.
"#;

    let essential = extract_essential_instructions(agents);
    assert!(essential.contains("## Session Startup"));
    assert!(essential.contains("## External vs Internal"));
    assert!(essential.contains("## Tools"));
    assert!(essential.contains("## 💓 Heartbeats - Be Proactive!"));
    assert!(!essential.contains("## Make It Yours"));
    assert!(essential.contains("Full instructions: `memory_read AGENTS.md`"));
}

#[test]
fn extract_essential_instructions_keeps_nested_policy_under_red_lines() {
    let agents = r#"
## Red Lines
- Don't exfiltrate private data.

### Protected Repo Boundary Policy (ThinClaw self-improvement + upgrade work)
- Treat ThinClaw-main as a protected codebase by default.
- Full autonomy does not override boundary rules.

## Group Chats
Know when to stay silent.
"#;

    let essential = extract_essential_instructions(agents);
    assert!(essential.contains("## Red Lines"));
    assert!(essential.contains("### Protected Repo Boundary Policy"));
    assert!(essential.contains("Treat ThinClaw-main as a protected codebase by default."));
    assert!(essential.contains("## Group Chats"));
}

#[test]
fn persona_seed_content_falls_back_to_default() {
    assert_eq!(
        personality_pack_content("unknown-seed"),
        personality_pack_content("balanced")
    );
    let mentor = personality_pack_content("MENTOR");
    assert!(mentor.contains("# Mentor Personality Pack"));
    assert!(mentor.contains("## Vibe"));
    assert!(mentor.contains("## Default Behaviors"));
}

#[cfg(any())]
#[tokio::test]
async fn seed_if_empty_migrates_main_workspace_legacy_soul_into_home() {
    let _lock = THINCLAW_HOME_LOCK.lock().unwrap();
    let (db, _temp_dir) = crate::testing::test_db().await;
    let temp_home = tempfile::tempdir().expect("temp home");
    let previous_home = std::env::var_os("THINCLAW_HOME");
    unsafe {
        std::env::set_var("THINCLAW_HOME", temp_home.path());
    }

    let workspace = Workspace::new_with_db("household-legacy", db);
    workspace
        .write(paths::SOUL, "# SOUL.md - Who You Are\n\nlegacy soul")
        .await
        .unwrap();

    workspace
        .seed_if_empty(Some("thinclaw"), Some("balanced"))
        .await
        .unwrap();

    let home = read_home_soul().unwrap();
    assert!(home.contains("legacy soul"));
    assert!(workspace.read(paths::SOUL).await.is_err());
    let archived = workspace.read(paths::SOUL_LEGACY).await.unwrap();
    assert!(archived.content.contains("legacy soul"));

    if let Some(previous_home) = previous_home {
        unsafe {
            std::env::set_var("THINCLAW_HOME", previous_home);
        }
    } else {
        unsafe {
            std::env::remove_var("THINCLAW_HOME");
        }
    }
}

#[cfg(any())]
#[tokio::test]
async fn seed_if_empty_migrates_agent_workspace_legacy_soul_into_local_overlay() {
    let _lock = THINCLAW_HOME_LOCK.lock().unwrap();
    let (db, _temp_dir) = crate::testing::test_db().await;
    let temp_home = tempfile::tempdir().expect("temp home");
    let previous_home = std::env::var_os("THINCLAW_HOME");
    unsafe {
        std::env::set_var("THINCLAW_HOME", temp_home.path());
    }
    crate::identity::soul_store::write_home_soul(
        &crate::identity::soul::compose_seeded_soul("balanced").unwrap(),
    )
    .unwrap();

    let workspace = Workspace::new_with_db("household-agent", db).with_agent(Uuid::new_v4());
    workspace
        .write(paths::SOUL, "# SOUL.md - Who You Are\n\nagent legacy soul")
        .await
        .unwrap();

    workspace
        .seed_if_empty(Some("thinclaw"), Some("balanced"))
        .await
        .unwrap();

    assert!(workspace.read(paths::SOUL).await.is_err());
    let local = workspace.read(paths::SOUL_LOCAL).await.unwrap();
    assert!(local.content.contains("agent legacy soul"));
    let archived = workspace.read(paths::SOUL_LEGACY).await.unwrap();
    assert!(archived.content.contains("agent legacy soul"));

    if let Some(previous_home) = previous_home {
        unsafe {
            std::env::set_var("THINCLAW_HOME", previous_home);
        }
    } else {
        unsafe {
            std::env::remove_var("THINCLAW_HOME");
        }
    }
}

#[cfg(any())]
#[tokio::test]
async fn system_prompt_redacts_actor_private_paths_for_non_discord_channels() {
    let (db, _temp_dir) = crate::testing::test_db().await;
    let workspace = Workspace::new_with_db("household-1", db);
    let actor_id = "15551234567";

    workspace
        .write(&paths::actor_memory(actor_id), "Private note")
        .await
        .unwrap();
    workspace
        .write(&paths::actor_user(actor_id), "- **Name:** Alex")
        .await
        .unwrap();
    workspace
        .write(&paths::actor_profile(actor_id), "{\"confidence\":0.0}")
        .await
        .unwrap();

    let prompt = workspace
        .system_prompt_for_identity(Some(&test_identity(actor_id)), "signal", true)
        .await
        .unwrap();

    assert!(!prompt.contains(actor_id));
    assert!(prompt.contains("Actor MEMORY.md (user_"));
    assert!(prompt.contains("use `memory_read` target: `memory`"));
    assert!(prompt.contains("Actor USER.md (user_"));
    assert!(prompt.contains("Actor profile.json (user_"));
}

#[cfg(any())]
#[tokio::test]
async fn system_prompt_preserves_raw_actor_paths_for_discord() {
    let (db, _temp_dir) = crate::testing::test_db().await;
    let workspace = Workspace::new_with_db("household-1", db);
    let actor_id = "15551234567";

    workspace
        .write(&paths::actor_memory(actor_id), "Private note")
        .await
        .unwrap();

    let prompt = workspace
        .system_prompt_for_identity(Some(&test_identity(actor_id)), "discord", true)
        .await
        .unwrap();

    assert!(prompt.contains("actors/15551234567/MEMORY.md"));
}
