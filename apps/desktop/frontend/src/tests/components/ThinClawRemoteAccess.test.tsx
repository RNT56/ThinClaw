import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({
    getRemoteAccessStatus: vi.fn(),
    startRemoteAccess: vi.fn(),
    stopRemoteAccess: vi.fn(),
}));

vi.mock('../../lib/thinclaw', () => api);
vi.mock('sonner', () => ({
    toast: { success: vi.fn(), error: vi.fn() },
}));

import { ThinClawRemoteAccess } from '../../components/thinclaw/ThinClawRemoteAccess';

const readyStatus = {
    runtime_mode: 'local',
    gateway_running: true,
    gateway_port: 18789,
    gateway_url: 'http://127.0.0.1:18789',
    tailscale_installed: true,
    tailscale_authenticated: true,
    tailscale_dns_name: 'desktop.tailnet.ts.net',
    tailscale_error: null,
    tunnel_running: false,
    exposure: null,
    access_url: null,
};

describe('ThinClawRemoteAccess', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        api.getRemoteAccessStatus.mockResolvedValue(readyStatus);
        api.startRemoteAccess.mockResolvedValue({
            ...readyStatus,
            tunnel_running: true,
            exposure: 'tailnet',
            access_url: 'https://desktop.tailnet.ts.net',
        });
        api.stopRemoteAccess.mockResolvedValue(readyStatus);
    });

    it('defaults to private tailnet access and starts it explicitly', async () => {
        render(<ThinClawRemoteAccess />);

        const start = await screen.findByRole('button', { name: 'Enable tailnet access' });
        fireEvent.click(start);

        await waitFor(() => {
            expect(api.startRemoteAccess).toHaveBeenCalledWith('tailnet', false);
        });
        expect(await screen.findByText('Private tailnet access active')).toBeInTheDocument();
    });

    it('keeps public Funnel disabled until the warning is acknowledged', async () => {
        render(<ThinClawRemoteAccess />);
        await screen.findByText('Signed in as desktop.tailnet.ts.net');

        fireEvent.click(screen.getByRole('button', { name: /Public Funnel/ }));
        const start = screen.getByRole('button', { name: 'Enable public Funnel' });
        expect(start).toBeDisabled();

        fireEvent.click(screen.getByRole('checkbox'));
        expect(start).toBeEnabled();
        fireEvent.click(start);

        await waitFor(() => {
            expect(api.startRemoteAccess).toHaveBeenCalledWith('public', true);
        });
    });
});
