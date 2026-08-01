import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ThinClawStatus } from '../../lib/thinclaw';

const api = vi.hoisted(() => ({
    getThinClawStatus: vi.fn(),
}));

vi.mock('../../lib/thinclaw', () => api);

import { AgentCockpitProvider, useAgentCockpit } from '../../components/thinclaw/AgentCockpitProvider';
import { AgentTabbedPage } from '../../components/thinclaw/AgentTabbedPage';
import type { AgentCapabilityKey } from '../../components/thinclaw/agent-routes';

function status(overrides: Partial<ThinClawStatus>): ThinClawStatus {
    return {
        engine_running: true,
        engine_connected: true,
        gateway_mode: 'local',
        ...overrides,
    } as ThinClawStatus;
}

function CapabilityProbe({ capability }: { capability: AgentCapabilityKey }) {
    const current = useAgentCockpit().capability(capability);
    return (
        <div>
            <output data-testid={`${capability}-state`}>{current.state}</output>
            <output data-testid={`${capability}-reason`}>{current.reason ?? ''}</output>
        </div>
    );
}

function SourceProbe() {
    return <output data-testid="profile-source">{useAgentCockpit().source}</output>;
}

describe('AgentCockpitProvider', () => {
    beforeEach(() => {
        api.getThinClawStatus.mockReset();
    });

    it('derives honest local and remote capability gates from the selected profile', async () => {
        api.getThinClawStatus.mockResolvedValue(status({
            gateway_mode: 'remote',
            engine_running: true,
            engine_connected: true,
        }));

        render(
            <AgentCockpitProvider>
                <SourceProbe />
                <CapabilityProbe capability="runtime" />
                <CapabilityProbe capability="local-host" />
                <CapabilityProbe capability="local-subagent" />
                <CapabilityProbe capability="remote-access" />
            </AgentCockpitProvider>,
        );

        await waitFor(() => expect(screen.getByTestId('profile-source')).toHaveTextContent('remote'));
        await waitFor(() => expect(screen.getByTestId('runtime-state')).toHaveTextContent('available'));
        expect(screen.getByTestId('local-host-state')).toHaveTextContent('unavailable');
        expect(screen.getByTestId('local-host-reason')).toHaveTextContent('Local host files belong to this Desktop');
        expect(screen.getByTestId('local-subagent-state')).toHaveTextContent('unavailable');
        expect(screen.getByTestId('local-subagent-reason')).toHaveTextContent('Desktop-managed sub-agents run in Local Core');
        expect(screen.getByTestId('remote-access-state')).toHaveTextContent('unavailable');
        expect(screen.getByTestId('remote-access-reason')).toHaveTextContent('Remote Access can only expose this Desktop');
    });

    it('does not imply that a stopped local runtime is usable', async () => {
        api.getThinClawStatus.mockResolvedValue(status({
            gateway_mode: 'local',
            engine_running: false,
            engine_connected: false,
        }));

        render(
            <AgentCockpitProvider>
                <CapabilityProbe capability="runtime" />
            </AgentCockpitProvider>,
        );

        await waitFor(() => expect(screen.getByTestId('runtime-state')).toHaveTextContent('unavailable'));
        expect(screen.getByTestId('runtime-reason')).toHaveTextContent('The local gateway is stopped.');
    });

    it('connects each tab to the currently active tabpanel for assistive technology', () => {
        api.getThinClawStatus.mockResolvedValue(status({}));

        render(
            <AgentCockpitProvider>
                <AgentTabbedPage
                    title="Test surface"
                    description="Test-only page"
                    tabs={[
                        { id: 'overview', label: 'Overview', content: <p>Overview content</p> },
                        { id: 'history', label: 'History', content: <p>History content</p> },
                    ]}
                />
            </AgentCockpitProvider>,
        );

        expect(screen.getByRole('tab', { name: 'Overview' })).toHaveAttribute('id', 'overview-tab');
        expect(screen.getByRole('tab', { name: 'Overview' })).toHaveAttribute('aria-controls', 'overview-panel');
        expect(screen.getByRole('tabpanel')).toHaveAttribute('aria-labelledby', 'overview-tab');
    });
});
