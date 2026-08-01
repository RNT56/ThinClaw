import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({
    listJobs: vi.fn(),
    getJobsSummary: vi.fn(),
    getJobDetail: vi.fn(),
    getJobEvents: vi.fn(),
    listJobFiles: vi.fn(),
    readJobFile: vi.fn(),
    cancelJob: vi.fn(),
    restartJob: vi.fn(),
    promptJob: vi.fn(),
}));

vi.mock('../../lib/thinclaw', () => api);
vi.mock('sonner', () => ({ toast: { success: vi.fn(), info: vi.fn(), error: vi.fn() } }));

import { ThinClawJobs } from '../../components/thinclaw/ThinClawJobs';

describe('ThinClawJobs capability gates', () => {
    it('does not render unsupported restart, prompt, or file actions for a local direct job', async () => {
        api.listJobs.mockResolvedValue({
            jobs: [{ id: 'job-1', title: 'Local job', state: 'running', created_at: '2026-01-01T00:00:00Z' }],
            capabilities: { detail: true, events: true, cancel: true, restart: false, prompt: false, files: false },
            unavailable: {
                restart: 'Remote gateway required.',
                prompt: 'Remote gateway required.',
                files: 'Remote gateway required.',
            },
        });
        api.getJobsSummary.mockResolvedValue({ total: 1, pending: 0, in_progress: 1, completed: 0, failed: 0, cancelled: 0, interrupted: 0, stuck: 0 });
        api.getJobDetail.mockResolvedValue({ id: 'job-1', title: 'Local job', state: 'running', description: '', execution_backend: 'local_host' });
        api.getJobEvents.mockResolvedValue({ job_id: 'job-1', events: [] });

        render(<ThinClawJobs />);

        expect((await screen.findAllByText('Local job')).length).toBeGreaterThan(0);
        expect(await screen.findByRole('button', { name: 'Cancel' })).toBeInTheDocument();
        expect(screen.queryByRole('button', { name: 'Restart' })).not.toBeInTheDocument();
        expect(screen.queryByRole('button', { name: 'Files' })).not.toBeInTheDocument();
        expect(screen.queryByPlaceholderText(/follow-up prompt/i)).not.toBeInTheDocument();
        expect(screen.getAllByText('Remote gateway required.').length).toBeGreaterThan(0);
    });
});
