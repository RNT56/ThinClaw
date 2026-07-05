import Foundation
import SwiftUI
import ThinClawAuth
import ThinClawCore
import ThinClawPersistence

/// Composition root: builds the real dependency graph once at launch and
/// hands it down via the SwiftUI environment. Every effectful boundary is a
/// protocol so features and tests can inject fakes.
@MainActor
@Observable
final class AppDependencies {
    let transcriptStore: any TranscriptStoring

    /// M1: replaced by a Keychain-backed credential lookup.
    private(set) var isPaired: Bool = false

    init(transcriptStore: any TranscriptStoring = InMemoryTranscriptStore()) {
        self.transcriptStore = transcriptStore
    }
}
