import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const commands = vi.hoisted(() => ({
    thinclawSecretRecoveryStatus: vi.fn(),
    thinclawSecretRecoveryExport: vi.fn(),
    thinclawSecretRecoveryImport: vi.fn(),
    thinclawSecretMasterKeyRotate: vi.fn(),
    cloudGetRecoveryKey: vi.fn(),
    cloudImportRecoveryKey: vi.fn(),
}));

vi.mock('../../lib/command-client', () => ({ commandClient: commands }));
vi.mock('sonner', () => ({
    toast: { success: vi.fn(), error: vi.fn() },
}));

import { RecoveryKeyPanel } from '../../components/settings/storage/RecoveryKeyPanel';

const status = {
    supported: true,
    unavailable_reason: null,
    cipher: 'AES-256-GCM',
    kdf: 'HKDF-SHA256',
    key_version: 4,
    stored_secrets: 7,
};

describe('RecoveryKeyPanel secret envelope mode', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        commands.thinclawSecretRecoveryStatus.mockResolvedValue(status);
        commands.thinclawSecretRecoveryExport.mockResolvedValue('thinclaw-secrets-v1:backup:checksum');
        commands.thinclawSecretMasterKeyRotate.mockResolvedValue({
            old_key_version: 4,
            new_key_version: 5,
            rotated_secrets: 7,
            recovery_key: 'thinclaw-secrets-v1:rotated:checksum',
        });
    });

    it('shows live encryption metadata and reveals only on explicit request', async () => {
        const view = render(<RecoveryKeyPanel mode="secrets" />);

        expect(await screen.findByText('AES-256-GCM')).toBeInTheDocument();
        expect(screen.getByText('7')).toBeInTheDocument();
        expect(screen.queryByText('thinclaw-secrets-v1:backup:checksum')).not.toBeInTheDocument();

        fireEvent.click(screen.getByRole('button', { name: 'Show Recovery Key' }));
        expect(await screen.findByText('thinclaw-secrets-v1:backup:checksum')).toBeInTheDocument();
        expect(commands.thinclawSecretRecoveryExport).toHaveBeenCalledTimes(1);

        view.unmount();
    });

    it('requires exact confirmation and surfaces the replacement key after rotation', async () => {
        const view = render(<RecoveryKeyPanel mode="secrets" />);
        await screen.findByText('AES-256-GCM');

        fireEvent.click(screen.getByRole('button', { name: 'Rotate Key' }));
        const rotate = screen.getByRole('button', { name: 'Rotate Master Key' });
        expect(rotate).toBeDisabled();

        fireEvent.change(screen.getByLabelText('Rotation confirmation'), { target: { value: 'ROTATE' } });
        expect(rotate).toBeEnabled();
        fireEvent.click(rotate);

        await waitFor(() => {
            expect(commands.thinclawSecretMasterKeyRotate).toHaveBeenCalledWith('ROTATE');
            expect(screen.getByText('thinclaw-secrets-v1:rotated:checksum')).toBeInTheDocument();
        });

        view.unmount();
    });

    it('fails closed when persistent envelope recovery is unavailable', async () => {
        commands.thinclawSecretRecoveryStatus.mockResolvedValue({
            ...status,
            supported: false,
            unavailable_reason: 'Persistent recovery is unavailable on this platform.',
        });
        render(<RecoveryKeyPanel mode="secrets" />);

        expect(await screen.findByText('Persistent recovery is unavailable on this platform.')).toBeInTheDocument();
        expect(screen.getByRole('button', { name: 'Show Recovery Key' })).toBeDisabled();
        expect(screen.getByRole('button', { name: 'Import Key' })).toBeDisabled();
        expect(screen.getByRole('button', { name: 'Rotate Key' })).toBeDisabled();
    });
});
