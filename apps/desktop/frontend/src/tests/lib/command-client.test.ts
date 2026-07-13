import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { commandClient } from '../../lib/command-client';

const mockInvoke = vi.mocked(invoke);

beforeEach(() => {
    mockInvoke.mockReset();
    mockInvoke.mockResolvedValue(undefined);
});

describe('commandClient', () => {
    it('uses the generated command signature and unwraps successful results', async () => {
        const status = { engine_running: true };
        mockInvoke.mockResolvedValueOnce(status);

        await expect(commandClient.thinclawGetStatus()).resolves.toEqual(status);
        expect(mockInvoke).toHaveBeenCalledWith('thinclaw_get_status');
    });

    it('turns generated string errors into Error instances', async () => {
        mockInvoke.mockRejectedValueOnce('gateway offline');

        await expect(commandClient.thinclawStartGateway()).rejects.toThrow('gateway offline');
    });

    it('preserves typed unavailable details in the user-facing error', async () => {
        mockInvoke.mockRejectedValueOnce({
            kind: 'unavailable',
            capability: 'job restart',
            reason: 'remote gateway required',
            remediation: 'connect a gateway',
            satisfied_by: 'remote_only',
        });

        await expect(commandClient.thinclawJobRestart('job-1')).rejects.toThrow(
            'job restart: remote gateway required (connect a gateway)',
        );
    });

    it('fails before IPC when the Tauri runtime is absent', async () => {
        const runtime = (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
        delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
        try {
            await expect(commandClient.thinclawGetStatus()).rejects.toThrow('Tauri runtime not available');
            expect(mockInvoke).not.toHaveBeenCalled();
        } finally {
            (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = runtime;
        }
    });
});
