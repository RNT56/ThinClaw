import { useCallback } from 'react';
import { motion } from 'framer-motion';
import { useChatLayout } from '../ChatProvider';
import { OpenClawSidebar } from '../../openclaw/OpenClawSidebar';
import type { OpenClawPage } from '../../openclaw/OpenClawSidebar';

export function OpenClawSidebarSlice() {
    const {
        sidebarOpen,
        setActiveTab,
        setSelectedOpenClawSession,
        handleNewOpenClawSession,
        selectedOpenClawSession,
        openclawGatewayRunning,
        activeOpenClawPage,
        setActiveOpenClawPage,
    } = useChatLayout();

    // When navigating to the chat page, ensure a session is selected so the
    // send button is never silently blocked by a null effectiveSessionKey.
    const handleSelectPage = useCallback((page: OpenClawPage) => {
        setActiveOpenClawPage(page);
        if (page === 'chat' && !selectedOpenClawSession) {
            setSelectedOpenClawSession('agent:main');
        }
    }, [setActiveOpenClawPage, selectedOpenClawSession, setSelectedOpenClawSession]);

    return (
        <motion.div
            key="openclaw-sidebar"
            initial={{ opacity: 0, x: 10 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: 10 }}
            transition={{ duration: 0.2 }}
            className="flex flex-col flex-1 h-full"
        >
            <OpenClawSidebar
                sidebarOpen={sidebarOpen}
                onBack={() => setActiveTab('chat')}
                onSelectSession={setSelectedOpenClawSession}
                onNewSession={handleNewOpenClawSession}
                selectedSessionKey={selectedOpenClawSession}
                gatewayRunning={openclawGatewayRunning}
                onNavigateToSettings={(page) => setActiveTab(page)}
                activePage={activeOpenClawPage}
                onSelectPage={handleSelectPage}
            />
        </motion.div>
    );
}
