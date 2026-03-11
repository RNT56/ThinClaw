import { AnimatePresence } from 'framer-motion';
import { cn } from '../../lib/utils';
import { useChatLayout } from './ChatProvider';
import { ModeNavigator } from '../navigation/ModeNavigator';
import { ChatSidebar } from './sidebars/ChatSidebar';
import { OpenClawSidebarSlice } from './sidebars/OpenClawSidebarSlice';
import { ImagineSidebarSlice } from './sidebars/ImagineSidebarSlice';
import { SettingsSidebarSlice } from './sidebars/SettingsSidebarSlice';

export function Sidebar() {
    const {
        sidebarOpen,
        setSidebarOpen,
        activeTab,
        appMode,
        isOpenClawMode,
        isImagineMode,
        isSettingsMode,
        openclawGatewayRunning,
        setActiveTab,
    } = useChatLayout();

    return (
        <div
            className={cn(
                "border-r border-border bg-card/50 backdrop-blur flex flex-col transition-all duration-300 relative z-20 overflow-hidden h-full",
                sidebarOpen ? "w-64 p-4" : "w-16 p-2"
            )}
            onMouseEnter={() => setSidebarOpen(true)}
            onMouseLeave={() => setSidebarOpen(false)}
            onDragEnter={() => setSidebarOpen(true)}
            onDragOver={(e) => e.preventDefault()}
        >
            {/* Scrollable sidebar content — fills all space above the bottom bar */}
            <div className="flex-1 min-h-0 overflow-hidden">
                <AnimatePresence mode="wait">
                    {activeTab === 'chat' ? (
                        <ChatSidebar key="chat-sidebar" />
                    ) : isOpenClawMode ? (
                        <OpenClawSidebarSlice key="openclaw-sidebar" />
                    ) : isImagineMode ? (
                        <ImagineSidebarSlice key="imagine-sidebar" />
                    ) : isSettingsMode ? (
                        <SettingsSidebarSlice key="settings-sidebar" />
                    ) : null}
                </AnimatePresence>
            </div>

            {/* Mode Navigator — always pinned to the bottom */}
            <div className="shrink-0">
                <ModeNavigator
                    activeMode={appMode}
                    onModeChange={(mode) => {
                        if (mode === 'settings') {
                            setActiveTab('models');
                        } else {
                            setActiveTab(mode);
                        }
                    }}
                    sidebarOpen={sidebarOpen}
                    gatewayRunning={openclawGatewayRunning}
                />
            </div>
        </div>
    );
}
