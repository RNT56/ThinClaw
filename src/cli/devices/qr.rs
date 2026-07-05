//! Terminal QR code rendering for `thinclaw devices pair`.
//!
//! Renders the `thinclaw://pair?d=...` payload as a scannable QR code using
//! Unicode half-block characters (two module rows per printed line), via the
//! already-vendored `qrcode` crate. No image/terminal-graphics dependency.

use qrcode::{Color, QrCode};

/// Render `data` as a QR code string suitable for direct terminal printing.
///
/// Uses ` `, `▀`, `▄`, `█` half-block glyphs so each printed line represents
/// two rows of QR modules, keeping the terminal output roughly square. Falls
/// back to a plain-text notice if the payload cannot be encoded (e.g. too
/// large for the QR spec).
pub fn render_qr_unicode(data: &str) -> String {
    let code = match QrCode::new(data.as_bytes()) {
        Ok(code) => code,
        Err(err) => return format!("(unable to render QR code: {err})"),
    };

    let width = code.width();
    // One module of quiet-zone padding on each side, per QR code convention.
    let is_dark = |x: i64, y: i64| -> bool {
        if x < 0 || y < 0 || x as usize >= width || y as usize >= width {
            return false;
        }
        code[(x as usize, y as usize)] == Color::Dark
    };

    let padded_width = width as i64 + 2;
    let mut out = String::new();

    // Iterate two module-rows at a time; each pair maps to one printed line
    // via half-block glyphs (top row = upper half, bottom row = lower half).
    let mut y = -1i64;
    while y < padded_width - 1 {
        for x in -1..padded_width - 1 {
            let top = is_dark(x, y);
            let bottom = is_dark(x, y + 1);
            let glyph = match (top, bottom) {
                (false, false) => ' ',
                (true, false) => '▀',
                (false, true) => '▄',
                (true, true) => '█',
            };
            out.push(glyph);
        }
        out.push('\n');
        y += 2;
    }

    out.trim_end_matches('\n').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_qr_unicode_produces_nonempty_grid() {
        let rendered = render_qr_unicode("thinclaw://pair?d=abc123");
        assert!(!rendered.is_empty());
        // Every line should have the same width (a rectangular grid).
        let lines: Vec<&str> = rendered.lines().collect();
        assert!(lines.len() > 1);
        let first_len = lines[0].chars().count();
        assert!(lines.iter().all(|line| line.chars().count() == first_len));
    }

    #[test]
    fn test_render_qr_unicode_only_uses_expected_glyphs() {
        let rendered = render_qr_unicode("hello world");
        for ch in rendered.chars() {
            assert!(
                matches!(ch, ' ' | '▀' | '▄' | '█' | '\n'),
                "unexpected glyph: {ch:?}"
            );
        }
    }

    #[test]
    fn test_render_qr_unicode_differs_for_different_payloads() {
        let a = render_qr_unicode("thinclaw://pair?d=aaaa");
        let b = render_qr_unicode("thinclaw://pair?d=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        assert_ne!(a, b);
    }
}
