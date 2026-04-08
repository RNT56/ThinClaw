# Agent Rules

## Protected Repo Boundary Policy

- Treat `ThinClaw-main` as a protected codebase by default.
- Do not modify ThinClaw source code unless the user explicitly says the task is about ThinClaw itself, its features, maintenance, fixes, or self-improvement.
- Do not interpret requests for standalone tools, monitors, scrapers, dashboards, coding projects, or experiments as permission to add them to ThinClaw.
- For standalone coding tasks, create or use a separate project/repo/folder outside ThinClaw unless the user explicitly asks for implementation inside ThinClaw.
- For routine-related tasks, prefer updating routine definitions, prompts, configs, stored data, or existing UI surfaces before proposing source-code changes to ThinClaw.
- Do not add new ThinClaw modules, routes, tabs, pages, APIs, background jobs, or source files unless the user explicitly approves ThinClaw code changes for that task.
- If the best solution appears to require changes to ThinClaw, stop and ask for approval before editing the repo.
- Full autonomy, system access, or external service access does not override these repository-boundary rules.

## Feature Parity Update Policy

- If you change implementation status for any feature tracked in `FEATURE_PARITY.md`, update that file in the same branch.
- Do not open a PR that changes feature behavior without checking `FEATURE_PARITY.md` for needed status updates (`❌`, `🚧`, `✅`, notes, and priorities).
