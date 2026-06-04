import { describe, it, expect, beforeEach } from 'vitest';
import {
    clearOnboardingProgress,
    isOnboardingInProgress,
    LEGACY_STORAGE_KEYS,
    migrateLocalStorageKey,
    startOnboardingProgress,
    STORAGE_KEYS,
} from '../../lib/local-storage-migration';

describe('local storage migration', () => {
    beforeEach(() => {
        localStorage.clear();
    });

    it('copies legacy model-context keys to ThinClaw keys and keeps legacy values', () => {
        localStorage.setItem(LEGACY_STORAGE_KEYS.modelPath, '/models/legacy.gguf');

        const value = migrateLocalStorageKey(localStorage, 'modelPath');

        expect(value).toBe('/models/legacy.gguf');
        expect(localStorage.getItem(STORAGE_KEYS.modelPath)).toBe('/models/legacy.gguf');
        expect(localStorage.getItem(LEGACY_STORAGE_KEYS.modelPath)).toBe('/models/legacy.gguf');
    });

    it('prefers an existing ThinClaw key over a legacy value', () => {
        localStorage.setItem(STORAGE_KEYS.maxContext, '65536');
        localStorage.setItem(LEGACY_STORAGE_KEYS.maxContext, '32768');

        expect(migrateLocalStorageKey(localStorage, 'maxContext')).toBe('65536');
        expect(localStorage.getItem(STORAGE_KEYS.maxContext)).toBe('65536');
    });

    it('migrates the onboarding in-progress key and clears both transient flags', () => {
        localStorage.setItem(LEGACY_STORAGE_KEYS.onboardingInProgress, 'true');

        expect(isOnboardingInProgress()).toBe(true);
        expect(localStorage.getItem(STORAGE_KEYS.onboardingInProgress)).toBe('true');

        clearOnboardingProgress();

        expect(localStorage.getItem(STORAGE_KEYS.onboardingInProgress)).toBeNull();
        expect(localStorage.getItem(LEGACY_STORAGE_KEYS.onboardingInProgress)).toBeNull();
    });

    it('writes new onboarding state with ThinClaw and legacy keys for rollback', () => {
        startOnboardingProgress();

        expect(localStorage.getItem(STORAGE_KEYS.onboardingInProgress)).toBe('true');
        expect(localStorage.getItem(LEGACY_STORAGE_KEYS.onboardingInProgress)).toBe('true');
    });
});
