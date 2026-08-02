use std::collections::HashSet;
use std::sync::Arc;

use thinclaw_tools::{RegistrySealError, ToolRegistry};

#[test]
fn static_catalog_identity_and_seal_contract() {
    assert_eq!(thinclaw_tools::STATIC_TOOL_CATALOG.len(), 124);
    let identities = thinclaw_tools::STATIC_TOOL_CATALOG
        .iter()
        .map(|descriptor| descriptor.name)
        .collect::<HashSet<_>>();
    assert_eq!(identities.len(), 124);

    let registry = ToolRegistry::new();
    assert_eq!(
        registry.seal_startup(),
        Err(RegistrySealError::EmptyRegistry)
    );
    assert!(
        registry
            .register_sync(Arc::new(thinclaw_tools::builtin::EchoTool))
            .changed()
    );
    let sealed = registry.seal_startup().expect("populated registry seals");
    assert!(sealed.sealed);
    assert_eq!(sealed.revision, 1);
    assert_eq!(sealed.identities.len(), 1);
    assert_eq!(sealed.identities[0].name, "echo");
}

#[test]
fn dynamic_origin_vocabulary_is_closed() {
    let dynamic = [
        thinclaw_tools::ToolOrigin::Wasm,
        thinclaw_tools::ToolOrigin::Mcp,
        thinclaw_tools::ToolOrigin::UserTool,
        thinclaw_tools::ToolOrigin::NativePlugin,
    ];
    assert_eq!(
        dynamic.map(|origin| origin.to_string()),
        ["wasm", "mcp", "user-tool", "native-plugin"]
    );
}
