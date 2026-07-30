export interface InitialModelState<TModel, TSpecs> {
    inventory: TModel[] | null;
    specs: TSpecs | null;
    specsError: unknown | null;
}

/**
 * Start inventory loading before optional hardware telemetry.
 *
 * The inventory promise is deliberately created first so a null, rejected, or
 * slow specs request cannot prevent the model library from being populated.
 */
export async function loadInitialModelState<TModel, TSpecs>({
    refreshInventory,
    getSystemSpecs,
}: {
    refreshInventory: () => Promise<TModel[] | null>;
    getSystemSpecs: () => Promise<TSpecs | null>;
}): Promise<InitialModelState<TModel, TSpecs>> {
    const inventoryPromise = refreshInventory();
    let specs: TSpecs | null = null;
    let specsError: unknown | null = null;
    try {
        specs = await getSystemSpecs();
    } catch (error) {
        specsError = error;
    }
    return {
        inventory: await inventoryPromise,
        specs,
        specsError,
    };
}
