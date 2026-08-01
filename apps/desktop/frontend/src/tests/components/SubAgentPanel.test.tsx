import { render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { ThinClawStatus } from '../../lib/thinclaw';

const api = vi.hoisted(() => ({
    getThinClawStatus: vi.fn(),
    listChildSessions: vi.fn(),
    spawnSession: vi.fn(),
    abortThinClawChat: vi.fn(),
    updateSubAgentStatus: vi.fn(),
}));

vi.mock('../../lib/thinclaw', () => api);
vi.mock('../../hooks/use-thinclaw-stream', () => ({ useThinClawEvents: vi.fn() }));

import { AgentCockpitProvider } from '../../components/thinclaw/AgentCockpitProvider';
import SubAgentPanel from '../../components/thinclaw/SubAgentPanel';

describe('SubAgentPanel remote-profile safety', () => {
    it('does not load or expose Desktop-managed sub-agent controls for a remote profile', async () => {
        api.getThinClawStatus.mockResolvedValue({
            engine_running: true,
            engine_connected: true,
            gateway_mode: 'remote',
        } as ThinClawStatus);

        render(
            <AgentCockpitProvider>
                <SubAgentPanel sessionKey="agent:main" />
            </AgentCockpitProvider>,
        );

        await waitFor(() => expect(screen.getByText('Desktop-managed sub-agents are unavailable')).toBeInTheDocument());
        expect(screen.getByRole('button', { name: 'Desktop-managed sub-agents unavailable' })).toBeDisabled();
        expect(screen.getByText(/Desktop-managed sub-agents run in Local Core, not the selected remote profile/)).toBeInTheDocument();
        expect(api.listChildSessions).not.toHaveBeenCalled();
    });
});
