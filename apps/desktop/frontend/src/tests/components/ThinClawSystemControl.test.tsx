import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { startGateway, stopGateway } = vi.hoisted(() => ({
    startGateway: vi.fn().mockResolvedValue(undefined),
    stopGateway: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../lib/thinclaw', () => ({
    getThinClawStatus: vi.fn().mockResolvedValue({ engine_running: true }),
    getThinClawLogsTail: vi.fn().mockResolvedValue({ logs: [] }),
    startThinClawGateway: startGateway,
    stopThinClawGateway: stopGateway,
}));

vi.mock('../../hooks/use-thinclaw-stream', () => ({
    useThinClawEvents: vi.fn(),
}));

vi.mock('sonner', () => ({
    toast: { success: vi.fn(), info: vi.fn(), error: vi.fn() },
}));

import { ThinClawSystemControl } from '../../components/thinclaw/ThinClawSystemControl';

describe('ThinClawSystemControl gateway operations', () => {
    beforeEach(() => {
        startGateway.mockClear();
        stopGateway.mockClear();
    });

    it('requires an explicit confirmation before stopping a running gateway', async () => {
        render(<ThinClawSystemControl />);

        const stop = await screen.findByRole('button', { name: 'Stop gateway' });
        fireEvent.click(stop);
        expect(stopGateway).not.toHaveBeenCalled();
        const dialog = await screen.findByRole('dialog', { name: 'Stop the local gateway?' });
        expect(dialog).toBeInTheDocument();

        fireEvent.click(within(dialog).getByRole('button', { name: 'Stop gateway' }));
        await waitFor(() => {
            expect(stopGateway).toHaveBeenCalledTimes(1);
        });
    });

    it('does not expose incompatible configuration or embedded update controls', async () => {
        render(<ThinClawSystemControl />);

        expect(await screen.findByText('Gateway and logs')).toBeInTheDocument();
        expect(screen.queryByText('Deploy Configuration')).not.toBeInTheDocument();
        expect(screen.queryByText('Run Update & Rebuild')).not.toBeInTheDocument();
    });
});
