//! Interactive setup wizard for ThinClaw.
//!
//! Provides a guided setup experience for:
//! 1. Database connection
//! 2. Security (secrets master key)
//! 3. Agent identity (name)
//! 4. Inference provider selection
//! 5. Model selection
//! 6. Embeddings
//! 7. Channel configuration (HTTP, Signal, Discord, Slack, Nostr, Gmail, iMessage, WASM)
//! 8. Extensions (tool installation from registry)
//! 9. Docker sandbox
//! 10. Routines (scheduled tasks)
//! 11. Skills
//! 12. Claude Code sandbox
//! 13. Smart routing (cheap model for lightweight tasks)
//! 14. Web UI (theme, accent color, branding)
//! 15. Observability (event recording backend)
//! 16. Heartbeat (background tasks)
//!
//! # Example
//!
//! ```ignore
//! use thinclaw::setup::SetupWizard;
//!
//! let mut wizard = SetupWizard::new();
//! wizard.run().await?;
//! ```

mod channels;
mod prompts;
#[cfg(any(feature = "postgres", feature = "libsql"))]
mod wizard;

pub use channels::{
    ChannelSetupError, SecretsContext, setup_http, setup_telegram, setup_tunnel,
    validate_telegram_token,
};
pub use prompts::{
    confirm, input, optional_input, print_error, print_header, print_info, print_step,
    print_success, secret_input, select_many, select_one,
};
#[cfg(any(feature = "postgres", feature = "libsql"))]
pub use wizard::{SetupConfig, SetupWizard};
