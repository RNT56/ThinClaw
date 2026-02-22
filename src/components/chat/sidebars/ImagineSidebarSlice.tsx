import { motion } from 'framer-motion';
import { useChatLayout } from '../ChatProvider';
import { ImagineSidebar } from '../../imagine';

export function ImagineSidebarSlice() {
    const { sidebarOpen, activeImagineTab, setActiveImagineTab } = useChatLayout();

    return (
        <motion.div
            key="imagine-sidebar"
            initial={{ opacity: 0, x: 10 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: 10 }}
            transition={{ duration: 0.2 }}
            className="flex flex-col flex-1 h-full"
        >
            <ImagineSidebar
                sidebarOpen={sidebarOpen}
                activeTab={activeImagineTab}
                onTabChange={setActiveImagineTab}
            />
        </motion.div>
    );
}
