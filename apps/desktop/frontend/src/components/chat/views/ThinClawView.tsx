import { lazy, Suspense, useEffect, useRef, useState } from 'react';
import { X } from 'lucide-react';

import { useChatLayout } from '../ChatProvider';
import * as thinclaw from '../../../lib/thinclaw';
import { AsyncState, Button } from '../../ui';
import { resolveAgentRoute } from '../../thinclaw/agent-routes';

const ThinClawChatView = lazy(() => import('../../thinclaw/ThinClawChatView').then((module) => ({ default: module.ThinClawChatView })));
const ThinClawHome = lazy(() => import('../../thinclaw/ThinClawHome').then((module) => ({ default: module.ThinClawHome })));
const ThinClawWorkspaceMemory = lazy(() => import('../../thinclaw/ThinClawWorkspaceMemory').then((module) => ({ default: module.ThinClawWorkspaceMemory })));
const ThinClawChannelCenter = lazy(() => import('../../thinclaw/ThinClawChannelCenter').then((module) => ({ default: module.ThinClawChannelCenter })));
const ThinClawAutomationCenter = lazy(() => import('../../thinclaw/ThinClawAutomationCenter').then((module) => ({ default: module.ThinClawAutomationCenter })));
const ThinClawJobs = lazy(() => import('../../thinclaw/ThinClawJobs').then((module) => ({ default: module.ThinClawJobs })));
const ThinClawCapabilitiesCenter = lazy(() => import('../../thinclaw/ThinClawCapabilitiesCenter').then((module) => ({ default: module.ThinClawCapabilitiesCenter })));
const ThinClawUsageCenter = lazy(() => import('../../thinclaw/ThinClawUsageCenter').then((module) => ({ default: module.ThinClawUsageCenter })));
const ThinClawOperationsCenter = lazy(() => import('../../thinclaw/ThinClawOperationsCenter').then((module) => ({ default: module.ThinClawOperationsCenter })));
const ThinClawAdvancedLabs = lazy(() => import('../../thinclaw/ThinClawAdvancedLabs').then((module) => ({ default: module.ThinClawAdvancedLabs })));
const ThinClawSessionSearch = lazy(() => import('../../thinclaw/ThinClawSessionSearch').then((module) => ({ default: module.ThinClawSessionSearch })));

function ThinClawPageSkeleton() {
    return <AsyncState kind="loading" title="Loading control surface" className="flex-1" />;
}

export function ThinClawView() {
    const {
        activeThinClawPage,
        selectedThinClawSession,
        thinclawGatewayRunning,
        setSelectedThinClawSession,
        setActiveThinClawPage,
        setActiveTab,
    } = useChatLayout();
    const resolved = resolveAgentRoute(activeThinClawPage);

    const bootstrapCheckedRef = useRef(false);
    const [bootstrapNeeded, setBootstrapNeeded] = useState<boolean | null>(null);

    const checkBootstrap = () => {
        thinclaw.checkBootstrapNeeded()
            .then((needed) => {
                setBootstrapNeeded(needed);
                if (needed) setActiveThinClawPage('chat');
            })
            .catch(() => setBootstrapNeeded(false));
    };

    useEffect(() => {
        if (bootstrapCheckedRef.current) return;
        bootstrapCheckedRef.current = true;
        checkBootstrap();
        // Bootstrap is intentionally checked once per mounted Agent Cockpit.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [setActiveThinClawPage]);

    const renderSubPage = () => {
        switch (resolved.destination) {
            case 'home': return <ThinClawHome />;
            case 'workspace': return <ThinClawWorkspaceMemory initialTab={resolved.tab} />;
            case 'channels': return <ThinClawChannelCenter initialTab={resolved.tab} />;
            case 'automations': return <ThinClawAutomationCenter initialTab={resolved.tab} />;
            case 'jobs': return <ThinClawJobs />;
            case 'capabilities': return <ThinClawCapabilitiesCenter initialTab={resolved.tab} />;
            case 'usage': return <ThinClawUsageCenter initialTab={resolved.tab} />;
            case 'operations': return <ThinClawOperationsCenter initialTab={resolved.tab} />;
            case 'advanced': return <ThinClawAdvancedLabs initialTab={resolved.tab} />;
            case 'chat': return null;
            default: return <AsyncState kind="empty" title="Select a control surface" description="Choose a page from the Agent Cockpit sidebar." className="flex-1" />;
        }
    };

    const chatVisible = resolved.destination === 'chat';
    return (
        <div className="relative flex h-full flex-1 flex-col overflow-hidden bg-surface-canvas text-content-primary" data-product-surface="agent-cockpit">
            {/* Always mounted: navigation must not interrupt a live agent run or draft. */}
            <div className="flex h-full flex-1 flex-col overflow-hidden" style={{ display: chatVisible ? undefined : 'none' }}>
                <Suspense fallback={<ThinClawPageSkeleton />}>
                    <ThinClawChatView
                        sessionKey={selectedThinClawSession}
                        gatewayRunning={thinclawGatewayRunning}
                        bootstrapNeeded={bootstrapNeeded ?? false}
                        onBootstrapComplete={() => setBootstrapNeeded(false)}
                        onFactoryReset={() => {
                            bootstrapCheckedRef.current = false;
                            checkBootstrap();
                            window.setTimeout(() => { void thinclaw.startThinClawGateway(); }, 2_000);
                        }}
                        onNavigateToSettings={(page) => setActiveTab(page as never)}
                        onViewSession={(key) => {
                            setSelectedThinClawSession(key);
                            setActiveThinClawPage('chat');
                        }}
                    />
                </Suspense>
            </div>

            {!chatVisible && <Suspense fallback={<ThinClawPageSkeleton />}>{renderSubPage()}</Suspense>}

            {/* Session search is a Chat inspector, not a second primary destination. */}
            {activeThinClawPage === 'session-search' && (
                <div className="absolute inset-0 z-20 flex bg-background/75 p-4 backdrop-blur-sm">
                    <div className="flex min-h-0 w-full flex-1 flex-col overflow-hidden rounded-[var(--radius-dialog)] border border-surface-outline bg-surface-elevated shadow-lg">
                        <div className="flex shrink-0 justify-end border-b border-surface-outline p-2">
                            <Button size="sm" variant="ghost" onClick={() => setActiveThinClawPage('chat')}>
                                <X className="size-3.5" aria-hidden="true" /> Close search
                            </Button>
                        </div>
                        <Suspense fallback={<ThinClawPageSkeleton />}><ThinClawSessionSearch /></Suspense>
                    </div>
                </div>
            )}
        </div>
    );
}
