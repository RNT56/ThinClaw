use serde::Serialize;
use specta::Type;
use std::collections::BTreeMap;

/// Complete translation payload consumed by every Desktop webview.
#[derive(Debug, Clone, Serialize, Type)]
pub struct DesktopI18nCatalog {
    pub locale: String,
    pub default_locale: String,
    pub available_locales: Vec<String>,
    pub messages: BTreeMap<String, String>,
}

#[tauri::command]
#[specta::specta]
pub fn direct_i18n_get_catalog(locale: String) -> DesktopI18nCatalog {
    let locale = thinclaw_core::i18n::normalize_locale(&locale).to_string();
    DesktopI18nCatalog {
        messages: thinclaw_core::i18n::messages(&locale),
        locale,
        default_locale: thinclaw_core::i18n::DEFAULT_LOCALE.to_string(),
        available_locales: thinclaw_core::i18n::available_locales()
            .iter()
            .map(|locale| (*locale).to_string())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_catalog_normalizes_locale_and_is_complete() {
        let german = direct_i18n_get_catalog("de-DE".to_string());
        let english = direct_i18n_get_catalog("en".to_string());
        assert_eq!(german.locale, "de");
        assert_eq!(german.messages.get("nav.settings").unwrap(), "Einstellungen");
        assert_eq!(german.messages.len(), english.messages.len());
        assert!(german.available_locales.contains(&"ko".to_string()));
    }
}
