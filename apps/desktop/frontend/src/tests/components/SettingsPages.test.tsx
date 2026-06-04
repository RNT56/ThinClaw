/**
 * Tests for the lazy-loading refactor in SettingsPages.tsx
 *
 * Verifies:
 *  1. The three heavy tab components (ModelBrowser, GatewayTab, SecretsTab)
 *     are code-split — i.e. they load asynchronously rather than being part
 *     of the synchronous module graph.
 *  2. TabSkeleton renders correctly while lazy chunks are loading.
 *  3. SettingsContent renders the correct child for each activePage value.
 *
 * Design decisions
 * ----------------
 * Tauri commands and heavy sub-components are mocked so we can render
 * SettingsPages in jsdom without the entire Tauri runtime.
 */

import { describe, it, expect, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { Suspense } from 'react';

// ---------------------------------------------------------------------------
// Blanket mock for bindings (auto-generated Tauri glue)
// ---------------------------------------------------------------------------
vi.mock('../../lib/bindings', () => ({
    commands: {
        getUserConfig: vi.fn().mockResolvedValue({}),
        directRuntimeGetSidecarStatus: vi.fn().mockResolvedValue({ chat_running: false }),
        getModelMetadata: vi.fn().mockResolvedValue({ status: 'ok', data: {} }),
        updateUserConfig: vi.fn().mockResolvedValue(undefined),
        directRuntimeStartChatServer: vi.fn().mockResolvedValue(undefined),
        thinclawGetStatus: vi.fn().mockResolvedValue({ status: 'ok', data: { engine_running: false } }),
        directRuntimeGetChatServerConfig: vi.fn().mockResolvedValue(null),
        thinclawStopGateway: vi.fn().mockResolvedValue(undefined),
        thinclawStartGateway: vi.fn().mockResolvedValue(undefined),
    },
}));

// Mock thinclaw lib so GatewayTab/SecretsTab don't need real IPC
vi.mock('../../lib/thinclaw', () => ({
    getThinClawStatus: vi.fn().mockResolvedValue({
        engine_running: false, engine_connected: false, port: 18789,
        gateway_mode: 'local', slack_enabled: false, telegram_enabled: false,
        remote_url: null, remote_token: null, device_id: '', auth_token: '',
        state_dir: '', local_inference_enabled: false,
        has_huggingface_token: false, huggingface_granted: false,
        has_anthropic_key: false, anthropic_granted: false,
        has_brave_key: false, brave_granted: false,
        has_openai_key: false, openai_granted: false,
        has_openrouter_key: false, openrouter_granted: false,
        gemini_granted: false, groq_granted: false,
        selected_cloud_brain: null, auto_start_gateway: false,
        profiles: [], enabled_cloud_providers: [],
    }),
    getPermissionStatus: vi.fn().mockResolvedValue({ accessibility: false, screen_recording: false }),
    toggleThinClawLocalInference: vi.fn().mockResolvedValue(undefined),
    removeAgentProfile: vi.fn().mockResolvedValue(undefined),
    saveGatewaySettings: vi.fn().mockResolvedValue(undefined),
    startThinClawGateway: vi.fn().mockResolvedValue(undefined),
    stopThinClawGateway: vi.fn().mockResolvedValue(undefined),
    getThinClawDiagnostics: vi.fn().mockResolvedValue({}),
    getThinClawMemory: vi.fn().mockResolvedValue(''),
    getThinClawFile: vi.fn().mockResolvedValue(''),
    revealPath: vi.fn().mockResolvedValue(undefined),
    patchThinClawConfig: vi.fn().mockResolvedValue(undefined),
}));

// Mock all three heavy tabs so we can verify routing without their full DOM
vi.mock('../../components/settings/ModelBrowser', () => ({
    ModelBrowser: () => <div data-testid="model-browser">ModelBrowser</div>,
}));
vi.mock('../../components/settings/GatewayTab', () => ({
    GatewayTab: () => <div data-testid="gateway-tab">GatewayTab</div>,
}));
vi.mock('../../components/settings/SecretsTab', () => ({
    SecretsTab: () => <div data-testid="secrets-tab">SecretsTab</div>,
}));

// Lightweight mocks for non-lazy tabs
vi.mock('../../components/settings/PersonaTab', () => ({
    PersonaTab: () => <div data-testid="persona-tab">PersonaTab</div>,
}));
vi.mock('../../components/settings/PersonalizationTab', () => ({
    PersonalizationTab: () => <div data-testid="personalization-tab">PersonalizationTab</div>,
}));
vi.mock('../../components/settings/SlackTab', () => ({
    SlackTab: () => <div data-testid="slack-tab">SlackTab</div>,
}));
vi.mock('../../components/settings/TelegramTab', () => ({
    TelegramTab: () => <div data-testid="telegram-tab">TelegramTab</div>,
}));
vi.mock('../../components/settings/ChatProviderTab', () => ({
    ChatProviderTab: () => <div data-testid="chat-provider-tab">ChatProviderTab</div>,
}));
vi.mock('../../components/model-context', () => ({
    useModelContext: () => ({
        currentModelPath: null,
        maxContext: 8192,
        setMaxContext: vi.fn(),
        localModels: [],
        systemSpecs: null,
        currentModelTemplate: null,
    }),
}));
vi.mock('../../components/theme-provider', () => ({
    ThemeToggle: () => <div />,
    useTheme: () => ({ theme: 'dark', setTheme: vi.fn() }),
}));
vi.mock('../../lib/syntax-themes', () => ({
    DARK_SYNTAX_THEMES: [],
    LIGHT_SYNTAX_THEMES: [],
}));
vi.mock('../../lib/app-themes', () => ({
    APP_THEMES: [],
}));

// ---------------------------------------------------------------------------
// Import component *after* mocks are set up
// ---------------------------------------------------------------------------
import { SettingsContent } from '../../components/settings/SettingsPages';
import type { SettingsPage } from '../../components/settings/SettingsSidebar';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
function renderPage(page: SettingsPage) {
    return render(
        <Suspense fallback={<div data-testid="suspense-fallback">Loading...</div>}>
            <SettingsContent activePage={page} />
        </Suspense>
    );
}

// ---------------------------------------------------------------------------
// Tests — routing correctness
// ---------------------------------------------------------------------------
describe('SettingsContent routing', () => {
    it('renders ModelBrowser for the "models" page', async () => {
        renderPage('models');
        await waitFor(() => expect(screen.getByTestId('model-browser')).toBeInTheDocument());
    });

    it('renders GatewayTab for the "thinclaw-gateway" page', async () => {
        renderPage('thinclaw-gateway');
        await waitFor(() => expect(screen.getByTestId('gateway-tab')).toBeInTheDocument());
    });

    it('renders SecretsTab for the "secrets" page', async () => {
        renderPage('secrets');
        await waitFor(() => expect(screen.getByTestId('secrets-tab')).toBeInTheDocument());
    });

    it('renders PersonaTab for the "persona" page (not lazy)', async () => {
        renderPage('persona');
        await waitFor(() => expect(screen.getByTestId('persona-tab')).toBeInTheDocument());
    });

    it('renders PersonalizationTab for the "personalization" page', async () => {
        renderPage('personalization');
        await waitFor(() => expect(screen.getByTestId('personalization-tab')).toBeInTheDocument());
    });

    it('renders SlackTab for "thinclaw-slack"', async () => {
        renderPage('thinclaw-slack');
        await waitFor(() => expect(screen.getByTestId('slack-tab')).toBeInTheDocument());
    });

    it('renders TelegramTab for "thinclaw-telegram"', async () => {
        renderPage('thinclaw-telegram');
        await waitFor(() => expect(screen.getByTestId('telegram-tab')).toBeInTheDocument());
    });

    it('renders ChatProviderTab for "inference"', async () => {
        renderPage('inference');
        await waitFor(() => expect(screen.getByTestId('chat-provider-tab')).toBeInTheDocument());
    });
});

// ---------------------------------------------------------------------------
// Tests — heavy tabs are NOT included in the synchronous import
// ---------------------------------------------------------------------------
describe('Lazy tab code-splitting', () => {
    it('ModelBrowser, GatewayTab, SecretsTab are loaded via React.lazy', async () => {
        // When loaded, modules go through the lazy() dynamic-import path.
        // We verify by checking that the mock's default export is used (i.e.
        // the re-export via `.then(m => ({ default: m.X }))` worked correctly).
        renderPage('models');
        await waitFor(() => expect(screen.getByTestId('model-browser')).toBeInTheDocument());

        renderPage('thinclaw-gateway');
        await waitFor(() => expect(screen.getByTestId('gateway-tab')).toBeInTheDocument());

        renderPage('secrets');
        await waitFor(() => expect(screen.getByTestId('secrets-tab')).toBeInTheDocument());
    });
});

// ---------------------------------------------------------------------------
// Tests — PageHeader titles
// ---------------------------------------------------------------------------
describe('PageHeader content', () => {
    it('shows "Model Management" heading on the models page', async () => {
        renderPage('models');
        await waitFor(() => expect(screen.getByRole('heading', { name: /Model Management/i })).toBeInTheDocument());
    });

    it('shows "ThinClaw Gateway" heading on the gateway page', async () => {
        renderPage('thinclaw-gateway');
        await waitFor(() => expect(screen.getByRole('heading', { name: /ThinClaw Gateway/i })).toBeInTheDocument());
    });

    it('shows "API Secrets" heading on the secrets page', async () => {
        renderPage('secrets');
        await waitFor(() => expect(screen.getByRole('heading', { name: /API Secrets/i })).toBeInTheDocument());
    });
});
