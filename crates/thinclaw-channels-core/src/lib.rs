//! Core channel message and draft types.

pub mod channel;

pub use channel::{
    Channel, DraftReplyState, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
    StreamMode,
};
