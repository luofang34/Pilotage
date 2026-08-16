import IndicateAppleDisplay
import PilotageCore
import SwiftUI

/// The Instruments destination: a host connection, the registry's panels,
/// and control state (ADR-0037's Instruments and Control modules).
struct InstrumentsView: View {
    @ObservedObject var model: HostLinkModel
    @AppStorage("pilotageHostUrl") private var hostUrl = "https://192.168.1.1:4433/pilotage"
    @AppStorage("pilotageHostCertHash") private var certificateHash = ""
    @AppStorage("pilotageManifestUrl") private var manifestUrl = "http://192.168.1.1:8080/session.json"

    var body: some View {
        VStack(spacing: 12) {
            connectBar
            if let fault = model.instrumentFault {
                // The gate refused or a write failed: the reason shows,
                // an unverified instrument never does.
                Label(fault, systemImage: "exclamationmark.triangle")
                    .font(.footnote)
                    .foregroundStyle(.red)
            }
            panelArea
            controlBar
        }
        .padding()
        .navigationTitle("Instruments")
        .onAppear {
            model.prepareInstruments()
            if LaunchRequest.openInstruments {
                model.connect(url: hostUrl, certificateSha256Hex: certificateHash)
            }
        }
    }

    private var connectBar: some View {
        VStack(spacing: 6) {
            HStack(spacing: 8) {
                TextField("http://host:8080/session.json", text: $manifestUrl)
                    .textFieldStyle(.roundedBorder)
                    .autocorrectionDisabled()
                    .textInputAutocapitalization(.never)
                Button("Fetch and connect") { model.connectFromManifest(manifestUrl) }
            }
            manualConnectRow
        }
    }

    private var manualConnectRow: some View {
        HStack(spacing: 8) {
            TextField("https://host:4433/pilotage", text: $hostUrl)
                .textFieldStyle(.roundedBorder)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
            TextField("certificate sha-256 (empty: dev)", text: $certificateHash)
                .textFieldStyle(.roundedBorder)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
            Button("Connect") {
                model.connect(url: hostUrl, certificateSha256Hex: certificateHash)
            }
            Button("Disconnect") { model.disconnect() }
            Text(model.status)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
    }

    @ViewBuilder
    private var panelArea: some View {
        Picker("Panel", selection: $model.selectedPanel) {
            ForEach(model.panels) { choice in
                Text(choice.descriptor.title).tag(choice.index)
            }
        }
        .pickerStyle(.segmented)
        if let display = model.display(for: model.selectedPanel) {
            InstrumentPanel(display: display)
                .aspectRatio(panelAspect, contentMode: .fit)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            ContentUnavailableView(
                "No verified instrument runtime",
                systemImage: "gauge",
                description: Text("The compatibility gate must pass before anything paints.")
            )
        }
    }

    private var panelAspect: CGFloat {
        guard let choice = model.panels.first(where: { $0.index == model.selectedPanel }),
              choice.descriptor.designHeight > 0
        else { return 4.0 / 3.0 }
        return CGFloat(choice.descriptor.designWidth) / CGFloat(choice.descriptor.designHeight)
    }

    private var controlBar: some View {
        HStack(spacing: 12) {
            Label(
                model.controllerAttached ? "controller attached" : "no controller",
                systemImage: model.controllerAttached
                    ? "gamecontroller.fill"
                    : "gamecontroller"
            )
            .foregroundStyle(model.controllerAttached ? .primary : .secondary)
            Spacer()
            if model.leaseHeld {
                Button("Release control", role: .destructive) { model.releaseLease() }
            } else {
                Button("Request control") { model.requestLease() }
                    .disabled(model.catalog == nil)
            }
        }
        .font(.callout)
    }
}
