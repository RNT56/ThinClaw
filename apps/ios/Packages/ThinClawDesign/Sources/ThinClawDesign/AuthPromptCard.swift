import SwiftUI

/// Inline extension-authorization prompt (`auth_required`).
///
/// When the gateway provides an OAuth/consent URL the card offers to open it
/// in a browser (`onOpenAuth`) so the operator can complete the handshake; the
/// phone never captures the resulting token — the v1 mobile contract has no
/// auth-token endpoint, so token exchange completes on the desktop
/// (docs/MOBILE_SECURITY.md, D-T4). With no URL the card is text-only and
/// points the operator to the desktop.
///
/// Kept dependency-free (plain values + a closure) so ThinClawDesign carries no
/// Core/Transport dependency and the card is previewable in isolation.
public struct AuthPromptCard: View {
    private let extensionName: String
    private let instructions: String?
    private let hasAuthURL: Bool
    private let onOpenAuth: () -> Void

    public init(
        extensionName: String,
        instructions: String?,
        hasAuthURL: Bool,
        onOpenAuth: @escaping () -> Void
    ) {
        self.extensionName = extensionName
        self.instructions = instructions
        self.hasAuthURL = hasAuthURL
        self.onOpenAuth = onOpenAuth
    }

    public var body: some View {
        GlassEffectContainer {
            VStack(alignment: .leading, spacing: ThinClawSpacing.md) {
                Label("Authorize \(extensionName)", systemImage: "person.badge.key")
                    .font(ThinClawTypography.cardTitle)
                Text(instructions ?? "This extension needs to be authorized before the agent can continue.")
                    .font(ThinClawTypography.body)
                    .foregroundStyle(.secondary)
                if hasAuthURL {
                    Button(action: onOpenAuth) {
                        Label("Continue in browser", systemImage: "safari")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.glassProminent)
                } else {
                    Text("Finish setup on the desktop app.")
                        .font(ThinClawTypography.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .padding(ThinClawSpacing.lg)
            .glassEffect(.regular, in: .rect(cornerRadius: ThinClawRadius.card))
        }
        .accessibilityElement(children: .contain)
    }
}

/// Inline credential request (`credential_prompt`).
///
/// Per D-T4 (docs/MOBILE_SECURITY.md) `credential_prompt` responses are
/// **excluded from v1 device scopes**: the phone must not submit the secret.
/// This card is informational only — it names the provider and reason and
/// directs the operator to the desktop; it never renders a secret field.
public struct CredentialPromptCard: View {
    private let provider: String
    private let secretName: String
    private let reason: String

    public init(provider: String, secretName: String, reason: String) {
        self.provider = provider
        self.secretName = secretName
        self.reason = reason
    }

    public var body: some View {
        GlassEffectContainer {
            VStack(alignment: .leading, spacing: ThinClawSpacing.md) {
                Label("\(provider) credential needed", systemImage: "key.horizontal")
                    .font(ThinClawTypography.cardTitle)
                Text(reason)
                    .font(ThinClawTypography.body)
                    .foregroundStyle(.secondary)
                Label("Enter “\(secretName)” on the desktop app", systemImage: "desktopcomputer")
                    .font(ThinClawTypography.caption)
                    .foregroundStyle(.secondary)
            }
            .padding(ThinClawSpacing.lg)
            .glassEffect(.regular, in: .rect(cornerRadius: ThinClawRadius.card))
        }
        .accessibilityElement(children: .contain)
    }
}
