
⸻

1 Install the OpenClaw Gateway & macOS Companion
	1.	Install the macOS app and CLI – Download the OpenClaw.app DMG from openclaw.ai and drag it to /Applications.  When you launch it, it runs as a menu‑bar companion that:
	•	Owns macOS “TCC” permissions (notifications, accessibility, screen recording, mic, speech recognition, AppleScript, etc.) so the agent can control the UI ￼.
	•	Starts or connects to a local gateway via launchd (default local mode) or attaches to a remote gateway via SSH/Tailscale (remote mode) ￼.
	•	Exposes macOS‑specific tools to the agent, such as canvas, camera, screen recording and system commands (system.run) ￼.
	•	Optionally runs PeekabooBridge for UI automation.
During the first launch the app will show TCC prompts; grant accessibility and screen‑recording permissions so the agent can control the Mac.  The app installs the openclaw CLI via npm when requested ￼.
	2.	Install via CLI (alternative) – If you prefer a headless installation, run the official one‑liner:

curl -fsSL https://openclaw.ai/install.sh | bash

After installation, run openclaw onboard --install-daemon to run the onboarding wizard and set up the launchd daemon (for the gateway) ￼.

	3.	Onboarding – In the onboarding wizard (GUI or CLI), choose Quickstart.  You’ll be asked to provide:
	•	Model provider credentials (OpenAI, Anthropic, or both) – supply API keys or subscription tokens if you plan to use cloud models.
	•	Preferred default model – choose a remote model (e.g., anthropic/claude-opus-4-5) which will be used when local inference fails.
	•	Messaging channel – choose Telegram (instructions below), Slack, Discord, etc., and follow the pairing prompts.
	•	Skills – select built‑in skills such as email, calendar, or file management.  You can enable/disable skills later via the Control UI or CLI.
Once the onboarding completes, the gateway will be running as a background service on port 18789, and the agent will be ready to chat via your chosen channel.

⸻

2 Configure local inference with Ministral‑3‑8b (Ollama or LM Studio)

OpenClaw can use local models via any service that exposes an OpenAI‑compatible /v1 endpoint.  Two popular choices are Ollama and LM Studio.

2.1 Using Ollama (simplest)
	1.	Install Ollama from ollama.ai￼ and pull a model.  The docs explain that OpenClaw auto‑discovers tool‑capable models when an OLLAMA_API_KEY environment variable is set ￼.  Run:

brew install ollama
ollama pull mistral-instruct:8b-q4      # or pull the 14b version

You can use any model supported by Ollama (e.g., Llama 3 0.3 or DeepSeek).

	2.	Enable Ollama – set an environment variable (any value works) and start the Ollama server:

export OLLAMA_API_KEY=ollama-local
ollama serve &

OpenClaw discovers models from http://127.0.0.1:11434 when this variable is set ￼.  You do not need an actual API key; the variable simply turns on discovery.

	3.	Configure OpenClaw – in your openclaw.json (or via the Control UI), specify the local model as the primary model and allow fallback to cloud models:

{
  "agents": {
    "defaults": {
      "model": {
        "primary": "ollama/ministral-instruct:8b-q4",
        "fallback": ["anthropic/claude-sonnet-4-5", "anthropic/claude-opus-4-5"]
      }
    }
  },
  "models": {
    "mode": "merge"                 // ensures fallbacks stay available
  }
}

In models.mode = "merge" OpenClaw merges local and hosted models, automatically falling back to the remote provider if the local server is unavailable ￼.  After editing, run openclaw gateway restart or use the Control UI’s “Apply & restart” button.

	4.	Check discovery – run ollama list or openclaw models list to verify that the Ministral model is detected ￼.  The contextWindow for auto‑discovered models defaults to what the model reports; you can override it in the explicit config if you need a 32 k token context.

2.2 Using LM Studio (more flexible)
	1.	Install LM Studio – download it from lmstudio.ai￼.  LM Studio is recommended for running high‑quality local models ￼.
	2.	Download the largest Ministral 3 8B or 14B build – inside LM Studio, open the “Discover” tab, search for Ministral 3 8B instruct (or 14B), choose the highest‑quality variant (avoid heavily quantized “small” builds), and click “Load model” ￼.
	3.	Start the local server – enable the Responses API and start the LM Studio server (default http://127.0.0.1:1234).  Confirm it’s serving by visiting http://127.0.0.1:1234/v1/models ￼.
	4.	Configure OpenClaw – add an lmstudio provider entry.  The docs give a template for MiniMax; adapt it for Ministral:

{
  "agents": {
    "defaults": {
      "model": {
        "primary": "lmstudio/ministral-3-8b-instruct-q4",
        "fallback": ["anthropic/claude-opus-4-5"]
      },
      "models": {
        "anthropic/claude-opus-4-5": { "alias": "Opus" },
        "lmstudio/ministral-3-8b-instruct-q4": { "alias": "Ministral Local" }
      }
    }
  },
  "models": {
    "mode": "merge",
    "providers": {
      "lmstudio": {
        "baseUrl": "http://127.0.0.1:1234/v1",
        "apiKey": "lmstudio",           // dummy value
        "api": "openai-responses",
        "models": [
          {
            "id": "ministral-3-8b-instruct-q4",
            "name": "Ministral 3 8B Instruct Q4",
            "reasoning": true,
            "contextWindow": 32768,
            "maxTokens": 32768
          }
        ]
      }
    }
  }
}

This defines the local model with a 32k context.  Setting api: "openai-responses" causes OpenClaw to use LM Studio’s responses API so reasoning traces remain local ￼.

	5.	Hybrid and failover – you can set a remote model (e.g., Claude Sonnet 4.5) as primary with the local model as fallback or vice‑versa.  See the “hybrid config” example in the docs ￼.  Keep hosted models configured to provide a safety net when your Mac mini is overloaded or offline.
	6.	Troubleshooting – if OpenClaw reports “no models available”, ensure the LM Studio server is running and that the baseUrl is correct.  If context‑window errors occur, lower contextWindow or increase the server’s limit ￼.

⸻

3 Connect remote cloud models (OpenAI, Anthropic, etc.)
	•	During onboarding you’ll be prompted for API keys.  For OpenAI, you can either supply an API key or use the ChatGPT/Codex subscription via OAuth.  After authenticating, run openclaw models list and openclaw models set <provider/model> to set the default model.  For Anthropic, either provide an API key or a Claude subscription setup token; the docs show CLI commands to obtain and configure these ￼.
	•	OpenClaw supports multiple providers simultaneously.  The agent chooses the primary model, but with models.mode: "merge", fallbacks remain available.  Use openclaw models status to view fallback chains ￼.

⸻

4 Set up the Telegram channel
	1.	Create a bot – open Telegram, chat with @BotFather and run /newbot.  Follow the prompts (name and username ending with “bot”), then copy the token ￼.
	2.	Configure the token – either:
	•	Set an environment variable: export TELEGRAM_BOT_TOKEN=<token>; or
	•	Add it to your config:

{
  "channels": {
    "telegram": {
      "enabled": true,
      "botToken": "<token>",
      "dmPolicy": "pairing",
      "groups": { "*": { "requireMention": true } }
    }
  }
}

The docs note that config entries override environment variables ￼.  dmPolicy: "pairing" means DM access requires explicit approval on first contact.

	3.	Start the gateway – run openclaw gateway restart or use the macOS app to ensure the gateway is running.  The Telegram bot will start automatically when a token is present ￼.
	4.	Pair with the bot – DM your bot from your personal Telegram account.  The bot responds with a pairing code.  In the terminal, run:

openclaw devices list
openclaw devices approve <requestId>

to approve the request ￼.  After approval, messages from your account will route to the agent.  Each browser profile or Telegram user generates a unique device ID; you may need to approve again if you clear data or change devices ￼.

	5.	Group chats – by default the bot only responds when mentioned; to allow full visibility, either disable privacy mode in BotFather with /setprivacy or give the bot admin rights in the group ￼.
	6.	Optional commands – you can add custom /commands to the Telegram bot menu via customCommands in config ￼.

⸻

5 Use the Control UI (browser‑based web UI)

The Control UI is a single‑page app served by the gateway.  It provides a chat interface, session management, skills management and configuration editing.
	1.	Open locally – if the gateway is running on the same Mac, open http://localhost:18789/ or http://127.0.0.1:18789/ ￼.  If the page doesn’t load, ensure the gateway is running.
	2.	First‑device pairing – connecting from a new browser or device triggers a one‑time pairing approval; you’ll see “disconnected (1008): pairing required” until you approve.  Run openclaw devices list and openclaw devices approve <requestId> to approve the browser session ￼.  Local connections from 127.0.0.1 are auto‑approved, but remote connections via LAN or Tailscale require pairing ￼.
	3.	Features – the Control UI lets you:
	•	Chat with the agent via chat.history, chat.send, and stream tool calls ￼.
	•	Manage channels (WhatsApp, Telegram, Discord, etc.), view QR codes for login, and update per‑channel settings ￼.
	•	View and modify sessions, run or cancel Cron jobs, manage skills (enable/disable or update API keys), inspect nodes (Mac nodes show Canvas, Camera, Screen, System capabilities), edit exec approvals, view logs and model status, and edit the openclaw.json config with live validation ￼.
	•	Apply updates and restart the gateway from the UI.
	4.	Remote access – for secure remote access, run openclaw gateway --tailscale serve and open the dashboard at https://<magicdns>/ ￼.  Tailscale Serve proxies the gateway over HTTPS and uses identity headers for auth ￼.  Alternatively, bind the gateway to the tailnet with a token: openclaw gateway --bind tailnet --token "$(openssl rand -hex 32)" and connect via http://<tailscale-ip>:18789/ ￼.  Avoid disabling device identity unless you fully trust your network ￼.

⸻

6 Letting the agent write new skills and run code
	•	Skill creation – OpenClaw agents can write or install new skills.  Use the CLI (openclaw skills create) or Control UI to scaffold a new skill.  Skills live in ~/.openclaw/skills and can call tools (browser, exec, apply_patch).  The agent can edit its own skills by invoking apply_patch on the skill file (this is how the agent writes code).  Provide explicit permission before the agent modifies or runs untrusted code.
	•	Exec approvals – system commands executed via system.run require approval and can be restricted by an allowlist.  The macOS app stores exec approvals in ~/.openclaw/exec-approvals.json ￼.  Set security: "allowlist" and add allowed binary paths (e.g., /opt/homebrew/bin/python) so the agent can run them without confirmation.  You can adjust these settings via the Control UI.
	•	Sandboxing – avoid running highly privileged commands with local models because smaller models are more susceptible to prompt injection ￼.  Keep the agent’s workspace limited, avoid automatically following links, and treat content from untrusted sources as potentially malicious ￼.  For high‑risk tasks, run the exec tool in sandbox mode or require manual approval.

⸻

7 Putting it all together
	1.	Install the macOS app, grant accessibility/screen‑recording permissions and ensure the gateway is running.
	2.	Install and configure local models (Ollama or LM Studio) for Ministral 3 8B/14B, using models.mode: "merge" and setting remote fallbacks.
	3.	Authenticate with remote providers (Anthropic/OpenAI) during onboarding; set a default remote model for heavy tasks.
	4.	Create and configure a Telegram bot with BotFather, set the token in openclaw.json, start the gateway, and approve the pairing.
	5.	Open the Control UI locally for management and chat; pair it on first use.  For remote use, expose the gateway via Tailscale or SSH tunnels.
	6.	Use CLI commands or the UI to manage skills, models and sessions; adjust exec approvals to allow safe code execution.  Encourage the agent to write new skills only within the allowed environment.

By following the steps above, you’ll have a self‑hosted OpenClaw agent on your Mac mini capable of local inference using Ministral‑3‑8b/14b models, with automatic failover to Anthropic or other cloud models, and able to communicate via Telegram and a browser‑based UI.  Use strong models for tasks involving tool access to minimize prompt‑injection risks ￼, and regularly review your configuration and logs to ensure safe operation.