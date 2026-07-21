import SwiftUI
import ThinClawDesign

#if canImport(VisionKit) && canImport(AVFoundation) && os(iOS)
    import AVFoundation
    import VisionKit

    /// Camera QR scanner for the pairing flow, built on VisionKit's
    /// `DataScannerViewController`. Gated behind device support + camera
    /// permission; there is always a manual fallback (``ManualEntrySheet``) for
    /// the simulator and permission-denied cases.
    struct QRScannerSheet: View {
        /// Called with the first recognized `thinclaw://pair` URL (or any URL —
        /// the store validates it).
        let onScanned: (URL) -> Void
        let onCancel: () -> Void

        @State private var permission: CameraPermission = .undetermined

        /// Whether this build can present the camera scanner at all (device
        /// supports `DataScannerViewController` and is available right now).
        static var isSupported: Bool {
            DataScannerViewController.isSupported && DataScannerViewController.isAvailable
        }

        var body: some View {
            NavigationStack {
                Group {
                    switch permission {
                    case .authorized:
                        DataScannerRepresentable(onScanned: onScanned)
                            .ignoresSafeArea()
                    case .undetermined:
                        ProgressView("Preparing camera…")
                            .task { await requestPermission() }
                    case .denied:
                        deniedView
                    }
                }
                .navigationTitle("Scan pairing code")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel", action: onCancel)
                    }
                }
            }
        }

        private var deniedView: some View {
            ContentUnavailableView {
                Label("Camera access needed", systemImage: "camera.badge.ellipsis")
            } description: {
                Text(
                    "Enable camera access in Settings to scan the QR code, or go "
                        + "back and enter the pairing link manually.")
            } actions: {
                Button("Open Settings") {
                    if let url = URL(string: UIApplication.openSettingsURLString) {
                        UIApplication.shared.open(url)
                    }
                }
                .thinClawButtonStyle(prominent: true)
            }
        }

        private func requestPermission() async {
            switch AVCaptureDevice.authorizationStatus(for: .video) {
            case .authorized:
                permission = .authorized
            case .notDetermined:
                let granted = await AVCaptureDevice.requestAccess(for: .video)
                permission = granted ? .authorized : .denied
            default:
                permission = .denied
            }
        }

        private enum CameraPermission {
            case undetermined, authorized, denied
        }
    }

    /// `UIViewControllerRepresentable` bridge to VisionKit's scanner, restricted
    /// to QR barcodes and reporting the first URL-shaped payload.
    private struct DataScannerRepresentable: UIViewControllerRepresentable {
        let onScanned: (URL) -> Void

        func makeCoordinator() -> Coordinator {
            Coordinator(onScanned: onScanned)
        }

        func makeUIViewController(context: Context) -> DataScannerViewController {
            let scanner = DataScannerViewController(
                recognizedDataTypes: [.barcode(symbologies: [.qr])],
                qualityLevel: .balanced,
                isHighFrameRateTrackingEnabled: false,
                isHighlightingEnabled: true)
            scanner.delegate = context.coordinator
            return scanner
        }

        func updateUIViewController(
            _ controller: DataScannerViewController, context: Context
        ) {
            try? controller.startScanning()
        }

        static func dismantleUIViewController(
            _ controller: DataScannerViewController, coordinator: Coordinator
        ) {
            controller.stopScanning()
        }

        final class Coordinator: NSObject, DataScannerViewControllerDelegate {
            private let onScanned: (URL) -> Void
            /// Latch so a rapid burst of recognitions only fires pairing once.
            private var handled = false

            init(onScanned: @escaping (URL) -> Void) {
                self.onScanned = onScanned
            }

            func dataScanner(
                _ dataScanner: DataScannerViewController,
                didAdd addedItems: [RecognizedItem],
                allItems: [RecognizedItem]
            ) {
                deliverFirstURL(from: addedItems)
            }

            func dataScanner(
                _ dataScanner: DataScannerViewController,
                didTapOn item: RecognizedItem
            ) {
                deliverFirstURL(from: [item])
            }

            private func deliverFirstURL(from items: [RecognizedItem]) {
                guard !handled else { return }
                for case let .barcode(barcode) in items {
                    guard let string = barcode.payloadStringValue,
                        let url = URL(string: string.trimmingCharacters(in: .whitespacesAndNewlines))
                    else { continue }
                    handled = true
                    onScanned(url)
                    return
                }
            }
        }
    }
#else
    /// Non-iOS / VisionKit-less builds: the scanner is never supported, so the
    /// flow always shows the manual path.
    struct QRScannerSheet: View {
        let onScanned: (URL) -> Void
        let onCancel: () -> Void

        static var isSupported: Bool { false }

        var body: some View {
            Text("QR scanning is not available on this device.")
                .padding()
        }
    }
#endif
