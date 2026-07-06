import Foundation

/// An extension-authorization prompt surfaced by the gateway (`auth_required`).
///
/// The agent is blocked because an extension needs the operator to authorize
/// it — typically an OAuth handshake. The mobile client renders this as an
/// inline timeline card: when an `authURL` is present the phone can open it in
/// a browser to complete the OAuth flow, but it never submits the resulting
/// token from the device — the v1 mobile contract has no auth-token endpoint,
/// so token capture stays on the desktop (see `docs/MOBILE_SECURITY.md`, D-T4).
///
/// Fields mirror the gateway's `SseEvent::AuthRequired`
/// (`crates/thinclaw-gateway/src/web/types/sse.rs`); only what the mobile card
/// needs is modeled.
public struct AuthPrompt: Hashable, Sendable, Codable {
    /// The extension requesting authorization (e.g. `"gmail"`).
    public var extensionName: String
    /// Optional human-readable instructions from the gateway.
    public var instructions: String?
    /// The OAuth / consent URL to open, when the gateway provides one.
    public var authURL: URL?
    /// A non-OAuth setup URL (e.g. a settings page), when provided.
    public var setupURL: URL?
    public var threadID: ThreadID?

    public init(
        extensionName: String,
        instructions: String? = nil,
        authURL: URL? = nil,
        setupURL: URL? = nil,
        threadID: ThreadID? = nil
    ) {
        self.extensionName = extensionName
        self.instructions = instructions
        self.authURL = authURL
        self.setupURL = setupURL
        self.threadID = threadID
    }
}

/// A credential request surfaced by the gateway (`credential_prompt`).
///
/// The agent wants a secret stored under `secretName` for `provider`. Per
/// D-T4 (`docs/MOBILE_SECURITY.md`), `credential_prompt` responses are
/// **excluded from v1 device scopes**: the phone must not submit the secret.
/// The client renders this as an inline "handle on desktop" affordance — it
/// carries no secret value and the card never captures one.
///
/// Fields mirror the gateway's `SseEvent::CredentialPrompt`
/// (`crates/thinclaw-gateway/src/web/types/sse.rs`).
public struct CredentialPrompt: Hashable, Sendable, Codable {
    /// Gateway-issued prompt id (opaque to the client in v1; the phone never
    /// answers the prompt, so it is carried only for identity/dedupe).
    public var promptID: String
    /// The name the secret would be stored under.
    public var secretName: String
    /// The provider requesting the credential (e.g. `"github"`).
    public var provider: String
    /// Why the credential is being requested.
    public var reason: String
    public var threadID: ThreadID?

    public init(
        promptID: String,
        secretName: String,
        provider: String,
        reason: String,
        threadID: ThreadID? = nil
    ) {
        self.promptID = promptID
        self.secretName = secretName
        self.provider = provider
        self.reason = reason
        self.threadID = threadID
    }
}
