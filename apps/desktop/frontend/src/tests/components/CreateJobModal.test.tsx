import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({
    createRoutine: vi.fn(),
    lintCronExpression: vi.fn(),
}));

vi.mock('../../lib/thinclaw', () => api);
vi.mock('sonner', () => ({
    toast: { success: vi.fn(), error: vi.fn() },
}));

import { CreateJobModal } from '../../components/thinclaw/automations/CreateJobModal';

describe('CreateJobModal', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        api.lintCronExpression.mockResolvedValue({
            valid: true,
            expression: '0 0 * * * * *',
            next_fire_times: ['2026-07-14T00:00:00Z'],
            checked_at: '2026-07-13T00:00:00Z',
        });
        api.createRoutine.mockResolvedValue({ id: 'routine-1' });
    });

    it('creates a system event using the generated command wrapper contract', async () => {
        const onClose = vi.fn();
        const onCreated = vi.fn();
        render(<CreateJobModal onClose={onClose} onCreated={onCreated} />);

        fireEvent.change(screen.getByLabelText(/Job Name/i), {
            target: { value: 'Review stalled work' },
        });
        fireEvent.click(screen.getByRole('radio', { name: /System event/i }));
        fireEvent.change(screen.getByLabelText(/System Event Message/i), {
            target: { value: 'Review stalled pull requests' },
        });

        await waitFor(() => expect(api.lintCronExpression).toHaveBeenCalled());
        fireEvent.click(screen.getByRole('button', { name: 'Create Job' }));

        await waitFor(() => {
            expect(api.createRoutine).toHaveBeenCalledWith(
                'Review stalled work',
                '',
                '0 0 * * * * *',
                'Review stalled pull requests',
                'system_event',
            );
            expect(onCreated).toHaveBeenCalledTimes(1);
            expect(onClose).toHaveBeenCalledTimes(1);
        });
    });
});
