import { ChatLayout } from "./components/chat/ChatLayout";
import { Toaster } from "sonner";
import { useState, useEffect } from "react";
import { OnboardingWizard } from "./components/onboarding/OnboardingWizard";
import * as clawdbot from "./lib/clawdbot";

import { ThemeProvider } from "./components/theme-provider";
import { ModelProvider } from "./components/model-context";
import { ChatProvider } from "./components/chat/chat-context";

function App() {
  const [showOnboarding, setShowOnboarding] = useState(false);
  const [checked, setChecked] = useState(false);

  useEffect(() => {
    checkSetup();
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

  return (
    <ThemeProvider defaultTheme="dark" storageKey="vite-ui-theme">
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
