import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

const refresh = vi.fn();
const onSelectPage = vi.fn();

vi.mock('../../components/thinclaw/AgentCockpitProvider', () => ({
    useAgentCockpit: () => ({
        status: null,
        checkedAt: null,
        error: null,
        isRefreshing: false,
        refresh,
        capability: (key: string) => ({
            state: key === 'always' || key === 'advanced' ? 'available' : 'loading',
            source: 'unknown',
            checkedAt: null,
        }),
    }),
}));

import { ThinClawSidebar } from '../../components/thinclaw/ThinClawSidebar';

describe('ThinClawSidebar', () => {
    it('keeps the roving navigation stop on an enabled route while profile status is loading', () => {
        onSelectPage.mockReset();
        render(
            <ThinClawSidebar
                sidebarOpen
                onBack={vi.fn()}
                onSelectSession={vi.fn()}
                onNewSession={vi.fn()}
                selectedSessionKey={null}
                gatewayRunning={false}
                onNavigateToSettings={vi.fn()}
                activePage="chat"
                onSelectPage={onSelectPage}
            />,
        );

        const home = screen.getByRole('button', { name: 'Home' });
        const chat = screen.getByRole('button', { name: 'Chat' });
        expect(home).toHaveAttribute('tabindex', '0');
        expect(chat).toBeDisabled();
        expect(chat).toHaveAttribute('tabindex', '-1');

        fireEvent.keyDown(home, { key: 'ArrowDown' });
        expect(onSelectPage).toHaveBeenCalledWith('workspace');
    });
});
