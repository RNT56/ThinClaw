//! Variant-aware channel catalog shared by administrative surfaces.

use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ChannelCatalogEntry {
    pub id: String,
    pub variant: String,
    pub origin: String,
    pub description: String,
    pub compiled: bool,
    pub local_surface: bool,
}

impl ChannelCatalogEntry {
    fn native(id: &str, description: &str, compiled: bool) -> Self {
        Self {
            id: id.to_string(),
            variant: "native".to_string(),
            origin: "builtin".to_string(),
            description: description.to_string(),
            compiled,
            local_surface: false,
        }
    }

    fn local(id: &str, description: &str) -> Self {
        Self {
            id: id.to_string(),
            variant: "local_surface".to_string(),
            origin: "builtin".to_string(),
            description: description.to_string(),
            compiled: true,
            local_surface: true,
        }
    }
}

/// Return the complete static catalog for this binary, including every
/// embedded registry channel manifest. Installed dynamic variants are merged
/// by runtime/CLI adapters because their directory is profile-specific.
pub fn static_channel_catalog() -> Vec<ChannelCatalogEntry> {
    let mut entries = BTreeMap::new();
    for entry in [
        ChannelCatalogEntry::local("repl", "Interactive terminal conversation"),
        ChannelCatalogEntry::local("tui", "Full-screen terminal conversation"),
        ChannelCatalogEntry::native("gateway", "Web gateway ingress", true),
        ChannelCatalogEntry::native("signal", "Signal messenger", true),
        ChannelCatalogEntry::native("matrix", "Matrix rooms and DMs", true),
        ChannelCatalogEntry::native(
            "voice-call",
            "Voice-call lifecycle",
            cfg!(feature = "voice"),
        ),
        ChannelCatalogEntry::native("apns", "APNs device notifications", true),
        ChannelCatalogEntry::native(
            "browser-push",
            "Browser push subscriptions",
            cfg!(feature = "browser"),
        ),
        ChannelCatalogEntry::native(
            "nostr",
            "Nostr owner DM control and social actions",
            cfg!(feature = "nostr"),
        ),
        ChannelCatalogEntry::native("http", "HTTP webhook ingress", true),
        ChannelCatalogEntry::native("discord", "Discord Gateway and REST", true),
        ChannelCatalogEntry::native("gmail", "Gmail Pub/Sub and replies", true),
        ChannelCatalogEntry::native("bluebubbles", "BlueBubbles iMessage bridge", true),
        ChannelCatalogEntry::native(
            "imessage",
            "iMessage chat database polling",
            cfg!(target_os = "macos"),
        ),
        ChannelCatalogEntry::native(
            "apple_mail",
            "Apple Mail polling",
            cfg!(target_os = "macos"),
        ),
    ] {
        entries.insert((entry.id.clone(), entry.variant.clone()), entry);
    }

    if let Ok(catalog) = crate::registry::catalog::RegistryCatalog::load_or_embedded() {
        for manifest in catalog.list(Some(crate::registry::manifest::ManifestKind::Channel), None) {
            let entry = ChannelCatalogEntry {
                id: manifest.name.clone(),
                variant: "wasm".to_string(),
                origin: "registry".to_string(),
                description: manifest.description.clone(),
                compiled: cfg!(feature = "wasm-runtime"),
                local_surface: false,
            };
            entries.insert((entry.id.clone(), entry.variant.clone()), entry);
        }
    }

    entries.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_explicit_local_surfaces_and_no_fictional_cli_channel() {
        let catalog = static_channel_catalog();
        assert!(
            catalog
                .iter()
                .any(|entry| entry.id == "repl" && entry.local_surface)
        );
        assert!(
            catalog
                .iter()
                .any(|entry| entry.id == "tui" && entry.local_surface)
        );
        assert!(!catalog.iter().any(|entry| entry.id == "cli"));
    }

    #[test]
    fn catalog_contains_every_embedded_channel_manifest() {
        let catalog = static_channel_catalog();
        let embedded = crate::registry::embedded::load_embedded();
        for (key, manifest) in embedded {
            if key.starts_with("channels/") {
                assert!(
                    catalog
                        .iter()
                        .any(|entry| entry.id == manifest.name && entry.variant == "wasm"),
                    "missing embedded channel {}",
                    manifest.name
                );
            }
        }
    }
}
