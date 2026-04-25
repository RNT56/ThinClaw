# ThinClaw Desktop Autonomy

This document is the canonical operator guide for ThinClaw's host-level desktop autonomy layer.

It covers:

- what `desktop_autonomy` enables
- which settings control it
- deployment modes and bootstrap flow
- desktop tools and seeded routines
- emergency stop, evidence capture, and rollback
- how reckless desktop code auto-rollout works

Desktop autonomy is a privileged mode. It is not equivalent to a normal local ThinClaw run.

## What It Is

When `desktop_autonomy.enabled = true` and `desktop_autonomy.profile = "reckless_desktop"`, ThinClaw can:

- interact with local desktop apps through structured native adapters and UI automation
- seed and run desktop-focused `RoutineAction::FullJob` routines
- capture screenshots, OCR output, and before/after evidence
- auto-apply outcome-backed `memory`, `skill`, `prompt`, `routine`, and `code` improvements
- promote or roll back managed ThinClaw builds through the local desktop autorollout path

This profile deliberately removes per-action approval prompts for the desktop tool bundle. It keeps auditability, evidence capture, canaries, promotion checks, pause thresholds, and rollback.

## Settings

ThinClaw exposes desktop autonomy through the top-level `desktop_autonomy` settings group.

| Setting | Purpose | Default |
|---|---|---|
| `enabled` | Enables the desktop autonomy subsystem | `false` |
| `profile` | Selects the autonomy profile | `off` |
| `deployment_mode` | Chooses host-session ownership | `whole_machine_admin` |
| `target_username` | Required for `dedicated_user` deployments | unset |
| `desktop_max_concurrent_jobs` | Maximum simultaneous leased desktop jobs | `1` |
| `desktop_action_timeout_secs` | Per-action timeout for sidecar operations | `30` |
| `capture_evidence` | Enables screenshot/OCR/action evidence capture | `true` |
| `emergency_stop_path` | Kill-switch file path checked before runs and actions | `~/.thinclaw/AUTONOMY_DISABLED` |
| `pause_on_bootstrap_failure` | Pauses autonomy when bootstrap fails | `true` |
| `kill_switch_hotkey` | Operator-facing kill-switch hint surfaced in status | `ctrl+option+command+period` |

When `reckless_desktop` is active, ThinClaw also resolves agent behavior toward local host control:

- `allow_local_tools = true`
- workspace mode resolves to a host-usable mode instead of the approval-heavy sandbox-only path
- evidence capture stays on by default
- learning auto-apply classes are expanded to include `memory`, `skill`, `prompt`, `routine`, and `code`
- code proposals publish through `local_autorollout`

Environment overrides are available through `DESKTOP_AUTONOMY_*` variables, including profile, deployment mode, target username, concurrency, timeout, emergency-stop path, pause-on-bootstrap-failure, and kill-switch hotkey.

## Deployment Modes

Desktop autonomy uses one code path with two deployment shapes.

| Mode | Intended Host Shape | Notes |
|---|---|---|
| `whole_machine_admin` | Main logged-in operator account on a desktop or Mac mini | Default mode |
| `dedicated_user` | Separate GUI user session reserved for autonomy | Supported on macOS/Windows only in this release; Linux returns `unsupported_deployment_mode` |

### `whole_machine_admin`

Use this when ThinClaw should act inside the current logged-in desktop session.

- macOS uses a per-user `LaunchAgent`
- Windows uses the scheduled-task/session bootstrap path
- Linux uses the desktop-autostart/session path and currently supports GNOME on X11 only

### `dedicated_user`

Use this when the autonomy runtime should live inside a separate desktop user.

- `desktop_autonomy.target_username` is required
- ThinClaw checks whether that user exists
- if the user is missing and bootstrap has privilege, ThinClaw can create the user and generate a one-time secret
- generated secrets are stored in the platform secure store and only returned once in bootstrap output
- ThinClaw installs the session launcher for that user, but still requires a real GUI login and one-time permission approval in that user's session

Desktop autonomy does not auto-login the dedicated user.

Linux dedicated-user creation/session ownership is disabled for this release and
reports `unsupported_deployment_mode`. Use `whole_machine_admin` from a logged-in
GNOME on X11 session instead.

## Platform Prerequisites

Desktop autonomy requires a real interactive desktop session. It is not a
headless-server feature.

| Platform | Required Before Bootstrap | Verify With |
|---|---|---|
| macOS | Logged-in GUI session, Calendar, Numbers, Pages, TextEdit, privacy/accessibility permissions approved for the ThinClaw launcher/session, secure store access | `thinclaw doctor`, `autonomy_control bootstrap` |
| Windows | Logged-in interactive session, Outlook, Excel, Word, Notepad or compatible local apps, PowerShell/COM access, service/session launcher access, secure store access | `thinclaw doctor`, `autonomy_control bootstrap` |
| Linux | GNOME on X11, DBus session, AT-SPI accessibility bus, LibreOffice, Evolution, OCR/screenshot tools, Python GI/pyatspi modules | `thinclaw doctor --profile desktop-gnome` |

Dedicated-user mode additionally requires:

- `desktop_autonomy.target_username`
- a real GUI login for that user
- one-time permission approval inside that user's session
- platform secure-store access for generated secrets

Desktop autonomy should stay disabled on:

- Raspberry Pi OS Lite
- headless Linux servers
- Linux Wayland-only sessions
- accounts where the operator cannot approve accessibility/privacy prompts
- machines where host-level app/UI/screen control is not intentionally granted

Linux GNOME/X11 readiness:

```bash
thinclaw doctor --profile desktop-gnome
sudo apt install python3 python3-gi python3-pyatspi libreoffice \
  libreoffice-script-provider-python evolution evolution-data-server-bin \
  xdotool wmctrl tesseract-ocr gnome-screenshot scrot imagemagick \
  at-spi2-core libglib2.0-bin geoclue-2.0 ffmpeg fswebcam
```

## Tool Surfaces

Reckless desktop autonomy registers the following built-in tools:

| Tool | Actions |
|---|---|
| `desktop_apps` | `list`, `open`, `focus`, `quit`, `windows`, `menus` |
| `desktop_ui` | `snapshot`, `click`, `double_click`, `type_text`, `set_value`, `keypress`, `chord`, `select_menu`, `scroll`, `drag`, `wait_for` |
| `desktop_screen` | `capture`, `window_capture`, `ocr`, `find_text` |
| `desktop_calendar_native` | `list`, `create`, `update`, `delete`, `find`, `ensure_calendar` |
| `desktop_numbers_native` | `create_doc`, `open_doc`, `read_range`, `write_range`, `set_formula`, `run_table_action`, `export` |
| `desktop_pages_native` | `create_doc`, `open_doc`, `insert_text`, `replace_text`, `export`, `find` |
| `autonomy_control` | `status`, `pause`, `resume`, `bootstrap`, `rollback` |

ThinClaw prefers native adapters first and uses generic UI automation as the fallback path.

## Bootstrap Flow

Run desktop autonomy bootstrap through `autonomy_control bootstrap` or the local API surface.

Bootstrap does all of the following:

1. Ensures the desktop autonomy state directories exist.
2. Verifies sidecar health and platform permissions.
3. Checks platform-specific prerequisites.
4. Handles dedicated-user setup when requested.
5. Generates canary fixtures.
6. Seeds desktop skills and starter routines.
7. Writes and optionally loads the session launcher.

The bootstrap report includes:

- `passed`
- `health`
- `permissions`
- `fixture_paths`
- `session_ready`
- `blocking_reason`
- `dedicated_user_keychain_label`
- `one_time_login_secret` only when a new dedicated user was created in that bootstrap call
- seeded skills/routines and launcher state

If bootstrap fails and `pause_on_bootstrap_failure = true`, ThinClaw pauses desktop autonomy automatically.

## Seeded Skills And Routines

Bootstrap seeds the following desktop skills:

- `desktop_recover_app`
- `calendar_reconcile`
- `numbers_update_sheet`
- `pages_prepare_report`
- `daily_desktop_heartbeat`

Bootstrap also seeds default routines when they do not already exist:

- `desktop_recover_app`
- `calendar_reconcile`
- `numbers_update_sheet`
- `pages_prepare_report`
- `daily_desktop_heartbeat`

The weekday heartbeat routine checks autonomy health, permission state, and emergency-stop state before recommending or queueing more desktop work.

## Canary Fixtures

Bootstrap creates and maintains fixtures under `~/.thinclaw/autonomy/fixtures`.

Current fixtures include:

- Calendar named `ThinClaw Canary`
- Numbers document `canary.numbers` on macOS, or the platform-equivalent document extension elsewhere
- Pages document `canary.pages` on macOS, or the platform-equivalent document extension elsewhere
- generic editor fixture `canary.txt`
- export directory for canary output

These fixtures are generated through the native bridge, not committed as repo artifacts.

## Runtime Model

Desktop routines still use the existing routine and job pipeline. The desktop-specific loop is:

1. Acquire a desktop session lease.
2. Snapshot app/UI state and screen evidence.
3. Pick one atomic action.
4. Execute through a native adapter first, then generic UI if needed.
5. Re-snapshot and verify the expected change.
6. Run bounded recovery if verification fails.
7. Persist evidence, metadata, and outcome state.

The session manager enforces a bounded number of active desktop jobs per GUI session. By default that lease count is `1`.

## Platform Expectations

Desktop autonomy is cross-platform in code, but not every platform is equally polished.

| Platform | Bridge / Launcher Shape | Current Operator Expectation |
|---|---|---|
| macOS | Swift bridge + `LaunchAgent` | Most polished path today |
| Windows | PowerShell/COM bridge + scheduled-session launcher | Supported, prerequisite-driven |
| Linux | Python/UI automation bridge + desktop autostart/session launcher | Best-effort, prerequisite-heavy |

Bootstrap enforces platform prerequisites rather than pretending all hosts are equivalent.

Examples:

- macOS expects Calendar, Numbers, Pages, and TextEdit
- Windows expects Outlook, Excel, Word, and Notepad plus an interactive session
- Linux expects a compatible desktop session plus LibreOffice, Evolution, OCR tooling, accessibility/session utilities, and Python modules

## Emergency Stop And Pause Controls

Desktop autonomy checks the emergency-stop file before desktop routines fire and before desktop actions run.

Default kill-switch path:

```text
~/.thinclaw/AUTONOMY_DISABLED
```

Create that file to halt autonomous execution immediately. Remove it and resume through `autonomy_control resume` after you are ready to continue.

Operators can also:

- inspect status with `autonomy_control status`
- pause with `autonomy_control pause`
- resume with `autonomy_control resume`
- roll back the last promoted managed build with `autonomy_control rollback`

## Outcome-Backed Learning And Code Autorollout

`reckless_desktop` changes learning behavior intentionally.

In normal mode, outcome-backed learning stays mostly review-oriented. In reckless desktop mode, ThinClaw auto-expands `learning.auto_apply_classes` to include:

- `memory`
- `skill`
- `prompt`
- `routine`
- `code`

`code` improvements publish through `local_autorollout`, which:

1. syncs a managed source clone under `~/.thinclaw/autonomy/agent-src`
2. creates a detached build worktree under `~/.thinclaw/autonomy/builds/<build-id>`
3. applies the generated patch
4. runs `cargo check`
5. runs `cargo test desktop_autonomy`
6. runs `cargo build`
7. writes a shadow-canary manifest
8. launches the candidate build with the hidden `autonomy-shadow-canary --manifest <path>` entrypoint
9. runs structured canaries
10. promotes the build atomically only if every check passes

Current shadow canaries include:

- `bridge_health`
- `permissions`
- `apps_list`
- `calendar_crud`
- `numbers_open_write_read_export`
- `pages_open_insert_find_export`
- `generic_ui_textedit_fallback`

Rollouts persist:

- build id and build dir
- check results
- canary report path
- bridge/platform/provider metadata
- promoted or failed artifact versions

Repeated failed promotions or canaries pause further code auto-rollout without disabling desktop routines entirely.

## Rollback

`autonomy_control rollback` restores the previous promoted managed build if one exists.

Rollback also records a negative durability observation against the most recent open promoted code-build contract when that outcome path is available.

ThinClaw never self-edits the running checkout in place. Managed self-editing stays inside the autonomy source/build directories.

## Live Smoke Coverage

ThinClaw ships ignored live desktop smoke coverage in:

- [../tests/desktop_autonomy_live_smoke.rs](../tests/desktop_autonomy_live_smoke.rs)

These tests are intentionally out of normal CI and are meant for a sacrificial machine with permissions already granted.

They cover:

- bootstrap or the correct blocking reason
- the 7 structured desktop canaries
- promotion blocker behavior
- rollback restoring the previous promoted build
- both `whole_machine_admin` and `dedicated_user` paths

Run them explicitly on the target host:

```bash
THINCLAW_LIVE_DESKTOP_SMOKE=1 cargo test --test desktop_autonomy_live_smoke -- --ignored
```

## Trust Boundary

Desktop autonomy is a privileged operator feature.

Enabling it means ThinClaw may:

- open and manipulate local applications
- read visible UI state and screenshots
- control input through desktop automation
- persist evidence from those actions
- promote and roll back managed ThinClaw code builds

Use it only on machines and accounts you are intentionally granting that level of control.
