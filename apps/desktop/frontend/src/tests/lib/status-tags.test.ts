import { describe, it, expect } from 'vitest';
import { parseStatusTaggedContent, STATUS_TAG_REGEX } from '../../lib/status-tags';

describe('status tag parsing', () => {
    it('parses ThinClaw status tags', () => {
        expect(parseStatusTaggedContent('A <thinclaw_status type="search" query="docs" /> B')).toEqual([
            { type: 'text', content: 'A ' },
            { type: 'status', props: { type: 'search', query: 'docs', name: undefined, status: undefined, msg: undefined, pct: undefined } },
            { type: 'text', content: ' B' },
        ]);
    });

    it('parses legacy Scrappy status tags', () => {
        expect(parseStatusTaggedContent('A <scrappy_status type="thinking" /> B')).toEqual([
            { type: 'text', content: 'A ' },
            { type: 'status', props: { type: 'thinking', query: undefined, name: undefined, status: undefined, msg: undefined, pct: undefined } },
            { type: 'text', content: ' B' },
        ]);
    });

    it('parses status tags with additional backend attributes', () => {
        expect(parseStatusTaggedContent('<thinclaw_status type="tool_call" name="search" query="docs" status="started" />')).toEqual([
            { type: 'status', props: { type: 'tool_call', query: 'docs', name: 'search', status: 'started', msg: undefined, pct: undefined } },
        ]);
    });

    it('strips both current and legacy tags with the shared regex', () => {
        const content = 'A <thinclaw_status type="search" /> B <scrappy_status type="thinking" /> C';

        expect(content.replace(STATUS_TAG_REGEX, '').trim()).toBe('A  B  C');
    });
});
