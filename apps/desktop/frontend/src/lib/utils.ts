import { type ClassValue, clsx } from "clsx"
import { twMerge } from "tailwind-merge"

import { Result } from "./bindings"
import { bridgeErrorMessage } from "./command-errors"

export function cn(...inputs: ClassValue[]) {
    return twMerge(clsx(inputs))
}

export function unwrap<T, E>(result: Result<T, E>): T {
    if (result.status === "error") throw new Error(bridgeErrorMessage(result.error));
    return result.data;
}
