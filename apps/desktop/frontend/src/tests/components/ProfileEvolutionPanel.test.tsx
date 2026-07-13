import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({
    getProfileEvolutionStatus: vi.fn(),
    runProfileEvolution: vi.fn(),
}));

vi.mock('../../lib/thinclaw', () => api);
vi.mock('sonner', () => ({
    toast: { success: vi.fn(), error: vi.fn() },
}));

import { ProfileEvolutionPanel } from '../../components/thinclaw/learning/ProfileEvolutionPanel';

const status = {
    profile_path: 'context/profile.json',
    profile_exists: true,
    profile_parse_error: null,
    preferred_name: 'Ada',
    confidence: 0.82,
    message_count: 42,
    profile_updated_at: '2026-07-13T10:00:00Z',
    profile: { preferred_name: 'Ada', confidence: 0.82 },
    routine_exists: true,
    routine_id: 'routine-1',
    routine_enabled: true,
    last_run_at: '2026-07-07T09:00:00Z',
    next_fire_at: '2026-07-14T09:00:00Z',
    run_count: 3,
    consecutive_failures: 0,
};

describe('ProfileEvolutionPanel', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        api.getProfileEvolutionStatus.mockResolvedValue(status);
        api.runProfileEvolution.mockResolvedValue({ routine_id: 'routine-1', run_id: 'run-1' });
    });

    it('shows live profile state and triggers an explicit manual run', async () => {
        render(<ProfileEvolutionPanel />);

        expect(await screen.findByText('Ada')).toBeInTheDocument();
        expect(screen.getByText('82%')).toBeInTheDocument();
        expect(screen.getByText('42 messages')).toBeInTheDocument();

        fireEvent.click(screen.getByRole('button', { name: 'Run evolution now' }));

        await waitFor(() => {
            expect(api.runProfileEvolution).toHaveBeenCalledTimes(1);
            expect(api.getProfileEvolutionStatus).toHaveBeenCalledTimes(2);
        });
    });
});
