import SwiftUI
import VisionKit

/// The camera finding the Invite's QR code (ADR 0040), through the platform's own scanner.
struct CodeScanner: UIViewControllerRepresentable {
    let found: (String) -> Void

    /// Whether this device can scan at all; the simulator cannot.
    static var available: Bool {
        DataScannerViewController.isSupported && DataScannerViewController.isAvailable
    }

    /// Whether a scanned code is an Invite link worth trying.
    static func isInvite(_ text: String) -> Bool {
        text.hasPrefix("p2p://") || text.hasPrefix("tcp://")
    }

    func makeUIViewController(context: Context) -> DataScannerViewController {
        let scanner = DataScannerViewController(
            recognizedDataTypes: [.barcode(symbologies: [.qr])], qualityLevel: .balanced,
            recognizesMultipleItems: false, isHighFrameRateTrackingEnabled: false,
            isHighlightingEnabled: true)
        scanner.delegate = context.coordinator
        // Scanning starts once the controller is on screen, a tick after it is made.
        Task { @MainActor in try? scanner.startScanning() }
        return scanner
    }

    func updateUIViewController(_ scanner: DataScannerViewController, context: Context) {}

    static func dismantleUIViewController(
        _ scanner: DataScannerViewController, coordinator: Coordinator
    ) {
        scanner.stopScanning()
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(found: found)
    }

    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        let found: (String) -> Void
        private var done = false

        init(found: @escaping (String) -> Void) {
            self.found = found
        }

        func dataScanner(
            _ dataScanner: DataScannerViewController, didAdd addedItems: [RecognizedItem],
            allItems: [RecognizedItem]
        ) {
            guard !done else { return }
            for case .barcode(let code) in addedItems {
                guard let link = code.payloadStringValue, CodeScanner.isInvite(link) else {
                    continue
                }
                done = true
                dataScanner.stopScanning()
                found(link)
                return
            }
        }
    }
}
