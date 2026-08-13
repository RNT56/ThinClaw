import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
    activateGatewayTarget,
    gatewayTargetForProfile,
    gatewayTargetIsEffective,
    type GatewayState,
} from '../../lib/thinclaw';

const mockInvoke = vi.mocked(invoke);

beforeEach(() => {
    mockInvoke.mockReset();
});

describe('revisioned gateway state', () => {
    it('normalizes legacy profile revision zero to revision one', () => {
        expect(gatewayTargetForProfile({ id: 'remote-one', revision: 0 })).toEqual({
            kind: 'profile',
            profile_id: 'remote-one',
            profile_revision: 1,
        });
    });

    it('requires profile id and revision to match the effective runtime', () => {
        const state = {
            desired: { kind: 'profile', profile_id: 'remote-one', profile_revision: 3 },
            effective: {
                kind: 'profile',
                profile_id: 'remote-one',
                profile_revision: 2,
                url: 'https://agent.example.test',
            },
            phase: 'failed',
            revision: 8,
            in_sync: false,
            attempt: 2,
            last_error: 'health check failed',
        } satisfies GatewayState;

        expect(gatewayTargetIsEffective(state, state.desired)).toBe(false);
        expect(gatewayTargetIsEffective(state, {
            kind: 'profile',
            profile_id: 'remote-one',
            profile_revision: 2,
        })).toBe(true);
    });

    it('sends the target and rendered CAS revision through generated bindings', async () => {
        const state = {
            desired: { kind: 'local' },
            effective: { kind: 'local' },
            phase: 'idle',
            revision: 12,
            in_sync: true,
            attempt: 4,
            last_error: null,
        } satisfies GatewayState;
        mockInvoke.mockResolvedValueOnce(state);

        await expect(activateGatewayTarget({ kind: 'local' }, 11)).resolves.toEqual(state);
        expect(mockInvoke).toHaveBeenCalledWith('thinclaw_activate_gateway_target', {
            target: { kind: 'local' },
            expectedRevision: 11,
        });
    });
});
