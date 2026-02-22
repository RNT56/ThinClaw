import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";
import "sonner/dist/styles.css";

// ---------------------------------------------------------------------------
// Error Boundary — catches render errors that would otherwise leave a blank page
// ---------------------------------------------------------------------------
class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { error: Error | null }
> {
  constructor(props: { children: React.ReactNode }) {
    super(props);
    this.state = { error: null };
  }

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("[ErrorBoundary] React tree crashed:", error, info.componentStack);
  }

  render() {
    if (this.state.error) {
      return (
        <div style={{
          padding: 32, fontFamily: 'monospace', color: '#ff4444',
          background: '#1a1a1a', height: '100vh', overflow: 'auto'
        }}>
          <h1 style={{ color: '#ff6666' }}>⚠️ Application Error</h1>
          <pre style={{ whiteSpace: 'pre-wrap', wordBreak: 'break-all' }}>
            {this.state.error.message}
          </pre>
          <pre style={{ fontSize: 12, color: '#888', whiteSpace: 'pre-wrap', marginTop: 16 }}>
            {this.state.error.stack}
          </pre>
          <button
            onClick={() => window.location.reload()}
            style={{
              marginTop: 16, padding: '8px 16px', cursor: 'pointer',
              background: '#333', color: '#fff', border: '1px solid #555', borderRadius: 4
            }}
          >
            Reload
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

// ---------------------------------------------------------------------------
// Mount
// ---------------------------------------------------------------------------
try {
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <ErrorBoundary>
        <App />
      </ErrorBoundary>
    </React.StrictMode>,
  );
} catch (e) {
  // If even the initial render throws (e.g. module-level import failure),
  // display something so the user doesn't stare at a blank page.
  console.error("[main.tsx] Failed to mount React app:", e);
  const root = document.getElementById("root");
  if (root) {
    root.innerHTML = `
      <div style="padding:32px;font-family:monospace;color:#ff4444;background:#1a1a1a;height:100vh">
        <h1 style="color:#ff6666">⚠️ Failed to start application</h1>
        <pre style="white-space:pre-wrap;word-break:break-all">${e instanceof Error ? e.message : String(e)}</pre>
        <pre style="font-size:12px;color:#888;white-space:pre-wrap;margin-top:16px">${e instanceof Error ? e.stack : ''}</pre>
      </div>
    `;
  }
}
