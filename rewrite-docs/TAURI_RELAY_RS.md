# Local Tauri App as a Headless Relay

You are absolutely correct. One of the core design goals of ThinClaw is that the user can run the Tauri application on their MacBook, and it acts **simultaneously** as a rich desktop chat interface _and_ as the background engine powering their Discord bot or Telegram agent.

This mirrors OpenClaw's design, where the process runs a "Gateway" (handling background jobs) alongside the interaction layer.

To achieve this in Rust/Tauri, you need to understand how the **Tauri App Lifecycle** intersects with the **Channel Manager** we discussed earlier.

## 1. The "Always-On" Background Service

A standard Tauri application closes its Rust backend when you click the red "X" to close the window.

If ThinClaw is functioning as the brain for a Telegram bot, **closing the window should not stop the bot**.

You must configure Tauri to keep the Rust backend running in the system tray even when all windows are closed:

```rust
// In src-tauri/src/main.rs — Tauri v2 System Tray API
use tauri::{
    Manager,
    tray::{TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState},
    menu::{Menu, MenuItem},
};

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let quit   = MenuItem::with_id(app, "quit",   "Quit Server",   true, None::<&str>)?;
            let open   = MenuItem::with_id(app, "open",   "Open ThinClaw", true, None::<&str>)?;
            let menu   = Menu::with_items(app, &[&open, &quit])?;

            TrayIconBuilder::new()
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => {
                        std::process::exit(0);
                    }
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        // Intercept window close → hide instead of exit
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                window.hide().unwrap();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

## 2. Bootstrapping Channel Plugins

When the Rust application starts (either on computer boot or manually), it must automatically initialize connections to configured messaging platforms.

In OpenClaw, this is handled by `src/gateway/server-channels.ts`. In our Rust port, you will define a `ChannelManager`:

```rust
// src-tauri/src/channels/manager.rs

pub struct ChannelManager {
    // Stores active connections to Discord, Telegram, etc.
    active_connections: Mutex<HashMap<String, ChannelConnection>>,
}

impl ChannelManager {
    pub async fn start_all_channels(&self, config: &Config) {
        // 1. Check if Telegram is configured
        if let Some(tg_token) = &config.telegram.token {
            let tg = TelegramAdapter::new(tg_token);
            tg.connect().await; // Starts background loop listening for messages
            self.active_connections.lock().await.insert("telegram".to_string(), tg);
        }

        // 2. Check if Discord is configured
        if let Some(discord_token) = &config.discord.token {
            // ... start discord ...
        }
    }
}
```

This ensures that the moment the user launches ThinClaw on their MacBook, their Telegram bot instantly comes online and begins routing messages through their local LLM (or Secure Cloud key).

## 3. Remote Control via Headless Channels

Once the background `ChannelManager` is running, a user sitting at their phone (away from their MacBook) can text their Telegram bot.

1. **Incoming Message:** The Telegram API sends the message to your MacBook's background Rust process.
2. **Slash Command Check:** The Rust Gatekeeper checks if the message is a slash command (e.g., `/status`). See `CHAT_COMMANDS_RS.md`.
3. **Trigger Mechanics Check:** If it's a group chat, the Gatekeeper ensures the bot was mentioned. See `TRIGGER_MECHANICS_RS.md`.
4. **Execution:** The message is sent to the LLM.
   - _Crucially: If the user commands the agent to read an email or write a Python script, the agent executes that skill locally on the Macbook!_
5. **Response:** The LLM's response is formatted and pushed back out via the Telegram API.

Because the Tauri UI acts as just another "Channel" to the Orchestrator, the experience of chatting in the desktop app or chatting remotely via Telegram relies on the exact same underlying logic.

## 4. Headless Linux Deployment

If you decide to deploy ThinClaw to an AWS EC2 instance instead of a MacBook, you do not want the heavy Tauri WebView binaries.

You can configure Rust Cargo features so that passing `--features headless` strips all the Tauri GUI code and simply compiles the `Orchestrator` and `ChannelManager` into a tiny, fast, pure-Rust binary that can run infinitely on a headless Ubuntu server.

```toml
# Cargo.toml
[features]
default = ["desktop-ui"]
desktop-ui = ["tauri"]
headless = []
```

## 5. Daemon / Background Service Installation

OpenClaw's `src/daemon/` subsystem (40 files) handles installing the Orchestrator as an OS-managed background service. For headless deployments, this is how the agent auto-starts on boot and auto-restarts on crash.

### macOS: launchd

For macOS (both desktop and headless Mac Mini deployments), generate a `launchd` plist:

```rust
pub fn install_launchd_service(binary_path: &Path, config_path: &Path) -> Result<()> {
    let plist = format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.thinclaw.orchestrator</string>
    <key>ProgramArguments</key>
    <array>
        <string>{binary}</string>
        <string>--headless</string>
        <string>--config</string>
        <string>{config}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/thinclaw.stdout.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/thinclaw.stderr.log</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>HOME</key>
        <string>{home}</string>
    </dict>
</dict>
</plist>"#,
        binary = binary_path.display(),
        config = config_path.display(),
        home = dirs::home_dir().unwrap().display(),
    );

    let plist_path = dirs::home_dir().unwrap()
        .join("Library/LaunchAgents/com.thinclaw.orchestrator.plist");
    std::fs::write(&plist_path, plist)?;

    // Load immediately
    std::process::Command::new("launchctl")
        .args(["load", "-w", &plist_path.to_string_lossy()])
        .status()?;

    Ok(())
}
```

### Linux: systemd

For Linux VPS/server deployments, generate a systemd user unit:

```ini
# ~/.config/systemd/user/thinclaw.service
[Unit]
Description=ThinClaw AI Orchestrator
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/thinclaw-server --headless --config %h/.config/thinclaw/config.toml
Restart=on-failure
RestartSec=5
Environment=HOME=%h
WorkingDirectory=%h

[Install]
WantedBy=default.target
```

Install and enable:
```bash
systemctl --user daemon-reload
systemctl --user enable --now thinclaw.service
loginctl enable-linger $USER   # Keep running after SSH disconnect
```

### Service Audit

The daemon module also includes a **service auditor** (`service-audit.ts`, 12KB) that checks:
- Is the service file correctly configured?
- Is the binary path valid and up-to-date?
- Are environment variables correctly set?
- Is `loginctl linger` enabled (Linux)?
- Is the service actually running?

Accessible via `/doctor daemon` command or the Tauri Settings UI.
