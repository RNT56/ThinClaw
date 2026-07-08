//! Minimal streaming chat example.
//!
//! Run against a live gateway:
//! ```sh
//! export THINCLAW_GATEWAY_URL=http://127.0.0.1:8080
//! export THINCLAW_GATEWAY_TOKEN=your-gateway-token
//! cargo run -p thinclaw-client --example chat_loop -- "hello there"
//! ```

use futures::StreamExt;
use thinclaw_client::{Client, SseEvent};

#[tokio::main(flavor = "current_thread")]
async fn main() -> thinclaw_client::Result<()> {
    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Say hello and tell me one fact.".to_string());

    let client = Client::from_env()?;

    // Open the event stream first so nothing is missed, then send.
    let mut events = Box::pin(client.events().await?);
    let accepted = client.send_message(&prompt, None).await?;
    println!("(sent, message_id={})", accepted.message_id);

    while let Some(event) = events.next().await {
        match event? {
            SseEvent::ToolStarted { name, .. } => println!("→ tool: {name}"),
            SseEvent::ToolResult { name, preview, .. } => {
                println!("  {name}: {}", preview.lines().next().unwrap_or(""));
            }
            SseEvent::StreamChunk { content, .. } => print!("{content}"),
            SseEvent::Response { content, .. } => {
                println!("\n{content}");
                break;
            }
            SseEvent::Error { message, .. } => {
                eprintln!("error: {message}");
                break;
            }
            _ => {}
        }
    }

    Ok(())
}
