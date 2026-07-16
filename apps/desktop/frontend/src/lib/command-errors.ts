import type { BridgeError } from './bindings';

export type CommandErrorKind = BridgeError['kind'] | 'unknown';

export interface CommandErrorDetails {
    kind: CommandErrorKind;
    title: string;
    message: string;
    remediation: string | null;
    retryable: boolean;
}

const TITLES: Record<CommandErrorKind, string> = {
    unavailable: 'Feature unavailable',
    invalid_input: 'Check your input',
    unauthorized: 'Authentication required',
    not_found: 'Not found',
    conflict: 'State changed',
    timeout: 'Request timed out',
    network: 'Connection problem',
    runtime: 'ThinClaw could not finish',
    unknown: 'ThinClaw could not finish',
};

function recordValue(error: unknown): Record<string, unknown> | null {
    return error && typeof error === 'object' ? error as Record<string, unknown> : null;
}

function stringValue(value: unknown): string | null {
    return typeof value === 'string' && value.trim() ? value.trim() : null;
}

export function describeCommandError(error: unknown): CommandErrorDetails {
    if (error instanceof ThinClawCommandError) {
        return {
            kind: error.kind,
            title: TITLES[error.kind],
            message: error.message,
            remediation: error.remediation,
            retryable: error.retryable,
        };
    }
    if (error instanceof Error) {
        return {
            kind: 'unknown',
            title: TITLES.unknown,
            message: error.message,
            remediation: null,
            retryable: false,
        };
    }
    if (typeof error === 'string') {
        return {
            kind: 'runtime',
            title: TITLES.runtime,
            message: error,
            remediation: null,
            retryable: false,
        };
    }

    const value = recordValue(error);
    if (value) {
        const rawKind = stringValue(value.kind);
        const kind = rawKind && rawKind in TITLES ? rawKind as CommandErrorKind : 'unknown';
        const capability = stringValue(value.capability);
        const reason = stringValue(value.reason);
        const message = stringValue(value.message)
            ?? (reason ? `${capability ? `${capability}: ` : ''}${reason}` : null)
            ?? 'An unknown command error occurred.';
        return {
            kind,
            title: TITLES[kind],
            message,
            remediation: stringValue(value.remediation),
            retryable: value.retryable === true,
        };
    }

    return {
        kind: 'unknown',
        title: TITLES.unknown,
        message: String(error ?? 'An unknown command error occurred.'),
        remediation: null,
        retryable: false,
    };
}

export function bridgeErrorMessage(error: unknown): string {
    const details = describeCommandError(error);
    return details.remediation
        ? `${details.message} (${details.remediation})`
        : details.message;
}

export class ThinClawCommandError extends Error {
    readonly kind: CommandErrorKind;
    readonly remediation: string | null;
    readonly retryable: boolean;
    readonly raw: unknown;

    constructor(error: unknown) {
        const details = describeCommandError(error);
        super(details.remediation
            ? `${details.message} (${details.remediation})`
            : details.message);
        this.name = 'ThinClawCommandError';
        this.kind = details.kind;
        this.remediation = details.remediation;
        this.retryable = details.retryable;
        this.raw = error;
    }
}
