// Global test setup for vitest + @testing-library/react
import '@testing-library/jest-dom';

// Silence Tauri IPC calls in unit-test environments — the real invoke is
// unavailable in jsdom. Tests that exercise IPC should mock it explicitly.
//
// commandClient checks globalThis.__TAURI_INTERNALS__ before calling the
// generated bindings. Stub it so the guard passes and mocked invoke is reached.
(globalThis as any).__TAURI_INTERNALS__ = { invoke: () => Promise.resolve() };

vi.mock('@tauri-apps/api/core', () => ({
    invoke: vi.fn().mockResolvedValue(undefined),
}));

// Suppress noisy console output during test runs unless a test explicitly
// expects it.
beforeEach(() => {
    vi.spyOn(console, 'error').mockImplementation(() => { });
    vi.spyOn(console, 'warn').mockImplementation(() => { });
});

afterEach(() => {
    vi.restoreAllMocks();
});
