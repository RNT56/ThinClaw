import { render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

const { securityPosture } = vi.hoisted(() => ({
    securityPosture: vi.fn(),
}));

vi.mock('../../lib/bindings', () => ({
    commands: {
        thinclawSecurityPosture: securityPosture,
    },
}));

import { SecurityPosturePanel } from '../../components/settings/SecurityPosturePanel';

const livePosture = {
    runtime_mode: 'local',
    evidence_available: true,
    unavailable_reason: null,
    telemetry: {
        sanitized: 3,
        redacted: 2,
        blocked: 1,
        warned: 4,
        recent_events: [{
            occurred_at_ms: 1_720_000_000_000,
            action: 'blocked',
            source: 'tool:browser',
            reason: 'prompt_injection',
            severity: 'high',
        }],
    },
    sandbox: {
        enabled: true,
        policy: 'workspace_write',
        network_allowlist: ['api.example.com'],
        timeout_secs: 60,
        memory_limit_mb: 512,
    },
    tools: {
        registered: 12,
        write_capable: 3,
        always_approval: 1,
        conditional_approval: 2,
        write_without_coarse_approval: 0,
        auto_approve_enabled: false,
        reviewed_tools: [{
            name: 'shell',
            side_effect: 'write',
            approval_class: 'always',
            empty_params_requirement: 'always',
            sanitizes_output: true,
            reason: 'Every invocation requires explicit human approval.',
        }],
    },
};

describe('SecurityPosturePanel', () => {
    afterEach(() => {
        vi.clearAllMocks();
    });

    it('renders authoritative local evidence and metadata-only decisions', async () => {
        securityPosture.mockResolvedValue({ status: 'ok', data: livePosture });
        const view = render(<SecurityPosturePanel />);

        expect(await screen.findByText('Live local evidence')).toBeInTheDocument();
        expect(screen.getByText('api.example.com')).toBeInTheDocument();
        expect(screen.getByText('prompt_injection · tool:browser')).toBeInTheDocument();
        expect(screen.getByText('empty call: always')).toBeInTheDocument();
        expect(screen.getByText('Every invocation requires explicit human approval.')).toBeInTheDocument();
        expect(screen.getByText(/never prompts, tool parameters, outputs, or secrets/i)).toBeInTheDocument();

        view.unmount();
    });

    it('explains when the current runtime cannot provide evidence', async () => {
        securityPosture.mockResolvedValue({
            status: 'ok',
            data: {
                ...livePosture,
                runtime_mode: 'remote',
                evidence_available: false,
                unavailable_reason: 'Remote security evidence is not exposed.',
                telemetry: { sanitized: 0, redacted: 0, blocked: 0, warned: 0, recent_events: [] },
                sandbox: null,
                tools: { ...livePosture.tools, registered: 0, reviewed_tools: [] },
            },
        });
        const view = render(<SecurityPosturePanel />);

        await waitFor(() => {
            expect(screen.getByText('Evidence unavailable')).toBeInTheDocument();
            expect(screen.getByText('Remote security evidence is not exposed.')).toBeInTheDocument();
        });

        view.unmount();
    });
});
