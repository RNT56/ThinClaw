//! Plugin HTTP path registration.
//!
//! Plugins can register custom HTTP routes that the gateway serves.
//! This allows WASM plugins to expose REST endpoints.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// HTTP method for a registered route.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
}

impl std::fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Get => write!(f, "GET"),
            Self::Post => write!(f, "POST"),
            Self::Put => write!(f, "PUT"),
            Self::Delete => write!(f, "DELETE"),
            Self::Patch => write!(f, "PATCH"),
        }
    }
}

/// A registered plugin route.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginRoute {
    /// The plugin that owns this route.
    pub plugin_id: String,
    /// HTTP method.
    pub method: HttpMethod,
    /// Path pattern (e.g., "/plugins/my-plugin/webhook").
    pub path: String,
    /// Description of what this endpoint does.
    pub description: Option<String>,
    /// Whether this route requires authentication.
    pub requires_auth: bool,
}

/// Plugin route registry.
pub struct PluginRouter {
    /// Routes indexed by (method, path).
    routes: HashMap<(HttpMethod, String), PluginRoute>,
    /// Base path prefix for all plugin routes.
    pub base_path: String,
}

impl PluginRouter {
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
            base_path: "/plugins".to_string(),
        }
    }

    /// Register a new route.
    pub fn register(&mut self, route: PluginRoute) -> Result<(), PluginRouterError> {
        let full_path = format!("{}{}", self.base_path, route.path);
        let key = (route.method.clone(), full_path.clone());

        if self.routes.contains_key(&key) {
            return Err(PluginRouterError::RouteConflict {
                method: route.method.to_string(),
                path: full_path,
            });
        }

        self.routes.insert(key, route);
        Ok(())
    }

    /// Unregister all routes for a plugin.
    pub fn unregister_plugin(&mut self, plugin_id: &str) -> usize {
        let before = self.routes.len();
        self.routes.retain(|_, r| r.plugin_id != plugin_id);
        before - self.routes.len()
    }

    /// Look up a route.
    pub fn resolve(&self, method: &HttpMethod, path: &str) -> Option<&PluginRoute> {
        self.routes.get(&(method.clone(), path.to_string()))
    }

    /// List all registered routes.
    pub fn list_routes(&self) -> Vec<&PluginRoute> {
        self.routes.values().collect()
    }

    /// List routes for a specific plugin.
    pub fn routes_for_plugin(&self, plugin_id: &str) -> Vec<&PluginRoute> {
        self.routes
            .values()
            .filter(|r| r.plugin_id == plugin_id)
            .collect()
    }

    /// Total number of registered routes.
    pub fn route_count(&self) -> usize {
        self.routes.len()
    }
}

impl Default for PluginRouter {
    fn default() -> Self {
        Self::new()
    }
}

/// Plugin router errors.
#[derive(Debug, Clone)]
pub enum PluginRouterError {
    RouteConflict { method: String, path: String },
}

impl std::fmt::Display for PluginRouterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RouteConflict { method, path } => {
                write!(f, "Route conflict: {} {}", method, path)
            }
        }
    }
}

impl std::error::Error for PluginRouterError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_route(plugin: &str, method: HttpMethod, path: &str) -> PluginRoute {
        PluginRoute {
            plugin_id: plugin.to_string(),
            method,
            path: path.to_string(),
            description: None,
            requires_auth: false,
        }
    }

    #[test]
    fn test_register_and_resolve() {
        let mut router = PluginRouter::new();
        router
            .register(test_route("my-plugin", HttpMethod::Get, "/status"))
            .unwrap();

        let resolved = router.resolve(&HttpMethod::Get, "/plugins/status");
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().plugin_id, "my-plugin");
    }

    #[test]
    fn test_route_conflict() {
        let mut router = PluginRouter::new();
        router
            .register(test_route("a", HttpMethod::Get, "/x"))
            .unwrap();
        let result = router.register(test_route("b", HttpMethod::Get, "/x"));
        assert!(result.is_err());
    }

    #[test]
    fn test_different_methods_no_conflict() {
        let mut router = PluginRouter::new();
        router
            .register(test_route("a", HttpMethod::Get, "/x"))
            .unwrap();
        router
            .register(test_route("a", HttpMethod::Post, "/x"))
            .unwrap();
        assert_eq!(router.route_count(), 2);
    }

    #[test]
    fn test_unregister_plugin() {
        let mut router = PluginRouter::new();
        router
            .register(test_route("a", HttpMethod::Get, "/1"))
            .unwrap();
        router
            .register(test_route("a", HttpMethod::Post, "/2"))
            .unwrap();
        router
            .register(test_route("b", HttpMethod::Get, "/3"))
            .unwrap();

        let removed = router.unregister_plugin("a");
        assert_eq!(removed, 2);
        assert_eq!(router.route_count(), 1);
    }

    #[test]
    fn test_routes_for_plugin() {
        let mut router = PluginRouter::new();
        router
            .register(test_route("a", HttpMethod::Get, "/1"))
            .unwrap();
        router
            .register(test_route("b", HttpMethod::Get, "/2"))
            .unwrap();

        let routes = router.routes_for_plugin("a");
        assert_eq!(routes.len(), 1);
    }

    #[test]
    fn test_error_display() {
        let err = PluginRouterError::RouteConflict {
            method: "GET".to_string(),
            path: "/x".to_string(),
        };
        assert!(format!("{}", err).contains("GET /x"));
    }
}
