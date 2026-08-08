import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { listTools, toggleTool } = vi.hoisted(() => ({
    listTools: vi.fn(),
    toggleTool: vi.fn(),
}));

vi.mock('../../lib/thinclaw', () => ({
    listTools,
    toggleTool,
}));

import { ThinClawToolPolicies } from '../../components/thinclaw/ThinClawToolPolicies';

const tools = [
    {
        name: 'write_file',
        description: 'Write a file in the workspace',
        enabled: false,
        source: 'container',
        risk: 'high' as const,
        risk_reasons: ['filesystem_mutation'],
    },
    {
        name: 'read_file',
        description: 'Read a file from the workspace',
        enabled: false,
        source: 'container',
        risk: 'low' as const,
        risk_reasons: ['read_only'],
    },
    {
        name: 'shell',
        description: 'Execute a shell command',
        enabled: true,
        source: 'container',
        risk: 'high' as const,
        risk_reasons: ['command_execution'],
    },
];

describe('ThinClawToolPolicies', () => {
    beforeEach(() => {
        listTools.mockReset();
        toggleTool.mockReset();
        listTools.mockResolvedValue({ tools, total: tools.length });
        toggleTool.mockImplementation(async (_name: string, currentlyEnabled: boolean) => !currentlyEnabled);
    });

    it('requires confirmation before enabling a high-risk tool and cancel is inert', async () => {
        render(<ThinClawToolPolicies />);

        fireEvent.click(await screen.findByRole('button', { name: 'Enable write_file' }));
        expect(toggleTool).not.toHaveBeenCalled();

        const dialog = await screen.findByRole('dialog', { name: 'Enable high-risk tool write_file?' });
        expect(within(dialog).getByText(/create, edit, move, or delete files/i)).toBeInTheDocument();

        fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
        await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
        expect(toggleTool).not.toHaveBeenCalled();
        expect(screen.getByRole('button', { name: 'Enable write_file' })).toBeInTheDocument();
    });

    it('inherits focus management and Escape cancellation from the shared dialog', async () => {
        const user = userEvent.setup();
        render(<ThinClawToolPolicies />);

        const trigger = await screen.findByRole('button', { name: 'Enable write_file' });
        await user.click(trigger);
        const dialog = await screen.findByRole('dialog', { name: 'Enable high-risk tool write_file?' });
        expect(within(dialog).getByRole('button', { name: 'Cancel' })).toHaveFocus();

        await user.keyboard('{Escape}');
        await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
        expect(toggleTool).not.toHaveBeenCalled();
        expect(trigger).toHaveFocus();
    });

    it('enables a high-risk tool only after explicit confirmation', async () => {
        render(<ThinClawToolPolicies />);

        fireEvent.click(await screen.findByRole('button', { name: 'Enable write_file' }));
        const dialog = await screen.findByRole('dialog', { name: 'Enable high-risk tool write_file?' });
        fireEvent.click(within(dialog).getByRole('button', { name: 'Enable high-risk tool' }));

        await waitFor(() => {
            expect(toggleTool).toHaveBeenCalledTimes(1);
            expect(toggleTool).toHaveBeenCalledWith('write_file', false);
            expect(screen.getByRole('button', { name: 'Disable write_file' })).toBeInTheDocument();
        });
        expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    });

    it('keeps low-risk enable and all disable operations confirmation-free', async () => {
        render(<ThinClawToolPolicies />);

        fireEvent.click(await screen.findByRole('button', { name: 'Enable read_file' }));
        await waitFor(() => {
            expect(toggleTool).toHaveBeenCalledWith('read_file', false);
            expect(screen.getByRole('button', { name: 'Disable read_file' })).toBeInTheDocument();
        });
        expect(screen.queryByRole('dialog')).not.toBeInTheDocument();

        fireEvent.click(screen.getByRole('button', { name: 'Disable shell' }));
        await waitFor(() => expect(toggleTool).toHaveBeenCalledWith('shell', true));
        expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    });

    it('keeps prior state and surfaces an accessible error when persistence fails', async () => {
        toggleTool.mockRejectedValueOnce(new Error('settings store unavailable'));
        render(<ThinClawToolPolicies />);

        fireEvent.click(await screen.findByRole('button', { name: 'Enable read_file' }));

        expect(await screen.findByRole('alert')).toHaveTextContent('settings store unavailable');
        expect(screen.getByRole('button', { name: 'Enable read_file' })).toBeInTheDocument();
    });
});
