import { commands, type Result } from './bindings';

type GeneratedCommands = typeof commands;
type GeneratedCommand = (...args: never[]) => Promise<unknown>;
type ResultData<T> = Awaited<T> extends Result<infer Data, unknown> ? Data : Awaited<T>;

/**
 * The generated binding surface with its transport Result unwrapped.
 *
 * Parameter lists and return data stay derived from bindings.ts, so a Rust
 * command rename or signature change fails TypeScript instead of reaching IPC
 * as an unchecked string.
 */
export type CommandClient = {
    [Name in keyof GeneratedCommands]: GeneratedCommands[Name] extends GeneratedCommand
        ? (...args: Parameters<GeneratedCommands[Name]>) => Promise<ResultData<ReturnType<GeneratedCommands[Name]>>>
        : never;
};

function bridgeErrorMessage(error: unknown): string {
    if (error instanceof Error) return error.message;
    if (typeof error === 'string') return error;
    if (error && typeof error === 'object') {
        const value = error as Record<string, unknown>;
        const reason = typeof value.reason === 'string' ? value.reason : null;
        const remediation = typeof value.remediation === 'string' ? value.remediation : null;
        const capability = typeof value.capability === 'string' ? value.capability : null;
        if (reason) {
            const prefix = capability ? `${capability}: ` : '';
            return `${prefix}${reason}${remediation ? ` (${remediation})` : ''}`;
        }

        try {
            return JSON.stringify(error);
        } catch {
            // Fall through to the stable generic conversion below.
        }
    }
    return String(error ?? 'Unknown ThinClaw command error');
}

function isTransportResult(value: unknown): value is Result<unknown, unknown> {
    if (!value || typeof value !== 'object') return false;
    const result = value as Record<string, unknown>;
    return result.status === 'ok'
        ? Object.prototype.hasOwnProperty.call(result, 'data')
        : result.status === 'error' && Object.prototype.hasOwnProperty.call(result, 'error');
}

function requireTauriRuntime(command: string): void {
    if (typeof globalThis === 'undefined' || !(globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__) {
        throw new Error(`Tauri runtime not available (calling ${command})`);
    }
}

export const commandClient = new Proxy(commands, {
    get(target, property, receiver) {
        const command = Reflect.get(target, property, receiver);
        if (typeof command !== 'function') return command;

        return async (...args: unknown[]) => {
            requireTauriRuntime(String(property));
            const result: unknown = await command(...args);
            if (!isTransportResult(result)) return result;
            if (result.status === 'error') throw new Error(bridgeErrorMessage(result.error));
            return result.data;
        };
    },
}) as unknown as CommandClient;

/**
 * Temporary return-type adapter for lib/thinclaw.ts.
 *
 * The legacy shim still exposes a few richer domain types over Rust commands
 * that currently serialize as JsonValue. Parameters remain generated and
 * checked; new code should use commandClient and its generated return types.
 */
export type CompatibilityCommandClient = {
    [Name in keyof GeneratedCommands]: GeneratedCommands[Name] extends GeneratedCommand
        ? <Return = never>(...args: Parameters<GeneratedCommands[Name]>) => Promise<Return>
        : never;
};

export const compatibilityCommands = commandClient as CompatibilityCommandClient;
