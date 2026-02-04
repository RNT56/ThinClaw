import { ChatLayout } from "./components/chat/ChatLayout";
import { Toaster } from "sonner";
import { useState, useEffect } from "react";
import { OnboardingWizard } from "./components/onboarding/OnboardingWizard";
import { SpotlightBar } from "./components/chat/SpotlightBar";
import * as clawdbot from "./lib/clawdbot";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

import { ThemeProvider } from "./components/theme-provider";
import { ModelProvider } from "./components/model-context";
import { ChatProvider } from "./components/chat/chat-context";

function App() {
  const [showOnboarding, setShowOnboarding] = useState(false);
  const [checked, setChecked] = useState(false);
  const [windowLabel, setWindowLabel] = useState<string>("");

  useEffect(() => {
    checkSetup();
    const currentWindow = getCurrentWebviewWindow();
    setWindowLabel(currentWindow.label);
  }, []);

  const checkSetup = async () => {
    try {
      const config = await clawdbot.getClawdbotConfig();
      // If config is empty or setup_completed is missing/false, show wizard
      if (!config || !(config as any).setup_completed) {
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
    document.documentElement.style.background = 'transparent';
    document.body.style.background = 'transparent';

    return (
      <ThemeProvider defaultTheme="system" storageKey="vite-ui-theme">
        <ModelProvider>
          <ChatProvider>
            <SpotlightBar />
            <Toaster closeButton position="top-right" />
          </ChatProvider>
        </ModelProvider>
      </ThemeProvider>
    );
  }

  return (
    <ThemeProvider defaultTheme="system" storageKey="vite-ui-theme">
      <ModelProvider>
        <ChatProvider>
          {showOnboarding ? (
            <OnboardingWizard onComplete={() => setShowOnboarding(false)} />
          ) : (
            <ChatLayout />
          )}
          <Toaster closeButton />
        </ChatProvider>
      </ModelProvider>
    </ThemeProvider>
  );
}

export default App;
