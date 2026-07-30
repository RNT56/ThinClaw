import type { BridgeError, Result } from "./bindings";
import { unwrapResult } from "./guards";

export type StartSummarizerRuntime = (
    modelPath: string,
    contextSize: number,
) => Promise<Result<null, BridgeError>>;

export interface ReconcileSummarizerOptions {
    modelPath: string;
    contextSize: number;
    start: StartSummarizerRuntime;
    persistSelection: (modelPath: string) => void;
}

/**
 * Reconcile the backend process before publishing the selected path.
 *
 * The backend command replaces any existing summarizer and waits for the new
 * server's health check. A failed Result or rejected transport therefore
 * leaves the persisted/UI selection untouched.
 */
export async function reconcileSummarizerRuntime({
    modelPath,
    contextSize,
    start,
    persistSelection,
}: ReconcileSummarizerOptions): Promise<void> {
    if (!modelPath) {
        throw new Error("No summarizer model is selected");
    }
    if (!Number.isInteger(contextSize) || contextSize <= 0) {
        throw new Error("Summarizer context size must be a positive integer");
    }

    unwrapResult(
        await start(modelPath, contextSize),
        "start summarizer",
    );
    persistSelection(modelPath);
}
