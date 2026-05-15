import { describe, expect, it, vi, beforeEach } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { directCommands } from '../../lib/generated/direct-commands';

const mockInvoke = vi.mocked(invoke);

beforeEach(() => {
    mockInvoke.mockReset();
});

describe('directCommands', () => {
    it('exposes only generated Direct command wrappers', () => {
        const commandNames = Object.keys(directCommands);

        expect(commandNames).toContain('directRuntimeSnapshot');
        expect(commandNames).toContain('directRuntimeStartEngine');
        expect(commandNames).toContain('directRuntimeDiscoverHfModels');
        expect(commandNames).toContain('directRuntimeGetModelFiles');
        expect(commandNames).toContain('directRuntimeDownloadHfModelFiles');
        expect(commandNames).not.toContain('thinclawGetStatus');
        expect(commandNames).not.toContain(['chat', 'Stream'].join(''));
        expect(commandNames).not.toContain(['discover', 'HfModels'].join(''));
        expect(commandNames.every((name) => name.startsWith('direct'))).toBe(true);
    });

    it('routes runtime snapshot through direct_runtime_snapshot', async () => {
        mockInvoke.mockResolvedValueOnce({
            kind: 'llama_cpp',
            displayName: 'llama.cpp',
            readiness: 'ready',
            endpoint: {
                baseUrl: 'http://127.0.0.1:53755/v1',
                apiKey: null,
                modelId: 'default',
                contextSize: 32768,
                modelFamily: 'qwen',
            },
            capabilities: ['chat'],
            supportedCapabilities: ['chat', 'embedding'],
            exposurePolicy: 'shared_when_enabled',
            unavailableReason: null,
        });

        const result = await directCommands.directRuntimeSnapshot();

        expect(mockInvoke).toHaveBeenCalledWith('direct_runtime_snapshot');
        expect(result.status).toBe('ok');
        if (result.status === 'ok') {
            expect(result.data.supportedCapabilities).toEqual(['chat', 'embedding']);
            expect(result.data.endpoint?.apiKey).toBeNull();
        }
    });

    it('routes Hugging Face discovery through direct_runtime_discover_hf_models', async () => {
        mockInvoke.mockResolvedValueOnce([
            {
                id: 'bartowski/Qwen3-GGUF',
                author: 'bartowski',
                name: 'Qwen3-GGUF',
                downloads: 1000,
                likes: 12,
                tags: ['gguf', 'text-generation'],
                last_modified: '2026-01-01T00:00:00.000Z',
                gated: false,
            },
        ]);

        const result = await directCommands.directRuntimeDiscoverHfModels(
            '',
            'llamacpp',
            5,
            ['text-generation'],
        );

        expect(mockInvoke).toHaveBeenCalledWith('direct_runtime_discover_hf_models', {
            query: '',
            engine: 'llamacpp',
            limit: 5,
            pipelineTags: ['text-generation'],
        });
        expect(result.status).toBe('ok');
        if (result.status === 'ok') {
            expect(result.data[0].tags).toContain('gguf');
        }
    });
});
