import { motion, AnimatePresence } from 'framer-motion';
import { lazy, Suspense } from 'react';
import { PersonaTab } from './PersonaTab';
import { PersonalizationTab } from './PersonalizationTab';
import { SlackTab } from './SlackTab';
import { TelegramTab } from './TelegramTab';
import { ChatProviderTab } from './ChatProviderTab';
import { SettingsPage } from './SettingsSidebar';
import { ServerSettings } from './ServerSettings';
import { TroubleshootingSettings } from './TroubleshootingSettings';
import { AppearanceSettings } from './AppearanceSettings';

// Heavy tabs — code-split so they don't inflate the initial bundle chunk.
// SecretsTab ~64KB, GatewayTab ~68KB, ModelBrowser ~63KB
const ModelBrowser = lazy(() => import('./ModelBrowser').then(m => ({ default: m.ModelBrowser })));
const GatewayTab = lazy(() => import('./GatewayTab').then(m => ({ default: m.GatewayTab })));
const SecretsTab = lazy(() => import('./SecretsTab').then(m => ({ default: m.SecretsTab })));
const McpTab = lazy(() => import('./McpTab').then(m => ({ default: m.McpTab })));
const InferenceModeTab = lazy(() => import('./InferenceModeTab').then(m => ({ default: m.InferenceModeTab })));
const StorageTab = lazy(() => import('./StorageTab').then(m => ({ default: m.StorageTab })));
import {
    Cpu,
    Server,
    Settings,
    KeyRound,
    Radio,
    ShieldAlert,
    Sparkles,
    Plug,
    Cloud
} from 'lucide-react';

interface SettingsContentProps {
    activePage: SettingsPage;
}

export function SettingsContent({ activePage }: SettingsContentProps) {
    return (
        <div className="flex-1 h-full overflow-hidden flex flex-col bg-background/50 backdrop-blur-sm">
            <AnimatePresence mode="wait">
                <motion.div
                    key={activePage}
                    initial={{ opacity: 0, y: 10 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: -10 }}
                    transition={{ duration: 0.2 }}
                    className="flex-1 overflow-y-auto p-8 max-w-5xl mx-auto w-full"
                >
                    <PageHeader page={activePage} />

                    <div className="mt-8">
                        {activePage === 'models' && <Suspense fallback={<TabSkeleton />}><ModelBrowser /></Suspense>}
                        {activePage === 'persona' && <PersonaTab />}
                        {activePage === 'personalization' && <PersonalizationTab />}
                        {activePage === 'server' && <ServerSettings />}
                        {activePage === 'troubleshooting' && <TroubleshootingSettings />}
                        {activePage === 'appearance' && <AppearanceSettings />}
                        {activePage === 'openclaw-slack' && <SlackTab />}
                        {activePage === 'openclaw-telegram' && <TelegramTab />}
                        {activePage === 'openclaw-gateway' && <Suspense fallback={<TabSkeleton />}><GatewayTab /></Suspense>}
                        {activePage === 'secrets' && <Suspense fallback={<TabSkeleton />}><SecretsTab /></Suspense>}
                        {activePage === 'inference' && <ChatProviderTab />}
                        {activePage === 'inference-mode' && <Suspense fallback={<TabSkeleton />}><InferenceModeTab /></Suspense>}
                        {activePage === 'mcp' && <Suspense fallback={<TabSkeleton />}><McpTab /></Suspense>}
                        {activePage === 'cloud-storage' && <Suspense fallback={<TabSkeleton />}><StorageTab /></Suspense>}
                    </div>
                </motion.div>
            </AnimatePresence>
        </div>
    );
}

/** Minimal loading skeleton shown while a lazy tab chunk is being fetched. */
function TabSkeleton() {
    return (
        <div className="space-y-4 animate-pulse pt-2">
            {[1, 2, 3].map(i => (
                <div key={i} className="h-20 rounded-xl bg-muted/40 border border-border/30" />
            ))}
        </div>
    );
}

function PageHeader({ page }: { page: SettingsPage }) {
    const titles: Record<SettingsPage, { title: string, description: string, icon: any }> = {
        models: {
            title: "Model Management",
            description: "Download and configure your local LLMs, Vision, and Image models.",
            icon: Cpu
        },
        inference: {
            title: "Chat Provider",
            description: "Select the primary intelligence engine for your workspace.",
            icon: Radio
        },
        persona: {
            title: "My Persona",
            description: "Define how the AI perceives itself and interacts with you.",
            icon: Settings
        },
        personalization: {
            title: "Global Instructions",
            description: "Custom system instructions and memory preferences.",
            icon: Settings
        },
        server: {
            title: "Server & Memory",
            description: "Monitor system performance and adjust inference parameters.",
            icon: Server
        },
        troubleshooting: {
            title: "Troubleshooting",
            description: "Diagnostic tools and access to configuration files.",
            icon: ShieldAlert
        },
        appearance: {
            title: "Appearance",
            description: "Customize the look and feel of your workspace.",
            icon: Settings
        },
        'openclaw-slack': {
            title: "Slack Integration",
            description: "Connect ThinClaw to your Slack workspace.",
            icon: Settings
        },
        'openclaw-telegram': {
            title: "Telegram Integration",
            description: "Connect ThinClaw to Telegram.",
            icon: Settings
        },
        'openclaw-gateway': {
            title: "ThinClaw Gateway",
            description: "Manage autonomy, connectivity and agent runtime.",
            icon: Radio
        },
        'secrets': {
            title: "API Secrets",
            description: "Manage API keys for cloud providers.",
            icon: KeyRound
        },
        'inference-mode': {
            title: "Inference Mode",
            description: "Configure which backend powers each AI modality — local, cloud, or hybrid.",
            icon: Sparkles
        },
        'mcp': {
            title: "MCP Server",
            description: "Connect your FastAPI MCP server to unlock remote tools, finance, news, and domain-specific capabilities for the AI agent.",
            icon: Plug
        },
        'cloud-storage': {
            title: "Cloud Storage",
            description: "Encrypt and sync your data to a cloud provider. Migrate seamlessly between local and cloud modes.",
            icon: Cloud
        }
    };

    const entry = titles[page];
    if (!entry) return null; // Not a settings page (e.g. 'chat', 'openclaw', 'imagine')
    const { title, description, icon: Icon } = entry;

    return (
        <div className="border-b border-border/50 pb-6">
            <div className="flex items-center gap-3 mb-2">
                <div className="p-2 bg-primary/10 rounded-lg">
                    <Icon className="w-6 h-6 text-primary" />
                </div>
                <h1 className="text-3xl font-bold tracking-tight">{title}</h1>
            </div>
            <p className="text-muted-foreground">{description}</p>
        </div>
    );
}
