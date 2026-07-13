import { motion } from 'framer-motion';
import { lazy, Suspense, useEffect, useState } from 'react';
import { ChatProvider, useChatLayout } from './ChatProvider';
import { Sidebar } from './Sidebar';
import { CanvasWindow } from '../thinclaw/canvas/CanvasWindow';
import { CanvasProviderWrapper } from '../thinclaw/canvas/CanvasProvider';
import { CanvasToolbar } from '../thinclaw/canvas/CanvasToolbar';
import { CommandPalette } from '../navigation/CommandPalette';

const ChatView = lazy(() => import('./views/ChatView').then((module) => ({ default: module.ChatView })));
const ThinClawView = lazy(() => import('./views/ThinClawView').then((module) => ({ default: module.ThinClawView })));
const ImagineView = lazy(() => import('./views/ImagineView').then((module) => ({ default: module.ImagineView })));
const SettingsView = lazy(() => import('./views/SettingsView').then((module) => ({ default: module.SettingsView })));

function ViewSkeleton() {
    return <div className="flex h-full items-center justify-center text-sm text-muted-foreground">Loading view…</div>;
}

// ---------------------------------------------------------------------------
// Shell — consumes ChatProvider context and wires up the root layout
// ---------------------------------------------------------------------------

function ChatLayoutShell() {
    const {
        isThinClawMode,
        isImagineMode,
        isSettingsMode,
        getInputProps,
        commandPaletteOpen,
        setCommandPaletteOpen,
        setActiveTab,
    } = useChatLayout();
    const [mountedModes, setMountedModes] = useState(() => ({
        thinclaw: isThinClawMode,
        imagine: isImagineMode,
    }));

    useEffect(() => {
        if (isThinClawMode) {
            setMountedModes((current) => current.thinclaw ? current : { ...current, thinclaw: true });
        }
        if (isImagineMode) {
            setMountedModes((current) => current.imagine ? current : { ...current, imagine: true });
        }
    }, [isImagineMode, isThinClawMode]);

    return (
        <div className="flex h-screen bg-background text-foreground overflow-hidden font-sans">
            {/* Hidden dropzone input — must be at root so drag events propagate */}
            <input {...(getInputProps as any)()} />

            {/* Collapsible Sidebar (mode-aware) */}
            <Sidebar />

            {/* Main content area — switches between modes */}
            <div className="flex-1 flex flex-col relative h-full overflow-hidden">
                {/* ThinClaw — always mounted to preserve chat state */}
                {mountedModes.thinclaw && (
                    <div
                        className="flex-1 flex flex-col h-full overflow-hidden"
                        style={{ display: isThinClawMode ? undefined : 'none' }}
                    >
                        <Suspense fallback={<ViewSkeleton />}><ThinClawView /></Suspense>
                    </div>
                )}

                {/* Imagine — always mounted to preserve generation state */}
                {mountedModes.imagine && (
                    <div
                        className="flex-1 flex flex-col h-full overflow-hidden"
                        style={{ display: isImagineMode ? undefined : 'none' }}
                    >
                        <Suspense fallback={<ViewSkeleton />}><ImagineView /></Suspense>
                    </div>
                )}

                {/* Settings — conditionally rendered (no state to preserve) */}
                {isSettingsMode && (
                    <motion.div
                        key="settings-area"
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        exit={{ opacity: 0 }}
                        className="flex-1 h-full overflow-hidden"
                    >
                        <Suspense fallback={<ViewSkeleton />}><SettingsView /></Suspense>
                    </motion.div>
                )}

                {/* Chat — shown when no other mode is active */}
                {!isThinClawMode && !isImagineMode && !isSettingsMode && (
                    <Suspense fallback={<ViewSkeleton />}><ChatView key="chat-area" /></Suspense>
                )}
            </div>

            {/* Global floating canvas panels + toolbar */}
            <CanvasWindow />
            <CanvasToolbar showAvailability={isThinClawMode} />
            <CommandPalette
                open={commandPaletteOpen}
                onOpenChange={setCommandPaletteOpen}
                onModeChange={setActiveTab}
                onSettingsChange={setActiveTab}
            />
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
