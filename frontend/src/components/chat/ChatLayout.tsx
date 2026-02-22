import { AnimatePresence, motion } from 'framer-motion';
import { ChatProvider, useChatLayout } from './ChatProvider';
import { Sidebar } from './Sidebar';
import { ChatView } from './views/ChatView';
import { OpenClawView } from './views/OpenClawView';
import { ImagineView } from './views/ImagineView';
import { SettingsView } from './views/SettingsView';
import { CanvasWindow } from '../openclaw/canvas/CanvasWindow';

// ---------------------------------------------------------------------------
// Shell — consumes ChatProvider context and wires up the root layout
// ---------------------------------------------------------------------------

function ChatLayoutShell() {
    const { isOpenClawMode, isImagineMode, isSettingsMode, getInputProps } = useChatLayout();

    return (
        <div className="flex h-screen bg-background text-foreground overflow-hidden font-sans">
            {/* Hidden dropzone input — must be at root so drag events propagate */}
            <input {...(getInputProps as any)()} />

            {/* Collapsible Sidebar (mode-aware) */}
            <Sidebar />

            {/* Main content area — switches between modes */}
            <div className="flex-1 flex flex-col relative h-full overflow-hidden">
                <AnimatePresence mode="wait">
                    {isOpenClawMode ? (
                        <motion.div
                            key="openclaw-area"
                            initial={{ opacity: 0 }}
                            animate={{ opacity: 1 }}
                            exit={{ opacity: 0 }}
                            className="flex-1 flex flex-col h-full overflow-hidden"
                        >
                            <OpenClawView />
                        </motion.div>
                    ) : isImagineMode ? (
                        <motion.div
                            key="imagine-area"
                            initial={{ opacity: 0 }}
                            animate={{ opacity: 1 }}
                            exit={{ opacity: 0 }}
                            className="flex-1 flex flex-col h-full overflow-hidden"
                        >
                            <ImagineView />
                        </motion.div>
                    ) : isSettingsMode ? (
                        <motion.div
                            key="settings-area"
                            initial={{ opacity: 0 }}
                            animate={{ opacity: 1 }}
                            exit={{ opacity: 0 }}
                            className="flex-1 h-full overflow-hidden"
                        >
                            <SettingsView />
                        </motion.div>
                    ) : (
                        <ChatView key="chat-area" />
                    )}
                </AnimatePresence>
            </div>

            {/* Global floating canvas window */}
            <CanvasWindow />
        </div>
    );
}

// ---------------------------------------------------------------------------
// Public export — wraps shell in provider
// ---------------------------------------------------------------------------

export function ChatLayout() {
    return (
        <ChatProvider>
            <ChatLayoutShell />
        </ChatProvider>
    );
}
