# Desktop accessibility contract

Last updated: 2026-08-01

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
- The Agent Cockpit has ten primary destinations sourced from one route registry.
  Its sidebar supports ArrowUp/ArrowDown, Home, and End navigation without
  changing the selected profile; legacy destinations resolve to a labeled center
  tab rather than disappearing. Disabled/loading destinations are excluded from
  the roving focus order so keyboard focus always lands on an operable route.
- Agent center tabs use roving keyboard navigation with Arrow keys, Home, and
  End, and expose linked `tablist`, `tab`, and `tabpanel` semantics with one
  keyboard-tab-stop per tab rail.
- Remote profiles keep Desktop-only local files, gateway exposure, and
  Desktop-managed sub-agent controls visibly unavailable with a named Local
  Core remediation; unavailable controls retain programmatic names.

## Visual and motion behavior

- Every interactive element receives the shared high-contrast focus ring.
- `prefers-reduced-motion` removes non-essential animation and smooth scrolling.
- Forced-colors mode delegates controls and focus outlines to system colors.
- Semantic status colors are never the only status signal; text or an
  accessible label accompanies them.

## Dialogs and asynchronous state

- Modal surfaces use native dialog semantics and restore/trap focus through
  Radix primitives where applicable.
- Destructive Agent actions use the shared confirmation dialog, with an explicit
  target in the description and focus returned to the invoking control on close.
- First-run setup exposes an accessible progress value and modal name.
- Shared loading, empty, error, and progress primitives use live-region,
  alert, and progressbar semantics appropriate to their state.

Component tests cover collapsed navigation availability, roving keyboard
focus, command-palette focus/search behavior, async-state roles, bounded
progress values, tab-to-panel relationships, and remote-profile control gates.
The browser E2E suite remains the full-shell regression gate.
