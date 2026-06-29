//! Core channel message and draft types.

pub mod channel;

pub use channel::{
    Channel, ConfigField, ConfigOption, ConfigSchema, DraftReplyState, IncomingMessage,
    MessageStream, OutgoingResponse, StatusUpdate, StreamMode,
};
