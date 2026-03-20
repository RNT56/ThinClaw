import { useState, useEffect } from 'react';
import {
    Settings,
    AlertTriangle,
    Command,
    Sparkles
} from 'lucide-react';
import { commands, type UserConfig } from '../../lib/bindings';
import { cn } from '../../lib/utils';
import { ThemeToggle, useTheme } from '../theme-provider';
import { DARK_SYNTAX_THEMES, LIGHT_SYNTAX_THEMES, SyntaxTheme } from '../../lib/syntax-themes';
import { APP_THEMES, AppTheme } from '../../lib/app-themes';

function SyntaxThemeOption({ theme, isActive, onClick }: { theme: SyntaxTheme, isActive: boolean, onClick: () => void }) {
    return (
        <button
            onClick={onClick}
            className={cn(
                "group relative flex flex-col items-start p-4 rounded-xl border transition-all duration-200 text-left w-full",
                isActive
                    ? "bg-primary/5 border-primary shadow-[0_0_20px_rgba(var(--primary),0.1)] ring-1 ring-primary/20"
                    : "bg-card/50 hover:bg-muted/50 border-border/50 hover:border-border shadow-sm"
            )}
        >
            <div className="flex items-center justify-between w-full mb-3">
                <span className={cn(
                    "text-[10px] font-bold transition-colors uppercase tracking-[0.15em]",
                    isActive ? "text-primary" : "text-muted-foreground group-hover:text-foreground"
                )}>
                    {theme.label}
                </span>
                {isActive && (
                    <div className="w-1.5 h-1.5 rounded-full bg-primary animate-pulse shadow-[0_0_8px_rgba(var(--primary),0.5)]" />
                )}
            </div>

            <div className="flex gap-2 p-1.5 rounded-lg bg-black/5 dark:bg-white/5 w-full border border-border/10 justify-center">
                <div className="w-3 h-3 rounded-full border border-black/10 dark:border-white/10" style={{ backgroundColor: `hsl(${theme.colors.keyword})` }} title="Keyword" />
                <div className="w-3 h-3 rounded-full border border-black/10 dark:border-white/10" style={{ backgroundColor: `hsl(${theme.colors.string})` }} title="String" />
                <div className="w-3 h-3 rounded-full border border-black/10 dark:border-white/10" style={{ backgroundColor: `hsl(${theme.colors.function})` }} title="Function" />
                <div className="w-3 h-3 rounded-full border border-black/10 dark:border-white/10" style={{ backgroundColor: `hsl(${theme.colors.number})` }} title="Number" />
            </div>
        </button>
    );
}

function AppThemeOption({ theme, isActive, onClick, currentMode }: { theme: AppTheme, isActive: boolean, onClick: () => void, currentMode: 'light' | 'dark' }) {
    const colors = currentMode === 'dark' ? theme.dark : theme.light;

    return (
        <button
            onClick={onClick}
            className={cn(
                "group relative flex flex-col items-start p-4 rounded-xl border transition-all duration-200 text-left w-full",
                isActive
                    ? "bg-primary/5 border-primary shadow-[0_0_20px_rgba(var(--primary),0.1)] ring-1 ring-primary/20"
                    : "bg-card/50 hover:bg-muted/50 border-border/50 hover:border-border shadow-sm"
            )}
        >
            <div className="flex items-center justify-between w-full mb-3">
                <span className={cn(
                    "text-[10px] font-bold transition-colors uppercase tracking-[0.15em]",
                    isActive ? "text-primary" : "text-muted-foreground group-hover:text-foreground"
                )}>
                    {theme.label}
                </span>
                {isActive && (
                    <div className="w-1.5 h-1.5 rounded-full bg-primary animate-pulse shadow-[0_0_8px_rgba(var(--primary),0.5)]" />
                )}
            </div>

            <div className="flex gap-2 p-1.5 rounded-lg w-full border border-border/10 justify-center" style={{ backgroundColor: `hsl(${colors.background})` }}>
                <div className="w-4 h-4 rounded-full border border-black/10 dark:border-white/10" style={{ backgroundColor: `hsl(${colors.primary})` }} title="Primary" />
                <div className="w-4 h-4 rounded-full border border-black/10 dark:border-white/10" style={{ backgroundColor: `hsl(${colors.accent})` }} title="Accent" />
                <div className="w-4 h-4 rounded-full border border-black/10 dark:border-white/10" style={{ backgroundColor: `hsl(${colors.secondary})` }} title="Secondary" />
            </div>
        </button>
    );
}

export function AppearanceSettings() {
    const {
        theme,
        darkSyntaxTheme,
        lightSyntaxTheme,
        setSyntaxTheme,
        appThemeId,
        setAppThemeId
    } = useTheme();

    const [config, setConfig] = useState<UserConfig | null>(null);

    useEffect(() => {
        commands.getUserConfig().then(setConfig);
    }, []);

    const updateShortcut = async (val: string) => {
        if (!config) return;
        const newConfig = { ...config, spotlight_shortcut: val };
        setConfig(newConfig);
        await commands.updateUserConfig(newConfig);
    };

    const effectiveMode = theme === 'system'
        ? (window.matchMedia("(prefers-color-scheme: dark)").matches ? 'dark' : 'light')
        : theme as 'light' | 'dark';

    return (
        <div className="space-y-10">
            {/* UI Theme Group */}
            <div className="p-8 border rounded-2xl bg-gradient-to-br from-card to-background shadow-xl border-border/30 flex items-center justify-between">
                <div className="space-y-1">
                    <h4 className="font-bold text-xl tracking-tight">Workspace Aesthetic</h4>
                    <p className="text-sm text-muted-foreground max-w-sm leading-relaxed">
                        Customize your environment interface. Mode switches instantly update your syntax palette.
                    </p>
                </div>
                <div className="scale-110">
                    <ThemeToggle />
                </div>
            </div>

            {/* App Style Templates */}
            <div className="space-y-6">
                <div className="pt-6 border-t border-border/10 space-y-1">
                    <h3 className="text-xl font-bold tracking-tight">App Style Templates</h3>
                    <p className="text-sm text-muted-foreground">Adjust the entire application styling with these curated templates.</p>
                </div>

                <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-5 gap-3">
                    {APP_THEMES.map(t => (
                        <AppThemeOption
                            key={t.id}
                            theme={t}
                            isActive={appThemeId === t.id}
                            onClick={() => setAppThemeId(t.id)}
                            currentMode={effectiveMode}
                        />
                    ))}
                </div>
            </div>

            <div className="pt-6 border-t border-border/10 space-y-1">
                <h3 className="text-xl font-bold tracking-tight">Syntax Highlight Palettes</h3>
                <p className="text-sm text-muted-foreground">Choose how code blocks and transcripts are rendered in your workspace.</p>
            </div>

            {/* Dark Mode Group */}
            <div className="space-y-4">
                <div className="flex items-center gap-3 px-1">
                    <div className="w-1 h-6 rounded-full bg-indigo-500 shadow-[0_0_10px_rgba(99,102,241,0.5)]" />
                    <h4 className="font-bold text-lg tracking-tight">Dark Mode Palette</h4>
                </div>
                <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-5 gap-3">
                    {DARK_SYNTAX_THEMES.map(t => (
                        <SyntaxThemeOption
                            key={t.id}
                            theme={t}
                            isActive={darkSyntaxTheme === t.id}
                            onClick={() => setSyntaxTheme('dark', t.id)}
                        />
                    ))}
                </div>
            </div>

            {/* Light Mode Group */}
            <div className="space-y-4">
                <div className="flex items-center gap-3 px-1">
                    <div className="w-1 h-6 rounded-full bg-orange-400 shadow-[0_0_10px_rgba(251,146,60,0.5)]" />
                    <h4 className="font-bold text-lg tracking-tight">Light Mode Palette</h4>
                </div>
                <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-5 gap-3">
                    {LIGHT_SYNTAX_THEMES.map(t => (
                        <SyntaxThemeOption
                            key={t.id}
                            theme={t}
                            isActive={lightSyntaxTheme === t.id}
                            onClick={() => setSyntaxTheme('light', t.id)}
                        />
                    ))}
                </div>
            </div>

            {/* Global Hotkeys */}
            <div className="pt-6 border-t border-border/10 space-y-4">
                <div className="space-y-1">
                    <h3 className="text-xl font-bold tracking-tight">Global Hotkeys</h3>
                    <p className="text-sm text-muted-foreground">Configure shortcuts to trigger Scrappy from anywhere on your system.</p>
                </div>

                <div className="p-6 border border-border/50 rounded-xl bg-card/50 flex items-center justify-between shadow-sm border-border/50">
                    <div className="space-y-1">
                        <label className="text-sm font-semibold flex items-center gap-2 text-foreground">
                            <Sparkles className="w-4 h-4 text-primary" />
                            Spotlight Chat Shortcut
                        </label>
                        <p className="text-xs text-muted-foreground">
                            Press this to instantly open the liquid glass chat bar.
                        </p>
                    </div>
                    <div className="relative">
                        <input
                            value={config?.spotlight_shortcut ?? "Command+Shift+K"}
                            onChange={(e) => updateShortcut(e.currentTarget.value)}
                            placeholder="e.g. Command+Shift+K"
                            className="bg-background border border-border/50 rounded-lg px-3 py-2 text-sm w-48 font-mono focus:ring-2 focus:ring-primary outline-none transition-all text-foreground"
                        />
                        <span className="absolute right-3 top-2.5 opacity-30 pointer-events-none">
                            <Command className="w-4 h-4 text-foreground" />
                        </span>
                    </div>
                </div>
                <p className="text-[10px] text-muted-foreground italic px-2 flex items-center gap-2">
                    <AlertTriangle className="w-3 h-3 text-amber-500" /> Note: Shortcut changes require application restart to register properly with the OS.
                </p>
            </div>

            {/* Hint Box */}
            <div className="bg-primary/5 border border-primary/10 rounded-2xl p-6 relative overflow-hidden group">
                <div className="absolute top-0 right-0 w-32 h-32 bg-primary/5 rounded-full blur-3xl -mr-16 -mt-16" />
                <div className="flex gap-4 relative z-10">
                    <div className="p-2.5 bg-primary/10 rounded-xl h-fit border border-primary/20">
                        <Settings className="w-5 h-5 text-primary" />
                    </div>
                    <div className="space-y-1">
                        <span className="font-bold text-[10px] text-primary uppercase tracking-[0.2em] block mb-1">Runtime Adaptive</span>
                        <p className="text-sm text-muted-foreground leading-relaxed">
                            Selections are applied globally and instantly. We persist your preferences to ensure a consistent experience across sessions.
                        </p>
                    </div>
                </div>
            </div>
        </div>
    );
}
