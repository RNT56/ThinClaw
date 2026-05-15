import { motion } from 'framer-motion';
import { ChatProvider, useChatLayout } from './ChatProvider';
import { Sidebar } from './Sidebar';
import { ChatView } from './views/ChatView';
import { ThinClawView } from './views/ThinClawView';
import { ImagineView } from './views/ImagineView';
import { SettingsView } from './views/SettingsView';
import { CanvasWindow } from '../thinclaw/canvas/CanvasWindow';
import { CanvasProviderWrapper } from '../thinclaw/canvas/CanvasProvider';
import { CanvasToolbar } from '../thinclaw/canvas/CanvasToolbar';

// ---------------------------------------------------------------------------
// Shell — consumes ChatProvider context and wires up the root layout
// ---------------------------------------------------------------------------

function ChatLayoutShell() {
    const { isThinClawMode, isImagineMode, isSettingsMode, getInputProps } = useChatLayout();

    return (
        <div className="flex h-screen bg-background text-foreground overflow-hidden font-sans">
            {/* Hidden dropzone input — must be at root so drag events propagate */}
            <input {...(getInputProps as any)()} />

            {/* Collapsible Sidebar (mode-aware) */}
            <Sidebar />

            {/* Main content area — switches between modes */}
            <div className="flex-1 flex flex-col relative h-full overflow-hidden">
                {/* ThinClaw — always mounted to preserve chat state */}
                <div
                    className="flex-1 flex flex-col h-full overflow-hidden"
                    style={{ display: isThinClawMode ? undefined : 'none' }}
                >
                    <ThinClawView />
                </div>

                {/* Imagine — always mounted to preserve generation state */}
                <div
                    className="flex-1 flex flex-col h-full overflow-hidden"
                    style={{ display: isImagineMode ? undefined : 'none' }}
                >
                    <ImagineView />
                </div>

                {/* Settings — conditionally rendered (no state to preserve) */}
                {isSettingsMode && (
                    <motion.div
                        key="settings-area"
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        exit={{ opacity: 0 }}
                        className="flex-1 h-full overflow-hidden"
                    >
                        <SettingsView />
                    </motion.div>
                )}

                {/* Chat — shown when no other mode is active */}
                {!isThinClawMode && !isImagineMode && !isSettingsMode && (
                    <ChatView key="chat-area" />
                )}
            </div>

            {/* Global floating canvas panels + toolbar */}
            <CanvasWindow />
            <CanvasToolbar showAvailability={isThinClawMode} />
        </div>
    );
}

// ---------------------------------------------------------------------------
// Public export — wraps shell in providers
// ---------------------------------------------------------------------------

export function ChatLayout() {
    return (
        <ChatProvider>
            <CanvasProviderWrapper>
                <ChatLayoutShell />
            </CanvasProviderWrapper>
        </ChatProvider>
    );
}
