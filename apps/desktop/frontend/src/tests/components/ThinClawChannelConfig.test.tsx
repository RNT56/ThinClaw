import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const commands = vi.hoisted(() => ({
    thinclawChannelConfigSchemas: vi.fn(),
    thinclawChannelConfigSubmit: vi.fn(),
}));

vi.mock('../../lib/generated/thinclaw-commands', () => ({
    thinclawCommands: commands,
}));
vi.mock('sonner', () => ({
    toast: { loading: vi.fn(() => 'toast-id'), success: vi.fn(), error: vi.fn() },
}));

import { ThinClawChannelConfig } from '../../components/thinclaw/ThinClawChannelConfig';

describe('ThinClawChannelConfig', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        commands.thinclawChannelConfigSchemas.mockResolvedValue({
            status: 'ok',
            data: {
                available: true,
                schemas: [{
                    channel_id: 'imessage',
                    channel_name: 'iMessage',
                    fields: [
                        {
                            id: 'allow_from',
                            label: 'Allowed contacts',
                            field_type: 'textarea',
                            required: false,
                            default_value: '+12025550100',
                        },
                        {
                            id: 'poll_interval',
                            label: 'Polling interval (seconds)',
                            field_type: 'number',
                            required: true,
                            default_value: 3,
                        },
                    ],
                }],
            },
        });
        commands.thinclawChannelConfigSubmit.mockResolvedValue({
            status: 'ok',
            data: { note: 'Saved' },
        });
    });

    it('submits number fields as numbers and preserves current schema values', async () => {
        render(<ThinClawChannelConfig />);

        const pollInterval = await screen.findByLabelText(/Polling interval \(seconds\)/);
        expect(pollInterval).toHaveValue(3);
        fireEvent.change(pollInterval, { target: { value: '9' } });
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));

        await waitFor(() => {
            expect(commands.thinclawChannelConfigSubmit).toHaveBeenCalledWith('imessage', {
                allow_from: '+12025550100',
                poll_interval: 9,
            });
        });
    });

    it('renders host-managed channels without a misleading save action', async () => {
        commands.thinclawChannelConfigSchemas.mockResolvedValue({
            status: 'ok',
            data: {
                available: true,
                schemas: [{
                    channel_id: 'apns',
                    channel_name: 'Apns',
                    fields: [],
                    help: 'APNs signing identity is host-managed with APNS_PRIVATE_KEY.',
                }],
            },
        });

        render(<ThinClawChannelConfig />);

        expect(await screen.findByText(/APNS_PRIVATE_KEY/)).toBeInTheDocument();
        expect(screen.queryByRole('button', { name: 'Save' })).not.toBeInTheDocument();
    });

    it('keeps manifest credentials opaque and submits replacements explicitly', async () => {
        commands.thinclawChannelConfigSchemas.mockResolvedValue({
            status: 'ok',
            data: {
                available: true,
                schemas: [{
                    channel_id: 'line',
                    channel_name: 'Line',
                    fields: [{
                        id: 'line_channel_secret',
                        label: 'Channel secret',
                        field_type: 'password',
                        required: true,
                        default_value: null,
                    }],
                }],
            },
        });

        render(<ThinClawChannelConfig />);
        const credential = await screen.findByLabelText(/Channel secret/);
        expect(credential).toHaveAttribute('type', 'password');
        expect(credential).toHaveValue('');
        fireEvent.change(credential, { target: { value: 'replacement' } });
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));

        await waitFor(() => {
            expect(commands.thinclawChannelConfigSubmit).toHaveBeenCalledWith('line', {
                line_channel_secret: 'replacement',
            });
        });
    });
});
