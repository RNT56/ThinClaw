/**
 * Tests for src/lib/thinclaw.ts
 *
 * Covers the Tauri IPC wrappers — we mock invoke() and verify that each
 * exported function calls it with the correct command name and payload.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

// Functions under test
import {
    getThinClawStatus,
    startThinClawGateway,
    stopThinClawGateway,
    saveGatewaySettings,
    toggleThinClawLocalInference,
    addAgentProfile,
    removeAgentProfile,
    saveCloudConfig,
    deleteThinClawSession,
    getThinClawSessions,
    patchThinClawConfig,
    saveRoutingPools,
    verifyConnection,
    type AgentProfile,
} from '../../lib/thinclaw';

// The global mock is registered in setup.ts, but we cast for type-safety here.
const mockInvoke = vi.mocked(invoke);

beforeEach(() => {
    mockInvoke.mockReset();
    mockInvoke.mockResolvedValue(undefined);
});

// ---------------------------------------------------------------------------
// Gateway status & lifecycle
// ---------------------------------------------------------------------------
describe('getThinClawStatus()', () => {
    it('invokes the correct command', async () => {
        const fakeStatus = { engine_running: true, engine_connected: false, port: 18789 };
        mockInvoke.mockResolvedValueOnce(fakeStatus);

        const result = await getThinClawStatus();

        expect(mockInvoke).toHaveBeenCalledOnce();
        // safeInvoke forwards (cmd, args?) — args is undefined here
        expect(mockInvoke).toHaveBeenCalledWith('thinclaw_get_status', undefined);
        expect(result).toEqual(fakeStatus);
    });
});

describe('startThinClawGateway()', () => {
    it('calls the start command', async () => {
        await startThinClawGateway();
        expect(mockInvoke).toHaveBeenCalledWith('thinclaw_start_gateway');
    });
});

describe('stopThinClawGateway()', () => {
    it('calls the stop command', async () => {
        await stopThinClawGateway();
        expect(mockInvoke).toHaveBeenCalledWith('thinclaw_stop_gateway');
    });
});

// ---------------------------------------------------------------------------
// Gateway settings
// ---------------------------------------------------------------------------
describe('saveGatewaySettings()', () => {
    it('passes mode, url, and token to Tauri', async () => {
        await saveGatewaySettings('remote', 'ws://10.0.0.1:18789', 'my-token');

        expect(mockInvoke).toHaveBeenCalledWith('thinclaw_save_gateway_settings', {
            mode: 'remote',
            url: 'ws://10.0.0.1:18789',
            token: 'my-token',
        });
    });

    it('passes null url/token for local mode', async () => {
        await saveGatewaySettings('local', null, null);

        expect(mockInvoke).toHaveBeenCalledWith('thinclaw_save_gateway_settings', {
            mode: 'local',
            url: null,
            token: null,
        });
    });
});

// ---------------------------------------------------------------------------
// Toggles
// ---------------------------------------------------------------------------
describe('toggleThinClawLocalInference()', () => {
    it('sends enabled=true', async () => {
        await toggleThinClawLocalInference(true);
        expect(mockInvoke).toHaveBeenCalledWith('thinclaw_toggle_local_inference', { enabled: true });
    });
});

// ---------------------------------------------------------------------------
// Agent profiles
// ---------------------------------------------------------------------------
describe('addAgentProfile()', () => {
    it('forwards the full profile object', async () => {
        const profile: AgentProfile = {
            id: 'abc',
            name: 'My Server',
            url: 'ws://192.168.1.10:18789',
            token: 'secret',
            mode: 'remote',
            auto_connect: false,
        };
        await addAgentProfile(profile);
        expect(mockInvoke).toHaveBeenCalledWith('thinclaw_add_agent_profile', { profile });
    });
});

describe('removeAgentProfile()', () => {
    it('forwards the id', async () => {
        await removeAgentProfile('my-id');
        expect(mockInvoke).toHaveBeenCalledWith('thinclaw_remove_agent_profile', { id: 'my-id' });
    });
});

// ---------------------------------------------------------------------------
// Cloud config
// ---------------------------------------------------------------------------
describe('saveCloudConfig()', () => {
    it('passes enabled providers, models and custom LLM correctly', async () => {
        const providers = ['anthropic', 'openai'];
        const models = { anthropic: ['claude-3-5-sonnet-latest'], openai: ['gpt-4o'] };
        const customLlm = null;

        await saveCloudConfig(providers, models, customLlm);

        expect(mockInvoke).toHaveBeenCalledWith('thinclaw_save_cloud_config', {
            enabledProviders: providers,
            enabledModels: models,
            customLlm: null,
        });
    });
});

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------
describe('deleteThinClawSession()', () => {
    it('sends the correct sessionKey', async () => {
        await deleteThinClawSession('sess-001');
        expect(mockInvoke).toHaveBeenCalledWith('thinclaw_delete_session', { sessionKey: 'sess-001' });
    });
});

describe('getThinClawSessions()', () => {
    it('returns the sessions response', async () => {
        const response = { sessions: [{ session_key: 's1', title: 'Test', updated_at_ms: null, source: null }] };
        mockInvoke.mockResolvedValueOnce(response);

        const result = await getThinClawSessions();
        expect(result).toEqual(response);
    });
});

// ---------------------------------------------------------------------------
// Config patching
// ---------------------------------------------------------------------------
describe('patchThinClawConfig()', () => {
    it('forwards the patch payload', async () => {
        const patch = { raw: JSON.stringify({ models: { providers: {} } }) };
        await patchThinClawConfig(patch);
        expect(mockInvoke).toHaveBeenCalledWith('thinclaw_config_patch', { patch });
    });
});

// ---------------------------------------------------------------------------
// Routing pools
// ---------------------------------------------------------------------------
describe('saveRoutingPools()', () => {
    it('forwards primary and cheap pool order', async () => {
        await saveRoutingPools(['anthropic', 'openai'], ['openai', 'groq']);
        expect(mockInvoke).toHaveBeenCalledWith('thinclaw_routing_pools_save', {
            primaryPoolOrder: ['anthropic', 'openai'],
            cheapPoolOrder: ['openai', 'groq'],
        });
    });
});

// ---------------------------------------------------------------------------
// Connection verification
// ---------------------------------------------------------------------------
describe('verifyConnection()', () => {
    it('returns true when connection succeeds', async () => {
        mockInvoke.mockResolvedValueOnce(true);
        const result = await verifyConnection('ws://192.168.1.1:18789', 'token');
        expect(result).toBe(true);
        expect(mockInvoke).toHaveBeenCalledWith('thinclaw_test_connection', {
            url: 'ws://192.168.1.1:18789',
            token: 'token',
        });
    });

    it('returns false when connection fails', async () => {
        mockInvoke.mockResolvedValueOnce(false);
        const result = await verifyConnection('ws://bad-host:18789', null);
        expect(result).toBe(false);
    });
});
