export interface ThemeColors {
    background: string;
    foreground: string;
    card: string;
    'card-foreground': string;
    popover: string;
    'popover-foreground': string;
    primary: string;
    'primary-foreground': string;
    secondary: string;
    'secondary-foreground': string;
    muted: string;
    'muted-foreground': string;
    accent: string;
    'accent-foreground': string;
    destructive: string;
    'destructive-foreground': string;
    border: string;
    input: string;
    ring: string;
    // Theme-derived accent palette — 5 harmonious tones from the theme's hue family
    'chart-1': string;
    'chart-2': string;
    'chart-3': string;
    'chart-4': string;
    'chart-5': string;
}

export interface AppTheme {
    id: string;
    label: string;
    light: ThemeColors;
    dark: ThemeColors;
}

export const APP_THEMES: AppTheme[] = [
    {
        id: "zinc",
        label: "Zinc (Default)",
        light: {
            background: "0 0% 100%",
            foreground: "240 10% 3.9%",
            card: "0 0% 100%",
            'card-foreground': "240 10% 3.9%",
            popover: "0 0% 100%",
            'popover-foreground': "240 10% 3.9%",
            primary: "240 5.9% 10%",
            'primary-foreground': "0 0% 98%",
            secondary: "240 4.8% 95.9%",
            'secondary-foreground': "240 5.9% 10%",
            muted: "240 4.8% 95.9%",
            'muted-foreground': "240 3.8% 46.1%",
            accent: "240 4.8% 95.9%",
            'accent-foreground': "240 5.9% 10%",
            destructive: "0 84.2% 60.2%",
            'destructive-foreground': "0 0% 98%",
            border: "240 5.9% 90%",
            input: "240 5.9% 90%",
            ring: "240 5.9% 10%",
            // Zinc light: neutral accent palette
            'chart-1': "240 10% 35%",
            'chart-2': "240 8% 50%",
            'chart-3': "200 15% 45%",
            'chart-4': "260 12% 42%",
            'chart-5': "220 10% 55%"
        },
        dark: {
            background: "240 10% 3.9%",
            foreground: "0 0% 98%",
            card: "240 10% 3.9%",
            'card-foreground': "0 0% 98%",
            popover: "240 10% 3.9%",
            'popover-foreground': "0 0% 98%",
            primary: "0 0% 98%",
            'primary-foreground': "240 5.9% 10%",
            secondary: "240 3.7% 15.9%",
            'secondary-foreground': "0 0% 98%",
            muted: "240 3.7% 15.9%",
            'muted-foreground': "240 5% 64.9%",
            accent: "240 3.7% 15.9%",
            'accent-foreground': "0 0% 98%",
            destructive: "0 62.8% 30.6%",
            'destructive-foreground': "0 0% 98%",
            border: "240 3.7% 15.9%",
            input: "240 3.7% 15.9%",
            ring: "240 4.9% 83.9%",
            // Zinc dark: neutral accent palette (lighter for dark bg)
            'chart-1': "240 10% 70%",
            'chart-2': "240 8% 60%",
            'chart-3': "200 15% 65%",
            'chart-4': "260 12% 62%",
            'chart-5': "220 10% 55%"
        }
    },
    {
        id: "indigo",
        label: "Indigo Breeze",
        light: {
            background: "226 100% 97%",
            foreground: "226 40% 10%",
            card: "226 100% 98%",
            'card-foreground': "226 40% 10%",
            popover: "226 100% 99%",
            'popover-foreground': "226 40% 10%",
            primary: "226 70% 50%",
            'primary-foreground': "0 0% 100%",
            secondary: "226 30% 90%",
            'secondary-foreground': "226 70% 40%",
            muted: "226 30% 92%",
            'muted-foreground': "226 20% 50%",
            accent: "226 50% 94%",
            'accent-foreground': "226 70% 40%",
            destructive: "0 84.2% 60.2%",
            'destructive-foreground': "0 0% 100%",
            border: "226 30% 88%",
            input: "226 30% 88%",
            ring: "226 70% 50%",
            'chart-1': "226 65% 45%",
            'chart-2': "210 55% 50%",
            'chart-3': "240 50% 48%",
            'chart-4': "216 60% 42%",
            'chart-5': "250 45% 52%"
        },
        dark: {
            background: "226 40% 4%",
            foreground: "226 20% 98%",
            card: "226 40% 6%",
            'card-foreground': "226 20% 98%",
            popover: "226 40% 5%",
            'popover-foreground': "226 20% 98%",
            primary: "226 70% 60%",
            'primary-foreground': "226 40% 4%",
            secondary: "226 30% 12%",
            'secondary-foreground': "226 70% 90%",
            muted: "226 30% 14%",
            'muted-foreground': "226 20% 60%",
            accent: "226 40% 16%",
            'accent-foreground': "226 20% 98%",
            destructive: "0 62.8% 30.6%",
            'destructive-foreground': "0 0% 98%",
            border: "226 30% 18%",
            input: "226 30% 18%",
            ring: "226 70% 60%",
            'chart-1': "226 65% 65%",
            'chart-2': "210 55% 60%",
            'chart-3': "240 50% 62%",
            'chart-4': "216 60% 55%",
            'chart-5': "250 45% 58%"
        }
    },
    {
        id: "emerald",
        label: "Emerald Forest",
        light: {
            background: "150 100% 97%",
            foreground: "160 40% 10%",
            card: "150 100% 98%",
            'card-foreground': "160 40% 10%",
            popover: "150 100% 99%",
            'popover-foreground': "160 40% 10%",
            primary: "160 84% 39%",
            'primary-foreground': "0 0% 100%",
            secondary: "150 30% 90%",
            'secondary-foreground': "160 84% 20%",
            muted: "150 30% 92%",
            'muted-foreground': "160 20% 45%",
            accent: "150 50% 94%",
            'accent-foreground': "160 84% 25%",
            destructive: "0 84.2% 60.2%",
            'destructive-foreground': "0 0% 100%",
            border: "150 30% 88%",
            input: "150 30% 88%",
            ring: "160 84% 39%",
            'chart-1': "160 70% 35%",
            'chart-2': "145 60% 40%",
            'chart-3': "175 55% 38%",
            'chart-4': "155 65% 32%",
            'chart-5': "140 50% 42%"
        },
        dark: {
            background: "160 40% 4%",
            foreground: "160 20% 98%",
            card: "160 40% 6%",
            'card-foreground': "160 20% 98%",
            popover: "160 40% 5%",
            'popover-foreground': "160 20% 98%",
            primary: "160 84% 45%",
            'primary-foreground': "160 40% 4%",
            secondary: "160 30% 12%",
            'secondary-foreground': "160 84% 90%",
            muted: "160 30% 14%",
            'muted-foreground': "160 20% 60%",
            accent: "160 40% 16%",
            'accent-foreground': "160 20% 98%",
            destructive: "0 62.8% 30.6%",
            'destructive-foreground': "0 0% 98%",
            border: "160 30% 18%",
            input: "160 30% 18%",
            ring: "160 84% 45%",
            'chart-1': "160 70% 55%",
            'chart-2': "145 60% 50%",
            'chart-3': "175 55% 52%",
            'chart-4': "155 65% 48%",
            'chart-5': "140 50% 55%"
        }
    },
    {
        id: "rose",
        label: "Rose Quartz",
        light: {
            background: "340 100% 98%",
            foreground: "340 40% 10%",
            card: "340 100% 99%",
            'card-foreground': "340 40% 10%",
            popover: "340 100% 100%",
            'popover-foreground': "340 40% 10%",
            primary: "330 81% 60%",
            'primary-foreground': "0 0% 100%",
            secondary: "340 30% 94%",
            'secondary-foreground': "330 81% 30%",
            muted: "340 30% 95%",
            'muted-foreground': "340 20% 50%",
            accent: "340 50% 96%",
            'accent-foreground': "330 81% 40%",
            destructive: "0 84.2% 60.2%",
            'destructive-foreground': "0 0% 100%",
            border: "340 30% 92%",
            input: "340 30% 92%",
            ring: "330 81% 60%",
            'chart-1': "330 70% 55%",
            'chart-2': "315 60% 50%",
            'chart-3': "345 55% 52%",
            'chart-4': "320 65% 45%",
            'chart-5': "350 50% 55%"
        },
        dark: {
            background: "340 40% 4%",
            foreground: "340 20% 98%",
            card: "340 40% 6%",
            'card-foreground': "340 20% 98%",
            popover: "340 40% 5%",
            'popover-foreground': "340 20% 98%",
            primary: "330 81% 65%",
            'primary-foreground': "340 40% 4%",
            secondary: "340 30% 12%",
            'secondary-foreground': "330 81% 90%",
            muted: "340 30% 14%",
            'muted-foreground': "340 20% 60%",
            accent: "340 40% 16%",
            'accent-foreground': "340 20% 98%",
            destructive: "0 62.8% 30.6%",
            'destructive-foreground': "0 0% 98%",
            border: "340 30% 18%",
            input: "340 30% 18%",
            ring: "330 81% 65%",
            'chart-1': "330 70% 68%",
            'chart-2': "315 60% 62%",
            'chart-3': "345 55% 65%",
            'chart-4': "320 65% 58%",
            'chart-5': "350 50% 60%"
        }
    },
    {
        id: "amber",
        label: "Amber Dusk",
        light: {
            background: "35 100% 98%",
            foreground: "35 40% 10%",
            card: "35 100% 99%",
            'card-foreground': "35 40% 10%",
            popover: "35 100% 100%",
            'popover-foreground': "35 40% 10%",
            primary: "35 90% 50%",
            'primary-foreground': "0 0% 100%",
            secondary: "35 30% 94%",
            'secondary-foreground': "35 90% 20%",
            muted: "35 30% 95%",
            'muted-foreground': "35 20% 45%",
            accent: "35 50% 96%",
            'accent-foreground': "35 90% 30%",
            destructive: "0 84.2% 60.2%",
            'destructive-foreground': "0 0% 100%",
            border: "35 30% 92%",
            input: "35 30% 92%",
            ring: "35 90% 50%",
            'chart-1': "35 80% 45%",
            'chart-2': "25 70% 48%",
            'chart-3': "45 65% 42%",
            'chart-4': "20 75% 40%",
            'chart-5': "50 60% 50%"
        },
        dark: {
            background: "35 40% 4%",
            foreground: "35 20% 98%",
            card: "35 40% 6%",
            'card-foreground': "35 20% 98%",
            popover: "35 40% 5%",
            'popover-foreground': "35 20% 98%",
            primary: "35 90% 60%",
            'primary-foreground': "35 40% 4%",
            secondary: "35 30% 12%",
            'secondary-foreground': "35 90% 90%",
            muted: "35 30% 14%",
            'muted-foreground': "35 20% 60%",
            accent: "35 40% 16%",
            'accent-foreground': "35 20% 98%",
            destructive: "0 62.8% 30.6%",
            'destructive-foreground': "0 0% 98%",
            border: "35 30% 18%",
            input: "35 30% 18%",
            ring: "35 90% 60%",
            'chart-1': "35 80% 62%",
            'chart-2': "25 70% 58%",
            'chart-3': "45 65% 55%",
            'chart-4': "20 75% 52%",
            'chart-5': "50 60% 58%"
        }
    }
];
