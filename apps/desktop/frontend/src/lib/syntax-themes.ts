
export interface SyntaxTheme {
    id: string;
    label: string;
    colors: {
        bg: string;
        color: string;
        comment: string;
        keyword: string;
        string: string;
        title: string;
        number: string;
        function: string;
        variable: string;
        attr: string;
        addition: string;
        deletion: string;
    };
}

export const DARK_SYNTAX_THEMES: SyntaxTheme[] = [
    {
        id: "tokyo-night",
        label: "Tokyo Night",
        colors: {
            bg: "240 10% 4%",
            color: "230 20% 90%",
            comment: "230 15% 45%",
            keyword: "260 85% 75%",
            string: "85 50% 65%",
            title: "215 85% 70%",
            number: "25 90% 65%",
            function: "195 85% 70%",
            variable: "210 20% 85%",
            attr: "35 75% 65%",
            addition: "85 50% 65%",   // Green
            deletion: "355 70% 65%", // Red
        }
    },
    {
        id: "one-dark-pro",
        label: "One Dark Pro",
        colors: {
            bg: "220 13% 18%",
            color: "220 14% 71%",
            comment: "220 10% 40%",
            keyword: "286 60% 67%",
            string: "95 38% 62%",
            title: "207 82% 66%",
            number: "29 54% 61%",
            function: "207 82% 66%",
            variable: "355 65% 65%",
            attr: "29 54% 61%",
            addition: "95 38% 62%",
            deletion: "355 65% 65%",
        }
    },
    {
        id: "dracula",
        label: "Dracula Official",
        colors: {
            bg: "231 15% 18%",
            color: "60 30% 96%",
            comment: "231 15% 45%",
            keyword: "326 100% 74%",
            string: "65 92% 76%",
            title: "191 100% 77%",
            number: "265 89% 78%",
            function: "115 100% 76%",
            variable: "31 100% 71%",
            attr: "115 100% 76%",
            addition: "115 100% 76%",
            deletion: "326 100% 74%",
        }
    },
    {
        id: "night-owl",
        label: "Night Owl",
        colors: {
            bg: "207 95% 8%",
            color: "200 15% 90%",
            comment: "200 15% 45%",
            keyword: "261 80% 80%",
            string: "80 60% 75%",
            title: "195 90% 75%",
            number: "35 95% 70%",
            function: "195 90% 75%",
            variable: "210 20% 90%",
            attr: "41 100% 50%",
            addition: "80 60% 75%",
            deletion: "355 80% 70%",
        }
    },
    {
        id: "nord",
        label: "Nord",
        colors: {
            bg: "216 18% 21%",
            color: "220 20% 82%",
            comment: "218 19% 36%",
            keyword: "215 35% 52%",
            string: "95 25% 65%",
            title: "198 33% 67%",
            number: "16 49% 63%",
            function: "198 33% 67%",
            variable: "220 20% 82%",
            attr: "39 76% 74%",
            addition: "95 25% 65%",
            deletion: "354 36% 62%",
        }
    },
    {
        id: "rose-pine",
        label: "Rosé Pine",
        colors: {
            bg: "249 22% 12%",
            color: "245 50% 91%",
            comment: "249 12% 47%",
            keyword: "267 57% 78%",
            string: "2 55% 83%",
            title: "197 49% 38%",
            number: "35 88% 72%",
            function: "343 76% 68%",
            variable: "245 50% 91%",
            attr: "189 43% 73%",
            addition: "2 55% 83%",
            deletion: "343 76% 68%",
        }
    },
    {
        id: "synthwave84",
        label: "SynthWave '84",
        colors: {
            bg: "253 44% 10%",
            color: "300 20% 90%",
            comment: "200 15% 45%",
            keyword: "320 100% 70%",
            string: "50 100% 70%",
            title: "190 100% 70%",
            number: "20 100% 70%",
            function: "190 100% 70%",
            variable: "300 20% 90%",
            attr: "320 100% 70%",
            addition: "50 100% 70%",
            deletion: "320 100% 70%",
        }
    },
    {
        id: "scrappy-dark",
        label: "ThinClaw Default",
        colors: {
            bg: "240 10% 4%",
            color: "0 0% 98%",
            comment: "240 5% 64.9%",
            keyword: "260 80% 70%",
            string: "140 60% 70%",
            title: "210 90% 70%",
            number: "30 90% 70%",
            function: "210 90% 70%",
            variable: "0 0% 98%",
            attr: "30 90% 70%",
            addition: "140 60% 70%",
            deletion: "0 80% 60%",
        }
    }
];

export const LIGHT_SYNTAX_THEMES: SyntaxTheme[] = [
    {
        id: "github-light",
        label: "GitHub Light",
        colors: {
            bg: "0 0% 100%",
            color: "240 10% 15%",
            comment: "210 10% 40%",
            keyword: "354 70% 45%",
            string: "210 100% 25%",
            title: "210 100% 25%",
            number: "210 100% 25%",
            function: "270 60% 40%",
            variable: "240 10% 15%",
            attr: "210 100% 25%",
            addition: "120 40% 30%", // Green
            deletion: "354 70% 45%", // Red
        }
    },
    {
        id: "atom-one-light",
        label: "Atom One Light",
        colors: {
            bg: "0 0% 99%",
            color: "240 10% 20%",
            comment: "230 6% 65%",
            keyword: "300 65% 45%",
            string: "120 40% 40%",
            title: "220 85% 55%",
            number: "40 100% 35%",
            function: "220 85% 55%",
            variable: "25 80% 50%",
            attr: "25 80% 50%",
            addition: "120 40% 40%",
            deletion: "0 80% 45%",
        }
    },
    {
        id: "solarized-light",
        label: "Solarized Light",
        colors: {
            bg: "44 87% 94%",
            color: "192 13% 40%",
            comment: "192 10% 60%",
            keyword: "60 100% 35%",
            string: "175 60% 40%",
            title: "200 70% 50%",
            number: "15 80% 45%",
            function: "200 70% 50%",
            variable: "192 13% 40%",
            attr: "15 80% 45%",
            addition: "175 60% 40%",
            deletion: "15 80% 45%",
        }
    },
    {
        id: "catppuccin-latte",
        label: "Catppuccin Latte",
        colors: {
            bg: "240 20% 98%",
            color: "234 16% 35%",
            comment: "233 10% 63%",
            keyword: "266 85% 58%",
            string: "109 58% 40%",
            title: "220 91% 54%",
            number: "22 93% 48%",
            function: "220 91% 54%",
            variable: "347 87% 60%",
            attr: "203 92% 45%",
            addition: "109 58% 40%",
            deletion: "347 87% 60%",
        }
    },
    {
        id: "rose-pine-dawn",
        label: "Rosé Pine Dawn",
        colors: {
            bg: "32 57% 95%",
            color: "248 19% 40%",
            comment: "257 9% 61%",
            keyword: "268 21% 57%",
            string: "3 53% 67%",
            title: "197 53% 34%",
            number: "35 81% 56%",
            function: "343 35% 55%",
            variable: "248 19% 40%",
            attr: "189 30% 48%",
            addition: "189 30% 48%",
            deletion: "343 35% 55%",
        }
    },
    {
        id: "scrappy-light",
        label: "ThinClaw Default",
        colors: {
            bg: "0 0% 100%",
            color: "240 10% 4%",
            comment: "240 5% 45%",
            keyword: "260 80% 45%",
            string: "140 60% 35%",
            title: "210 90% 40%",
            number: "30 90% 40%",
            function: "210 90% 40%",
            variable: "240 10% 4%",
            attr: "30 90% 40%",
            addition: "140 60% 35%",
            deletion: "0 80% 45%",
        }
    }
];
