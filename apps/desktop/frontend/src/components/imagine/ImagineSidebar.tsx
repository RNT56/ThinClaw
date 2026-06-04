import { motion, AnimatePresence } from 'framer-motion';
import { Sparkles, Images } from 'lucide-react';
import { ImagineModeIcon } from '../icons/ModeIcons';
import { cn } from '../../lib/utils';

export type ImagineTab = 'generate' | 'gallery';

interface ImagineSidebarProps {
    sidebarOpen: boolean;
    activeTab: ImagineTab;
    onTabChange: (tab: ImagineTab) => void;
    generationCount?: number;
}

const containerVariants = {
    hidden: { opacity: 0 },
    visible: {
        opacity: 1,
        transition: {
            staggerChildren: 0.05,
        }
    }
};

const itemVariants = {
    hidden: { opacity: 0, x: -10 },
    visible: { opacity: 1, x: 0 },
};

export function ImagineSidebar({
    sidebarOpen,
    activeTab,
    onTabChange,
    generationCount = 0
}: ImagineSidebarProps) {
    const tabs = [
        { id: 'generate' as const, label: 'Generate', icon: ImagineModeIcon, desc: 'Create new images' },
        { id: 'gallery' as const, label: 'Gallery', icon: Images, desc: 'Browse your creations' },
    ];

    return (
        <motion.div
            className="flex flex-col flex-1 h-full overflow-hidden"
            variants={containerVariants}
            initial="hidden"
            animate="visible"
        >
            {/* Header */}
            <motion.div
                variants={itemVariants}
                className={cn(
                    "flex items-center gap-3 transition-all duration-300 h-8 shrink-0",
                    sidebarOpen ? "px-1" : "justify-center px-0"
                )}
            >
                <div className="w-8 h-8 rounded-lg bg-gradient-to-br from-primary/30 to-primary/10 flex items-center justify-center shrink-0">
                    <ImagineModeIcon size={18} isActive={true} />
                </div>
                <AnimatePresence>
                    {sidebarOpen && (
                        <motion.div
                            initial={{ opacity: 0, width: 0 }}
                            animate={{ opacity: 1, width: "auto" }}
                            exit={{ opacity: 0, width: 0 }}
                            className="overflow-hidden"
                        >
                            <span className="font-bold text-base bg-gradient-to-r from-primary via-primary/80 to-primary bg-clip-text text-transparent whitespace-nowrap">
                                Imagine
                            </span>
                        </motion.div>
                    )}
                </AnimatePresence>
            </motion.div>

            {/* Spacing */}
            <div className="h-4 shrink-0" />

            {/* Tab Navigation */}
            <motion.div variants={itemVariants} className="space-y-1 shrink-0">
                {tabs.map((tab) => {
                    const isActive = activeTab === tab.id;
                    const showCount = tab.id === 'gallery' && generationCount > 0;

                    return (
                        <motion.button
                            key={tab.id}
                            variants={itemVariants}
                            onClick={() => onTabChange(tab.id)}
                            className={cn(
                                "flex items-center gap-3 rounded-lg transition-all duration-200 group relative",
                                sidebarOpen ? "w-full px-3 py-2.5" : "w-10 h-10 justify-center mx-auto",
                                isActive
                                    ? "bg-gradient-to-r from-primary/20 to-primary/5 text-primary shadow-sm ring-1 ring-primary/30"
                                    : "text-muted-foreground hover:bg-accent hover:text-foreground"
                            )}
                            title={!sidebarOpen ? tab.label : undefined}
                        >
                            {tab.id === 'generate' ? (
                                <ImagineModeIcon
                                    isActive={isActive}
                                    size={18}
                                    className={cn(
                                        "shrink-0 transition-colors",
                                        isActive ? "text-primary" : "group-hover:text-primary"
                                    )}
                                />
                            ) : (
                                <tab.icon className={cn(
                                    "w-4 h-4 shrink-0 transition-colors",
                                    isActive ? "text-primary" : "group-hover:text-primary"
                                )} />
                            )}
                            <AnimatePresence>
                                {sidebarOpen && (
                                    <motion.div
                                        initial={{ opacity: 0, width: 0 }}
                                        animate={{ opacity: 1, width: "auto" }}
                                        exit={{ opacity: 0, width: 0 }}
                                        className="flex-1 text-left overflow-hidden"
                                    >
                                        <span className={cn(
                                            "text-sm whitespace-nowrap block",
                                            isActive && "font-semibold"
                                        )}>
                                            {tab.label}
                                        </span>
                                        {!isActive && (
                                            <span className="text-[10px] text-muted-foreground whitespace-nowrap block">
                                                {tab.desc}
                                            </span>
                                        )}
                                    </motion.div>
                                )}
                            </AnimatePresence>

                            {/* Count indicator */}
                            {showCount && (
                                <motion.div
                                    initial={{ scale: 0 }}
                                    animate={{ scale: 1 }}
                                    className={cn(
                                        "min-w-[18px] h-[18px] rounded-full bg-primary/20 text-primary text-[10px] font-bold flex items-center justify-center",
                                        !sidebarOpen && "absolute -top-1 -right-1 bg-primary text-primary-foreground text-[9px] min-w-[14px] h-[14px]"
                                    )}
                                >
                                    {generationCount}
                                </motion.div>
                            )}
                        </motion.button>
                    );
                })}
            </motion.div>

            {/* Flex spacer */}
            <div className="flex-1" />

            {/* Footer info - only when sidebar is open */}
            <AnimatePresence>
                {sidebarOpen && (
                    <motion.div
                        initial={{ opacity: 0, y: 10 }}
                        animate={{ opacity: 1, y: 0 }}
                        exit={{ opacity: 0, y: 10 }}
                        className="shrink-0 pb-2"
                    >
                        <div className="flex items-center gap-2 px-2 text-[10px] text-muted-foreground/50">
                            <Sparkles className="w-3 h-3" />
                            <span>Local & Cloud Models</span>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </motion.div>
    );
}
