import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({ listSettings: vi.fn() }));

vi.mock('../../lib/thinclaw', () => api);
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

import { ThinClawConfig } from '../../components/thinclaw/ThinClawConfig';

describe('ThinClawConfig safety', () => {
    it('redacts sensitive-looking stored values before display', async () => {
        api.listSettings.mockResolvedValue({
            settings: [
                { key: 'channels.discord_token', value: 'do-not-display', updated_at: '2026-01-01T00:00:00Z' },
                { key: 'agent.max_steps', value: 4, updated_at: '2026-01-01T00:00:00Z' },
            ],
        });

        render(<ThinClawConfig />);

        expect(await screen.findByText('channels.discord_token')).toBeInTheDocument();
        expect(screen.getByText('[redacted by Desktop]')).toBeInTheDocument();
        expect(screen.queryByText('do-not-display')).not.toBeInTheDocument();
        expect(screen.getByText('4')).toBeInTheDocument();
    });
});
