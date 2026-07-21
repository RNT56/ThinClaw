import { describe, expect, it } from 'vitest';
import {
    bridgeErrorMessage,
    describeCommandError,
    ThinClawCommandError,
} from '../../lib/command-errors';

describe('command error surfaces', () => {
    it('renders unavailable capabilities with their remediation', () => {
        const error = {
            kind: 'unavailable',
            capability: 'remote route simulation',
            reason: 'the gateway does not expose this endpoint',
            remediation: 'upgrade the gateway',
            satisfied_by: 'local_only',
        };

        expect(bridgeErrorMessage(error)).toBe(
            'remote route simulation: the gateway does not expose this endpoint (upgrade the gateway)',
        );
        expect(describeCommandError(error)).toMatchObject({
            kind: 'unavailable',
            retryable: false,
            remediation: 'upgrade the gateway',
        });
    });

    it('preserves retryability for transport failures', () => {
        const error = new ThinClawCommandError({
            kind: 'network',
            message: 'gateway disconnected',
            retryable: true,
        });

        expect(error.message).toBe('gateway disconnected');
        expect(error.kind).toBe('network');
        expect(error.retryable).toBe(true);
    });

    it('never renders typed errors as object placeholders', () => {
        expect(bridgeErrorMessage({ kind: 'runtime', message: 'engine stopped' }))
            .toBe('engine stopped');
        expect(bridgeErrorMessage({ unexpected: true })).not.toContain('[object Object]');
    });
});
