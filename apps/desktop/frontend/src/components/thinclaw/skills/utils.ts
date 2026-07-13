import type { Skill } from '../../../lib/thinclaw';

export function normalizeSkill(raw: any): Skill {
    const name = raw.skillKey || raw.name || raw.key || 'unknown';
    return {
        skillKey: name,
        name: raw.name || name,
        description: raw.description || '',
        disabled: raw.disabled ?? false,
        eligible: raw.eligible ?? true,
        emoji: raw.emoji,
        homepage: raw.homepage,
        source: raw.source || 'installed',
        requirements: raw.requirements,
        missing: raw.missing,
        install: raw.install,
        version: raw.version,
        trust: raw.trust,
        keywords: raw.keywords || []
    };
}

export function actionOk(response: any): boolean {
    return Boolean(response?.success ?? response?.ok);
}
