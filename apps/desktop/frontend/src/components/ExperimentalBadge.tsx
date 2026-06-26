/**
 * Small, unobtrusive "Experimental" marker for ThinClaw Desktop.
 *
 * The desktop app is under heavy active development and some surfaces were built
 * for an earlier agent framework, so it currently ships flagged experimental to
 * set expectations. Rendered only in the main window (not the spotlight window).
 *
 * Inline styles (not Tailwind classes) so it renders identically regardless of
 * the app theme / Tailwind JIT.
 */
export function ExperimentalBadge() {
  return (
    <div
      style={{
        position: "fixed",
        bottom: 8,
        right: 8,
        zIndex: 50,
        pointerEvents: "none",
      }}
    >
      <span
        title="ThinClaw Desktop is experimental and under active development. Some features were built for an earlier agent framework and are still being migrated."
        style={{
          display: "inline-block",
          padding: "2px 8px",
          borderRadius: 9999,
          fontSize: 10,
          fontWeight: 600,
          letterSpacing: "0.04em",
          textTransform: "uppercase",
          color: "#b45309",
          background: "rgba(245, 158, 11, 0.15)",
          border: "1px solid rgba(245, 158, 11, 0.4)",
          cursor: "help",
          userSelect: "none",
          pointerEvents: "auto",
        }}
      >
        Experimental
      </span>
    </div>
  );
}
