import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({
    configureExternalMemoryProvider: vi.fn(),
    disableExternalMemoryProvider: vi.fn(),
}));

vi.mock('../../lib/thinclaw', () => api);
vi.mock('sonner', () => ({
    toast: { success: vi.fn(), error: vi.fn() },
}));

import { ExternalMemoryPanel } from '../../components/thinclaw/learning/ExternalMemoryPanel';

describe('ExternalMemoryPanel', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        api.configureExternalMemoryProvider.mockResolvedValue({ providers: [] });
        api.disableExternalMemoryProvider.mockResolvedValue({ providers: [] });
    });

    it('submits provider references without exposing a raw secret field', async () => {
        const onChanged = vi.fn().mockResolvedValue(undefined);
        const { container } = render(
            <ExternalMemoryPanel
                providers={[{
                    provider: 'openmemory',
                    active: true,
                    enabled: true,
                    healthy: true,
                    readiness: 'ready',
                    latency_ms: 12,
                }]}
                onChanged={onChanged}
            />,
        );

        expect(screen.queryByLabelText('API key')).not.toBeInTheDocument();
        expect(container.querySelector('input[type="password"]')).toBeNull();
        expect(screen.getByText('openmemory · ready')).toBeInTheDocument();

        fireEvent.change(screen.getByLabelText('External memory provider'), { target: { value: 'qdrant' } });
        fireEvent.change(screen.getByLabelText('Provider endpoint'), { target: { value: 'http://localhost:6333' } });
        fireEvent.change(screen.getByLabelText('API key environment variable'), { target: { value: 'QDRANT_API_KEY' } });
        fireEvent.change(screen.getByLabelText('Embedding endpoint'), { target: { value: 'http://localhost:8080/v1/embeddings' } });
        fireEvent.change(screen.getByLabelText('Collection'), { target: { value: 'thinclaw' } });
        fireEvent.click(screen.getByRole('button', { name: 'Save provider' }));

        await waitFor(() => {
            expect(api.configureExternalMemoryProvider).toHaveBeenCalledWith(expect.objectContaining({
                provider: 'qdrant',
                base_url: 'http://localhost:6333',
                api_key_env: 'QDRANT_API_KEY',
                embedding_url: 'http://localhost:8080/v1/embeddings',
                collection: 'thinclaw',
                collection_id: null,
                enabled: true,
                activate: true,
            }));
            expect(onChanged).toHaveBeenCalledTimes(1);
        });
    });

    it('deactivates the active provider and refreshes health', async () => {
        const onChanged = vi.fn().mockResolvedValue(undefined);
        render(
            <ExternalMemoryPanel
                providers={[{
                    provider: 'zep',
                    active: true,
                    enabled: true,
                    healthy: false,
                    readiness: 'unavailable',
                }]}
                onChanged={onChanged}
            />,
        );

        fireEvent.click(screen.getByRole('button', { name: 'Deactivate' }));

        await waitFor(() => {
            expect(api.disableExternalMemoryProvider).toHaveBeenCalledTimes(1);
            expect(onChanged).toHaveBeenCalledTimes(1);
        });
    });
});
