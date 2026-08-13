import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const commands = vi.hoisted(() => ({
    thinclawChannelConfigSchemas: vi.fn(),
    thinclawChannelConfigSubmit: vi.fn(),
    thinclawChannelSettingsSnapshot: vi.fn(),
    thinclawUpdateSlackChannelSettings: vi.fn(),
    thinclawUpdateTelegramChannelSettings: vi.fn(),
}));
const sonner = vi.hoisted(() => ({
    toast: { loading: vi.fn(() => 'toast-id'), success: vi.fn(), info: vi.fn(), error: vi.fn() },
}));

vi.mock('../../lib/generated/thinclaw-commands', () => ({
    thinclawCommands: commands,
}));
vi.mock('sonner', () => ({
    toast: sonner.toast,
}));

import { ThinClawChannelConfig } from '../../components/thinclaw/ThinClawChannelConfig';

describe('ThinClawChannelConfig', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        commands.thinclawChannelConfigSchemas.mockResolvedValue({
            available: true,
            schemas: [{
                channel_id: 'imessage',
                channel_name: 'iMessage',
                fields: [
                    {
                        id: 'allow_from',
                        label: 'Allowed contacts',
                        field_type: 'textarea',
                        required: false,
                        default_value: '+12025550100',
                    },
                    {
                        id: 'poll_interval',
                        label: 'Polling interval (seconds)',
                        field_type: 'number',
                        required: true,
                        default_value: 3,
                    },
                ],
            }],
        });
        commands.thinclawChannelConfigSubmit.mockResolvedValue({ note: 'Saved' });
        commands.thinclawChannelSettingsSnapshot.mockResolvedValue(channelSnapshot());
        commands.thinclawUpdateSlackChannelSettings.mockImplementation(async () => channelMutation(channelSnapshot()));
        commands.thinclawUpdateTelegramChannelSettings.mockImplementation(async () => channelMutation(channelSnapshot()));
    });

    it('submits number fields as numbers and preserves current schema values', async () => {
        render(<ThinClawChannelConfig />);

        const pollInterval = await screen.findByLabelText(/Polling interval \(seconds\)/);
        expect(pollInterval).toHaveValue(3);
        fireEvent.change(pollInterval, { target: { value: '9' } });
        fireEvent.click(screen.getByRole('button', { name: 'Save' }));

        await waitFor(() => {
            expect(commands.thinclawChannelConfigSubmit).toHaveBeenCalledWith('imessage', {
                allow_from: '+12025550100',
                poll_interval: 9,
            });
        });
    });

    it('renders host-managed channels without a misleading save action', async () => {
        commands.thinclawChannelConfigSchemas.mockResolvedValue({
            available: true,
            schemas: [{
                channel_id: 'apns',
                channel_name: 'Apns',
                fields: [],
                help: 'APNs signing identity is host-managed with APNS_PRIVATE_KEY.',
            }],
        });

        render(<ThinClawChannelConfig />);

        expect(await screen.findByText(/APNS_PRIVATE_KEY/)).toBeInTheDocument();
        expect(screen.queryByRole('button', { name: /^Save$/ })).not.toBeInTheDocument();
    });

    it('keeps manifest credentials opaque until encrypted secret binding is available', async () => {
        commands.thinclawChannelConfigSchemas.mockResolvedValue({
            available: true,
            schemas: [{
                channel_id: 'line',
                channel_name: 'Line',
                fields: [{
                    id: 'line_channel_secret',
                    label: 'Channel secret',
                    field_type: 'password',
                    required: true,
                    default_value: null,
                }],
            }],
        });

        render(<ThinClawChannelConfig />);
        expect(await screen.findByText(/Secret configuration is not available in Desktop yet/)).toBeInTheDocument();
        expect(screen.queryByLabelText(/Channel secret/)).not.toBeInTheDocument();
        expect(screen.queryByRole('button', { name: /^Save$/ })).not.toBeInTheDocument();
    });

    it('does not report persisted-but-not-forwarded settings as a full success', async () => {
        commands.thinclawChannelConfigSubmit.mockResolvedValue({
            ok: false,
            persisted: true,
            forwarded: false,
            note: 'Settings were saved and will apply when the channel starts.',
        });

        render(<ThinClawChannelConfig />);
        fireEvent.click(await screen.findByRole('button', { name: 'Save' }));

        await waitFor(() => {
            expect(sonner.toast.info).toHaveBeenCalledWith(
                'Settings were saved and will apply when the channel starts.',
                { id: 'toast-id' },
            );
        });
    });

    it('loads redacted credential state and blank fields preserve stored secrets', async () => {
        render(<ThinClawChannelConfig />);

        const botToken = await screen.findByLabelText('Bot token', { selector: '#channel-secret-slack_bot' });
        expect(botToken).toHaveValue('');
        expect(screen.getAllByText('Configured securely')).toHaveLength(3);
        fireEvent.click(screen.getByRole('button', { name: 'Save Slack' }));

        await waitFor(() => expect(commands.thinclawUpdateSlackChannelSettings).toHaveBeenCalledWith({
            expected_revision: 'sha256:one',
            enabled: null,
            dm_policy: null,
            bot_token: { action: 'preserve' },
            app_token: { action: 'preserve' },
            signing_secret: { action: 'preserve' },
        }));
    });

    it('turns a whitespace-only replacement input back into preserve, never implicit clear', async () => {
        render(<ThinClawChannelConfig />);

        const botToken = await screen.findByLabelText('Bot token', { selector: '#channel-secret-slack_bot' });
        fireEvent.change(botToken, { target: { value: 'replacement' } });
        fireEvent.change(botToken, { target: { value: '   ' } });
        fireEvent.click(screen.getByRole('button', { name: 'Save Slack' }));

        await waitFor(() => expect(commands.thinclawUpdateSlackChannelSettings).toHaveBeenCalledWith(
            expect.objectContaining({ bot_token: { action: 'preserve' } }),
        ));
    });

    it('labels legacy credentials honestly and migrates them through preserve on save', async () => {
        const legacy = channelSnapshot();
        legacy.slack.bot_token_migration_required = true;
        commands.thinclawChannelSettingsSnapshot.mockResolvedValue(legacy);
        render(<ThinClawChannelConfig />);

        expect(await screen.findByText('Legacy storage — save to migrate')).toBeInTheDocument();
        fireEvent.click(screen.getByRole('button', { name: 'Save Slack' }));
        await waitFor(() => expect(commands.thinclawUpdateSlackChannelSettings).toHaveBeenCalledWith(
            expect.objectContaining({ bot_token: { action: 'preserve' } }),
        ));
    });

    it('requires an explicit confirmation before clearing a credential', async () => {
        render(<ThinClawChannelConfig />);

        await screen.findByLabelText('Slack settings');
        const slackCard = screen.getByLabelText('Slack settings');
        fireEvent.click(within(slackCard).getAllByRole('button', { name: 'Clear' })[0]);
        fireEvent.click(screen.getByRole('button', { name: 'Select clear' }));
        fireEvent.click(screen.getByRole('button', { name: 'Save Slack' }));

        await waitFor(() => expect(commands.thinclawUpdateSlackChannelSettings).toHaveBeenCalledWith(
            expect.objectContaining({ bot_token: { action: 'clear' } }),
        ));
    });

    it('fails closed for a remote profile and exposes no mutation action', async () => {
        commands.thinclawChannelSettingsSnapshot.mockResolvedValue({
            ...channelSnapshot(),
            editable: false,
            source: 'remote',
            reason: 'Configure these channels on the gateway host.',
        });
        render(<ThinClawChannelConfig />);

        expect(await screen.findByText('Remote channel settings are read-only')).toBeInTheDocument();
        expect(screen.queryByRole('button', { name: 'Save Slack' })).not.toBeInTheDocument();
        expect(screen.queryByRole('button', { name: 'Save Telegram' })).not.toBeInTheDocument();
        expect(screen.getByLabelText('Enable Slack')).toBeDisabled();
    });

    it('keeps unrelated channel schemas usable when the redacted snapshot is unavailable', async () => {
        commands.thinclawChannelSettingsSnapshot.mockRejectedValue(new Error('Channel Center unavailable'));
        render(<ThinClawChannelConfig />);

        expect(await screen.findByText('iMessage')).toBeInTheDocument();
        expect(screen.getByText('Channel Center unavailable')).toBeInTheDocument();
        expect(screen.queryByLabelText('Slack settings')).not.toBeInTheDocument();
    });

    it('keeps an entered replacement after a revision conflict', async () => {
        commands.thinclawUpdateSlackChannelSettings.mockRejectedValue({
            kind: 'conflict',
            message: 'Channel settings changed in another window',
            remediation: 'Reload Channel Center',
        });
        render(<ThinClawChannelConfig />);

        const botToken = await screen.findByLabelText('Bot token', { selector: '#channel-secret-slack_bot' });
        fireEvent.change(botToken, { target: { value: 'replacement' } });
        fireEvent.click(screen.getByRole('button', { name: 'Save Slack' }));

        expect(await screen.findByText('State changed')).toBeInTheDocument();
        expect(botToken).toHaveValue('replacement');
    });
});

function channelSnapshot() {
    return {
        available: true,
        editable: true,
        source: 'local',
        revision: 'sha256:one',
        reason: null,
        slack: {
            enabled: true,
            dm_policy: 'pairing',
            bot_token_configured: true,
            bot_token_migration_required: false,
            app_token_configured: true,
            app_token_migration_required: false,
            signing_secret_configured: true,
            active: false,
            status: 'configured_not_running',
        },
        telegram: {
            enabled: false,
            dm_policy: 'pairing',
            groups_enabled: true,
            require_mention: true,
            bot_token_configured: false,
            bot_token_migration_required: false,
            active: false,
            status: 'disabled',
        },
    };
}

function channelMutation(snapshot: ReturnType<typeof channelSnapshot>) {
    return {
        snapshot: { ...snapshot, revision: 'sha256:two' },
        persisted: true,
        applied: false,
        restart_required: false,
        note: 'Settings saved.',
    };
}
