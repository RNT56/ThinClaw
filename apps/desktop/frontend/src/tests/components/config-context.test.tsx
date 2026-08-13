import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { ConfigProvider, useConfigContext } from '../../components/config-context';
import type { UserConfig } from '../../lib/bindings';
import { commandClient as commands } from '../../lib/command-client';
import { directCommands } from '../../lib/generated/direct-commands';
import { LOCAL_CHAT_RUNTIME_RESTART_EVENT } from '../../lib/local-runtime-start';

const toastError = vi.fn();

vi.mock('sonner', () => ({
    toast: {
        error: (...args: unknown[]) => toastError(...args),
    },
}));

const initialConfig = {
    selected_chat_provider: 'local',
    chat_backend: 'local',
    inference_models: {},
} as UserConfig;

function ConfigProbe() {
    const { config, updateConfig } = useConfigContext();
    const selectCloud = async () => {
        if (!config) return;
        try {
            await updateConfig({
                ...config,
                selected_chat_provider: 'openai',
                chat_backend: 'openai',
            });
        } catch {
            document.body.dataset.updateFailed = 'true';
        }
    };

    return (
        <>
            <output data-testid="provider">{config?.selected_chat_provider ?? 'loading'}</output>
            <button onClick={selectCloud}>Select cloud</button>
        </>
    );
}

describe('ConfigProvider updates', () => {
    beforeEach(() => {
        delete document.body.dataset.updateFailed;
        toastError.mockReset();
        vi.spyOn(commands, 'getUserConfig').mockResolvedValue(initialConfig);
        vi.spyOn(directCommands, 'directRuntimeStopChatServer').mockResolvedValue(null);
        vi.spyOn(directCommands, 'directRuntimeStopEngine').mockResolvedValue(null);
    });

    it('publishes config state only after the backend accepts the patch', async () => {
        let resolveUpdate!: (value: Awaited<ReturnType<typeof commands.updateUserConfig>>) => void;
        const update = vi.spyOn(commands, 'updateUserConfig').mockImplementation(() =>
            new Promise(resolve => {
                resolveUpdate = resolve;
            }),
        );

        render(
            <ConfigProvider>
                <ConfigProbe />
            </ConfigProvider>,
        );
        await screen.findByText('local');

        fireEvent.click(screen.getByRole('button', { name: 'Select cloud' }));
        expect(screen.getByTestId('provider')).toHaveTextContent('local');
        await waitFor(() =>
            expect(update).toHaveBeenCalledOnce(),
        );

        await act(async () => {
            resolveUpdate(null);
        });

        await waitFor(() =>
            expect(screen.getByTestId('provider')).toHaveTextContent('openai'),
        );
        expect(update).toHaveBeenCalledWith({
            selected_chat_provider: 'openai',
            chat_backend: 'openai',
        });
        expect(directCommands.directRuntimeStopChatServer).toHaveBeenCalledWith('');
        expect(directCommands.directRuntimeStopEngine).toHaveBeenCalledOnce();
    });

    it('keeps the prior config and rejects when the backend rejects the patch', async () => {
        const restartRequested = vi.fn();
        window.addEventListener(LOCAL_CHAT_RUNTIME_RESTART_EVENT, restartRequested);
        vi.spyOn(commands, 'updateUserConfig').mockRejectedValue(new Error('write failed'));

        render(
            <ConfigProvider>
                <ConfigProbe />
            </ConfigProvider>,
        );
        await screen.findByText('local');

        fireEvent.click(screen.getByRole('button', { name: 'Select cloud' }));

        await waitFor(() =>
            expect(document.body.dataset.updateFailed).toBe('true'),
        );
        expect(screen.getByTestId('provider')).toHaveTextContent('local');
        expect(toastError).toHaveBeenCalledWith('Failed to save settings');
        expect(restartRequested).toHaveBeenCalledOnce();
        window.removeEventListener(LOCAL_CHAT_RUNTIME_RESTART_EVENT, restartRequested);
    });

    it('keeps local selected when either local runtime cannot be stopped', async () => {
        const restartRequested = vi.fn();
        window.addEventListener(LOCAL_CHAT_RUNTIME_RESTART_EVENT, restartRequested);
        vi.mocked(directCommands.directRuntimeStopEngine).mockRejectedValue(new Error('engine stop failed'));
        const update = vi.spyOn(commands, 'updateUserConfig').mockResolvedValue(null);

        render(
            <ConfigProvider>
                <ConfigProbe />
            </ConfigProvider>,
        );
        await screen.findByText('local');

        fireEvent.click(screen.getByRole('button', { name: 'Select cloud' }));

        await waitFor(() =>
            expect(document.body.dataset.updateFailed).toBe('true'),
        );
        expect(screen.getByTestId('provider')).toHaveTextContent('local');
        expect(update).not.toHaveBeenCalled();
        expect(directCommands.directRuntimeStopChatServer).toHaveBeenCalledOnce();
        expect(directCommands.directRuntimeStopEngine).toHaveBeenCalledOnce();
        expect(restartRequested).toHaveBeenCalledOnce();
        window.removeEventListener(LOCAL_CHAT_RUNTIME_RESTART_EVENT, restartRequested);
    });
});
