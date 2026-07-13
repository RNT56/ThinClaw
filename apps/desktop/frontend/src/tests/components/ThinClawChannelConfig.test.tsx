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
});
