import type { JsonValue } from '../bindings';

export type ThinClawJson = null | boolean | number | string | ThinClawJson[] | { [key: string]: ThinClawJson };

export function jsonValue(value: unknown): JsonValue {
    return value as JsonValue;
}
