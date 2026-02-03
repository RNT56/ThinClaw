import { createContext, useContext, useEffect, useState, useCallback } from "react"
import { Monitor, Moon, Sun } from "lucide-react"
import { DARK_SYNTAX_THEMES, LIGHT_SYNTAX_THEMES } from "../lib/syntax-themes"

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
}

const initialState: ThemeProviderState = {
    theme: "system",
    setTheme: () => null,
    darkSyntaxTheme: "tokyo-night",
    lightSyntaxTheme: "atom-one-light",
    setSyntaxTheme: () => null,
}

const ThemeProviderContext = createContext<ThemeProviderState>(initialState)

export function ThemeProvider({
    children,
    defaultTheme = "system",
    storageKey = "vite-ui-theme",
}: ThemeProviderProps) {
    const [theme, setTheme] = useState<Theme>(
        () => (localStorage.getItem(storageKey) as Theme) || defaultTheme
    )

    const [darkSyntaxTheme, setDarkSyntaxTheme] = useState(
        () => localStorage.getItem("syntax-theme-dark") || "tokyo-night"
    )
    const [lightSyntaxTheme, setLightSyntaxTheme] = useState(
        () => localStorage.getItem("syntax-theme-light") || "atom-one-light"
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
        }

        updateTheme()

        if (theme === "system") {
            const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)")
            mediaQuery.addEventListener("change", updateTheme)
            return () => mediaQuery.removeEventListener("change", updateTheme)
        }
    }, [theme, applySyntaxColors])

    const value = {
        theme,
        setTheme: (theme: Theme) => {
            localStorage.setItem(storageKey, theme)
            setTheme(theme)
        },
        darkSyntaxTheme,
        lightSyntaxTheme,
        setSyntaxTheme: (type: 'dark' | 'light', themeId: string) => {
            if (type === 'dark') {
                setDarkSyntaxTheme(themeId)
                localStorage.setItem("syntax-theme-dark", themeId)
            } else {
                setLightSyntaxTheme(themeId)
                localStorage.setItem("syntax-theme-light", themeId)
            }
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
