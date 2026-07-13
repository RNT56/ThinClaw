import { Toaster } from "sonner";
import { lazy, Suspense, useState, useEffect, type ReactNode } from "react";
import * as thinclaw from "./lib/thinclaw";
import { UpdateChecker } from "./components/UpdateChecker";
import { ExperimentalBadge } from "./components/ExperimentalBadge";

import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

import { ThemeProvider } from "./components/theme-provider";
import { ModelProvider } from "./components/model-context";
import { ChatProvider } from "./components/chat/chat-context";
import { ConfigProvider } from "./components/config-context";
import { ServicesProvider } from "./components/services-context";
import { recordRendererReady } from "./lib/performance-budgets";
import { I18nProvider, useI18n } from "./components/i18n-provider";

const ChatLayout = lazy(() =>
  import("./components/chat/ChatLayout").then((module) => ({ default: module.ChatLayout })),
);
const OnboardingWizard = lazy(() =>
  import("./components/onboarding/OnboardingWizard").then((module) => ({
    default: module.OnboardingWizard,
  })),
);
const SpotlightBar = lazy(() =>
  import("./components/chat/SpotlightBar").then((module) => ({ default: module.SpotlightBar })),
);

function AppLoadingScreen() {
  const { t } = useI18n();
  return (
    <div className="flex h-screen items-center justify-center bg-background text-sm text-muted-foreground">
      {t("common.loading")}
    </div>
  );
}

function AppProviders({ children }: { children: ReactNode }) {
  return (
    <ServicesProvider>
      <ThemeProvider defaultTheme="system">
        <I18nProvider>
          <ConfigProvider>
            <ModelProvider>
              <ChatProvider>{children}</ChatProvider>
            </ModelProvider>
          </ConfigProvider>
        </I18nProvider>
      </ThemeProvider>
    </ServicesProvider>
  );
}

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

  useEffect(() => {
    if (checked) recordRendererReady();
  }, [checked]);

  const checkSetup = async () => {
    try {
      const status = await thinclaw.getThinClawStatus();
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
      <AppProviders>
        <Suspense fallback={null}>
          <SpotlightBar />
        </Suspense>
        <Toaster closeButton position="top-right" />
      </AppProviders>
    );
  }

  return (
    <AppProviders>
      <Suspense fallback={<AppLoadingScreen />}>
        {showOnboarding ? (
          <OnboardingWizard onComplete={() => setShowOnboarding(false)} />
        ) : (
          <ChatLayout />
        )}
      </Suspense>
      <Toaster closeButton position="top-right" richColors expand={true} />
      <UpdateChecker />
      <ExperimentalBadge />
    </AppProviders>
  );
}

export default App;
