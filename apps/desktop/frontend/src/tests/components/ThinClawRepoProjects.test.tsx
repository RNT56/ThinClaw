import { renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({
    listRepoProjects: vi.fn(),
}));

vi.mock('../../lib/thinclaw', () => api);
vi.mock('../../hooks/use-thinclaw-stream', () => ({ useThinClawEvents: vi.fn() }));
vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

import { useRepoProjects } from '../../components/thinclaw/repo-projects/use-repo-projects';

describe('useRepoProjects truthfulness', () => {
    it('keeps an empty live response empty and marks setup as required', async () => {
        api.listRepoProjects.mockResolvedValue({ projects: [] });

        const { result } = renderHook(() => useRepoProjects());

        await waitFor(() => expect(result.current.isLoading).toBe(false));
        expect(result.current.projects).toEqual([]);
        expect(result.current.events).toEqual([]);
        expect(result.current.mergeGates).toEqual([]);
        expect(result.current.isSetupRequired).toBe(true);
    });
});
