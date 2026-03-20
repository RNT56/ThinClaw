//! QR Code pairing for non-Tailscale setups.
//!
//! When Tailscale is not available, the orchestrator generates a
//! self-signed TLS certificate (using `rcgen`), encodes the connection
//! info + pinned certificate fingerprint into a QR code, and displays
//! it in the terminal. The Tauri client scans the QR code and connects
//! with certificate pinning.
//!
//! This provides a secure fallback pairing mechanism without requiring
//! Tailscale or any third-party networking infrastructure.

use serde::{Deserialize, Serialize};

/// Pairing session information encoded in the QR code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingInfo {
    /// Orchestrator host address (IP or hostname).
    pub host: String,
    /// Port the WebSocket server is listening on.
    pub port: u16,
    /// Protocol to use (ws or wss).
    pub protocol: String,
    /// SHA-256 fingerprint of the self-signed certificate for pinning.
    pub cert_fingerprint: String,
    /// One-time pairing token (expires after first use).
    pub pairing_token: String,
    /// Server version.
    pub version: String,
}

impl PairingInfo {
    /// Encode pairing info as a URL for the QR code.
    ///
    /// Format: `ironclaw://pair?host=<host>&port=<port>&fp=<fingerprint>&token=<token>`
    pub fn to_url(&self) -> String {
        format!(
            "ironclaw://pair?host={}&port={}&proto={}&fp={}&token={}&v={}",
            self.host,
            self.port,
            self.protocol,
            self.cert_fingerprint,
            self.pairing_token,
            self.version,
        )
    }

    /// Parse pairing info from a URL.
    pub fn from_url(url: &str) -> Result<Self, String> {
        let url = url
            .strip_prefix("ironclaw://pair?")
            .ok_or_else(|| "Invalid pairing URL scheme".to_string())?;

        let params: std::collections::HashMap<String, String> = url
            .split('&')
            .filter_map(|pair| {
                let (key, value) = pair.split_once('=')?;

                Some((key.to_string(), value.to_string()))
            })
            .collect();

        Ok(PairingInfo {
            host: params.get("host").cloned().ok_or("Missing host")?,
            port: params
                .get("port")
                .and_then(|v| v.parse().ok())
                .ok_or("Missing or invalid port")?,
            protocol: params
                .get("proto")
                .cloned()
                .unwrap_or_else(|| "wss".to_string()),
            cert_fingerprint: params.get("fp").cloned().ok_or("Missing fingerprint")?,
            pairing_token: params.get("token").cloned().ok_or("Missing token")?,
            version: params
                .get("v")
                .cloned()
                .unwrap_or_else(|| "unknown".to_string()),
        })
    }

    /// Get the WebSocket connection URL.
    pub fn ws_url(&self) -> String {
        format!("{}://{}:{}/ws", self.protocol, self.host, self.port)
    }
}

/// Generate a pairing token (cryptographically random, URL-safe).
pub fn generate_pairing_token() -> String {
    let bytes: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();
    base64_encode(&bytes)
}

/// Simple base64 encoding (URL-safe, no padding).
fn base64_encode(data: &[u8]) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);

        result.push(CHARSET[((b0 >> 2) & 0x3F) as usize] as char);
        result.push(CHARSET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);

        if chunk.len() > 1 {
            result.push(CHARSET[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize] as char);
        }
        if chunk.len() > 2 {
            result.push(CHARSET[(b2 & 0x3F) as usize] as char);
        }
    }
    result
}

/// Generate a QR code as a string of Unicode block characters for terminal display.
///
/// Uses the `qrcode` crate to produce a real scannable QR matrix. Two QR rows
/// are packed per terminal line using Unicode half-block characters (▀, ▄, █, ' ')
/// for a compact display.
pub fn render_qr_terminal(data: &str) -> String {
    use qrcode::QrCode;

    let code = match QrCode::new(data) {
        Ok(c) => c,
        Err(e) => {
            // Fall back to text display if QR generation fails
            return format!(
                "\n  QR code generation failed: {}\n\n  \
                 Manually enter this URL:\n    {}\n",
                e, data
            );
        }
    };

    let matrix = code.to_colors();
    let width = code.width();

    // Build the QR image with a quiet-zone border (2 modules on each side).
    let quiet = 2;
    let total_w = width + quiet * 2;

    // Helper: is the module at (row, col) dark?  Quiet zone modules are white.
    let is_dark = |row: i32, col: i32| -> bool {
        let r = row - quiet as i32;
        let c = col - quiet as i32;
        if r < 0 || c < 0 || r >= width as i32 || c >= width as i32 {
            return false; // quiet zone → white
        }
        matrix[r as usize * width + c as usize] == qrcode::Color::Dark
    };

    // Render two rows per terminal line using Unicode half-blocks:
    //   top=dark, bot=dark  → '█'  (full block)
    //   top=dark, bot=light → '▀'  (upper half)
    //   top=light, bot=dark → '▄'  (lower half)
    //   top=light, bot=light → ' ' (space)
    let total_h = total_w + (total_w % 2); // pad to even height
    let mut lines = Vec::new();
    let mut row = 0i32;
    while row < total_h as i32 {
        let mut line = String::with_capacity(total_w + 2);
        line.push(' '); // left margin
        for col in 0..total_w as i32 {
            let top = is_dark(row, col);
            let bot = is_dark(row + 1, col);
            line.push(match (top, bot) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        lines.push(line);
        row += 2;
    }

    let qr_display = lines.join("\n");

    format!(
        "\n{qr_display}\n\n  \
         Scan the QR code above with your IronClaw companion app.\n  \
         Or manually enter this URL:\n    {data}\n"
    )
}

/// Pairing session manager.
pub struct PairingSession {
    info: PairingInfo,
    /// Whether the pairing token has been used.
    used: bool,
}

impl PairingSession {
    /// Create a new pairing session.
    pub fn new(host: String, port: u16, cert_fingerprint: String) -> Self {
        Self {
            info: PairingInfo {
                host,
                port,
                protocol: "wss".to_string(),
                cert_fingerprint,
                pairing_token: generate_pairing_token(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            used: false,
        }
    }

    /// Get the pairing info.
    pub fn info(&self) -> &PairingInfo {
        &self.info
    }

    /// Validate a pairing token. Consumes the token on success (one-time use).
    pub fn validate_token(&mut self, token: &str) -> bool {
        if self.used {
            return false;
        }
        if self.info.pairing_token == token {
            self.used = true;
            true
        } else {
            false
        }
    }

    /// Check if the pairing session has been used.
    pub fn is_used(&self) -> bool {
        self.used
    }

    /// Display the QR code in the terminal.
    pub fn display_qr(&self) -> String {
        let url = self.info.to_url();
        render_qr_terminal(&url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pairing_info_url_roundtrip() {
        let info = PairingInfo {
            host: "192.168.1.100".to_string(),
            port: 3000,
            protocol: "wss".to_string(),
            cert_fingerprint: "abc123def456".to_string(),
            pairing_token: "token_xyz".to_string(),
            version: "1.0.0".to_string(),
        };

        let url = info.to_url();
        assert!(url.starts_with("ironclaw://pair?"));
        assert!(url.contains("host=192.168.1.100"));
        assert!(url.contains("port=3000"));

        let parsed = PairingInfo::from_url(&url).unwrap();
        assert_eq!(parsed.host, "192.168.1.100");
        assert_eq!(parsed.port, 3000);
        assert_eq!(parsed.cert_fingerprint, "abc123def456");
        assert_eq!(parsed.pairing_token, "token_xyz");
    }

    #[test]
    fn test_pairing_info_ws_url() {
        let info = PairingInfo {
            host: "10.0.0.1".to_string(),
            port: 8080,
            protocol: "wss".to_string(),
            cert_fingerprint: "fp".to_string(),
            pairing_token: "tok".to_string(),
            version: "1.0.0".to_string(),
        };
        assert_eq!(info.ws_url(), "wss://10.0.0.1:8080/ws");
    }

    #[test]
    fn test_pairing_info_from_invalid_url() {
        assert!(PairingInfo::from_url("https://example.com").is_err());
    }

    #[test]
    fn test_generate_pairing_token() {
        let token1 = generate_pairing_token();
        let token2 = generate_pairing_token();
        // Tokens should be non-empty and unique
        assert!(!token1.is_empty());
        assert_ne!(token1, token2);
        // Should be URL-safe (no + or /)
        assert!(!token1.contains('+'));
        assert!(!token1.contains('/'));
    }

    #[test]
    fn test_pairing_session_validate_token() {
        let mut session =
            PairingSession::new("10.0.0.1".to_string(), 3000, "fingerprint".to_string());

        assert!(!session.is_used());

        // Wrong token should fail
        assert!(!session.validate_token("wrong_token"));

        // Correct token should succeed
        let token = session.info().pairing_token.clone();
        assert!(session.validate_token(&token));
        assert!(session.is_used());

        // Second use should fail (one-time)
        assert!(!session.validate_token(&token));
    }

    #[test]
    fn test_render_qr_terminal() {
        let output = render_qr_terminal("ironclaw://pair?host=10.0.0.1");
        assert!(output.contains("ironclaw://pair"));
        assert!(output.contains("Scan the QR code"));
    }

    #[test]
    fn test_base64_encode() {
        let encoded = base64_encode(b"Hello, World!");
        assert!(!encoded.is_empty());
        assert!(!encoded.contains('+'));
        assert!(!encoded.contains('/'));
    }
}
