import { createElement } from "react"
import { act, render, screen } from "@testing-library/react"
import { beforeEach, describe, expect, it, vi } from "vitest"
import {
    applyThemeTokens,
    normalizeThemePreferences,
    readThemePreferences,
    ThemeProvider,
    THEME_PREFERENCES_KEY,
    useTheme,
} from "../../components/theme-provider"

function ThemeProbe() {
    const { theme, appThemeId } = useTheme()
    return createElement("output", null, `${theme}:${appThemeId}`)
}

describe("unified theme preferences", () => {
    beforeEach(() => {
        localStorage.clear()
        document.documentElement.className = ""
        document.documentElement.removeAttribute("style")
        delete document.documentElement.dataset.appTheme
        delete document.documentElement.dataset.colorScheme
        delete document.documentElement.dataset.density
        vi.stubGlobal("matchMedia", vi.fn().mockReturnValue({
            matches: false,
            addEventListener: vi.fn(),
            removeEventListener: vi.fn(),
        }))
    })

    it("migrates all legacy keys into one versioned record", () => {
        localStorage.setItem("vite-ui-theme", "dark")
        localStorage.setItem("app-theme", "emerald")
        localStorage.setItem("syntax-theme-dark", "scrappy-dark")
        localStorage.setItem("syntax-theme-light", "scrappy-light")

        const preferences = readThemePreferences()

        expect(preferences).toEqual({
            version: 2,
            mode: "dark",
            appThemeId: "emerald",
            density: "comfortable",
            syntaxTheme: {
                dark: "thinclaw-dark",
                light: "thinclaw-light",
            },
        })
        expect(JSON.parse(localStorage.getItem(THEME_PREFERENCES_KEY) ?? "null"))
            .toEqual(preferences)
    })

    it("repairs invalid modes and palette identifiers", () => {
        expect(normalizeThemePreferences({
            mode: "sepia",
            appThemeId: "missing",
            syntaxTheme: { dark: "missing", light: "missing" },
        }, "light")).toEqual({
            version: 2,
            mode: "light",
            appThemeId: "zinc",
            density: "comfortable",
            syntaxTheme: {
                dark: "tokyo-night",
                light: "atom-one-light",
            },
        })
    })

    it("applies one semantic token set to every product surface", () => {
        const preferences = normalizeThemePreferences({
            mode: "light",
            appThemeId: "indigo",
            syntaxTheme: { dark: "tokyo-night", light: "atom-one-light" },
        })

        applyThemeTokens(document.documentElement, preferences, "light")

        expect(document.documentElement).toHaveClass("light")
        expect(document.documentElement.dataset.appTheme).toBe("indigo")
        expect(document.documentElement.dataset.colorScheme).toBe("light")
        expect(document.documentElement.dataset.density).toBe("comfortable")
        expect(document.documentElement.style.getPropertyValue("--surface-canvas"))
            .toBe("hsl(var(--background))")
        expect(document.documentElement.style.getPropertyValue("--color-zinc-950"))
            .toBe("hsl(var(--background))")
        expect(document.documentElement.style.getPropertyValue("--color-cyan-500"))
            .toBe("hsl(var(--primary))")
        expect(document.documentElement.style.getPropertyValue("--background"))
            .toBe("226 100% 97%")
    })

    it("migrates v1 preferences and applies compact density", () => {
        const preferences = normalizeThemePreferences({
            version: 1,
            mode: "dark",
            appThemeId: "zinc",
            density: "compact",
            syntaxTheme: { dark: "tokyo-night", light: "atom-one-light" },
        })

        expect(preferences.version).toBe(2)
        expect(preferences.density).toBe("compact")
        applyThemeTokens(document.documentElement, preferences, "dark")
        expect(document.documentElement.dataset.density).toBe("compact")
    })

    it("synchronizes preference changes emitted by another window", async () => {
        render(createElement(ThemeProvider, null, createElement(ThemeProbe)))
        expect(screen.getByText("system:zinc")).toBeInTheDocument()

        const preferences = normalizeThemePreferences({
            mode: "dark",
            appThemeId: "rose",
            syntaxTheme: { dark: "tokyo-night", light: "atom-one-light" },
        })
        const serialized = JSON.stringify(preferences)
        localStorage.setItem(THEME_PREFERENCES_KEY, serialized)

        await act(async () => {
            window.dispatchEvent(new StorageEvent("storage", {
                key: THEME_PREFERENCES_KEY,
                newValue: serialized,
                storageArea: localStorage,
            }))
        })

        expect(screen.getByText("dark:rose")).toBeInTheDocument()
        expect(document.documentElement.dataset.appTheme).toBe("rose")
        expect(document.documentElement.dataset.colorScheme).toBe("dark")
    })
})
