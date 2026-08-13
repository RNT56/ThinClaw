import { useEffect, useState } from 'react';
import { KeyRound, MessageSquare, RotateCcw, Save, Send, Trash2 } from 'lucide-react';
import { toast } from 'sonner';

import type {
    ChannelSecretMutation,
    ChannelSettingsMutationResponse,
    ChannelSettingsSnapshot,
    SlackChannelSettingsUpdate,
    TelegramChannelSettingsUpdate,
} from '../../lib/bindings';
import { describeCommandError } from '../../lib/command-errors';
import { thinclawCommands } from '../../lib/generated/thinclaw-commands';
import { Button, ConfirmDialog, Notice, StatusBadge, Surface } from '../ui';

type SecretField = 'slack_bot' | 'slack_signing' | 'slack_app' | 'telegram_bot';

interface ClearTarget {
    field: SecretField;
    label: string;
}

interface ChannelCredentialSettingsProps {
    snapshot: ChannelSettingsSnapshot;
    onSnapshot: (snapshot: ChannelSettingsSnapshot) => void;
}

const PRESERVE: ChannelSecretMutation = { action: 'preserve' };

function secretValue(mutation: ChannelSecretMutation): string {
    return mutation.action === 'replace' ? mutation.value : '';
}

function secretChange(value: string): ChannelSecretMutation {
    return value.trim().length === 0 ? PRESERVE : { action: 'replace', value };
}

function credentialLabel(configured: boolean, migrationRequired: boolean, mutation: ChannelSecretMutation): string {
    if (mutation.action === 'clear') return 'Clear selected';
    if (mutation.action === 'replace') return 'Replacement ready';
    if (migrationRequired) return 'Legacy storage — save to migrate';
    return configured ? 'Configured securely' : 'Not configured';
}

function outcomeMessage(response: ChannelSettingsMutationResponse): string {
    if (response.restart_required) return `${response.note} Restart required.`;
    return response.note;
}

export function ChannelCredentialSettings({ snapshot, onSnapshot }: ChannelCredentialSettingsProps) {
    const [slackEnabled, setSlackEnabled] = useState(snapshot.slack.enabled);
    const [slackPolicy, setSlackPolicy] = useState(snapshot.slack.dm_policy);
    const [slackBot, setSlackBot] = useState<ChannelSecretMutation>(PRESERVE);
    const [slackSigning, setSlackSigning] = useState<ChannelSecretMutation>(PRESERVE);
    const [slackApp, setSlackApp] = useState<ChannelSecretMutation>(PRESERVE);
    const [telegramEnabled, setTelegramEnabled] = useState(snapshot.telegram.enabled);
    const [telegramPolicy, setTelegramPolicy] = useState(snapshot.telegram.dm_policy);
    const [telegramGroups, setTelegramGroups] = useState(snapshot.telegram.groups_enabled);
    const [telegramBot, setTelegramBot] = useState<ChannelSecretMutation>(PRESERVE);
    const [saving, setSaving] = useState<'slack' | 'telegram' | null>(null);
    const [notice, setNotice] = useState<{ title: string; message: string } | null>(null);
    const [clearTarget, setClearTarget] = useState<ClearTarget | null>(null);

    useEffect(() => {
        setSlackEnabled(snapshot.slack.enabled);
        setSlackPolicy(snapshot.slack.dm_policy);
        setSlackBot(PRESERVE);
        setSlackSigning(PRESERVE);
        setSlackApp(PRESERVE);
        setTelegramEnabled(snapshot.telegram.enabled);
        setTelegramPolicy(snapshot.telegram.dm_policy);
        setTelegramGroups(snapshot.telegram.groups_enabled);
        setTelegramBot(PRESERVE);
    }, [snapshot.revision]);

    const setSecret = (field: SecretField, mutation: ChannelSecretMutation) => {
        if (field === 'slack_bot') setSlackBot(mutation);
        if (field === 'slack_signing') setSlackSigning(mutation);
        if (field === 'slack_app') setSlackApp(mutation);
        if (field === 'telegram_bot') setTelegramBot(mutation);
    };

    const saveSlack = async () => {
        const update: SlackChannelSettingsUpdate = {
            expected_revision: snapshot.revision,
            enabled: slackEnabled === snapshot.slack.enabled ? null : slackEnabled,
            dm_policy: slackPolicy === snapshot.slack.dm_policy ? null : slackPolicy,
            bot_token: slackBot,
            app_token: slackApp,
            signing_secret: slackSigning,
        };
        setSaving('slack');
        setNotice(null);
        const toastId = toast.loading('Saving Slack channel settings…');
        try {
            const response = await thinclawCommands.thinclawUpdateSlackChannelSettings(update);
            onSnapshot(response.snapshot);
            const message = outcomeMessage(response);
            setNotice({ title: response.applied ? 'Slack updated' : 'Slack settings saved', message });
            response.applied && !response.restart_required
                ? toast.success(message, { id: toastId })
                : toast.info(message, { id: toastId });
        } catch (error) {
            const details = describeCommandError(error);
            setNotice({ title: details.title, message: details.remediation ? `${details.message} ${details.remediation}` : details.message });
            toast.error(details.message, { id: toastId });
        } finally {
            setSaving(null);
        }
    };

    const saveTelegram = async () => {
        const update: TelegramChannelSettingsUpdate = {
            expected_revision: snapshot.revision,
            enabled: telegramEnabled === snapshot.telegram.enabled ? null : telegramEnabled,
            dm_policy: telegramPolicy === snapshot.telegram.dm_policy ? null : telegramPolicy,
            groups_enabled: telegramGroups === snapshot.telegram.groups_enabled ? null : telegramGroups,
            bot_token: telegramBot,
        };
        setSaving('telegram');
        setNotice(null);
        const toastId = toast.loading('Saving Telegram channel settings…');
        try {
            const response = await thinclawCommands.thinclawUpdateTelegramChannelSettings(update);
            onSnapshot(response.snapshot);
            const message = outcomeMessage(response);
            setNotice({ title: response.applied ? 'Telegram updated' : 'Telegram settings saved', message });
            response.applied && !response.restart_required
                ? toast.success(message, { id: toastId })
                : toast.info(message, { id: toastId });
        } catch (error) {
            const details = describeCommandError(error);
            setNotice({ title: details.title, message: details.remediation ? `${details.message} ${details.remediation}` : details.message });
            toast.error(details.message, { id: toastId });
        } finally {
            setSaving(null);
        }
    };

    const confirmClear = () => {
        if (!clearTarget) return;
        setSecret(clearTarget.field, { action: 'clear' });
        setClearTarget(null);
    };

    const editable = snapshot.editable;
    const inputClass = 'h-[var(--control-height)] w-full rounded-[var(--radius-control)] border border-surface-outline bg-surface-subtle px-3 text-xs text-content-primary outline-none focus-visible:ring-2 focus-visible:ring-primary/20 disabled:cursor-not-allowed disabled:opacity-60';

    return (
        <div className="space-y-4" aria-label="Slack and Telegram channel settings">
            <div className="flex flex-wrap items-center justify-between gap-3">
                <div>
                    <h2 className="text-sm font-semibold">Slack and Telegram</h2>
                    <p className="mt-1 text-xs text-content-muted">Current values are loaded from the effective runtime. Credentials remain redacted and blank inputs preserve what is stored.</p>
                </div>
                <StatusBadge status={snapshot.source === 'remote' ? 'stopped' : 'healthy'} label={snapshot.source === 'remote' ? 'Remote read-only' : 'Local secure settings'} />
            </div>

            {!editable && (
                <Notice tone="warning" title="Remote channel settings are read-only">
                    {snapshot.reason ?? 'Configure these credentials on the selected gateway host.'}
                </Notice>
            )}
            {notice && <Notice tone="warning" title={notice.title}>{notice.message}</Notice>}

            <div className="grid gap-4 lg:grid-cols-2">
                <Surface className="space-y-4 p-5" aria-label="Slack settings">
                    <div className="flex items-start justify-between gap-3">
                        <div className="flex items-center gap-2">
                            <MessageSquare className="size-4 text-primary" aria-hidden="true" />
                            <div>
                                <h3 className="text-sm font-semibold">Slack</h3>
                                <p className="text-[11px] text-content-muted">Events API bot and request-signing credentials</p>
                            </div>
                        </div>
                        <StatusBadge status={snapshot.slack.status} label={snapshot.slack.status.replace(/_/g, ' ')} />
                    </div>

                    <label className="flex items-center justify-between gap-3 text-xs font-medium">
                        Enabled
                        <input aria-label="Enable Slack" type="checkbox" checked={slackEnabled} onChange={(event) => setSlackEnabled(event.target.checked)} disabled={!editable} className="size-4 accent-primary" />
                    </label>
                    <label className="block space-y-1.5 text-xs font-medium">
                        Direct-message policy
                        <select aria-label="Slack direct-message policy" value={slackPolicy} onChange={(event) => setSlackPolicy(event.target.value)} disabled={!editable} className={inputClass}>
                            <option value="pairing">Pairing</option>
                            <option value="allowlist">Allowlist</option>
                            <option value="open">Open</option>
                        </select>
                    </label>
                    <SecretEditor label="Bot token" field="slack_bot" configured={snapshot.slack.bot_token_configured} migrationRequired={snapshot.slack.bot_token_migration_required} mutation={slackBot} editable={editable} inputClass={inputClass} onChange={setSlackBot} onClear={setClearTarget} />
                    <SecretEditor label="Signing secret" field="slack_signing" configured={snapshot.slack.signing_secret_configured} migrationRequired={false} mutation={slackSigning} editable={editable} inputClass={inputClass} onChange={setSlackSigning} onClear={setClearTarget} />
                    <details className="rounded-[var(--radius-control)] border border-surface-outline p-3">
                        <summary className="cursor-pointer text-xs font-medium">Legacy Socket Mode credential</summary>
                        <p className="mb-3 mt-1 text-[10px] text-content-muted">Kept for one-release compatibility. The current Slack channel uses the signing secret above.</p>
                        <SecretEditor label="Legacy app token" field="slack_app" configured={snapshot.slack.app_token_configured} migrationRequired={snapshot.slack.app_token_migration_required} mutation={slackApp} editable={editable} inputClass={inputClass} onChange={setSlackApp} onClear={setClearTarget} />
                    </details>
                    {editable && (
                        <Button size="sm" variant="primary" onClick={() => void saveSlack()} disabled={saving !== null}>
                            <Save className="size-3.5" aria-hidden="true" /> {saving === 'slack' ? 'Saving…' : 'Save Slack'}
                        </Button>
                    )}
                </Surface>

                <Surface className="space-y-4 p-5" aria-label="Telegram settings">
                    <div className="flex items-start justify-between gap-3">
                        <div className="flex items-center gap-2">
                            <Send className="size-4 text-primary" aria-hidden="true" />
                            <div>
                                <h3 className="text-sm font-semibold">Telegram</h3>
                                <p className="text-[11px] text-content-muted">Bot delivery and group admission</p>
                            </div>
                        </div>
                        <StatusBadge status={snapshot.telegram.status} label={snapshot.telegram.status.replace(/_/g, ' ')} />
                    </div>
                    <label className="flex items-center justify-between gap-3 text-xs font-medium">
                        Enabled
                        <input aria-label="Enable Telegram" type="checkbox" checked={telegramEnabled} onChange={(event) => setTelegramEnabled(event.target.checked)} disabled={!editable} className="size-4 accent-primary" />
                    </label>
                    <label className="block space-y-1.5 text-xs font-medium">
                        Direct-message policy
                        <select aria-label="Telegram direct-message policy" value={telegramPolicy} onChange={(event) => setTelegramPolicy(event.target.value)} disabled={!editable} className={inputClass}>
                            <option value="pairing">Pairing</option>
                            <option value="allowlist">Allowlist</option>
                            <option value="open">Open</option>
                        </select>
                    </label>
                    <label className="flex items-center justify-between gap-3 text-xs font-medium">
                        Accept group messages
                        <input aria-label="Accept Telegram group messages" type="checkbox" checked={telegramGroups} onChange={(event) => setTelegramGroups(event.target.checked)} disabled={!editable} className="size-4 accent-primary" />
                    </label>
                    <p className="text-[10px] text-content-muted">{snapshot.telegram.require_mention ? 'Group messages must mention the bot.' : 'The bot may respond without a mention.'}</p>
                    <SecretEditor label="Bot token" field="telegram_bot" configured={snapshot.telegram.bot_token_configured} migrationRequired={snapshot.telegram.bot_token_migration_required} mutation={telegramBot} editable={editable} inputClass={inputClass} onChange={setTelegramBot} onClear={setClearTarget} />
                    {editable && (
                        <Button size="sm" variant="primary" onClick={() => void saveTelegram()} disabled={saving !== null}>
                            <Save className="size-3.5" aria-hidden="true" /> {saving === 'telegram' ? 'Saving…' : 'Save Telegram'}
                        </Button>
                    )}
                </Surface>
            </div>

            <ConfirmDialog
                open={clearTarget !== null}
                onOpenChange={(open) => { if (!open) setClearTarget(null); }}
                title={`Clear ${clearTarget?.label ?? 'credential'}?`}
                description="The credential will be deleted only when you save this channel. Other fields and credentials are preserved."
                confirmLabel="Select clear"
                onConfirm={confirmClear}
            />
        </div>
    );
}

function SecretEditor({
    label,
    field,
    configured,
    migrationRequired,
    mutation,
    editable,
    inputClass,
    onChange,
    onClear,
}: {
    label: string;
    field: SecretField;
    configured: boolean;
    migrationRequired: boolean;
    mutation: ChannelSecretMutation;
    editable: boolean;
    inputClass: string;
    onChange: (mutation: ChannelSecretMutation) => void;
    onClear: (target: ClearTarget) => void;
}) {
    return (
        <div className="space-y-1.5">
            <div className="flex items-center justify-between gap-2">
                <label htmlFor={`channel-secret-${field}`} className="flex items-center gap-1.5 text-xs font-medium">
                    <KeyRound className="size-3" aria-hidden="true" /> {label}
                </label>
                <span className="text-[10px] text-content-muted">{credentialLabel(configured, migrationRequired, mutation)}</span>
            </div>
            <input
                id={`channel-secret-${field}`}
                type="password"
                autoComplete="new-password"
                value={secretValue(mutation)}
                onChange={(event) => onChange(secretChange(event.target.value))}
                disabled={!editable}
                placeholder={mutation.action === 'clear' ? 'Will be cleared on save' : migrationRequired ? 'Legacy credential — blank preserves and migrates' : configured ? 'Stored securely — blank preserves' : 'Enter a credential'}
                className={inputClass}
            />
            {editable && (
                <div className="flex flex-wrap gap-2">
                    {mutation.action !== 'preserve' && (
                        <Button size="sm" variant="ghost" onClick={() => onChange(PRESERVE)}>
                            <RotateCcw className="size-3" aria-hidden="true" /> Preserve stored
                        </Button>
                    )}
                    {(configured || mutation.action === 'replace') && mutation.action !== 'clear' && (
                        <Button size="sm" variant="ghost" onClick={() => onClear({ field, label })}>
                            <Trash2 className="size-3" aria-hidden="true" /> Clear
                        </Button>
                    )}
                </div>
            )}
        </div>
    );
}
