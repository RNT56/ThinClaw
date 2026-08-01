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
        expect(commandNames).toContain('directRuntimeGetHfCapabilities');
        expect(commandNames).toContain('directRuntimeDiscoverHfModelsV2');
        expect(commandNames).toContain('directRuntimeGetModelFilesV2');
        expect(commandNames).toContain('directRuntimeDownloadHfSelection');
        expect(commandNames).not.toContain('directRuntimeDiscoverHfModels');
        expect(commandNames).not.toContain('directRuntimeGetModelFiles');
        expect(commandNames).not.toContain('directRuntimeDownloadHfModelFiles');
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
                endpointId: 'llamacpp-chat-53755',
                baseUrl: 'http://127.0.0.1:53755/v1',
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
            expect(result.data.endpoint).not.toHaveProperty('apiKey');
        }
    });

    it('routes backend-owned Hugging Face capabilities', async () => {
        mockInvoke.mockResolvedValueOnce([
            {
                engine_id: 'llamacpp',
                task: 'chat',
                category: 'LLM',
                pipeline_tags: ['text-generation'],
                format_tag: 'gguf',
                layout: 'gguf_variants',
                searchable: true,
                compatibility_hint: null,
            },
        ]);

        const result = await directCommands.directRuntimeGetHfCapabilities();

        expect(mockInvoke).toHaveBeenCalledWith('direct_runtime_get_hf_capabilities');
        expect(result[0].pipeline_tags).toEqual(['text-generation']);
    });

    it('routes capability-owned Hugging Face discovery without caller-supplied engine tags', async () => {
        mockInvoke.mockResolvedValueOnce({
            engine_id: 'llamacpp',
            task: 'embedding',
            models: [],
            has_more: false,
        });

        const result = await directCommands.directRuntimeDiscoverHfModelsV2(
            'mxbai',
            'embedding',
            20,
        );

        expect(mockInvoke).toHaveBeenCalledWith('direct_runtime_discover_hf_models_v2', {
            query: 'mxbai',
            task: 'embedding',
            limit: 20,
        });
        expect(result).toEqual({
            status: 'ok',
            data: {
                engine_id: 'llamacpp',
                task: 'embedding',
                models: [],
                has_more: false,
            },
        });
    });

    it('routes a task-scoped file-plan request and preserves the pinned artifact plan', async () => {
        const plan = {
            repo_id: 'owner/vision-model',
            revision: 'b'.repeat(40),
            engine_id: 'llamacpp',
            task: 'vision' as const,
            category: 'LLM',
            format_tag: 'gguf',
            layout: 'gguf_variants' as const,
            artifacts: [
                {
                    id: 'artifact-q4',
                    download_id: 'download-q4',
                    label: 'Q4_K_M',
                    layout: 'gguf_variants' as const,
                    files: [
                        {
                            path: 'model-q4_k_m.gguf',
                            size: 4_096,
                            size_display: '4 KB',
                            sha256: 'c'.repeat(64),
                        },
                    ],
                    primary_file: 'model-q4_k_m.gguf',
                    quant_type: 'Q4_K_M',
                    is_mmproj: false,
                    total_size: 4_096,
                    total_size_display: '4 KB',
                },
            ],
            companion_artifacts: [
                {
                    id: 'artifact-mmproj',
                    download_id: 'download-mmproj',
                    label: 'Projector F16',
                    layout: 'gguf_variants' as const,
                    files: [
                        {
                            path: 'mmproj-model-f16.gguf',
                            size: 1_024,
                            size_display: '1 KB',
                            sha256: null,
                        },
                    ],
                    primary_file: 'mmproj-model-f16.gguf',
                    quant_type: 'F16',
                    is_mmproj: true,
                    total_size: 1_024,
                    total_size_display: '1 KB',
                },
            ],
            warnings: ['Choose the matching projector for vision input.'],
        };
        mockInvoke.mockResolvedValueOnce(plan);

        const result = await directCommands.directRuntimeGetModelFilesV2(
            plan.repo_id,
            plan.task,
        );

        expect(mockInvoke).toHaveBeenCalledWith('direct_runtime_get_model_files_v2', {
            repoId: plan.repo_id,
            task: 'vision',
        });
        expect(result).toEqual({ status: 'ok', data: plan });
    });

    it('preserves a structured file-plan bridge failure', async () => {
        const error = {
            kind: 'network',
            message: 'Hugging Face is unavailable',
            retryable: true,
        };
        mockInvoke.mockRejectedValueOnce(error);

        const result = await directCommands.directRuntimeGetModelFilesV2(
            'owner/model',
            'chat',
        );

        expect(mockInvoke).toHaveBeenCalledWith('direct_runtime_get_model_files_v2', {
            repoId: 'owner/model',
            task: 'chat',
        });
        expect(result).toEqual({ status: 'error', error });
    });

    it('passes a structured pinned artifact selection to the download command', async () => {
        const request = {
            repo_id: 'owner/model',
            revision: 'a'.repeat(40),
            task: 'chat' as const,
            artifact_id: 'artifact-q4',
            companion_artifact_id: null,
            destination_name: null,
        };
        mockInvoke.mockResolvedValueOnce({
            download_id: 'download-q4',
            repo_id: request.repo_id,
            revision: request.revision,
            engine_id: 'llamacpp',
            task: request.task,
            category: 'LLM',
            artifact_id: request.artifact_id,
            companion_artifact_id: null,
            destination_dir: '/models/LLM/install',
            model_path: '/models/LLM/install/model.gguf',
            companion_path: null,
            downloaded_files: ['model.gguf'],
            total_bytes: 1024,
        });

        await directCommands.directRuntimeDownloadHfSelection(request);

        expect(mockInvoke).toHaveBeenCalledWith('direct_runtime_download_hf_selection', {
            request,
        });
    });
});
