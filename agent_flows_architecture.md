# ThinClaw Agent Flows — Complete Architecture

> **Generated:** 2026-03-25 11:46 CET

---

## Master Agent Flow Diagram

```mermaid
flowchart TB
    subgraph OS["OS Service Layer"]
        LAUNCHD["launchd / systemd<br/>KeepAlive=true, Restart=always"]
        LAUNCHD -->|spawns| PROCESS
    end

    subgraph PROCESS["Agent Process Startup"]
        BOOT_PROC["Binary starts<br/>thinclaw run"]
        CONFIG["Load config<br/>.env + settings.toml + env vars"]
        DB["Connect database<br/>postgres / libsql"]
        WORKSPACE["Initialize workspace<br/>Seed BOOT.md, BOOTSTRAP.md,<br/>HEARTBEAT.md, AGENTS.md"]
        CHANNELS["Start channels<br/>Telegram, Discord, Signal,<br/>Web, CLI, iMessage, etc."]
        TOOLS["Register tools<br/>Shell, HTTP, Browser, Canvas,<br/>WASM extensions, Subagent"]
        
        BOOT_PROC --> CONFIG --> DB --> WORKSPACE --> CHANNELS --> TOOLS
    end

    subgraph BG_SPAWN["Background Task Spawn"]
        TOOLS -->|spawn| SELF_REPAIR
        TOOLS -->|spawn| SESSION_PRUNE
        TOOLS -->|spawn| JOB_PRUNE
        TOOLS -->|spawn| MEM_HYGIENE
        TOOLS -->|spawn| ROUTINE_ENGINE
        TOOLS -->|spawn| HEALTH_MON
        TOOLS -->|spawn| CRON_TICKER
        TOOLS -->|spawn| NOTIF_FWD
        TOOLS -->|spawn| ZOMBIE_REAPER
        TOOLS -->|spawn| CONFIG_WATCH
    end

    subgraph HOOKS["Startup Hooks"]
        BG_SPAWN -->|fires| BEFORE_START["BeforeAgentStart Hook<br/>Can reject startup"]
        BEFORE_START -->|pass| BOOT_HOOK
    end

    subgraph PROACTIVE["🟢 PROACTIVE Flows"]
        BOOT_HOOK["BOOT.md Execution<br/>Every startup"]
        BOOTSTRAP_HOOK["BOOTSTRAP.md Execution<br/>First run only → delete after"]
        BOOT_HOOK --> BOOTSTRAP_HOOK
        
        BOOT_HOOK -->|synthetic message| HANDLE_MSG
        BOOTSTRAP_HOOK -->|synthetic message| HANDLE_MSG
        BOOT_HOOK -->|response| BROADCAST["broadcast() to<br/>preferred channel"]
        BOOTSTRAP_HOOK -->|response| BROADCAST
    end

    subgraph LOOP["Main Message Loop (tokio::select!)"]
        BOOTSTRAP_HOOK -->|then enters| SELECT
        
        SELECT{"tokio::select!<br/>(biased)"}
        CTRL_C["Ctrl+C signal"] -->|priority 1| SELECT
        CHAN_MSG["Channel messages<br/>Telegram, Discord, CLI,<br/>Web, Signal, iMessage"] -->|priority 2| SELECT
        SYS_EVENT["System events<br/>Heartbeat injection"] -->|priority 3| SELECT
        
        SELECT -->|message| HANDLE_MSG
    end

    subgraph REACTIVE["🔵 REACTIVE Flows"]
        HANDLE_MSG["handle_message()"]
        HANDLE_MSG --> EVENT_CHECK["check_event_triggers()<br/>regex match → fire routines"]
    end

    subgraph SHUTDOWN["Shutdown"]
        CTRL_C -->|or /quit| ABORT_BG["Abort all background tasks"]
        ABORT_BG --> CLOSE_CH["Shutdown channels"]
        CLOSE_CH --> EXIT["Process exits"]
        EXIT -->|KeepAlive| LAUNCHD
    end

    style PROACTIVE fill:#0d472a,stroke:#1db954,color:#fff
    style REACTIVE fill:#0a2744,stroke:#2196f3,color:#fff
    style OS fill:#3a1a00,stroke:#ff9800,color:#fff
```

---

## Background Subprocess Map

```mermaid
flowchart LR
    subgraph BG["Background Tasks (all tokio::spawn)"]
        direction TB
        
        SR["🔧 Self-Repair<br/>Interval: config.repair_check_interval<br/>Detects stuck jobs + broken tools<br/>Auto-repairs or notifies user"]
        
        SP["🗑️ Session Pruning<br/>Interval: 10 min<br/>Removes idle sessions older<br/>than session_idle_timeout"]
        
        JP["🧹 Job Context Pruning<br/>Interval: 5 min<br/>Safety net for leaked<br/>ContextManager slots"]
        
        MH["💾 Memory Hygiene<br/>Interval: hygiene.cadence_hours<br/>Deletes stale daily logs,<br/>runs workspace cleanup"]
        
        HM["💓 Channel Health Monitor<br/>Periodic health_check()<br/>Auto-restart channels with<br/>failure tracking + cooldown"]
        
        CW["📄 Config Watcher<br/>Polls mtime of settings.toml<br/>Broadcasts changes via channel"]
    end

    subgraph RE["Routine Engine Subsystem"]
        direction TB
        
        CT["⏰ Cron Ticker<br/>Interval: cron_check_interval_secs<br/>Checks all routines for<br/>due cron expressions"]
        
        ZR["🧟 Zombie Reaper<br/>Interval: 60s<br/>Aborts stuck routine tasks<br/>exceeding max duration"]
        
        NF["📬 Notification Forwarder<br/>Receives routine outputs<br/>Routes to target channel<br/>(Telegram, Discord, web, etc.)"]
        
        ET["🔔 Event Trigger Cache<br/>In-memory regex patterns<br/>Checked per incoming message"]
    end

    subgraph TRIGGERS["What Triggers Each"]
        T1["Timer"] --> SR
        T2["Timer"] --> SP
        T3["Timer"] --> JP
        T4["Timer"] --> MH
        T5["Timer"] --> HM
        T6["Timer (mtime poll)"] --> CW
        T7["Timer (cron)"] --> CT
        T8["Timer"] --> ZR
        T9["Channel (mpsc)"] --> NF
        T10["Every message"] --> ET
    end
```

---

## Message Processing Pipeline

```mermaid
flowchart TB
    MSG["IncomingMessage<br/>(channel, user_id, content)"] --> PARSE["SubmissionParser::parse()"]
    
    PARSE --> UI["UserInput"]
    PARSE --> CMD["SystemCommand<br/>/status, /model, /agent, etc."]
    PARSE --> UNDO["/undo"]
    PARSE --> REDO["/redo"]
    PARSE --> INT["/interrupt"]
    PARSE --> COMPACT["/compact"]
    PARSE --> CLEAR["/clear"]
    PARSE --> THREAD["/new, /switch"]
    PARSE --> HB["Heartbeat"]
    PARSE --> APPROVE["/approve, /deny"]
    PARSE --> QUIT["/quit → shutdown"]
    
    UI --> HOOK_IN["BeforeInbound Hook<br/>Can reject or modify"]
    HOOK_IN --> HYDRATE["Hydrate thread from DB<br/>(if historical)"]
    HYDRATE --> SESSION["Resolve session + thread<br/>SessionManager"]
    SESSION --> ROUTER["Multi-agent routing<br/>AgentRouter.route()"]
    ROUTER --> AUTH_CHECK{"Auth mode<br/>pending?"}
    
    AUTH_CHECK -->|yes| AUTH["process_auth_token()<br/>Credential store"]
    AUTH_CHECK -->|no| PROCESS["process_user_input()"]
    
    PROCESS --> MEDIA["Media pipeline<br/>Images, PDF, audio, video"]
    MEDIA --> SKILLS["Skill selection<br/>prefilter_skills()"]
    SKILLS --> CTX["Build context<br/>System prompt + history +<br/>skills + workspace + tools"]
    CTX --> DISPATCH["Dispatcher::dispatch()<br/>Agentic tool loop"]
    
    subgraph AGENTIC["Agentic Loop (Dispatcher)"]
        DISPATCH --> LLM["LLM call<br/>complete_with_tools()"]
        LLM --> TC{"Tool calls<br/>returned?"}
        TC -->|no| RESPONSE["Return text response"]
        TC -->|yes| APPROVAL{"Needs<br/>approval?"}
        
        APPROVAL -->|no| EXEC["Execute tools<br/>(parallel if multiple)"]
        APPROVAL -->|yes| WAIT["Wait for user<br/>approval/denial"]
        WAIT -->|approved| EXEC
        WAIT -->|denied| RESPONSE
        
        EXEC --> INJECT["Inject tool results<br/>into context"]
        INJECT -->|loop, max 50 iters| LLM
    end
    
    RESPONSE --> HOOK_OUT["BeforeOutbound Hook<br/>Can modify or suppress"]
    HOOK_OUT --> SEND["Channel.respond()<br/>Send to user"]
    
    SEND --> EVT_CHECK["check_event_triggers()<br/>Fire matching routines"]
```

---

## Tool Approval Decision Flow

```mermaid
flowchart TB
    TOOL["Tool call from LLM"] --> REQ["tool.requires_approval(params)"]
    
    REQ --> NEVER["ApprovalRequirement::Never<br/>(e.g., echo, memory_read)"]
    REQ --> UNLESS["ApprovalRequirement::UnlessAutoApproved<br/>(e.g., shell safe commands,<br/>http, file operations)"]
    REQ --> ALWAYS["ApprovalRequirement::Always<br/>(e.g., rm -rf, DROP TABLE,<br/>git push --force, kill -9)"]
    
    NEVER -->|"any mode"| AUTO_YES["✅ Auto-approved"]
    
    UNLESS --> MODE{"auto_approve_tools<br/>setting?"}
    MODE -->|true| AUTO_YES
    MODE -->|false| SESS{"Session<br/>auto-approved?"}
    SESS -->|yes| AUTO_YES
    SESS -->|no| ASK["⏳ Ask user"]
    
    ALWAYS --> ASK_ALWAYS["⏳ ALWAYS ask user<br/>🛡️ Even in auto-approve mode"]
    
    ASK --> USER_CHOICE{"User response"}
    ASK_ALWAYS --> USER_CHOICE
    USER_CHOICE -->|Approve once| EXEC["Execute tool"]
    USER_CHOICE -->|Always allow| EXEC_REMEMBER["Execute + remember"]
    USER_CHOICE -->|Deny| SKIP["Skip tool"]

    style ALWAYS fill:#8b0000,color:#fff
    style ASK_ALWAYS fill:#8b0000,color:#fff
    style AUTO_YES fill:#006400,color:#fff
```

---

## All Proactive vs Reactive Flows

| # | Flow | Type | Trigger | Frequency | Target |
|---|------|------|---------|-----------|--------|
| 1 | **BOOT.md** | 🟢 Proactive | Agent process starts | Every startup | Preferred notification channel |
| 2 | **BOOTSTRAP.md** | 🟢 Proactive | First run (file exists) | Once (deleted after) | Preferred notification channel |
| 3 | **Heartbeat** | 🟢 Proactive | Cron schedule (routine engine) | Configurable (e.g. every 6h) | Heartbeat notify channel |
| 4 | **Cron routines** | 🟢 Proactive | Cron expressions | Per schedule | Routine notify channel |
| 5 | **Event-triggered routines** | 🟠 Reactive-Proactive | Regex match on incoming messages | Per matching message | Routine notify channel |
| 6 | **Self-repair notifications** | 🟢 Proactive | Stuck job / broken tool detected | On detection | Web channel |
| 7 | **Channel health restart** | 🟢 Proactive | Channel health check fails | On failure + cooldown | Internal (auto-restart) |
| 8 | **Memory hygiene** | 🟢 Proactive | Timer (cadence_hours) | Periodic | Internal (workspace cleanup) |
| 9 | **Session pruning** | 🟢 Proactive | Timer (10 min) | Every 10 min | Internal (memory cleanup) |
| 10 | **Config hot-reload** | 🟢 Proactive | File mtime change | On change | Internal (log + restart hint) |
| 11 | **User message** | 🔵 Reactive | User sends text via channel | On demand | Same channel |
| 12 | **System command** | 🔵 Reactive | User sends /command | On demand | Same channel |
| 13 | **Tool approval** | 🔵 Reactive | Agent needs approval | On demand | Same channel |
| 14 | **Auth token** | 🔵 Reactive | Extension needs credentials | On demand | Same channel |
| 15 | **Sub-agent spawn** | 🔵 Reactive | Main agent spawns sub-agent | During tool execution | Internal → main agent |
| 16 | **Webhook inbound** | 🔵 Reactive | External HTTP POST | On demand | Agent loop via inject_tx |

---

## Self-Update Architecture

```mermaid
flowchart TB
    subgraph CURRENT["Current Capabilities"]
        CLI_CMD["thinclaw update check<br/>thinclaw update install --yes"]
        CLI_CMD --> FETCH["Fetch GitHub Releases API<br/>github.com/RNT56/ThinClaw/releases"]
        FETCH --> COMPARE["Compare semver<br/>current vs available"]
        COMPARE --> DOWNLOAD["Download platform binary<br/>(os + arch match)"]
        DOWNLOAD --> BACKUP["Backup current binary<br/>→ thinclaw.bak"]
        BACKUP --> REPLACE["Atomic rename<br/>new binary → current path"]
        REPLACE --> RESTART["⚠️ Manual restart required<br/>'Restart ThinClaw for changes'"]
    end
    
    subgraph SERVICE["Service Auto-Restart"]
        RESTART --> EXIT["Process exits"]
        EXIT --> LAUNCHD["launchd KeepAlive=true<br/>systemd Restart=always"]
        LAUNCHD --> NEW["New binary starts<br/>with updated code"]
    end
    
    subgraph GAP["❌ Missing: Agent-Initiated Update"]
        AGENT["Agent receives<br/>'update yourself'"]
        AGENT -->|"can run"| SHELL["shell tool:<br/>thinclaw update install --yes"]
        SHELL -->|"but"| PROBLEM["⚠️ Replaces its own binary<br/>while process is running"]
        PROBLEM -->|"needs"| GRACEFUL["Graceful self-restart<br/>(not implemented)"]
    end

    style GAP fill:#4a1a00,stroke:#ff6600,color:#fff
    style CURRENT fill:#1a3a1a,stroke:#4caf50,color:#fff
```

---

## Can the Agent "Update Itself"?

### Short Answer: **Partially — it can update the binary but cannot gracefully restart itself yet.**

### What Works Today

1. **`thinclaw update check`** — Fetches latest release from `https://api.github.com/repos/RNT56/ThinClaw/releases`, compares semver, shows what's available ([update.rs](file:///Users/vespian/coding/ThinClaw-main/src/cli/update.rs#L233-L267))

2. **`thinclaw update install --yes`** — Downloads the platform binary from GitHub Releases, backs up the current binary to `thinclaw.bak`, and atomically replaces the running binary ([update.rs](file:///Users/vespian/coding/ThinClaw-main/src/cli/update.rs#L269-L343))

3. **`thinclaw update rollback`** — Restores from backup if something goes wrong ([update.rs](file:///Users/vespian/coding/ThinClaw-main/src/cli/update.rs#L345-L357))

4. **Service auto-restart** — If ThinClaw is installed as a service (`thinclaw service install`), the launchd plist has `KeepAlive=true` and the systemd unit has `Restart=always`. So if the process exits, the OS will restart it with the new binary ([service.rs](file:///Users/vespian/coding/ThinClaw-main/src/service.rs#L66-L98))

### What's Missing

| Gap | Impact | Fix Required |
|-----|--------|-------------|
| **No graceful self-restart** | Agent can replace its binary via shell tool, but can't trigger a clean process restart from within | Add `/restart` command or `restart_self()` that does orderly shutdown → exec() |
| **Source build not supported** | `thinclaw update install` downloads **pre-built binaries** from GitHub Releases, not source. If no release binary exists for the user's platform, the update fails | Need `git pull` + `cargo build --release` fallback for source installs |
| **No auto-update check** | Agent never proactively checks for updates | Could add to BOOT.md or heartbeat |
| **Shell tool safety** | `thinclaw update install --yes` runs fine, but asking the agent to do `git pull && cargo build` would work if the source is available and cargo is in PATH | Already possible in autonomous mode |

### If You Asked the Agent "Update Yourself"

With the current code, here's what would happen:

1. **In autonomous mode** (`auto_approve_tools = true`): The agent would likely run `thinclaw update check` via its shell tool — this would succeed and show available updates. It could then run `thinclaw update install --channel stable --yes` which would:
   - Download the new binary ✅
   - Replace the current binary ✅
   - Print "Restart ThinClaw for changes to take effect" ⚠️
   - **But the agent is still running the old binary in memory** — it needs a process restart

2. **With service installed**: If `thinclaw service install` was run, the agent could then run `kill -TERM $$` or just `exit 0` — the OS service manager would restart the process with the new binary. **However**, `kill` matches `NEVER_AUTO_APPROVE_PATTERNS` so it would require manual approval even in autonomous mode 🛡️

3. **Without service**: The process would need to be manually restarted, or the agent could exec() itself (not implemented).

### Recommendation

To make "update yourself" work end-to-end, you'd need:

```
1. thinclaw update install --yes   ← already works
2. Graceful self-restart            ← needs implementation
   Option A: exec() syscall (replaces process in-place)
   Option B: /restart command → orderly shutdown + launchd/systemd picks it up
```

**Option B** is the safer approach and already works if the service is installed—the agent just needs a `/restart` command that does a clean shutdown (which the OS service layer will detect and restart).

---

## Sub-Agent Flow

```mermaid
sequenceDiagram
    participant User
    participant Main as Main Agent Loop
    participant Dispatch as Dispatcher
    participant LLM as LLM Provider
    participant SubExec as SubagentExecutor
    participant SubAgent as Sub-Agent Loop

    User->>Main: "Research X and summarize Y"
    Main->>Dispatch: process_user_input()
    Dispatch->>LLM: complete_with_tools()
    LLM-->>Dispatch: spawn_subagent(task="Research X")
    Dispatch->>SubExec: spawn(task, tools, timeout)
    SubExec->>SubAgent: New isolated agentic loop
    
    Note over SubAgent: Own context, filtered tools,<br/>configurable timeout
    
    SubAgent->>LLM: complete_with_tools()
    LLM-->>SubAgent: tool calls (http, search)
    SubAgent->>SubAgent: Execute tools
    SubAgent->>LLM: Results + continue
    LLM-->>SubAgent: Final response
    SubAgent-->>SubExec: SubagentResult(findings)
    SubExec-->>Dispatch: Tool result string
    Dispatch->>LLM: Continue main loop with sub-agent results
    LLM-->>Dispatch: Final summary
    Dispatch-->>Main: Response text
    Main->>User: Combined answer
```
