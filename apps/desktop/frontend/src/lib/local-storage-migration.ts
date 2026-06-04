type StorageLike = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

export const STORAGE_KEYS = {
    modelPath: 'thinclaw_model_path',
    embeddingModelPath: 'thinclaw_embedding_model_path',
    visionModelPath: 'thinclaw_vision_model_path',
    sttModelPath: 'thinclaw_stt_model_path',
    imageGenModelPath: 'thinclaw_image_gen_model_path',
    summarizerModelPath: 'thinclaw_summarizer_model_path',
    modelTemplate: 'thinclaw_model_template',
    maxContext: 'thinclaw_max_context',
    firstRunCheck: 'thinclaw_first_run_check_v3',
    onboardingInProgress: 'thinclaw_onboarding_in_progress',
} as const;

export const LEGACY_STORAGE_KEYS: Record<keyof typeof STORAGE_KEYS, string> = {
    modelPath: 'scrappy_model_path',
    embeddingModelPath: 'scrappy_embedding_model_path',
    visionModelPath: 'scrappy_vision_model_path',
    sttModelPath: 'scrappy_stt_model_path',
    imageGenModelPath: 'scrappy_image_gen_model_path',
    summarizerModelPath: 'scrappy_summarizer_model_path',
    modelTemplate: 'scrappy_model_template',
    maxContext: 'scrappy_max_context',
    firstRunCheck: 'scrappy_first_run_check_v3',
    onboardingInProgress: 'scrappy_onboarding_in_progress',
};

export function migrateLocalStorageKey(
    storage: StorageLike,
    keyName: keyof typeof STORAGE_KEYS,
): string | null {
    const currentKey = STORAGE_KEYS[keyName];
    const legacyKey = LEGACY_STORAGE_KEYS[keyName];
    const currentValue = storage.getItem(currentKey);

    if (currentValue !== null) return currentValue;

    const legacyValue = storage.getItem(legacyKey);
    if (legacyValue !== null) {
        storage.setItem(currentKey, legacyValue);
        return legacyValue;
    }

    return null;
}

export function getMigratedLocalStorageItem(keyName: keyof typeof STORAGE_KEYS): string | null {
    return migrateLocalStorageKey(localStorage, keyName);
}

export function setMigratedLocalStorageItem(
    keyName: keyof typeof STORAGE_KEYS,
    value: string,
): void {
    localStorage.setItem(STORAGE_KEYS[keyName], value);
    localStorage.setItem(LEGACY_STORAGE_KEYS[keyName], value);
}

export function startOnboardingProgress(): void {
    setMigratedLocalStorageItem('onboardingInProgress', 'true');
}

export function clearOnboardingProgress(): void {
    localStorage.removeItem(STORAGE_KEYS.onboardingInProgress);
    localStorage.removeItem(LEGACY_STORAGE_KEYS.onboardingInProgress);
}

export function isOnboardingInProgress(): boolean {
    return getMigratedLocalStorageItem('onboardingInProgress') === 'true';
}
