import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({
    getExperimentEnvironments: vi.fn(),
    runExperimentEvaluation: vi.fn(),
}));

vi.mock('../../lib/thinclaw', () => api);
vi.mock('sonner', () => ({
    toast: { success: vi.fn(), error: vi.fn() },
}));

import { BenchmarkPanel } from '../../components/thinclaw/experiments/BenchmarkPanel';

describe('BenchmarkPanel', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        api.getExperimentEnvironments.mockResolvedValue({
            available: true,
            environments: [
                {
                    id: 'agent_loop',
                    name: 'Agent Loop',
                    description: 'Evaluate the embedded agent.',
                    runnable: true,
                },
                {
                    id: 'terminal_bench',
                    name: 'Terminal Benchmark',
                    description: 'Requires CLI case definitions.',
                    runnable: false,
                },
            ],
        });
        api.runExperimentEvaluation.mockResolvedValue({
            available: true,
            env_id: 'agent_loop',
            episodes: 2,
            summary: {
                env_names: ['agent_loop'],
                episode_count: 2,
                step_count: 4,
                score: 0.75,
                exact_tokens_supported: true,
                logprobs_supported: false,
                token_capture_steps: 2,
                captured_token_ids: 24,
                captured_logprobs: 0,
            },
            trajectories: [],
        });
    });

    it('runs the selected environment and renders its scored summary', async () => {
        render(<BenchmarkPanel />);

        expect(await screen.findByRole('radio', { name: /Agent Loop/i })).toBeChecked();
        expect(screen.getByRole('radio', { name: /Terminal Benchmark/i })).toBeDisabled();

        fireEvent.change(screen.getByLabelText('Evaluation prompt'), {
            target: { value: 'Inspect the active tool surface.' },
        });
        fireEvent.change(screen.getByLabelText('Episodes'), { target: { value: '2' } });
        fireEvent.change(screen.getByLabelText('Max steps'), { target: { value: '6' } });
        fireEvent.click(screen.getByRole('button', { name: 'Run benchmark' }));

        await waitFor(() => {
            expect(api.runExperimentEvaluation).toHaveBeenCalledWith(
                'agent_loop',
                'Inspect the active tool surface.',
                2,
                6,
            );
        });
        expect(await screen.findByText('75% score')).toBeInTheDocument();
        expect(screen.getByText('4')).toBeInTheDocument();
        expect(screen.getByText('Unavailable')).toBeInTheDocument();
    });
});
