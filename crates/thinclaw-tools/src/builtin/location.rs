//! Location (GPS) tool.
//!
//! Gets the device's current geographic location.
//! Uses platform-native approaches:
//! - macOS: CoreLocation via a small inline Swift script
//! - Linux: GeoClue D-Bus service; optional IP fallback only when explicitly enabled
//!
//! This replaces `LocationCommands.swift` from the companion app.

use std::time::Instant;

use async_trait::async_trait;
use serde::Serialize;
#[cfg(target_os = "macos")]
use tokio::process::Command;

use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput};
use thinclaw_types::JobContext;

/// Location/GPS tool.
pub struct LocationTool;

impl Default for LocationTool {
    fn default() -> Self {
        Self::new()
    }
}

impl LocationTool {
    pub fn new() -> Self {
        Self
    }
}

impl std::fmt::Debug for LocationTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocationTool").finish()
    }
}

#[derive(Debug, Serialize)]
struct LocationResult {
    latitude: f64,
    longitude: f64,
    accuracy_meters: Option<f64>,
    altitude_meters: Option<f64>,
    source: String,
}

/// Get location on macOS using CoreLocation via Swift.
#[cfg(target_os = "macos")]
async fn get_location() -> Result<LocationResult, ToolError> {
    // Inline Swift script that uses CoreLocation
    let swift_code = r#"
import CoreLocation
import Foundation

class LocationDelegate: NSObject, CLLocationManagerDelegate {
    let semaphore = DispatchSemaphore(value: 0)
    var location: CLLocation?
    var error: Error?
    
    func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        location = locations.last
        semaphore.signal()
    }
    
    func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
        self.error = error
        semaphore.signal()
    }
}

let manager = CLLocationManager()
let delegate = LocationDelegate()
manager.delegate = delegate
manager.desiredAccuracy = kCLLocationAccuracyBest
manager.startUpdatingLocation()

let result = delegate.semaphore.wait(timeout: .now() + 10)
manager.stopUpdatingLocation()

if result == .timedOut {
    fputs("ERROR:timeout", stderr)
    exit(1)
}

if let error = delegate.error {
    fputs("ERROR:\(error.localizedDescription)", stderr)
    exit(1)
}

if let loc = delegate.location {
    print("\(loc.coordinate.latitude),\(loc.coordinate.longitude),\(loc.horizontalAccuracy),\(loc.altitude)")
} else {
    fputs("ERROR:no_location", stderr)
    exit(1)
}
"#;

    let output = Command::new("swift")
        .arg("-e")
        .arg(swift_code)
        .output()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("swift: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ToolError::ExecutionFailed(format!(
            "CoreLocation failed: {stderr}"
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let parts: Vec<&str> = stdout.split(',').collect();
    if parts.len() < 4 {
        return Err(ToolError::ExecutionFailed(format!(
            "Unexpected location output: {stdout}"
        )));
    }

    let latitude: f64 = parts[0]
        .parse()
        .map_err(|_| ToolError::ExecutionFailed("Invalid latitude".to_string()))?;
    let longitude: f64 = parts[1]
        .parse()
        .map_err(|_| ToolError::ExecutionFailed("Invalid longitude".to_string()))?;
    let accuracy: f64 = parts[2].parse().unwrap_or(-1.0);
    let altitude: f64 = parts[3].parse().unwrap_or(0.0);

    Ok(LocationResult {
        latitude,
        longitude,
        accuracy_meters: if accuracy >= 0.0 {
            Some(accuracy)
        } else {
            None
        },
        altitude_meters: Some(altitude),
        source: "CoreLocation".to_string(),
    })
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
async fn get_ip_geolocation() -> Result<LocationResult, ToolError> {
    let client = reqwest::Client::new();
    let resp = client
        .get("http://ip-api.com/json/?fields=lat,lon")
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("IP geolocation: {e}")))?;

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Parse: {e}")))?;

    let lat = body.get("lat").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let lon = body.get("lon").and_then(|v| v.as_f64()).unwrap_or(0.0);

    Ok(LocationResult {
        latitude: lat,
        longitude: lon,
        accuracy_meters: None,
        altitude_meters: None,
        source: "ip-geolocation".to_string(),
    })
}

#[cfg(target_os = "linux")]
fn linux_ip_fallback_allowed() -> bool {
    thinclaw_platform::env_flag_enabled("LOCATION_ALLOW_IP_FALLBACK")
}

/// Get location on Linux using GeoClue D-Bus. IP lookup is an explicit fallback.
#[cfg(target_os = "linux")]
async fn get_location() -> Result<LocationResult, ToolError> {
    match get_geoclue_location().await {
        Ok(location) => Ok(location),
        Err(geoclue_error) if linux_ip_fallback_allowed() => {
            tracing::warn!(error = %geoclue_error, "GeoClue location failed; using explicit IP fallback");
            get_ip_geolocation().await
        }
        Err(geoclue_error) => Err(ToolError::ExecutionFailed(format!(
            "GeoClue location failed and IP fallback is disabled: {geoclue_error}. \
             Install/configure GeoClue or set LOCATION_ALLOW_IP_FALLBACK=true to allow approximate network geolocation."
        ))),
    }
}

#[cfg(target_os = "linux")]
async fn get_geoclue_location() -> Result<LocationResult, ToolError> {
    use zbus::zvariant::OwnedObjectPath;

    let connection = zbus::Connection::system()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("GeoClue system D-Bus: {e}")))?;
    let manager = zbus::Proxy::new(
        &connection,
        "org.freedesktop.GeoClue2",
        "/org/freedesktop/GeoClue2/Manager",
        "org.freedesktop.GeoClue2.Manager",
    )
    .await
    .map_err(|e| ToolError::ExecutionFailed(format!("GeoClue manager proxy: {e}")))?;

    let client_path: OwnedObjectPath = manager
        .call("GetClient", &())
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("GeoClue GetClient: {e}")))?;
    let client = zbus::Proxy::new(
        &connection,
        "org.freedesktop.GeoClue2",
        client_path.as_str(),
        "org.freedesktop.GeoClue2.Client",
    )
    .await
    .map_err(|e| ToolError::ExecutionFailed(format!("GeoClue client proxy: {e}")))?;

    let _ = client.set_property("DesktopId", &"thinclaw").await;
    let _ = client.set_property("RequestedAccuracyLevel", &4u32).await;
    let _: () = client
        .call("Start", &())
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("GeoClue Start: {e}")))?;

    let mut last_error = "GeoClue did not publish a location".to_string();
    for _ in 0..20 {
        let location_path: Result<OwnedObjectPath, _> = client.get_property("Location").await;
        match location_path {
            Ok(path) if path.as_str() != "/" => {
                let location = zbus::Proxy::new(
                    &connection,
                    "org.freedesktop.GeoClue2",
                    path.as_str(),
                    "org.freedesktop.GeoClue2.Location",
                )
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("GeoClue location proxy: {e}")))?;
                let latitude: f64 = location
                    .get_property("Latitude")
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(format!("GeoClue latitude: {e}")))?;
                let longitude: f64 = location
                    .get_property("Longitude")
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(format!("GeoClue longitude: {e}")))?;
                let accuracy_meters = location.get_property("Accuracy").await.ok();
                let altitude_meters = location.get_property("Altitude").await.ok();
                let _: Result<(), _> = client.call("Stop", &()).await;
                return Ok(LocationResult {
                    latitude,
                    longitude,
                    accuracy_meters,
                    altitude_meters,
                    source: "GeoClue".to_string(),
                });
            }
            Ok(_) => {
                last_error = "GeoClue returned an empty location path".to_string();
            }
            Err(error) => {
                last_error = format!("GeoClue Location property: {error}");
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    let _: Result<(), _> = client.call("Stop", &()).await;
    Err(ToolError::ExecutionFailed(last_error))
}

/// Get location on Windows using IP-based geolocation.
#[cfg(target_os = "windows")]
async fn get_location() -> Result<LocationResult, ToolError> {
    get_ip_geolocation().await
}

#[async_trait]
impl Tool for LocationTool {
    fn name(&self) -> &str {
        "location"
    }

    fn description(&self) -> &str {
        "Get the device's current geographic location. Use this when the task depends \
         on where the device is physically located, not just the user's configured timezone or locale."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let location = get_location().await?;

        Ok(ToolOutput::success(
            serde_json::to_value(&location).unwrap_or_default(),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Always // Location is privacy-sensitive
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_name() {
        let tool = LocationTool::new();
        assert_eq!(tool.name(), "location");
    }

    #[test]
    fn test_approval_always() {
        let tool = LocationTool::new();
        assert!(matches!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Always
        ));
    }

    #[test]
    fn test_location_result_serialization() {
        let result = LocationResult {
            latitude: 37.7749,
            longitude: -122.4194,
            accuracy_meters: Some(5.0),
            altitude_meters: Some(10.0),
            source: "test".to_string(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["latitude"], 37.7749);
        assert_eq!(json["longitude"], -122.4194);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_ip_fallback_defaults_off() {
        assert!(!linux_ip_fallback_allowed());
    }
}
