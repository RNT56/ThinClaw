use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock, RwLock as StdRwLock};

use chrono::{DateTime, Utc};
use rand::Rng;
use secrecy::ExposeSecret;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::{Mutex, OwnedSemaphorePermit, RwLock, Semaphore};
use uuid::Uuid;

use crate::agent::routine::{
    NotifyConfig, Routine, RoutineAction, RoutineGuardrails, Trigger, canonicalize_schedule_expr,
    next_schedule_fire_for_user,
};
use crate::config::{DatabaseConfig, DesktopAutonomyConfig};
use crate::db::Database;
use crate::tools::ToolProfile;

mod bootstrap;
mod bridge;
mod canaries;
mod dedicated_user;
mod fixtures;
mod global;
mod helpers;
mod launchers;
mod manager;
mod manager_core;
mod persistence;
mod prerequisites;
mod rollout;
mod rollout_helpers;
mod seeded_content;
mod sessions;
mod source_sync;
mod types;

pub use global::*;
pub use helpers::run_shadow_canary_entrypoint;
pub use manager::DesktopAutonomyManager;
pub use sessions::*;
pub use types::*;

pub(super) use helpers::*;
pub(super) use types::{
    ActionReadinessSnapshot, BuildManifest, DEFAULT_SESSION_ID, DedicatedUserBootstrap,
    DesktopBootstrapPrerequisites, DesktopBridgeBackend, DesktopBridgeSpec, GLOBAL_MANAGER,
    RolloutState, RuntimeState,
};
#[cfg(test)]
pub(super) use types::{LINUX_SIDECAR_FILENAME, MACOS_SIDECAR_FILENAME, WINDOWS_SIDECAR_FILENAME};

#[cfg(test)]
mod tests;
