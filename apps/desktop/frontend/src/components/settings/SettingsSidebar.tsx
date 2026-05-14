import { motion, AnimatePresence } from 'framer-motion';
import {
    Cpu,
    UserCircle,
    Server,
    ChevronLeft,
    Palette,
    ShieldAlert,
    Layout,
    Info,
    MessageSquare,
    Send,
    Radio,
    KeyRound,
    Plug,
    Sparkles,
    Cloud
} from 'lucide-react';
import { cn } from '../../lib/utils';

export type SettingsPage = 'models' | 'inference' | 'inference-mode' | 'persona' | 'personalization' | 'server' | 'troubleshooting' | 'appearance' | 'openclaw-slack' | 'openclaw-telegram' | 'openclaw-gateway' | 'secrets' | 'mcp' | 'cloud-storage';

interface SettingsSidebarProps {
    activePage: SettingsPage;
    onPageChange: (page: SettingsPage) => void;
    onBack: () => void;
    sidebarOpen: boolean;
}

const NAV_ITEMS = [
    {
        section: "General",
        items: [
            { id: 'models', label: 'Models', icon: Cpu },
            { id: 'inference', label: 'Chat Provider', icon: Radio },
            { id: 'inference-mode', label: 'Inference Mode', icon: Sparkles },
            { id: 'secrets', label: 'Secrets', icon: KeyRound },
            { id: 'mcp', label: 'MCP Server', icon: Plug },
            { id: 'appearance', label: 'Appearance', icon: Palette },
        ]
    },
    {
        section: "AI Personality",
        items: [
            { id: 'persona', label: 'My Persona', icon: UserCircle },
            { id: 'personalization', label: 'Global Instructions', icon: Layout },
        ]
    },
    {
        section: "OpenClaw",
        items: [
            { id: 'openclaw-slack', label: 'Slack', icon: MessageSquare },
            { id: 'openclaw-telegram', label: 'Telegram', icon: Send },
            { id: 'openclaw-gateway', label: 'Gateway', icon: Radio },
        ]
    },
    {
        section: "Storage",
        items: [
            { id: 'cloud-storage', label: 'Cloud Storage', icon: Cloud },
        ]
    },
    {
        section: "System",
        items: [
            { id: 'server', label: 'Server & Memory', icon: Server },
            { id: 'troubleshooting', label: 'Troubleshooting', icon: ShieldAlert },
        ]
    }
];

const containerVariants = {
    hidden: { opacity: 0 },
    visible: {
        opacity: 1,
        transition: {
            staggerChildren: 0.05,
        }
    },
    exit: {
        opacity: 0,
    }
};

const itemVariants = {
    hidden: { opacity: 0 },
    visible: { opacity: 1 },
    exit: { opacity: 0 }
};

export function SettingsSidebar({ activePage, onPageChange, onBack, sidebarOpen }: SettingsSidebarProps) {
    return (
        <motion.div
            className="flex flex-col w-full h-full"
            variants={containerVariants}
            initial="hidden"
            animate="visible"
            exit="exit"
        >
            <motion.button
                variants={itemVariants}
                onClick={onBack}
                className={cn(
                    "flex items-center text-muted-foreground hover:text-foreground mb-6 transition-all duration-300 group rounded-lg hover:bg-accent shrink-0 h-10 px-3",
                    sidebarOpen ? "w-full" : "w-10 mx-auto"
                )}
            >
                <ChevronLeft className="w-5 h-5 transition-transform duration-300 group-hover:-translate-x-0.5 shrink-0" />
                <AnimatePresence>
                    {sidebarOpen && (
                        <motion.span
                            initial={{ opacity: 0, width: 0, marginLeft: 0 }}
                            animate={{ opacity: 1, width: "auto", marginLeft: 8 }}
                            exit={{ opacity: 0, width: 0, marginLeft: 0 }}
                            transition={{ duration: 0.3, ease: "easeInOut" }}
                            className="font-semibold whitespace-nowrap overflow-hidden"
                        >
                            Back to Chat
                        </motion.span>
                    )}
                </AnimatePresence>
            </motion.button>

            <div className="flex-1 space-y-6 overflow-y-auto scrollbar-hide">
                {NAV_ITEMS.map((section) => (
                    <div key={section.section} className="space-y-2">
                        <AnimatePresence>
                            {sidebarOpen && (
                                <motion.div
                                    initial={{ opacity: 0, height: 0 }}
                                    animate={{ opacity: 1, height: "auto" }}
                                    exit={{ opacity: 0, height: 0 }}
                                    className="px-2 text-[10px] font-bold text-muted-foreground/50 uppercase tracking-widest overflow-hidden whitespace-nowrap"
                                >
                                    {section.section}
                                </motion.div>
                            )}
                        </AnimatePresence>
                        <div className="space-y-1">
                            {section.items.map((item) => {
                                const Icon = item.icon;
                                const isActive = activePage === item.id;
                                return (
                                    <motion.button
                                        key={item.id}
                                        variants={itemVariants}
                                        onClick={() => onPageChange(item.id as SettingsPage)}
                                        className={cn(
                                            "flex items-center rounded-lg text-sm transition-all duration-300 group w-full px-3 h-10 shrink-0",
                                            isActive
                                                ? "bg-accent text-foreground font-semibold shadow-sm ring-1 ring-primary/20"
                                                : "hover:bg-accent text-muted-foreground hover:text-foreground",
                                            sidebarOpen ? "w-full" : "w-10 mx-auto"
                                        )}
                                        title={!sidebarOpen ? item.label : undefined}
                                    >
                                        <Icon className={cn("w-4 h-4 shrink-0 transition-colors duration-300", isActive ? "text-primary" : "group-hover:text-primary")} />
                                        <AnimatePresence>
                                            {sidebarOpen && (
                                                <motion.span
                                                    initial={{ opacity: 0, width: 0, marginLeft: 0 }}
                                                    animate={{ opacity: 1, width: "auto", marginLeft: 8 }}
                                                    exit={{ opacity: 0, width: 0, marginLeft: 0 }}
                                                    transition={{ duration: 0.3, ease: "easeInOut" }}
                                                    className="font-medium whitespace-nowrap overflow-hidden"
                                                >
                                                    {item.label}
                                                </motion.span>
                                            )}
                                        </AnimatePresence>
                                    </motion.button>
                                );
                            })}
                        </div>
                    </div>
                ))}
            </div>

            <AnimatePresence>
                {sidebarOpen && (
                    <motion.div
                        initial={{ opacity: 0, y: 10 }}
                        animate={{ opacity: 1, y: 0 }}
                        exit={{ opacity: 0, y: 10 }}
                        className="mt-auto pt-4 border-t border-border/50 text-[10px] text-muted-foreground/50 flex items-center gap-1 px-2 whitespace-nowrap overflow-hidden"
                    >
                        <Info className="w-3 h-3 shrink-0" />
                        <span>ThinClaw Desktop Settings v1.2</span>
                    </motion.div>
                )}
            </AnimatePresence>
        </motion.div>
    );
}
