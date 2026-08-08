import { describe, expect, it } from 'vitest';

import { sanitizeMessageContent } from '../../components/chat/MessageBubble';

describe('message HTML sanitization', () => {
    it('removes executable markup while retaining safe message content', () => {
        const sanitized = sanitizeMessageContent(
            '<p>Hello <strong>world</strong></p><img src="x" onerror="alert(1)"><script>alert(2)</script>',
        );

        expect(sanitized).toContain('<p>Hello <strong>world</strong></p>');
        expect(sanitized).toContain('<img src="x">');
        expect(sanitized).not.toContain('onerror');
        expect(sanitized).not.toContain('<script');
        expect(sanitized).not.toContain('alert(2)');
    });
});
