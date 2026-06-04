import { motion } from 'framer-motion';
import { useChatLayout } from '../ChatProvider';
import { SettingsSidebar, SettingsPage } from '../../settings/SettingsSidebar';

export function SettingsSidebarSlice() {
    const { sidebarOpen, activeTab, setActiveTab } = useChatLayout();

    return (
        <motion.div
            key="settings-sidebar"
            initial={{ opacity: 0, x: 10 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: 10 }}
            transition={{ duration: 0.2 }}
            className="flex flex-col flex-1 h-full"
        >
            <SettingsSidebar
                activePage={activeTab as SettingsPage}
                onPageChange={setActiveTab}
                onBack={() => setActiveTab('chat')}
                sidebarOpen={sidebarOpen}
            />
        </motion.div>
    );
}
