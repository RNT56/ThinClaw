import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { patchConfig } = vi.hoisted(() => ({
    patchConfig: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../lib/thinclaw', () => ({
    getThinClawStatus: vi.fn().mockResolvedValue({ engine_running: false }),
    getThinClawConfig: vi.fn().mockResolvedValue({
        version: 1,
        workbench: { max_search_results: 5, mcp_sandbox_enabled: false },
        agent: { 'llm.backend': 'anthropic' },
        settings: [],
    }),
    getThinClawConfigSchema: vi.fn().mockResolvedValue({
        views: {
            workbench: {
                description: 'Direct settings',
                schema: { properties: { max_search_results: { type: 'number' } } },
            },
            agent: { description: 'Agent settings', schema: { properties: {} } },
        },
    }),
    patchThinClawConfig: patchConfig,
    getThinClawLogsTail: vi.fn().mockResolvedValue({ logs: [] }),
    runThinClawUpdate: vi.fn().mockResolvedValue(undefined),
    startThinClawGateway: vi.fn().mockResolvedValue(undefined),
    stopThinClawGateway: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../hooks/use-thinclaw-stream', () => ({
    useThinClawEvents: vi.fn(),
}));

vi.mock('sonner', () => ({
    toast: {
        success: vi.fn(),
        error: vi.fn(),
        promise: vi.fn(),
    },
}));

import { ThinClawSystemControl } from '../../components/thinclaw/ThinClawSystemControl';

describe('ThinClawSystemControl unified settings views', () => {
    beforeEach(() => {
        patchConfig.mockClear();
    });

    it('switches between Workbench and Agent views and saves the typed envelope', async () => {
        render(<ThinClawSystemControl />);

        expect(await screen.findByDisplayValue('5')).toBeInTheDocument();
        fireEvent.click(screen.getByRole('button', { name: 'Agent Cockpit' }));
        expect(await screen.findByDisplayValue('anthropic')).toBeInTheDocument();

        fireEvent.click(screen.getByRole('button', { name: 'Deploy Configuration' }));
        await waitFor(() => {
            expect(patchConfig).toHaveBeenCalledWith({
                workbench: { max_search_results: 5, mcp_sandbox_enabled: false },
                agent: { 'llm.backend': 'anthropic' },
            });
        });
        expect(patchConfig.mock.calls[0]?.[0]).not.toHaveProperty('raw');
        expect(patchConfig.mock.calls[0]?.[0]).not.toHaveProperty('baseHash');
    });
});
