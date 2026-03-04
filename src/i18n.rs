//! Internationalization (i18n) support for the IronClaw control UI.
//!
//! Provides a simple, static translation system based on message keys.
//! Translation files are embedded at compile time for zero-config startup.
//!
//! ```text
//! i18n::t("greeting", "en")  →  "Hello!"
//! i18n::t("greeting", "es")  →  "¡Hola!"
//! i18n::t("greeting", "zh")  →  "你好！"
//! ```
//!
//! # Adding a new language
//!
//! 1. Add a new locale function (e.g., `locale_fr()`)
//! 2. Register it in `get_catalog()`
//! 3. All missing keys fall back to English.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Supported locales.
pub const SUPPORTED_LOCALES: &[&str] = &["en", "es", "zh", "ja", "ko", "de", "fr", "pt", "ru"];

/// Default locale.
pub const DEFAULT_LOCALE: &str = "en";

/// A translation catalog: locale → (key → translated string).
type Catalog = HashMap<&'static str, HashMap<&'static str, &'static str>>;

/// Global translation catalog, lazily initialized.
static CATALOG: OnceLock<Catalog> = OnceLock::new();

/// Translate a message key for the given locale.
///
/// Falls back to English if the key is not found in the requested locale.
/// Returns the key itself if not found in any locale.
pub fn t<'a>(key: &'a str, locale: &str) -> &'a str {
    let catalog = get_catalog();

    // Try requested locale first.
    if let Some(messages) = catalog.get(locale)
        && let Some(translation) = messages.get(key)
    {
        return translation;
    }

    // Fall back to English.
    if locale != DEFAULT_LOCALE
        && let Some(messages) = catalog.get(DEFAULT_LOCALE)
        && let Some(translation) = messages.get(key)
    {
        return translation;
    }

    // Return key as last resort (leak to get 'static lifetime for catalog miss).
    // In practice, all keys should exist in English.
    key
}

/// Get all available locale codes.
pub fn available_locales() -> &'static [&'static str] {
    SUPPORTED_LOCALES
}

/// Check if a locale is supported.
pub fn is_supported(locale: &str) -> bool {
    SUPPORTED_LOCALES.contains(&locale)
}

/// Normalize a locale string (e.g., "en-US" → "en", "zh-CN" → "zh").
pub fn normalize_locale(locale: &str) -> &str {
    let base = locale.split(['-', '_']).next().unwrap_or(locale);
    if SUPPORTED_LOCALES.contains(&base) {
        base
    } else {
        DEFAULT_LOCALE
    }
}

/// Get or initialize the translation catalog.
fn get_catalog() -> &'static Catalog {
    CATALOG.get_or_init(|| {
        let mut catalog = Catalog::new();
        catalog.insert("en", locale_en());
        catalog.insert("es", locale_es());
        catalog.insert("zh", locale_zh());
        catalog.insert("ja", locale_ja());
        catalog
    })
}

// ---- Locale definitions ----

fn locale_en() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();

    // Navigation
    m.insert("nav.chat", "Chat");
    m.insert("nav.memory", "Memory");
    m.insert("nav.extensions", "Extensions");
    m.insert("nav.settings", "Settings");
    m.insert("nav.routines", "Routines");
    m.insert("nav.logs", "Logs");

    // Chat
    m.insert("chat.placeholder", "Type a message...");
    m.insert("chat.send", "Send");
    m.insert("chat.thinking", "Thinking...");
    m.insert("chat.stop", "Stop");
    m.insert("chat.retry", "Retry");
    m.insert("chat.copy", "Copy");
    m.insert("chat.clear", "Clear conversation");

    // Extensions
    m.insert("ext.search", "Search extensions...");
    m.insert("ext.install", "Install");
    m.insert("ext.uninstall", "Uninstall");
    m.insert("ext.activate", "Activate");
    m.insert("ext.deactivate", "Deactivate");
    m.insert("ext.installed", "Installed");
    m.insert("ext.available", "Available");
    m.insert("ext.auth_required", "Authentication Required");

    // Settings
    m.insert("settings.title", "Settings");
    m.insert("settings.general", "General");
    m.insert("settings.model", "Model");
    m.insert("settings.api_key", "API Key");
    m.insert("settings.save", "Save");
    m.insert("settings.saved", "Settings saved");
    m.insert("settings.language", "Language");
    m.insert("settings.theme", "Theme");

    // Routines
    m.insert("routines.title", "Routines");
    m.insert("routines.create", "Create Routine");
    m.insert("routines.trigger", "Trigger");
    m.insert("routines.enabled", "Enabled");
    m.insert("routines.disabled", "Disabled");
    m.insert("routines.delete", "Delete");
    m.insert("routines.history", "Run History");

    // Status
    m.insert("status.connected", "Connected");
    m.insert("status.disconnected", "Disconnected");
    m.insert("status.reconnecting", "Reconnecting...");
    m.insert("status.error", "Error");

    // Approvals
    m.insert("approval.approve", "Approve");
    m.insert("approval.deny", "Deny");
    m.insert("approval.always", "Always Allow");
    m.insert("approval.pending", "Approval Required");

    m
}

fn locale_es() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();

    m.insert("nav.chat", "Chat");
    m.insert("nav.memory", "Memoria");
    m.insert("nav.extensions", "Extensiones");
    m.insert("nav.settings", "Configuración");
    m.insert("nav.routines", "Rutinas");
    m.insert("nav.logs", "Registros");

    m.insert("chat.placeholder", "Escribe un mensaje...");
    m.insert("chat.send", "Enviar");
    m.insert("chat.thinking", "Pensando...");
    m.insert("chat.stop", "Detener");
    m.insert("chat.retry", "Reintentar");
    m.insert("chat.copy", "Copiar");
    m.insert("chat.clear", "Limpiar conversación");

    m.insert("ext.search", "Buscar extensiones...");
    m.insert("ext.install", "Instalar");
    m.insert("ext.uninstall", "Desinstalar");
    m.insert("ext.activate", "Activar");
    m.insert("ext.deactivate", "Desactivar");
    m.insert("ext.installed", "Instaladas");
    m.insert("ext.available", "Disponibles");

    m.insert("settings.title", "Configuración");
    m.insert("settings.language", "Idioma");
    m.insert("settings.save", "Guardar");
    m.insert("settings.saved", "Configuración guardada");

    m.insert("status.connected", "Conectado");
    m.insert("status.disconnected", "Desconectado");
    m.insert("status.reconnecting", "Reconectando...");

    m.insert("approval.approve", "Aprobar");
    m.insert("approval.deny", "Denegar");
    m.insert("approval.always", "Permitir Siempre");

    m
}

fn locale_zh() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();

    m.insert("nav.chat", "聊天");
    m.insert("nav.memory", "记忆");
    m.insert("nav.extensions", "扩展");
    m.insert("nav.settings", "设置");
    m.insert("nav.routines", "例程");
    m.insert("nav.logs", "日志");

    m.insert("chat.placeholder", "输入消息...");
    m.insert("chat.send", "发送");
    m.insert("chat.thinking", "思考中...");
    m.insert("chat.stop", "停止");
    m.insert("chat.retry", "重试");
    m.insert("chat.copy", "复制");
    m.insert("chat.clear", "清除对话");

    m.insert("ext.search", "搜索扩展...");
    m.insert("ext.install", "安装");
    m.insert("ext.uninstall", "卸载");

    m.insert("settings.title", "设置");
    m.insert("settings.language", "语言");
    m.insert("settings.save", "保存");
    m.insert("settings.saved", "设置已保存");

    m.insert("status.connected", "已连接");
    m.insert("status.disconnected", "已断开");
    m.insert("status.reconnecting", "重新连接中...");

    m.insert("approval.approve", "批准");
    m.insert("approval.deny", "拒绝");
    m.insert("approval.always", "始终允许");

    m
}

fn locale_ja() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();

    m.insert("nav.chat", "チャット");
    m.insert("nav.memory", "メモリ");
    m.insert("nav.extensions", "拡張機能");
    m.insert("nav.settings", "設定");
    m.insert("nav.routines", "ルーティン");
    m.insert("nav.logs", "ログ");

    m.insert("chat.placeholder", "メッセージを入力...");
    m.insert("chat.send", "送信");
    m.insert("chat.thinking", "考え中...");
    m.insert("chat.stop", "停止");
    m.insert("chat.retry", "再試行");
    m.insert("chat.copy", "コピー");

    m.insert("settings.title", "設定");
    m.insert("settings.language", "言語");
    m.insert("settings.save", "保存");
    m.insert("settings.saved", "設定を保存しました");

    m.insert("status.connected", "接続済み");
    m.insert("status.disconnected", "切断");
    m.insert("status.reconnecting", "再接続中...");

    m.insert("approval.approve", "承認");
    m.insert("approval.deny", "拒否");
    m.insert("approval.always", "常に許可");

    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_translate_en() {
        assert_eq!(t("nav.chat", "en"), "Chat");
        assert_eq!(t("chat.send", "en"), "Send");
    }

    #[test]
    fn test_translate_es() {
        assert_eq!(t("chat.send", "es"), "Enviar");
        assert_eq!(t("nav.memory", "es"), "Memoria");
    }

    #[test]
    fn test_translate_zh() {
        assert_eq!(t("chat.send", "zh"), "发送");
        assert_eq!(t("nav.chat", "zh"), "聊天");
    }

    #[test]
    fn test_translate_ja() {
        assert_eq!(t("chat.send", "ja"), "送信");
    }

    #[test]
    fn test_fallback_to_english() {
        // Key exists in English but not in Spanish
        assert_eq!(t("routines.create", "es"), "Create Routine");
    }

    #[test]
    fn test_unknown_key_returns_key() {
        assert_eq!(t("nonexistent.key", "en"), "nonexistent.key");
    }

    #[test]
    fn test_unknown_locale_falls_back() {
        // Unknown locale should fall back to English
        assert_eq!(t("nav.chat", "xx"), "Chat");
    }

    #[test]
    fn test_normalize_locale() {
        assert_eq!(normalize_locale("en-US"), "en");
        assert_eq!(normalize_locale("zh-CN"), "zh");
        assert_eq!(normalize_locale("ja_JP"), "ja");
        assert_eq!(normalize_locale("xx"), "en"); // unsupported
    }

    #[test]
    fn test_is_supported() {
        assert!(is_supported("en"));
        assert!(is_supported("es"));
        assert!(is_supported("zh"));
        assert!(!is_supported("xx"));
    }

    #[test]
    fn test_available_locales() {
        let locales = available_locales();
        assert!(locales.contains(&"en"));
        assert!(locales.contains(&"es"));
        assert!(locales.len() >= 4);
    }

    #[test]
    fn test_all_en_keys_present() {
        // All English keys should be non-empty.
        let catalog = get_catalog();
        let en = catalog.get("en").unwrap();
        for (key, value) in en {
            assert!(!value.is_empty(), "Empty translation for key: {}", key);
        }
    }
}
