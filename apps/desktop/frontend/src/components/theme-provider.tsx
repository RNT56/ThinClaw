import { createContext, useContext, useEffect, useState, useCallback } from "react"
import { Monitor, Moon, Sun } from "lucide-react"
import { DARK_SYNTAX_THEMES, LIGHT_SYNTAX_THEMES, normalizeSyntaxThemeId } from "../lib/syntax-themes"
import { APP_THEMES } from "../lib/app-themes"

type Theme = "dark" | "light" | "system"

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
    setSyntaxTheme: (type: 'dark' | 'light', themeId: string) => void
    appThemeId: string
    setAppThemeId: (themeId: string) => void
}

const initialState: ThemeProviderState = {
    theme: "system",
    setTheme: () => null,
    darkSyntaxTheme: "tokyo-night",
    lightSyntaxTheme: "atom-one-light",
    setSyntaxTheme: () => null,
    appThemeId: "zinc",
    setAppThemeId: () => null,
}

const ThemeProviderContext = createContext<ThemeProviderState>(initialState)

function storedSyntaxThemeId(key: string, fallback: string): string {
    const stored = localStorage.getItem(key)
    if (!stored) return fallback
    const normalized = normalizeSyntaxThemeId(stored)
    if (normalized !== stored) {
        localStorage.setItem(key, normalized)
    }
    return normalized
}

export function ThemeProvider({
    children,
    defaultTheme = "system",
    storageKey = "vite-ui-theme",
}: ThemeProviderProps) {
    const [theme, setTheme] = useState<Theme>(
        () => (localStorage.getItem(storageKey) as Theme) || defaultTheme
    )

    const [darkSyntaxTheme, setDarkSyntaxTheme] = useState(
        () => storedSyntaxThemeId("syntax-theme-dark", "tokyo-night")
    )
    const [lightSyntaxTheme, setLightSyntaxTheme] = useState(
        () => storedSyntaxThemeId("syntax-theme-light", "atom-one-light")
    )

    const [appThemeId, setAppThemeId] = useState(
        () => localStorage.getItem("app-theme") || "zinc"
    )

    const applySyntaxColors = useCallback((uiTheme: 'dark' | 'light') => {
        const root = window.document.documentElement
        const themeList = uiTheme === 'dark' ? DARK_SYNTAX_THEMES : LIGHT_SYNTAX_THEMES
        const selectedId = uiTheme === 'dark' ? darkSyntaxTheme : lightSyntaxTheme
        const themeData = themeList.find(t => t.id === selectedId) || themeList[0]

        Object.entries(themeData.colors).forEach(([key, value]) => {
            root.style.setProperty(`--hljs-${key}`, value)
        })
    }, [darkSyntaxTheme, lightSyntaxTheme])

    const applyAppTheme = useCallback((uiTheme: 'dark' | 'light') => {
        const root = window.document.documentElement
        const themeData = APP_THEMES.find(t => t.id === appThemeId) || APP_THEMES[0]
        const colors = uiTheme === 'dark' ? themeData.dark : themeData.light

        Object.entries(colors).forEach(([key, value]) => {
            root.style.setProperty(`--${key}`, value)
        })
    }, [appThemeId])

    useEffect(() => {
        const root = window.document.documentElement

        const updateTheme = () => {
            root.classList.remove("light", "dark")

            let effectiveTheme: 'dark' | 'light' = 'dark'
            if (theme === "system") {
                const isDark = window.matchMedia("(prefers-color-scheme: dark)").matches
                effectiveTheme = isDark ? "dark" : "light"
            } else {
                effectiveTheme = theme as 'dark' | 'light'
            }

            root.classList.add(effectiveTheme)
            applySyntaxColors(effectiveTheme)
            applyAppTheme(effectiveTheme)
        }

        updateTheme()

        // Re-apply theme when window gains focus (for spotlight window sync)
        const handleFocus = () => {
            // Re-read from localStorage in case main window changed theme
            const storedTheme = localStorage.getItem("vite-ui-theme") as Theme
            const storedAppTheme = localStorage.getItem("app-theme")
            const storedDarkSyntax = localStorage.getItem("syntax-theme-dark")
            const storedLightSyntax = localStorage.getItem("syntax-theme-light")

            if (storedTheme && storedTheme !== theme) {
                setTheme(storedTheme)
            }
            if (storedAppTheme && storedAppTheme !== appThemeId) {
                setAppThemeId(storedAppTheme)
            }
            if (storedDarkSyntax) {
                const normalized = normalizeSyntaxThemeId(storedDarkSyntax)
                if (normalized !== storedDarkSyntax) {
                    localStorage.setItem("syntax-theme-dark", normalized)
                }
                if (normalized !== darkSyntaxTheme) {
                    setDarkSyntaxTheme(normalized)
                }
            }
            if (storedLightSyntax) {
                const normalized = normalizeSyntaxThemeId(storedLightSyntax)
                if (normalized !== storedLightSyntax) {
                    localStorage.setItem("syntax-theme-light", normalized)
                }
                if (normalized !== lightSyntaxTheme) {
                    setLightSyntaxTheme(normalized)
                }
            }
            // Still re-apply regardless to ensure CSS vars are set
            updateTheme()
        }

        window.addEventListener('focus', handleFocus)

        if (theme === "system") {
            const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)")
            mediaQuery.addEventListener("change", updateTheme)
            return () => {
                mediaQuery.removeEventListener("change", updateTheme)
                window.removeEventListener('focus', handleFocus)
            }
        }

        return () => {
            window.removeEventListener('focus', handleFocus)
        }
    }, [theme, applySyntaxColors, applyAppTheme])

    const value = {
        theme,
        setTheme: (theme: Theme) => {
            localStorage.setItem(storageKey, theme)
            setTheme(theme)
        },
        darkSyntaxTheme,
        lightSyntaxTheme,
        setSyntaxTheme: (type: 'dark' | 'light', themeId: string) => {
            const normalizedThemeId = normalizeSyntaxThemeId(themeId)
            if (type === 'dark') {
                setDarkSyntaxTheme(normalizedThemeId)
                localStorage.setItem("syntax-theme-dark", normalizedThemeId)
            } else {
                setLightSyntaxTheme(normalizedThemeId)
                localStorage.setItem("syntax-theme-light", normalizedThemeId)
            }
        },
        appThemeId,
        setAppThemeId: (themeId: string) => {
            setAppThemeId(themeId)
            localStorage.setItem("app-theme", themeId)
        }
    }

    return (
        <ThemeProviderContext.Provider value={value}>
            {children}
        </ThemeProviderContext.Provider>
    )
}

export const useTheme = () => {
    const context = useContext(ThemeProviderContext)

    if (context === undefined)
        throw new Error("useTheme must be used within a ThemeProvider")

    return context
}

export function ThemeToggle() {
    const { setTheme, theme } = useTheme()

    return (
        <div className="flex gap-1 p-1 bg-muted/20 rounded-lg border border-border/50">
            <button
                onClick={() => setTheme("light")}
                className={`p-2 rounded-md transition-all ${theme === 'light' ? 'bg-background shadow-sm text-primary' : 'text-muted-foreground hover:text-foreground'}`}
                title="Light Mode"
            >
                <Sun className="w-4 h-4" />
            </button>
            <button
                onClick={() => setTheme("dark")}
                className={`p-2 rounded-md transition-all ${theme === 'dark' ? 'bg-background shadow-sm text-primary' : 'text-muted-foreground hover:text-foreground'}`}
                title="Dark Mode"
            >
                <Moon className="w-4 h-4" />
            </button>
            <button
                onClick={() => setTheme("system")}
                className={`p-2 rounded-md transition-all ${theme === 'system' ? 'bg-background shadow-sm text-primary' : 'text-muted-foreground hover:text-foreground'}`}
                title="System"
            >
                <Monitor className="w-4 h-4" />
            </button>
        </div>
    )
}
