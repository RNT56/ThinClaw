import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({
    getTrajectoryStats: vi.fn(),
    getTrajectoryRecords: vi.fn(),
    exportTrajectory: vi.fn(),
}));

vi.mock('../../lib/thinclaw', () => api);
vi.mock('sonner', () => ({
    toast: { success: vi.fn(), error: vi.fn() },
}));

import { ThinClawTrajectory } from '../../components/thinclaw/ThinClawTrajectory';

describe('ThinClawTrajectory', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        api.getTrajectoryStats.mockResolvedValue({
            log_root: '/tmp/trajectories',
            file_count: 1,
            record_count: 2,
            session_count: 1,
            first_seen: '2026-07-12T00:00:00Z',
            last_seen: '2026-07-13T00:00:00Z',
            success_count: 1,
            failure_count: 1,
            neutral_count: 0,
        });
        api.getTrajectoryRecords.mockResolvedValue([]);
        api.exportTrajectory.mockResolvedValue({
            format: 'sft',
            payload: '{"messages":[]}\n',
            source_record_count: 2,
            exported_record_count: 1,
            skipped_counts: { failed_turn: 1 },
        });
        URL.createObjectURL = vi.fn(() => 'blob:trajectory-export');
        URL.revokeObjectURL = vi.fn();
        vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined);
    });

    it('downloads the canonical SFT payload only after an explicit click', async () => {
        render(<ThinClawTrajectory />);

        const exportButton = await screen.findByRole('button', { name: 'Export SFT' });
        fireEvent.click(exportButton);

        await waitFor(() => {
            expect(api.exportTrajectory).toHaveBeenCalledWith('sft');
            expect(URL.createObjectURL).toHaveBeenCalledTimes(1);
            expect(HTMLAnchorElement.prototype.click).toHaveBeenCalledTimes(1);
            expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:trajectory-export');
        });
    });
});
