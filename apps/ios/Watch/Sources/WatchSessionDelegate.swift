import Foundation
import ThinClawWatchBridge
import WatchConnectivity

/// Watch-side WCSession lifecycle: receives the provisioned watch token via
/// applicationContext, mirrors status snapshots, and sends relay RPCs.
/// Fleshed out at milestone M4.
final class WatchSessionDelegate: NSObject, WCSessionDelegate {
    func session(
        _ session: WCSession,
        activationDidCompleteWith activationState: WCSessionActivationState,
        error: (any Error)?
    ) {}

    func session(
        _ session: WCSession,
        didReceiveApplicationContext applicationContext: [String: Any]
    ) {
        // M4: store the provisioned watch credential in the watch keychain;
        // refresh the mirrored status snapshot.
    }
}
