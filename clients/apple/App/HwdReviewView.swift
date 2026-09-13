import SwiftUI
import WebKit

struct HwdReviewView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var exportedImage: HwdReviewExport?

    var body: some View {
        NavigationStack {
            Group {
                if let page = Bundle.main.url(forResource: "hwd-gallery", withExtension: "html",
                                              subdirectory: "HwdReview") {
                    HwdReviewPage(page: page) { exportedImage = HwdReviewExport(url: $0) }
                } else {
                    ContentUnavailableView("Review unavailable", systemImage: "airplane",
                        description: Text("This build does not include the Indicate review cases."))
                }
            }
            .sheet(item: $exportedImage) { HwdReviewShare(url: $0.url) }
            .navigationTitle("HWD instrument review")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }
}

private struct HwdReviewPage: UIViewRepresentable {
    let page: URL
    let onExport: (URL) -> Void

    func makeUIView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        let view = WKWebView(frame: .zero, configuration: configuration)
        view.navigationDelegate = context.coordinator
        view.loadFileURL(page, allowingReadAccessTo: page.deletingLastPathComponent())
        return view
    }

    func updateUIView(_ view: WKWebView, context: Context) {}

    static func dismantleUIView(_ view: WKWebView, coordinator: Coordinator) {
        view.stopLoading()
        view.navigationDelegate = nil
    }

    func makeCoordinator() -> Coordinator { Coordinator(onExport: onExport) }

    @MainActor
    final class Coordinator: NSObject, WKNavigationDelegate, WKDownloadDelegate {
        private let onExport: (URL) -> Void
        private var destinations: [ObjectIdentifier: URL] = [:]

        init(onExport: @escaping (URL) -> Void) { self.onExport = onExport }

        func webView(_ webView: WKWebView, navigationAction: WKNavigationAction,
                     didBecome download: WKDownload) {
            download.delegate = self
        }

        func download(_ download: WKDownload, decideDestinationUsing response: URLResponse,
                      suggestedFilename: String, completionHandler: @escaping @MainActor @Sendable (URL?) -> Void) {
            let url = FileManager.default.temporaryDirectory
                .appendingPathComponent("hwd-review-\(UUID().uuidString).png")
            destinations[ObjectIdentifier(download)] = url
            completionHandler(url)
        }

        func downloadDidFinish(_ download: WKDownload) {
            if let url = destinations.removeValue(forKey: ObjectIdentifier(download)) { onExport(url) }
        }

        func download(_ download: WKDownload, didFailWithError error: Error, resumeData: Data?) {
            if let url = destinations.removeValue(forKey: ObjectIdentifier(download)) {
                try? FileManager.default.removeItem(at: url)
            }
        }

        func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
                     decisionHandler: @escaping @MainActor @Sendable (WKNavigationActionPolicy) -> Void) {
            guard let url = action.request.url else {
                decisionHandler(.cancel)
                return
            }
            if action.shouldPerformDownload, url.scheme == "data" {
                decisionHandler(.download)
                return
            }
            if url.isFileURL || url.scheme == "about" || url.scheme == "data" {
                decisionHandler(.allow)
            } else {
                decisionHandler(.cancel)
                if action.navigationType == .linkActivated, url.scheme == "https" {
                    UIApplication.shared.open(url)
                }
            }
        }
    }
}

private struct HwdReviewExport: Identifiable {
    let id = UUID()
    let url: URL
}

private struct HwdReviewShare: UIViewControllerRepresentable {
    let url: URL

    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: [url], applicationActivities: nil)
    }

    func updateUIViewController(_ controller: UIActivityViewController, context: Context) {}
}
