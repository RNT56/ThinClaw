/**
 * Stable ThinClaw compatibility API.
 *
 * Domain implementations live in lib/api; this barrel preserves existing
 * component imports while allowing new code to depend on focused modules.
 */

export * from './api/core';
export * from './api/gateway';
export * from './api/integrations';
export * from './api/operations';
export * from './api/repo-projects';
export type { ThinClawJson } from './api/shared';
