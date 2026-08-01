import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({
    getFleetStatus: vi.fn(),
    broadcastCommand: vi.fn(),
}));

vi.mock('../../lib/thinclaw', () => api);
vi.mock('sonner', () => ({ toast: { success: vi.fn(), warning: vi.fn(), error: vi.fn() } }));

import { FleetCommandCenter } from '../../components/thinclaw/fleet/FleetCommandCenter';

describe('FleetCommandCenter truthfulness', () => {
    it('shows observations and broadcast only, never inferred tasks or targeted dispatch', async () => {
        api.getFleetStatus.mockResolvedValue([{
            id: 'remote-1',
            name: 'Remote One',
            url: 'https://gateway.example.test',
            online: true,
            latency_ms: 42,
            model: 'model-a',
            capabilities: ['chat'],
            run_status: 'idle',
            current_task: 'fabricated active task',
        }]);

        render(<FleetCommandCenter />);

        expect(await screen.findByText('Remote One')).toBeInTheDocument();
        expect(screen.getByRole('button', { name: 'Broadcast' })).toBeInTheDocument();
        expect(screen.queryByText('fabricated active task')).not.toBeInTheDocument();
        expect(screen.queryByRole('button', { name: /spawn|abort|dispatch/i })).not.toBeInTheDocument();
    });
});
