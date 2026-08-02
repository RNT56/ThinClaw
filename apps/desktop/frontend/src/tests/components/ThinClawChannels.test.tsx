import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({
    getThinClawChannelsList: vi.fn(),
    setSetting: vi.fn(),
}));

vi.mock('../../lib/thinclaw', () => api);
vi.mock('../../components/thinclaw/AgentCockpitProvider', () => ({
    useAgentCockpit: () => ({ status: { engine_running: true }, source: 'local' }),
}));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

import { ThinClawChannels } from '../../components/thinclaw/ThinClawChannels';

describe('ThinClawChannels truthfulness', () => {
    it('does not invent channel cards after an inventory failure', async () => {
        api.getThinClawChannelsList.mockRejectedValue(new Error('Gateway unavailable'));

        render(<ThinClawChannels />);

        expect(await screen.findByText('Channel inventory is unavailable')).toBeInTheDocument();
        expect(screen.queryByText('Slack')).not.toBeInTheDocument();
        expect(screen.queryByText('Discord')).not.toBeInTheDocument();
    });
});
