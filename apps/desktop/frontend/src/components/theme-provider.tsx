import { createContext, useCallback, useContext, useEffect, useState } from "react"
import { Monitor, Moon, Sun } from "lucide-react"
import { DARK_SYNTAX_THEMES, LIGHT_SYNTAX_THEMES, normalizeSyntaxThemeId } from "../lib/syntax-themes"
import { APP_THEMES } from "../lib/app-themes"

export type Theme = "dark" | "light" | "system"
export type EffectiveTheme = Exclude<Theme, "system">

export interface ThemePreferences {
    version: 1
    mode: Theme
    appThemeId: string
    syntaxTheme: {
        dark: string
        light: string
    }
}

type ThemeProviderProps = {
    children: React.ReactNode
    defaultTheme?: Theme
    storageKey?: string
}

type ThemeProviderState = {
    theme: Theme
    setTheme: (theme: Theme) => void
    darkSyntaxTheme: string
    lightSyntaxTheme: string
    setSyntaxTheme: (type: EffectiveTheme, themeId: string) => void
    appThemeId: string
    setAppThemeId: (themeId: string) => void
}

export const THEME_PREFERENCES_KEY = "thinclaw-ui-theme"
const LEGACY_MODE_KEY = "vite-ui-theme"
const LEGACY_APP_THEME_KEY = "app-theme"
const LEGACY_DARK_SYNTAX_KEY = "syntax-theme-dark"
const LEGACY_LIGHT_SYNTAX_KEY = "syntax-theme-light"

const DEFAULT_PREFERENCES: ThemePreferences = {
    version: 1,
    mode: "system",
    appThemeId: "zinc",
    syntaxTheme: {
        dark: "tokyo-night",
        light: "atom-one-light",
    },
}

const ThemeProviderContext = createContext<ThemeProviderState | undefined>(undefined)

function isRecord(value: unknown): value is Record<string, unknown> {
    return typeof value === "object" && value !== null && !Array.isArray(value)
}

function validThemeMode(value: unknown, fallback: Theme): Theme {
    return value === "dark" || value === "light" || value === "system" ? value : fallback
}

function validAppThemeId(value: unknown): string {
    return typeof value === "string" && APP_THEMES.some((theme) => theme.id === value)
        ? value
        : DEFAULT_PREFERENCES.appThemeId
}

function validSyntaxThemeId(value: unknown, mode: EffectiveTheme): string {
    const fallback = DEFAULT_PREFERENCES.syntaxTheme[mode]
    if (typeof value !== "string") return fallback

    const normalized = normalizeSyntaxThemeId(value)
    const themes = mode === "dark" ? DARK_SYNTAX_THEMES : LIGHT_SYNTAX_THEMES
    return themes.some((theme) => theme.id === normalized) ? normalized : fallback
}

export function normalizeThemePreferences(
    value: unknown,
    defaultTheme: Theme = DEFAULT_PREFERENCES.mode,
): ThemePreferences {
    const input = isRecord(value) ? value : {}
    const syntax = isRecord(input.syntaxTheme) ? input.syntaxTheme : {}

    return {
        version: 1,
        mode: validThemeMode(input.mode, defaultTheme),
        appThemeId: validAppThemeId(input.appThemeId),
        syntaxTheme: {
            dark: validSyntaxThemeId(syntax.dark, "dark"),
            light: validSyntaxThemeId(syntax.light, "light"),
        },
    }
}

function legacyPreferences(defaultTheme: Theme): ThemePreferences {
    return normalizeThemePreferences({
        mode: localStorage.getItem(LEGACY_MODE_KEY) ?? defaultTheme,
        appThemeId: localStorage.getItem(LEGACY_APP_THEME_KEY),
        syntaxTheme: {
            dark: localStorage.getItem(LEGACY_DARK_SYNTAX_KEY),
            light: localStorage.getItem(LEGACY_LIGHT_SYNTAX_KEY),
        },
    }, defaultTheme)
}

function writePreferences(storageKey: string, preferences: ThemePreferences): void {
    localStorage.setItem(storageKey, JSON.stringify(preferences))
}

export function readThemePreferences(
    storageKey = THEME_PREFERENCES_KEY,
    defaultTheme: Theme = DEFAULT_PREFERENCES.mode,
): ThemePreferences {
    const stored = localStorage.getItem(storageKey)
    if (stored) {
        try {
            const parsed: unknown = JSON.parse(stored)
            // A caller using the old mode-only key may still provide a JSON
            // string such as "dark". Treat that as a legacy mode value.
            const preferences = typeof parsed === "string"
                ? normalizeThemePreferences(
                    { ...legacyPreferences(defaultTheme), mode: parsed },
                    defaultTheme,
                )
                : normalizeThemePreferences(parsed, defaultTheme)
            writePreferences(storageKey, preferences)
            return preferences
        } catch {
            // Old localStorage values were unquoted strings (for example
            // `dark`). Migrate those before falling back to the other keys.
            if (stored === "dark" || stored === "light" || stored === "system") {
                const preferences = normalizeThemePreferences({
                    ...legacyPreferences(defaultTheme),
                    mode: stored,
                }, defaultTheme)
                writePreferences(storageKey, preferences)
                return preferences
            }
        }
    }

    const preferences = legacyPreferences(defaultTheme)
    writePreferences(storageKey, preferences)
    return preferences
}

/**
 * Semantic aliases consumed by both Direct Workbench and Agent Cockpit.
 *
 * The `--color-zinc-*` aliases intentionally absorb older Cockpit classes into
 * the selected application palette while those components move to the named
 * surface/content tokens. Status colors (emerald, amber, rose, red) remain
 * independent because they communicate meaning rather than product mode.
 */
export const DESIGN_SYSTEM_TOKEN_ALIASES: Readonly<Record<string, string>> = {
    "--surface-canvas": "hsl(var(--background))",
    "--surface-panel": "hsl(var(--card))",
    "--surface-elevated": "hsl(var(--popover))",
    "--surface-subtle": "hsl(var(--muted))",
    "--content-primary": "hsl(var(--foreground))",
    "--content-muted": "hsl(var(--muted-foreground))",
    "--surface-outline": "hsl(var(--border))",
    "--surface-focus": "hsl(var(--ring))",
    "--color-zinc-950": "hsl(var(--background))",
    "--color-zinc-900": "hsl(var(--card))",
    "--color-zinc-800": "hsl(var(--secondary))",
    "--color-zinc-700": "hsl(var(--border))",
    "--color-zinc-600": "hsl(var(--muted-foreground) / 0.65)",
    "--color-zinc-500": "hsl(var(--muted-foreground))",
    "--color-zinc-400": "hsl(var(--muted-foreground))",
    "--color-zinc-300": "hsl(var(--foreground) / 0.82)",
    "--color-zinc-200": "hsl(var(--foreground) / 0.9)",
    "--color-zinc-100": "hsl(var(--foreground))",
    "--color-zinc-50": "hsl(var(--foreground))",
    "--color-cyan-300": "hsl(var(--primary) / 0.82)",
    "--color-cyan-400": "hsl(var(--primary) / 0.9)",
    "--color-cyan-500": "hsl(var(--primary))",
    "--color-cyan-600": "hsl(var(--primary))",
    "--color-indigo-100": "hsl(var(--primary-foreground))",
    "--color-indigo-300": "hsl(var(--primary) / 0.82)",
    "--color-indigo-400": "hsl(var(--primary) / 0.9)",
    "--color-indigo-500": "hsl(var(--primary))",
    "--color-indigo-600": "hsl(var(--primary))",
}

export function effectiveTheme(mode: Theme): EffectiveTheme {
    if (mode !== "system") return mode
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light"
}

export function applyThemeTokens(
    root: HTMLElement,
    preferences: ThemePreferences,
    colorScheme: EffectiveTheme,
): void {
    const appTheme = APP_THEMES.find((theme) => theme.id === preferences.appThemeId) ?? APP_THEMES[0]
    const appColors = appTheme[colorScheme]
    const syntaxThemes = colorScheme === "dark" ? DARK_SYNTAX_THEMES : LIGHT_SYNTAX_THEMES
    const syntaxThemeId = preferences.syntaxTheme[colorScheme]
    const syntaxTheme = syntaxThemes.find((theme) => theme.id === syntaxThemeId) ?? syntaxThemes[0]

    root.classList.remove("light", "dark")
    root.classList.add(colorScheme)
    root.dataset.colorScheme = colorScheme
    root.dataset.appTheme = appTheme.id
    root.style.colorScheme = colorScheme

    Object.entries(appColors).forEach(([key, value]) => {
        root.style.setProperty(`--${key}`, value)
    })
    Object.entries(syntaxTheme.colors).forEach(([key, value]) => {
        root.style.setProperty(`--hljs-${key}`, value)
    })
    Object.entries(DESIGN_SYSTEM_TOKEN_ALIASES).forEach(([key, value]) => {
        root.style.setProperty(key, value)
    })
}

export function ThemeProvider({
    children,
    defaultTheme = DEFAULT_PREFERENCES.mode,
    storageKey = THEME_PREFERENCES_KEY,
}: ThemeProviderProps) {
    const [preferences, setPreferences] = useState<ThemePreferences>(
        () => readThemePreferences(storageKey, defaultTheme),
    )

    const updatePreferences = useCallback(
        (update: (current: ThemePreferences) => ThemePreferences) => {
            setPreferences((current) => {
                const next = normalizeThemePreferences(update(current), defaultTheme)
                writePreferences(storageKey, next)
                return next
            })
        },
        [defaultTheme, storageKey],
    )

    const applyCurrentTheme = useCallback(() => {
        applyThemeTokens(
            window.document.documentElement,
            preferences,
            effectiveTheme(preferences.mode),
        )
    }, [preferences])

    useEffect(() => {
        applyCurrentTheme()

        const syncFromStorage = () => {
            setPreferences(readThemePreferences(storageKey, defaultTheme))
        }
        const handleStorage = (event: StorageEvent) => {
            if (event.storageArea === localStorage && event.key === storageKey) {
                syncFromStorage()
            }
        }
        const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)")

        window.addEventListener("focus", syncFromStorage)
        window.addEventListener("storage", handleStorage)
        if (preferences.mode === "system") {
            mediaQuery.addEventListener("change", applyCurrentTheme)
        }

        return () => {
            window.removeEventListener("focus", syncFromStorage)
            window.removeEventListener("storage", handleStorage)
            mediaQuery.removeEventListener("change", applyCurrentTheme)
        }
    }, [applyCurrentTheme, defaultTheme, preferences.mode, storageKey])

    const value: ThemeProviderState = {
        theme: preferences.mode,
        setTheme: (theme) => {
            updatePreferences((current) => ({ ...current, mode: theme }))
        },
        darkSyntaxTheme: preferences.syntaxTheme.dark,
        lightSyntaxTheme: preferences.syntaxTheme.light,
        setSyntaxTheme: (type, themeId) => {
            updatePreferences((current) => ({
                ...current,
                syntaxTheme: {
                    ...current.syntaxTheme,
                    [type]: validSyntaxThemeId(themeId, type),
                },
            }))
        },
        appThemeId: preferences.appThemeId,
        setAppThemeId: (themeId) => {
            updatePreferences((current) => ({
                ...current,
                appThemeId: validAppThemeId(themeId),
            }))
        },
    }

    return (
        <ThemeProviderContext.Provider value={value}>
            {children}
        </ThemeProviderContext.Provider>
    )
}

export const useTheme = () => {
    const context = useContext(ThemeProviderContext)
    if (!context) throw new Error("useTheme must be used within a ThemeProvider")
    return context
}

export function ThemeToggle() {
    const { setTheme, theme } = useTheme()

    return (
        <div className="flex gap-1 p-1 bg-muted/20 rounded-lg border border-border/50">
            <button
                onClick={() => setTheme("light")}
                className={`p-2 rounded-md transition-all ${theme === "light" ? "bg-background shadow-xs text-primary" : "text-muted-foreground hover:text-foreground"}`}
                title="Light Mode"
                aria-label="Use light appearance"
                aria-pressed={theme === "light"}
            >
                <Sun className="w-4 h-4" />
            </button>
            <button
                onClick={() => setTheme("dark")}
                className={`p-2 rounded-md transition-all ${theme === "dark" ? "bg-background shadow-xs text-primary" : "text-muted-foreground hover:text-foreground"}`}
                title="Dark Mode"
                aria-label="Use dark appearance"
                aria-pressed={theme === "dark"}
            >
                <Moon className="w-4 h-4" />
            </button>
            <button
                onClick={() => setTheme("system")}
                className={`p-2 rounded-md transition-all ${theme === "system" ? "bg-background shadow-xs text-primary" : "text-muted-foreground hover:text-foreground"}`}
                title="System"
                aria-label="Follow system appearance"
                aria-pressed={theme === "system"}
            >
                <Monitor className="w-4 h-4" />
            </button>
        </div>
    )
}
