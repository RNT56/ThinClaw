export type ParsedStatusPart =
    | { type: 'text'; content: string }
    | { type: 'status'; props: StatusTagProps };

export type StatusTagProps = {
    type: string;
    query?: string;
    name?: string;
    status?: string;
    msg?: string;
    pct?: string;
};

export const STATUS_TAG_REGEX = /<(?:thinclaw_status|scrappy_status)\b([^>]*)\/>/g;

function parseStatusAttributes(rawAttrs: string): StatusTagProps | null {
    const attrs: Record<string, string> = {};
    const attrRegex = /([a-zA-Z_][\w:-]*)="([^"]*)"/g;
    let attrMatch;

    while ((attrMatch = attrRegex.exec(rawAttrs)) !== null) {
        attrs[attrMatch[1]] = attrMatch[2];
    }

    if (!attrs.type) return null;

    return {
        type: attrs.type,
        query: attrs.query,
        name: attrs.name,
        status: attrs.status,
        msg: attrs.msg,
        pct: attrs.pct,
    };
}

export function parseStatusTaggedContent(content: string): ParsedStatusPart[] {
    const parts: ParsedStatusPart[] = [];
    const regex = new RegExp(STATUS_TAG_REGEX);

    let lastIndex = 0;
    let match;

    while ((match = regex.exec(content)) !== null) {
        if (match.index > lastIndex) {
            parts.push({ type: 'text', content: content.slice(lastIndex, match.index) });
        }
        const props = parseStatusAttributes(match[1] ?? '');
        if (props) {
            parts.push({ type: 'status', props });
        }
        lastIndex = regex.lastIndex;
    }

    if (lastIndex < content.length) {
        parts.push({ type: 'text', content: content.slice(lastIndex) });
    }

    const lastPart = parts[parts.length - 1];
    if (lastPart && lastPart.type === 'text' && lastPart.content.trim().endsWith('[Stopped]')) {
        lastPart.content = lastPart.content.replace(/\[Stopped\]\s*$/, '');
        parts.push({ type: 'status', props: { type: 'stopped' } });
    }

    return parts;
}
