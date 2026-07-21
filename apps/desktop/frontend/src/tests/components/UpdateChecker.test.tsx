import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const updater = vi.hoisted(() => ({
    check: vi.fn(),
    downloadAndInstall: vi.fn(),
    relaunch: vi.fn(),
}));

vi.mock('@tauri-apps/plugin-updater', () => ({
    check: updater.check,
}));
vi.mock('@tauri-apps/plugin-process', () => ({
    relaunch: updater.relaunch,
}));

import { UpdateChecker } from '../../components/UpdateChecker';

async function runStartupCheck() {
    render(<UpdateChecker />);
    await act(async () => {
        await vi.advanceTimersByTimeAsync(5000);
    });
}

describe('UpdateChecker', () => {
    beforeEach(() => {
        vi.useFakeTimers();
        vi.clearAllMocks();
        updater.downloadAndInstall.mockImplementation(async (onEvent: (event: unknown) => void) => {
            onEvent({ event: 'Started', data: { contentLength: 1024 } });
            onEvent({ event: 'Progress', data: { chunkLength: 1024 } });
            onEvent({ event: 'Finished' });
        });
        updater.relaunch.mockResolvedValue(undefined);
        updater.check.mockResolvedValue({
            version: '0.16.0',
            body: 'Signed Desktop update',
            downloadAndInstall: updater.downloadAndInstall,
        });
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    it('checks the configured channel, installs the signed update, and relaunches', async () => {
        await runStartupCheck();

        expect(updater.check).toHaveBeenCalledTimes(1);
        expect(screen.getByRole('status')).toHaveTextContent('Update 0.16.0 available');
        expect(screen.getByText('Signed Desktop update')).toBeInTheDocument();

        await act(async () => {
            fireEvent.click(screen.getByRole('button', { name: 'Download & Install' }));
        });
        expect(updater.downloadAndInstall).toHaveBeenCalledTimes(1);
        expect(screen.getByRole('status')).toHaveTextContent('Ready to restart');

        await act(async () => {
            fireEvent.click(screen.getByRole('button', { name: 'Restart Now' }));
        });
        expect(updater.relaunch).toHaveBeenCalledTimes(1);
    });

    it('surfaces channel failures without crashing startup', async () => {
        updater.check.mockRejectedValue(new Error('manifest signature rejected'));

        await runStartupCheck();

        expect(screen.getByRole('alert')).toHaveTextContent('Update failed');
        expect(screen.getByRole('alert')).toHaveTextContent('manifest signature rejected');
        expect(screen.getByRole('button', { name: 'Close update status' })).toBeInTheDocument();
    });
});
