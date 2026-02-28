# Deployment Modes & Cloud LLM Execution

To clarify exactly how Cloud LLMs (like OpenAI's GPT-4o or Anthropic's Claude 3.5 Sonnet) interact with your hardware, we need to distinguish between _the LLM API_ and _the Orchestrator_.

When you use a Cloud LLM, you are sending a REST API request to OpenAI's servers. **OpenAI's servers do not execute any tools.** Instead, the OpenAI API returns a JSON response called a "Tool Call" (e.g., `{"name": "run_bash", "arguments": "ls -la"}`).

The **Rust Orchestrator** is the software that receives this JSON and actually executes the command on its host operating system.

Because of this, the location of tool execution is entirely dependent on **where you run the Rust Orchestrator.**

## 1. Local Mode (Tauri Desktop App)

**The Setup:** The user installs ThinClaw on their MacBook. The Rust Orchestrator runs locally inside the Tauri backend.
**The Flow:**

1. Tauri sends a prompt to OpenAI: _"List my desktop files."_
2. OpenAI's API "bounces" a tool call back over the internet to your MacBook: `{"name": "run_bash", "arguments": "ls ~/Desktop"}`.
3. The local Rust Orchestrator receives this, pops up an approval dialog in Tauri, and executes the bash command on the **user's MacBook**.

_In this mode, Cloud LLMs perfectly control your local hardware._

## 2. Remote Mode (Headless Cloud Server)

**The Setup:** The user compiles the Rust Orchestrator as a headless binary (`--features headless`) and deploys it to a DigitalOcean Ubuntu VPS. The user connects their Tauri app to it simply as a "Dumb Chat UI".
**The Flow:**

1. The user types a message in Tauri: _"List my desktop files."_
2. Tauri sends this text message over WebSockets to the Remote Ubuntu VPS.
3. The Remote Orchestrator sends a prompt to OpenAI: _"List my desktop files."_
4. OpenAI's API "bounces" a tool call back over the internet to the Ubuntu VPS: `{"name": "run_bash", "arguments": "ls ~/Desktop"}`.
5. The Remote Orchestrator executes this bash command on the **Ubuntu Server's filesystem**, completely isolated from the user's local MacBook.

_In this mode, Cloud LLMs perfectly control the remote server's hardware._

## 3. Remote Mode with Local Hardware Bridging (Opt-In)

What if the agent is running remotely on a VPS, but it needs to see your screen or hear your voice to help you?

Even though the Tauri app is acting as a "Thin Client," it still has privileged access to your local MacBook hardware (Camera, Microphone, Screen).

If the user explicitly grants permission in the UI, the architecture allows for **Hardware Bridging**:
1. The Remote Orchestrator receives a tool call from the LLM to `take_screenshot` or `record_audio`.
2. Instead of trying to take a screenshot of the headless Linux VPS, the Remote Orchestrator recognizes this as a **Client-Bridged Tool**.
3. It sends an RPC payload down the WebSocket connection to your local Tauri Companion App: `{"action": "take_screenshot"}`.
4. The local Tauri app executes the screenshot command natively and sends the resulting image bytes back up the WebSocket to the Remote Orchestrator.
5. The Remote Orchestrator processes the image as normal.

_This ensures that computationally heavy tasks and bash executions remain on the VPS, while sensory inputs (sight, hearing) can be securely sourced from the user's local device._

## The Beauty of this Architecture

Because the core Agent logic and standard Tool Definitions (like `run_bash` or `write_file`) live entirely within the Rust Orchestrator, **you do not need to write complex networking for 95% of tool executions.** They execute wherever the Orchestrator lives.

If the user wants the agent to control their personal laptop filesystem, they run ThinClaw locally.
If the user wants a 24/7 background agent scraping the web, they run ThinClaw on a VPS—and the agent happily executes its `run_bash` scripts on the VPS linux filesystem without ever risking the user's personal laptop data.

And for the 5% of sensory tools (Camera, Mic), a secure WebSocket bridge handles the exact permissions needed.
