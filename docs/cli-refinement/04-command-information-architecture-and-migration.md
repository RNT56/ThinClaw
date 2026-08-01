# CR-04 — Command Information Architecture and Migration

- **Priority:** P1
- **Depends on:** CR-01 contracts, CR-02 corrected shared handlers, CR-03 command registry
- **Goal:** make the default surface small without deleting real expert capabilities

## CR-04.1 — Ship this canonical command tree

```text
thinclaw                         Start interactive chat
├── run                         Start interactive chat explicitly
├── tui                         Start full-screen terminal UI
├── ask <text>                  One local agent turn, then exit
├── send <text>                 Inject a message through the running web runtime
├── setup                       Configure ThinClaw and exit (`--run` opts into REPL)
│   ├── edit                    Focused AI/channels/agent/tools/automation/runtime/appearance edit
│   └── reset                   Scoped/dry-run/exclusive ThinClaw state reset
├── status [--readiness-profile P] [--live]
│   └── tools [NAME] [--all]     Exact revisioned tool inventory and fact filters
│                                Compact capability/runtime status at the parent
├── doctor [--readiness-profile P]
│                                Active bounded readiness diagnosis
├── agents                      Durable agent workspace administration
│   ├── list | show | add | update | remove | set-default
├── config                      Settings and credentials
│   ├── get | set | list | path | init | reset
│   ├── secrets                 status/list/show/set/delete/rotate-master
│   └── models                  Model list/info/test/verify/sync
├── runtime                     Long-running process operations
│   ├── web                     Headless runtime/web ingress start/stop/reload/status/access
│   ├── service                 OS service install/start/stop/status/uninstall
│   ├── logs                    Tail/search/show/levels
│   └── update                  Check/install/info/rollback
├── extensions                  Agent integrations and capabilities
│   ├── activate                Activate an installed extension, with explicit kind if ambiguous
│   ├── channels                list/info/check-config/probe
│   ├── tools                   WASM tool install/list/remove/info/auth
│   ├── registry                list/search/info/install/install-defaults/remove/check
│   ├── mcp                     server/resource/prompt/root/log administration
│   └── skills                  inspect/read/list/search/check/install/update/audit/
│                               snapshot/publish/taps/remove/reload/trust
├── automation                  Repeated and supervised work
│   ├── routines                list/add/edit/remove/trigger/runs/lint
│   ├── jobs                    list/summary/show/cancel/restart/prompt/events/files
│   └── projects                Repository project supervisor
├── data                        Durable user/agent data
│   ├── memory                  search/read/write/delete/tree/status
│   ├── conversations           list/show/search/export/delete/prune
│   ├── learning                status/history/outcomes/feedback/proposals/rollbacks/external-memory
│   │   └── external-memory     status/configure/activate/deactivate
│   ├── backup                  encrypted export/import/inspect
│   └── trajectories            export/stats archived trajectories
├── access                      People, senders, and devices
│   ├── identities              list/show/create/link/unlink/rename/preferred-channel
│   ├── senders                 list/approve/block/unblock/blocked inbound senders
│   └── devices                 pair/list/rename/revoke
├── labs
│   └── experiments             Optional research automation
├── media
│   └── comfy                   ComfyUI generation administration
├── dev
│   └── browser                 External Chrome/Chromium utility
├── completion --shell SHELL    Generate a raw shell completion script
└── help [PATH...] [--all]      Common, nested, or expert help
```

`status` is one parent parser with optional `StatusArea::Tools`. Its readiness/live scope is shared inside that subtree, so those flags may be written before or after `tools` but are stored once and reject conflicting duplicates; they do not become root-global flags. Help and generated references always print the canonical before-subcommand order above.

Default `thinclaw --help` shows the common commands (`run`, `tui`, `ask`, `send`, `setup`, `status`, `doctor`) followed by one-line expert category entries (`agents`, `config`, `runtime`, `extensions`, `automation`, `data`, `access`, `labs`, `media`, `dev`). It does not dump every leaf command. `thinclaw help --all` expands canonical expert commands. Internal and compatibility-only paths never appear in either.

## CR-04.2 — Map every existing public command

**Direct move/group coverage not owned by a deeper behavior task:** INV-26, INV-30, INV-31, INV-32, INV-41, INV-45, INV-46, INV-47. The exhaustive leaf proof is in the cross-cutting matrix.

| Existing 0.16 path | Canonical path | Disposition and compatibility |
|---|---|---|
| `thinclaw` | `thinclaw` | Keep. |
| `run` | `run` | Keep; runtime-specific flags live here. |
| `tui` | `tui` | Keep and overhaul behavior under CR-03. |
| global `-m/--message TEXT` | `ask TEXT` | Hidden root compatibility for 0.17 only; remove in 0.18. |
| `message send` | `send` | Hidden forwarding path through 0.18; remove in 0.19. |
| `onboard` | `setup` | Hidden forwarding path through 0.18. Canonical explicit setup exits; legacy `onboard` temporarily forwards with `--run` to preserve its current continuation. |
| `onboard --guide[=TOPIC]` | `setup edit TOPIC` or full `setup --mode advanced` | Hidden forwarding through 0.18. The old one-topic path maps to `edit ... --run`; “Advanced” now means the complete path. |
| `onboard --channels-only` | `setup edit channels` | Hidden forwarding through 0.18; old form temporarily continues to runtime, canonical edit exits. |
| `reset` | `setup reset` | Hidden forwarding path through 0.18; same confirmation semantics. |
| `secrets` | `config secrets` | Hidden forwarding path through 0.18. The current `set --value` plaintext-argument option is removed immediately; hidden `--provider`/`--user` map to typed purpose/principal only. |
| `config` | `config` | Keep; nest existing settings leaves plus secrets/models. |
| `models` | `config models` | Hidden forwarding path through 0.18. |
| `cron` | `automation routines` | Hidden forwarding path through 0.18; terminology says routines. |
| `cron trigger` fake behavior | `automation routines trigger` | Old path forwards only to the new real trigger; fake print-only code is deleted immediately. |
| `repo-projects` | `automation projects` | Hidden forwarding path through 0.18, except plaintext `set-credential --value`, which is removed immediately. The hidden command accepts only the canonical masked/stdin/env/file source-creation form. |
| no job CLI | `automation jobs` | New, wired to existing job API. |
| `experiments` | `labs experiments` | Hidden forwarding path through 0.18. |
| `gateway` | `runtime web` | Hidden forwarding path through 0.18; docs clarify full headless runtime. |
| `service` | `runtime service` | Hidden forwarding path through 0.18. |
| `logs` | `runtime logs` | Hidden forwarding path through 0.18. |
| `update` | `runtime update` | Hidden forwarding path through 0.18. |
| `channels` | `extensions channels` | Hidden forwarding path through 0.18. |
| `channels validate` | `extensions channels check-config` | Hidden leaf alias through 0.18; it never claims live health. |
| no live channel probe | `extensions channels probe` | New. |
| no generic activation CLI | `extensions activate NAME [--kind KIND]` | New running-runtime action through `GatewayClient` and the shared `ExtensionManager` API; no throwaway local activation and no unsupported deactivate leaf. |
| `tool` | `extensions tools` | Hidden forwarding path through 0.18. |
| `registry` | `extensions registry` | Hidden forwarding path through 0.18; canonical leaf is `check`, and `validate` is a hidden leaf alias through 0.18. |
| `mcp` | `extensions mcp` | Hidden forwarding path through 0.18. |
| no skills admin CLI | `extensions skills` | New, backed by existing runtime service. |
| `memory` | `data memory` | Hidden forwarding path through 0.18. |
| no memory delete CLI | `data memory delete` | New guarded parity with the existing memory policy/service. |
| no learning admin CLI | `data learning` | New deep expert read/review/provider-administration surface; contextual learning execution tools stay agent-only. |
| `sessions` | `data conversations` | Hidden forwarding path through 0.18, but uses durable semantics immediately; old ephemeral handler deleted. |
| `backup` | `data backup` | Hidden forwarding path through 0.18. |
| `trajectory` | `data trajectories` | Hidden forwarding path through 0.18. |
| `identity` | `access identities` | Hidden forwarding path through 0.18. |
| `pairing` | `access senders` | Hidden forwarding path through 0.18, except `approve CHANNEL CODE`, which is removed immediately; approval uses the pending request ID and no result/error exposes the code. |
| `devices` | `access devices` | Hidden forwarding path through 0.18. |
| `agents` | `agents` | Keep top-level expert command; replace ephemeral implementation. |
| `comfy` | `media comfy` | Hidden forwarding path through 0.18. |
| `browser` | `dev browser` | Hidden forwarding path through 0.18. |
| `completion` | `completion` | Keep; omit only from common synopsis. |
| `doctor [--profile PROFILE]` | `doctor [--readiness-profile PROFILE]` | Hidden old flag through 0.18; map current values, with `desktop-linux`/`desktop-gnome` to canonical `desktop`; overhaul report/exit contract. |
| `status [--profile PROFILE]` | `status [--readiness-profile PROFILE]` | Hidden old flag/value aliases through 0.18; overhaul static/live/tool-inventory behavior. |
| `worker` | unchanged internal path | Hide immediately; no user compatibility promise. |
| `claude-bridge` | unchanged internal path | Hide immediately. |
| `codex-bridge` | unchanged internal path | Hide immediately. |
| `experiment-runner` | internal stdin/private-artifact auth protocol | Hide immediately; remove `--token` and every token-bearing generated bootstrap/result path immediately. |
| `network-relay` | unchanged internal path | Keep hidden. |
| `autonomy-shadow-canary` | unchanged internal path | Keep hidden. |
| `__windows-service` | unchanged internal path | Keep hidden. |

Global flag migration:

| Existing | Canonical | Lifecycle |
|---|---|---|
| `--debug` | `--debug` | Keep truly global. |
| `--cli-only` | `run/tui/ask --channels none` | Hidden alias through 0.18; exact semantics fixed immediately. |
| `--no-db` | scoped hidden/testing `run/tui/ask --no-db` | Remove global parsing in 0.17. |
| `--config PATH` | global explicit config input honored uniformly by `CliContext` | Keep global; missing/unreadable explicit paths fail. |
| `--no-onboard` | scoped `run/tui/ask --skip-setup-check` | Remove global parsing in 0.17; hidden forwarding flag through 0.18. |
| setup `--skip-auth` | setup `--skip-provider-auth` | Hidden forwarding leaf alias through 0.18; scope is provider authentication, not all authentication. |
| command-local presentation `--format` | global `--output-format` | Hidden leaf compatibility through 0.18 where unambiguous; domain artifact formats remain leaf options. |
| command-local artifact `--output/-o FILE` | `--out FILE` | Hidden leaf alias through 0.18; never conflicts with global presentation. |

## CR-04.3 — Implement grouping without handler duplication

1. Define nested Clap enums (`ConfigArea`, `RuntimeArea`, `ExtensionsArea`, `AutomationArea`, `DataArea`, `AccessArea`, `LabsArea`, `MediaArea`, `DevArea`) in focused modules, not one ever-growing `src/cli/mod.rs`.
2. Keep `src/cli/mod.rs` as the root parse façade and re-export canonical leaf command types.
3. Convert parsed canonical and legacy variants into typed requests before dispatch. Example:

   ```text
   Command::Automation(Automation::Routines(cmd)) ┐
                                                   ├─> RoutineRequest -> routine handler
   LegacyCommand::Cron(cmd) ──────────────────────┘
   ```

4. No legacy enum/adapter may contain persistence, HTTP, validation, confirmation, runtime-owner selection, or rendering logic. Canonical/legacy requests retain one idempotency/request ID and generated D-54 execution policy, then receive the same `MutationReceipt`/lifecycle events.
5. Preserve intentional feature-gated parsing according to the exhaustive matrix. Feature-exclusive commands do not appear when code truly is absent. Commands that administer artifacts but cannot execute them remain visible and return typed capability state. `runtime service`, REPL, and TUI are no longer incorrectly gated by `repl`.
6. Compile after each category is introduced; migrate dispatch arms one category at a time. Delete old exports only after all references and tests use canonical request types.

Acceptance:

- canonical and legacy forms produce the same typed request/result fixture;
- mutating aliases choose the same runtime instance/owner and return the same durable/live revisions; no alias can fall back to direct storage;
- only the adapter path differs in coverage traces;
- `src/cli/mod.rs` remains navigable and category modules own their leaf parsing;
- no circular dependency is introduced between CLI presentation and domain crates.

## CR-04.4 — Use consistent nouns, verbs, and destructive semantics

Apply these rules across help and implementations:

| Verb | Meaning |
|---|---|
| `list` | collection summary, filterable/paginatable |
| `show` | one resource with detail |
| `add` / `create` | `add` for configuration/registration; `create` for durable domain records where already idiomatic |
| `set` / `edit` | `set` for one config value; `edit`/`update` for a resource |
| `remove` | unregister/uninstall configuration |
| `delete` | permanently delete durable user data; always identifies the exact target and requires confirmation; multi-target/query-selected deletion also implements dry-run parity |
| `revoke` | invalidate access/device authorization |
| `cancel` | terminate active work |
| `check-config` | static/no-network validation |
| `probe` | bounded live external check |
| `status` | current dimension-qualified snapshot, not a mutation |

Standard options:

- IDs are positional when exactly one primary resource is required;
- `--limit`, `--cursor`, and `--channel` mean the same thing wherever declared. `--principal` is declared only on surfaces with an explicit admin override contract; it never grants authority and is intentionally absent from `data learning`, which derives authenticated/local context;
- destructive noninteractive operations require `--yes`; every multi-target or query-selected deletion supports `--dry-run` and defaults to dry-run until `--yes`; a single explicit-ID deletion renders that ID and impact before interactive confirmation;
- global output/color/verbosity flags are not redefined locally;
- time values accept and document one shared duration parser;
- paths use `--out` for output artifacts and refuse accidental overwrite unless `--force` is explicitly supported;
- public presentation is always `--output-format`; artifact format is always named `--artifact-format` when a leaf also supports machine presentation;
- secret values are never ordinary positional/options: public consumers use `--credential-source`/`--secret-env` IDs, local creation uses the shared masked/stdin/env/file grammar, and short-lived internal authentication uses the CR-01.8 private envelope protocol;
- commands with secret/cost/high-impact exceptions follow the exact policies in CR-02.14…CR-02.16 and the leaf matrix, rather than treating all mutations as equivalent.

## CR-04.5 — Common help, expert help, and generated completion

Implement help from structured command metadata:

1. Add `CliVisibility::{Common, Category, Expert, CompatibilityHidden, InternalHidden}` distinct from slash-command visibility but following the same principle.
2. Keep Clap's `-h/--help` and `-V/--version` raw parser-information behavior and implement a typed metadata-only `help [PATH...] [--all]` command. An empty path renders root help; a path resolves an exact canonical command/category and rejects compatibility/internal names as not found; `--all` is valid only with an empty path and recursively lists common/category/expert canonical paths. Category `--help` and `help category` are byte-equivalent after normalizing the invocation line. Help/version perform no state bootstrap/network access and always exit `0` when the requested canonical path exists; unknown help paths are usage exit `2`. Per D-48, `--output-format` never wraps these parser information actions.
3. Compatibility and internal commands are excluded from both help tiers, manpage/reference generation, and completion.
4. Completions include all canonical commands supported by the active feature build, including expert commands.
5. Examples use canonical paths only. Never advertise a deprecated path merely to explain migration; migration notes own that content.
6. Add width-stable help snapshot tests with color disabled and representative feature sets.
7. Render availability/dependency annotations from the same command/capability metadata. The default help does not emit a wall of unavailable leaves; category help gives concise requirements such as `[running runtime]`, `[database]`, `[external Chrome]`, or `[feature: wasm-runtime]`.
8. Fix root ordering to Start (`run`, `tui`, `ask`, `send`), Configure (`setup`, `config`), Inspect (`status`, `doctor`), then one-line category summaries (`agents`, `runtime`, `extensions`, `automation`, `data`, `access`, `labs`, `media`, `dev`). At 80 columns the default root help is at most 30 nonblank lines and no rendered line exceeds the terminal width; snapshot 80/100/120 columns. Detailed leaves, compatibility notes, and examples live in category help/reference rather than wrapping the root page.

Acceptance:

- default root help meets the 80-column/30-line budget without the current flat command wall;
- a user can reach every real user capability through category help or `help --all`;
- internal/compatibility paths do not leak;
- generated completions parse and contain canonical paths only;
- help descriptions identify local vs direct-DB vs running-runtime requirements.

The exhaustive current-to-target leaf mapping and output/state/dependency contract lives in `06-feature-command-and-output-matrix.md`; it is part of CR-04 acceptance, not optional background.

## CR-04.6 — Compatibility behavior and sunset

Version policy from the audited `0.16.0` baseline:

- **0.17.x:** canonical tree ships. Hidden aliases work. Human stderr warns. Global `-m` remains only for this release. Unsafe PATH/shell and false ephemeral/fake implementations are already removed.
- **0.18.x:** all mapped hidden command aliases and `--cli-only` remain; global `-m` is removed. Release notes give final removal notice.
- **0.19.0:** remove root command aliases, old leaf aliases, command-local format aliases, and `--cli-only`. Retain only truly internal entrypoint tokens required by orchestrators.

Warning format:

```text
warning: `thinclaw cron` is deprecated; use `thinclaw automation routines` (removed in 0.19)
```

Rules:

1. Warn once per invocation on human stderr only.
2. JSON/JSONL stdout is unaffected; machine mode omits the human warning.
3. Old and new forms return the same exit class and typed data.
4. If old and new flags conflict, fail with a usage error; never guess.
5. Add release-note entries at introduction and removal.

## CR-04.7 — Explicit removals versus retained capability

Remove immediately:

- onboarding PATH/symlink mutation and target deletion;
- raw TUI `!command` process execution;
- old in-memory terminal agent/session construction;
- print-only routine trigger;
- `/think` toggle and its false documentation;
- banners/ANSI in machine or piped output;
- public visibility of internal process entrypoints;
- static channel “validate” claims of live health;
- tokenized gateway URLs in boot/TUI.

Retain but overhaul/group:

- agent administration, durable conversations, routines, jobs, projects, experiments;
- config, secrets, models;
- generic extension activation, channels, WASM tools, registry, MCP, skills;
- memory including guarded deletion, conversations, learning administration, backup, trajectories;
- identity/senders/devices;
- gateway/headless runtime, OS service, logs, updates;
- ComfyUI and browser tooling;
- completion, status, doctor, setup/reset, REPL, and TUI.

No underlying real capability is deleted merely because it moves out of common help. Agent-only execution capabilities in file 07 are likewise retained, but they are intentionally represented through exact capability inventory rather than duplicated as unsafe terminal leaves.

## CR-04 definition of done

- [ ] Canonical tree matches CR-04.1 exactly or the dossier is amended before implementation.
- [ ] Every current command/flag in CR-04.2 maps to a tested target disposition.
- [ ] Canonical and legacy routes share typed handlers.
- [ ] Default help is compact; expert help is complete; internals/aliases are absent.
- [ ] Naming, pagination, confirmation, and output conventions are consistent.
- [ ] 0.17/0.18/0.19 compatibility lifecycle is encoded in tests and release notes.
- [ ] Completions contain canonical supported commands only.
