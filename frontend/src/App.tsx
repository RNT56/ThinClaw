import { ChatLayout } from "./components/chat/ChatLayout";
import { Toaster } from "sonner";
import { useState, useEffect } from "react";
import { OnboardingWizard } from "./components/onboarding/OnboardingWizard";
import { SpotlightBar } from "./components/chat/SpotlightBar";
import * as openclaw from "./lib/openclaw";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

import { ThemeProvider } from "./components/theme-provider";
import { ModelProvider } from "./components/model-context";
import { ChatProvider } from "./components/chat/chat-context";
import { ConfigProvider } from "./components/config-context";

function App() {
  const [showOnboarding, setShowOnboarding] = useState(false);
  const [checked, setChecked] = useState(false);
  const [windowLabel, setWindowLabel] = useState<string>("");

  useEffect(() => {
    checkSetup();
    try {
      const currentWindow = getCurrentWebviewWindow();
      setWindowLabel(currentWindow.label);
    } catch (e) {
      console.warn("getCurrentWebviewWindow() failed (not in Tauri context?):", e);
      setWindowLabel("");
    }
  }, []);

  const checkSetup = async () => {
    try {
      const status = await openclaw.getOpenClawStatus();
      // If status is empty or setup_completed is missing/false, show wizard
      // Also show if dev_mode_wizard is enabled
      if (!status || !status.setup_completed || status.dev_mode_wizard) {
        setShowOnboarding(true);
      }
    } catch (e) {
      console.error("Failed to check setup status:", e);
      // Default to showing if check fails (safer to show than hide)
      setShowOnboarding(true);
    } finally {
      setChecked(true);
    }
  };

  if (!checked) return null; // Or a splash screen

  // If we are in the spotlight window, we have a different layout
  if (windowLabel === "spotlight") {
    // Apply transparent background to document for true transparency
    // Also remove any potential border styling from the window
    const html = document.documentElement;
    const body = document.body;

    html.style.background = 'transparent';
    html.style.backgroundColor = 'transparent';
    body.style.background = 'transparent';
    body.style.backgroundColor = 'transparent';
    body.style.border = 'none';
    body.style.outline = 'none';
    body.style.margin = '0';
    body.style.padding = '0';
    body.classList.add('spotlight-window');

    // Also apply a style element to override any CSS that might add borders
    const styleId = 'spotlight-transparent-override';
    if (!document.getElementById(styleId)) {
      const style = document.createElement('style');
      style.id = styleId;
      style.textContent = `
        html, body, #root {
          background: transparent !important;
          background-color: transparent !important;
          border: none !important;
          outline: none !important;
          box-shadow: none !important;
        }
      `;
      document.head.appendChild(style);
    }

    // Force theme sync by ensuring the ThemeProvider applies themes on mount
    // The ThemeProvider reads from localStorage which is shared across windows

    return (
      <ThemeProvider defaultTheme="system" storageKey="vite-ui-theme">
        <ConfigProvider>
          <ModelProvider>
            <ChatProvider>
              <SpotlightBar />
              <Toaster closeButton position="top-right" />
            </ChatProvider>
          </ModelProvider>
        </ConfigProvider>
      </ThemeProvider>
    );
  }

  return (
    <ThemeProvider defaultTheme="system" storageKey="vite-ui-theme">
      <ConfigProvider>
        <ModelProvider>
          <ChatProvider>
            {showOnboarding ? (
              <OnboardingWizard onComplete={() => setShowOnboarding(false)} />
            ) : (
              <ChatLayout />
            )}
            <Toaster closeButton position="top-right" richColors expand={true} />
          </ChatProvider>
        </ModelProvider>
      </ConfigProvider>
    </ThemeProvider>
  );
}

export default App;
