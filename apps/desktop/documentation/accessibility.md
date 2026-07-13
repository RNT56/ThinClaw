# Desktop accessibility contract

Last updated: 2026-07-13

ThinClaw Desktop treats keyboard, focus, screen-reader, contrast, and motion
behavior as product contracts shared by Direct Workbench and Agent Cockpit.

## Shell navigation

- Workbench, Agent Cockpit, and Imagine remain visible and reachable when the
  sidebar is collapsed.
- The product-mode tab list supports arrow keys, Home, End, and direct
  Mod+1/2/3 shortcuts. Mod+K opens the focus-trapped command palette.
- Keyboard focus expands the sidebar; pointer exit must not collapse it while
  focus remains inside.
- Settings destinations expose their current-page state and icon-only buttons
  retain programmatic names.

## Visual and motion behavior

- Every interactive element receives the shared high-contrast focus ring.
- `prefers-reduced-motion` removes non-essential animation and smooth scrolling.
- Forced-colors mode delegates controls and focus outlines to system colors.
- Semantic status colors are never the only status signal; text or an
  accessible label accompanies them.

## Dialogs and asynchronous state

- Modal surfaces use native dialog semantics and restore/trap focus through
  Radix primitives where applicable.
- First-run setup exposes an accessible progress value and modal name.
- Shared loading, empty, error, and progress primitives use live-region,
  alert, and progressbar semantics appropriate to their state.

Component tests cover collapsed navigation availability, roving keyboard
focus, command-palette focus/search behavior, async-state roles, and bounded
progress values. The browser E2E suite remains the full-shell regression gate.
