import { type ClassValue, clsx } from "clsx"
import { twMerge } from "tailwind-merge"

import { Result } from "./bindings"

export function cn(...inputs: ClassValue[]) {
    return twMerge(clsx(inputs))
}

export function unwrap<T>(result: Result<T, string>): T {
    if (result.status === "error") throw new Error(result.error);
    return result.data;
}
