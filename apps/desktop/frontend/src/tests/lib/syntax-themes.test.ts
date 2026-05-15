import { describe, expect, it } from 'vitest';
import { DARK_SYNTAX_THEMES, LIGHT_SYNTAX_THEMES, normalizeSyntaxThemeId } from '../../lib/syntax-themes';

describe('syntax theme naming migration', () => {
    it('uses ThinClaw identifiers for default syntax themes', () => {
        expect(DARK_SYNTAX_THEMES.some(theme => theme.id === 'thinclaw-dark')).toBe(true);
        expect(LIGHT_SYNTAX_THEMES.some(theme => theme.id === 'thinclaw-light')).toBe(true);
        expect(DARK_SYNTAX_THEMES.some(theme => theme.id === 'scrappy-dark')).toBe(false);
        expect(LIGHT_SYNTAX_THEMES.some(theme => theme.id === 'scrappy-light')).toBe(false);
    });

    it('normalizes legacy stored syntax theme identifiers', () => {
        expect(normalizeSyntaxThemeId('scrappy-dark')).toBe('thinclaw-dark');
        expect(normalizeSyntaxThemeId('scrappy-light')).toBe('thinclaw-light');
        expect(normalizeSyntaxThemeId('tokyo-night')).toBe('tokyo-night');
    });
});
