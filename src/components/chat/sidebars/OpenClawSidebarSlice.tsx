import { motion } from 'framer-motion';
import { useChatLayout } from '../ChatProvider';
import { OpenClawSidebar } from '../../openclaw/OpenClawSidebar';

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
                onSelectPage={setActiveOpenClawPage}
            />
        </motion.div>
    );
}
