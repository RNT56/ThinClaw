# WebUI Skin QA Matrix

## Intent

ThinClaw now treats the CLI skin system as the shared brand source for both terminal clients and the WebUI. The WebUI follows `AGENT_CLI_SKIN` by default and may optionally override that with `WEBCHAT_SKIN`.

`WEBCHAT_ACCENT_COLOR` remains supported as a legacy accent-only override. It must not replace the resolved skin name, tagline, prompt symbol, or tool emoji mapping.

## Configuration Matrix

1. Default follow behavior
   Set `AGENT_CLI_SKIN=cockpit` and leave `WEBCHAT_SKIN` unset.
   Expected: WebUI chrome, auth tagline, chat composer prompt chip, tool emoji labels, and branding pill all reflect `cockpit`.

2. Explicit WebUI override
   Set `AGENT_CLI_SKIN=midnight` and `WEBCHAT_SKIN=athena`.
   Expected: CLI/TUI remain `midnight`; WebUI surfaces render `athena`.

3. Clear override back to follow mode
   Set `WEBCHAT_SKIN=athena`, then clear the setting from the WebUI Presentation section.
   Expected: WebUI reloads and returns to the active CLI skin.

4. Legacy accent override
   Set `AGENT_CLI_SKIN=cockpit`, leave `WEBCHAT_SKIN` unset, and set `WEBCHAT_ACCENT_COLOR=#22c55e`.
   Expected: accent highlights retint, but skin name, tagline, prompt symbol, and tool iconography still come from `cockpit`.

5. User skin without `[web]`
   Load a custom user skin from `~/.thinclaw/skins/` that omits `[web]`.
   Expected: CLI skin loads successfully and WebUI aura colors are derived automatically.

## Surface Checks

1. Auth screen
   Verify the subtitle uses the resolved skin tagline.
   Verify the card and page background show subtle aura treatment, not a flat generic panel.

2. Top chrome
   Verify the brand chip shows `ThinClaw`, the resolved skin name, and the active prompt symbol.
   Verify the active tab state uses the resolved accent colors.

3. Sidebar and thread list
   Verify active thread rows pick up the resolved accent treatment.
   Verify assistant subsession cards inherit the updated branded styling.

4. Chat transcript
   Verify user messages use `chatUserBg` and `chatUserFg`.
   Verify assistant messages stay neutral but adopt the skin border treatment.
   Verify system messages remain centered ribbons.
   Verify role kickers render for user, assistant, and system entries.

5. Composer
   Verify the prompt chip shows the resolved skin prompt symbol.
   Verify textarea focus and button states use the resolved accent variables.

6. Tool activity
   Verify mapped tools show skin emoji when available.
   Verify unknown tools still render generic status icons.
   Verify tool cards and transcript blocks set `data-tool-kind` values for `shell`, `browser`, `memory`, `search_files`, `todo`, `subagent`, and `default`.
   Verify the collapsed summary reads `Tool pass · {count} tool(s) · {duration}`.

7. Approval and auth inline cards
   Verify these cards adopt the refreshed branded chrome without losing readability.

8. Subsession panel
   Verify empty, running, and completed states all use the updated skin-driven visual treatment.

## Settings and Setup Checks

1. WebUI Presentation settings
   Verify `agent.cli_skin`, `webchat_skin`, `webchat_theme`, and `webchat_show_branding` appear in the Presentation section under General.
   Verify `webchat_skin` includes a nullable `Follow agent skin` option.
   Verify `WEBCHAT_ACCENT_COLOR` does not appear in the Presentation section.

2. Persistence
   Verify saving `webchat_skin` stores `WEBCHAT_SKIN`.
   Verify clearing `webchat_skin` removes `WEBCHAT_SKIN`.
   Verify `WEBCHAT_THEME` and `WEBCHAT_SHOW_BRANDING` persist as before.

3. Setup wizard
   Verify the wizard asks for follow-vs-override skin mode, theme, and branding visibility.
   Verify the wizard no longer asks for a freeform accent color in the normal flow.
   Verify the setup summary reports follow mode or explicit skin name, theme, branding state, and accent override only when one exists.

## Layout Regression Checks

1. Mobile
   Verify the brand chip, chat composer, thread sidebar, and subsession panel remain usable at narrow widths.

2. Dense operational tabs
   Verify Jobs, Routines, Providers, Costs, and Settings remain readable and do not become overly decorative.

3. Existing behavior
   Verify SSE streaming, history loading, thread switching, tool event rendering, and settings save/reset flows still behave as before.
