import { motion, AnimatePresence } from 'framer-motion';
import { Settings } from 'lucide-react';
import { cn } from '../../lib/utils';
import { ChatModeIcon, OpenClawModeIcon, ImagineModeIcon } from '../icons/ModeIcons';
import { CloudSyncIndicator } from './CloudSyncIndicator';

export type AppMode = 'chat' | 'openclaw' | 'imagine' | 'settings';

interface ModeNavigatorProps {
    activeMode: AppMode;
    onModeChange: (mode: AppMode) => void;
    sidebarOpen: boolean;
    gatewayRunning?: boolean;
}

const MODES = [
    { id: 'chat' as const, label: 'Chat', Icon: ChatModeIcon },
    { id: 'openclaw' as const, label: 'ThinClaw', Icon: OpenClawModeIcon },
    { id: 'imagine' as const, label: 'Imagine', Icon: ImagineModeIcon },
];

export function ModeNavigator({ activeMode, onModeChange, sidebarOpen, gatewayRunning }: ModeNavigatorProps) {
    const isSettingsActive = activeMode === 'settings';

    // Get the active mode (excluding settings) for collapsed view
    const activeModeData = MODES.find(m => m.id === activeMode) || MODES[0];

    // Filter modes to show - when collapsed, only show active mode icon
    const modesToShow = sidebarOpen ? MODES : [activeModeData];

    return (
        <div className="pt-4 border-t border-border/50 space-y-2">
            {/* Mode Icons Row */}
            <motion.div
                className={cn(
                    "flex gap-1.5 transition-all duration-300",
                    sidebarOpen ? "justify-center px-2" : "justify-center"
                )}
                layout
            >
                <AnimatePresence mode="popLayout">
                    {modesToShow.map((mode) => {
                        const isActive = activeMode === mode.id;
                        const showGatewayPulse = mode.id === 'openclaw' && gatewayRunning;

                        return (
                            <motion.button
                                key={mode.id}
                                layout
                                initial={{ opacity: 0, scale: 0.8 }}
                                animate={{ opacity: 1, scale: 1 }}
                                exit={{ opacity: 0, scale: 0.8 }}
                                transition={{ duration: 0.2 }}
                                onClick={() => onModeChange(mode.id)}
                                className={cn(
                                    "relative flex items-center justify-center rounded-xl transition-all duration-300",
                                    "w-10 h-10",
                                    isActive
                                        ? "bg-accent/80 shadow-lg ring-1 ring-primary/20 backdrop-blur-sm"
                                        : "text-muted-foreground hover:text-foreground hover:bg-accent/40"
                                )}
                                title={mode.label}
                            >
                                <mode.Icon
                                    isActive={isActive}
                                    size={22}
                                    className="shrink-0"
                                />

                                {/* Gateway running indicator for the agent runtime */}
                                {showGatewayPulse && (
                                    <motion.div
                                        className="absolute -top-0.5 -right-0.5 w-2.5 h-2.5 rounded-full bg-emerald-500 border-2 border-background"
                                        animate={{ scale: [1, 1.3, 1], opacity: [1, 0.7, 1] }}
                                        transition={{ repeat: Infinity, duration: 2 }}
                                    />
                                )}
                            </motion.button>
                        );
                    })}
                </AnimatePresence>
            </motion.div>

            {/* Cloud Sync Indicator — A5-9 */}
            <CloudSyncIndicator sidebarOpen={sidebarOpen} />

            {/* Settings Button - Full button with text when expanded */}
            <motion.button
                layout
                onClick={() => onModeChange('settings')}
                className={cn(
                    "flex items-center rounded-lg transition-all duration-300",
                    sidebarOpen
                        ? "w-full px-3 h-10"
                        : "w-10 h-10 justify-center mx-auto",
                    isSettingsActive
                        ? "bg-accent text-foreground"
                        : "text-muted-foreground hover:text-foreground hover:bg-accent"
                )}
                title={!sidebarOpen ? "Settings" : undefined}
            >
                <Settings className="w-4 h-4 shrink-0" />
                <AnimatePresence>
                    {sidebarOpen && (
                        <motion.div
                            initial={{ opacity: 0, width: 0 }}
                            animate={{ opacity: 1, width: "auto" }}
                            exit={{ opacity: 0, width: 0 }}
                            className="overflow-hidden flex items-center ml-2"
                        >
                            <span className="whitespace-nowrap text-sm">Settings</span>
                        </motion.div>
                    )}
                </AnimatePresence>
            </motion.button>
        </div>
    );
}
