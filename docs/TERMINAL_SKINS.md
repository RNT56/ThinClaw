# Terminal Skins

ThinClaw uses one skin system across local terminal clients, the full-screen TUI, onboarding, setup prompts, human-readable CLI output, and the WebUI.

Local terminal clients use the active CLI skin for palette, prompt symbol, tool labels, boot art, and command presentation. The WebUI follows the active CLI skin by default and can optionally override it with a dedicated WebUI skin.

## Built-In Skins

- `cockpit`
- `midnight`
- `solar`
- `athena`
- `delphi`
- `olympus`

## Runtime Commands

| Command | Use |
|---|---|
| `/skin` | Show the current skin |
| `/skin list` | List available skins |
| `/skin <name>` | Switch to a skin |
| `/skin reset` | Return to the default skin |

## Environment Settings

| Setting | Use |
|---|---|
| `AGENT_CLI_SKIN=<name>` | Persistent CLI/TUI default |
| `WEBCHAT_SKIN=<name>` | WebUI-specific override |
| unset `WEBCHAT_SKIN` | Make WebUI follow `AGENT_CLI_SKIN` |
| `WEBCHAT_ACCENT_COLOR=<hex>` | Legacy accent-only WebUI retint |

`WEBCHAT_ACCENT_COLOR` still works, but it only retints accent surfaces in the WebUI. It does not replace the shared skin identity, tagline, prompt symbol, or tool iconography.

## Custom Skins

Drop custom skin files into:

```text
~/.thinclaw/skins/<name>.toml
```

Skin TOML files support:

- core palette tokens: `accent`, `border`, `body`, `muted`, `good`, `warn`, `bad`, `header`
- prompt symbol: `prompt_symbol`
- skin-specific TUI hero art: `hero_art`
- optional skin subtitle: `tagline`
- tool label embellishments: `tool_emojis`
- optional WebUI aura colors: `[web].aura_primary`, `[web].aura_secondary`

Skins that omit optional TUI or WebUI fields receive safe defaults.

## WebUI QA

For the WebUI validation checklist, see [WEBUI_SKIN_QA.md](WEBUI_SKIN_QA.md).
