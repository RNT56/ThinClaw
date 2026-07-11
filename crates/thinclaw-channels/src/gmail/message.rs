use base64::Engine;

pub(super) fn build_multipart_message(
    mut headers: String,
    body_text: &str,
    attachments: &[thinclaw_media::MediaContent],
) -> String {
    let boundary = format!("thinclaw-{}", uuid::Uuid::new_v4());
    headers.push_str(&format!(
        "MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"{}\"\r\n\r\n",
        boundary
    ));
    let mut raw = headers;
    raw.push_str(&format!(
        "--{}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{}\r\n",
        boundary, body_text
    ));
    for attachment in attachments {
        let filename = attachment.filename.as_deref().unwrap_or("attachment");
        let encoded = base64::engine::general_purpose::STANDARD.encode(&attachment.data);
        raw.push_str(&format!(
            "--{}\r\nContent-Type: {}; name=\"{}\"\r\nContent-Disposition: attachment; filename=\"{}\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n",
            boundary,
            attachment.mime_type,
            sanitize_header_value(filename),
            sanitize_header_value(filename),
            encoded
        ));
    }
    raw.push_str(&format!("--{}--\r\n", boundary));
    raw
}

pub(super) fn sanitize_header_value(value: &str) -> String {
    value.replace(['\r', '\n', '"'], "_")
}
