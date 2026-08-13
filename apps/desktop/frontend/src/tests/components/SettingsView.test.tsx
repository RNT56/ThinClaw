import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const layout = vi.hoisted(() => ({
    activeTab: 'thinclaw-slack',
    setActiveTab: vi.fn(),
    setActiveThinClawPage: vi.fn(),
}));

vi.mock('../../components/chat/ChatProvider', () => ({
    useChatLayout: () => layout,
}));

vi.mock('../../components/settings/SettingsPages', () => ({
    SettingsContent: ({ activePage }: { activePage: string }) => <div>page:{activePage}</div>,
}));

import { SettingsView } from '../../components/chat/views/SettingsView';

describe('SettingsView legacy channel redirects', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        layout.activeTab = 'thinclaw-slack';
    });

    it.each(['thinclaw-slack', 'thinclaw-telegram'])('routes %s into Channel Center without mounting a legacy editor', async (activeTab) => {
        layout.activeTab = activeTab;
        render(<SettingsView />);

        expect(screen.getByText(`page:${activeTab}`)).toBeInTheDocument();
        await waitFor(() => {
            expect(layout.setActiveThinClawPage).toHaveBeenCalledWith('channel-config');
            expect(layout.setActiveTab).toHaveBeenCalledWith('thinclaw');
        });
    });
});
