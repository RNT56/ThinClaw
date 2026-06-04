import { useCallback } from 'react';
import { motion } from 'framer-motion';
import { useChatLayout } from '../ChatProvider';
import { ThinClawSidebar } from '../../thinclaw/ThinClawSidebar';
import type { ThinClawPage } from '../../thinclaw/ThinClawSidebar';

export function ThinClawSidebarSlice() {
    const {
        sidebarOpen,
        setActiveTab,
        setSelectedThinClawSession,
        handleNewThinClawSession,
        selectedThinClawSession,
        thinclawGatewayRunning,
        activeThinClawPage,
        setActiveThinClawPage,
    } = useChatLayout();

    // When navigating to the chat page, ensure a session is selected so the
    // send button is never silently blocked by a null effectiveSessionKey.
    const handleSelectPage = useCallback((page: ThinClawPage) => {
        setActiveThinClawPage(page);
        if (page === 'chat' && !selectedThinClawSession) {
            setSelectedThinClawSession('agent:main');
        }
    }, [setActiveThinClawPage, selectedThinClawSession, setSelectedThinClawSession]);

    return (
        <motion.div
            key="thinclaw-sidebar"
            initial={{ opacity: 0, x: 10 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: 10 }}
            transition={{ duration: 0.2 }}
            className="flex flex-col flex-1 h-full"
        >
            <ThinClawSidebar
                sidebarOpen={sidebarOpen}
                onBack={() => setActiveTab('chat')}
                onSelectSession={setSelectedThinClawSession}
                onNewSession={handleNewThinClawSession}
                selectedSessionKey={selectedThinClawSession}
                gatewayRunning={thinclawGatewayRunning}
                onNavigateToSettings={(page) => setActiveTab(page)}
                activePage={activeThinClawPage}
                onSelectPage={handleSelectPage}
            />
        </motion.div>
    );
}
