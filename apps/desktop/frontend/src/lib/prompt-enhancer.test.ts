import { describe, expect, it } from 'vitest';

import { cleanEnhancedPrompt, enhancedPromptWordCount } from './prompt-enhancer';

describe('prompt enhancer output contract', () => {
    it('removes template pollution, headers, and extra whitespace', () => {
        const cleaned = cleanEnhancedPrompt(
            '<think>hidden reasoning</think>\nEnhanced Prompt: cinematic   fox\nunder moonlight'
        );

        expect(cleaned).toBe('cinematic fox under moonlight');
    });

    it('counts Unicode prompt words without byte-based assumptions', () => {
        expect(enhancedPromptWordCount('portrait of a 🦀 at dawn')).toBe(6);
        expect(enhancedPromptWordCount('   ')).toBe(0);
    });

    it('makes the 75-word contract mechanically testable', () => {
        expect(enhancedPromptWordCount(Array.from({ length: 75 }, () => 'light').join(' '))).toBe(75);
        expect(enhancedPromptWordCount(Array.from({ length: 76 }, () => 'light').join(' '))).toBe(76);
    });
});
