import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { commands } from "../lib/bindings";

export const I18N_STORAGE_KEY = "thinclaw-ui-locale";

export const LOCALE_LABELS: Readonly<Record<string, string>> = {
    en: "English",
    es: "Español",
    zh: "中文",
    ja: "日本語",
    ko: "한국어",
    de: "Deutsch",
    fr: "Français",
    pt: "Português",
    ru: "Русский",
};

const FALLBACK_MESSAGES: Readonly<Record<string, string>> = {
    "nav.workbench": "Workbench",
    "nav.cockpit": "Agent Cockpit",
    "nav.imagine": "Imagine",
    "nav.commands": "Commands",
    "nav.settings": "Settings",
    "settings.language": "Language",
    "settings.appearance": "Appearance and language",
    "settings.density": "Interface density",
    "common.loading": "Loading ThinClaw...",
    "common.loading_view": "Loading view...",
    "command.search": "Search modes and settings...",
    "command.empty": "No matching commands",
    "command.palette": "Command palette",
};

export function normalizeUiLocale(locale: string | null | undefined): string {
    const base = locale?.trim().toLowerCase().split(/[-_]/)[0] ?? "en";
    return Object.prototype.hasOwnProperty.call(LOCALE_LABELS, base) ? base : "en";
}

interface I18nContextValue {
    locale: string;
    availableLocales: readonly string[];
    setLocale: (locale: string) => void;
    t: (key: string) => string;
}

const fallbackContext: I18nContextValue = {
    locale: "en",
    availableLocales: Object.keys(LOCALE_LABELS),
    setLocale: () => undefined,
    t: (key) => FALLBACK_MESSAGES[key] ?? key,
};

const I18nContext = createContext<I18nContextValue>(fallbackContext);

function initialLocale(): string {
    return normalizeUiLocale(localStorage.getItem(I18N_STORAGE_KEY) || navigator.language);
}

export function I18nProvider({ children }: { children: ReactNode }) {
    const [locale, setLocaleState] = useState(initialLocale);
    const [messages, setMessages] = useState<Record<string, string>>({ ...FALLBACK_MESSAGES });
    const [availableLocales, setAvailableLocales] = useState<string[]>(Object.keys(LOCALE_LABELS));

    const setLocale = (nextLocale: string) => {
        const normalized = normalizeUiLocale(nextLocale);
        localStorage.setItem(I18N_STORAGE_KEY, normalized);
        setLocaleState(normalized);
    };

    useEffect(() => {
        document.documentElement.lang = locale;
        let active = true;
        commands.directI18nGetCatalog(locale)
            .then((catalog) => {
                if (!active || !catalog?.messages) return;
                setMessages({ ...FALLBACK_MESSAGES, ...catalog.messages });
                setAvailableLocales(catalog.available_locales.map(normalizeUiLocale));
                if (catalog.locale !== locale) setLocaleState(normalizeUiLocale(catalog.locale));
            })
            .catch(() => {
                if (active) setMessages({ ...FALLBACK_MESSAGES });
            });
        return () => { active = false; };
    }, [locale]);

    useEffect(() => {
        const handleStorage = (event: StorageEvent) => {
            if (event.storageArea === localStorage && event.key === I18N_STORAGE_KEY) {
                setLocaleState(normalizeUiLocale(event.newValue));
            }
        };
        window.addEventListener("storage", handleStorage);
        return () => window.removeEventListener("storage", handleStorage);
    }, []);

    const value = useMemo<I18nContextValue>(() => ({
        locale,
        availableLocales,
        setLocale,
        t: (key) => messages[key] ?? FALLBACK_MESSAGES[key] ?? key,
    }), [availableLocales, locale, messages]);

    return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nContextValue {
    return useContext(I18nContext);
}
