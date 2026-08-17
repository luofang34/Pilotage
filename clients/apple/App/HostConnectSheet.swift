import SwiftUI

/// The connection flow, in the order an operator actually has it: the
/// session on the computer printed a manifest address — paste that one
/// line and go. The manual facts stay available for a host that serves
/// no manifest, behind a disclosure rather than in the operator's way.
struct HostConnectSheet: View {
    @ObservedObject var model: HostLinkModel
    @Environment(\.dismiss) private var dismiss
    @AppStorage("pilotageManifestUrl") private var manifestUrl = "http://192.168.1.224:8080/session.json"
    @AppStorage("pilotageHostUrl") private var hostUrl = "https://192.168.1.224:4433/pilotage"
    @AppStorage("pilotageHostCertHash") private var certificateHash = ""
    @State private var manualExpanded = false

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("http://computer:8080/session.json", text: $manifestUrl)
                        .keyboardType(.URL)
                        .autocorrectionDisabled()
                        .textInputAutocapitalization(.never)
                    Button {
                        model.connectFromManifest(manifestUrl)
                    } label: {
                        Label("Fetch and connect", systemImage: "antenna.radiowaves.left.and.right")
                    }
                } header: {
                    Text("Session manifest")
                } footer: {
                    Text(
                        "Run `cargo xtask sim --lan` on the computer. The ready banner "
                            + "prints this address; it carries the host, the port, and the "
                            + "certificate to pin, so nothing is typed by hand."
                    )
                }

                Section {
                    DisclosureGroup("Manual connection", isExpanded: $manualExpanded) {
                        TextField("https://computer:4433/pilotage", text: $hostUrl)
                            .keyboardType(.URL)
                            .autocorrectionDisabled()
                            .textInputAutocapitalization(.never)
                        TextField("certificate sha-256 (empty: trust anything, dev only)",
                                  text: $certificateHash)
                            .autocorrectionDisabled()
                            .textInputAutocapitalization(.never)
                            .font(.footnote.monospaced())
                        Button("Connect") {
                            model.connect(url: hostUrl, certificateSha256Hex: certificateHash)
                        }
                    }
                } footer: {
                    Text("For a host without a manifest. An empty certificate hash "
                        + "accepts any certificate and belongs on a bench, not in the air.")
                }

                Section("Status") {
                    HStack {
                        ConnectionChip(phase: model.phase) {}
                            .allowsHitTesting(false)
                        Spacer()
                        if case .idle = model.phase {} else {
                            Button("Disconnect", role: .destructive) { model.disconnect() }
                        }
                    }
                    Text(model.status)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                    if !model.padHints.isEmpty {
                        Text(model.padHints)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .navigationTitle("Connect to a session")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
            .onChange(of: model.phase) { _, phase in
                // Admission is the flow's end; the sheet leaves on success
                // instead of asking to be dismissed.
                if case .observing = phase { dismiss() }
            }
        }
    }
}
