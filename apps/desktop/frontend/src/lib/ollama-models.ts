import type { BridgeError, Result } from "./bindings";
import { unwrapResult } from "./guards";

export interface OllamaModelCommandClient {
    directRuntimeListOllamaModels(): Promise<Result<string[], BridgeError>>;
}

export async function loadInstalledOllamaModels(
    client: OllamaModelCommandClient,
): Promise<string[]> {
    const models = unwrapResult(
        await client.directRuntimeListOllamaModels(),
        "list installed Ollama models",
    );
    return [...new Set(models)].sort((left, right) => left.localeCompare(right));
}

export function chooseInstalledOllamaModel(
    models: readonly string[],
    configuredModel: string | null | undefined,
): string | null {
    if (configuredModel && models.includes(configuredModel)) {
        return configuredModel;
    }
    return models[0] ?? null;
}

export function isInstalledOllamaModel(
    models: readonly string[],
    model: string | null | undefined,
): model is string {
    return Boolean(model && models.includes(model));
}
