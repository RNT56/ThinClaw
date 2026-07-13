import { describe, expect, it } from 'vitest';

import { CATEGORY_LABELS, HOOK_TEMPLATES } from '../../components/thinclaw/hooks/templates';

describe('hook template catalog', () => {
    it('keeps template identifiers unique and categories renderable', () => {
        const ids = HOOK_TEMPLATES.map((template) => template.id);

        expect(new Set(ids).size).toBe(ids.length);
        for (const template of HOOK_TEMPLATES) {
            expect(CATEGORY_LABELS[template.category]?.label).toBeTruthy();
        }
    });

    it('ships every template with at least one named lifecycle rule', () => {
        for (const template of HOOK_TEMPLATES) {
            const rules = (template.bundle as { rules?: Array<{ name?: string; points?: string[] }> }).rules;
            expect(rules?.length, template.id).toBeGreaterThan(0);
            for (const rule of rules ?? []) {
                expect(rule.name, template.id).toBeTruthy();
                expect(rule.points?.length, template.id).toBeGreaterThan(0);
            }
        }
    });
});
