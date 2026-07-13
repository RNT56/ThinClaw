import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { getCatalog } = vi.hoisted(() => ({ getCatalog: vi.fn() }));
vi.mock("../../lib/bindings", () => ({
    commands: { directI18nGetCatalog: getCatalog },
}));

import { I18nProvider, I18N_STORAGE_KEY, normalizeUiLocale, useI18n } from "../../components/i18n-provider";

function Probe() {
    const { locale, setLocale, t } = useI18n();
    return <button onClick={() => setLocale("de-DE")}>{locale}:{t("nav.settings")}</button>;
}

describe("frontend i18n bridge", () => {
    beforeEach(() => {
        localStorage.clear();
        getCatalog.mockImplementation(async (locale: string) => ({
            locale: normalizeUiLocale(locale),
            default_locale: "en",
            available_locales: ["en", "de"],
            messages: { "nav.settings": normalizeUiLocale(locale) === "de" ? "Einstellungen" : "Settings" },
        }));
    });

    it("normalizes browser locale variants", () => {
        expect(normalizeUiLocale("pt-BR")).toBe("pt");
        expect(normalizeUiLocale("unknown")).toBe("en");
    });

    it("loads the Rust catalog and persists language changes", async () => {
        render(<I18nProvider><Probe /></I18nProvider>);
        await waitFor(() => expect(screen.getByRole("button")).toHaveTextContent("en:Settings"));
        await userEvent.click(screen.getByRole("button"));
        await waitFor(() => expect(screen.getByRole("button")).toHaveTextContent("de:Einstellungen"));
        expect(localStorage.getItem(I18N_STORAGE_KEY)).toBe("de");
        expect(document.documentElement.lang).toBe("de");
    });
});
