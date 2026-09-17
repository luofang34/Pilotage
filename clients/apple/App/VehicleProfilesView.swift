import SwiftUI
import UniformTypeIdentifiers

struct VehicleProfilesView: View {
    @ObservedObject var library: VehicleProfileLibrary
    let selected: VehicleProfile?
    let select: (VehicleProfile) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var editing: VehicleProfile?
    @State private var creating = false
    @State private var importing = false
    @State private var imported: VehicleProfileDocument?
    @State private var exporting = false
    @State private var exportFile = MissionReviewFile()
    @State private var failure: String?

    var body: some View {
        NavigationStack {
            List {
                if let message = failure ?? library.errorMessage { Text(message).foregroundStyle(.red) }
                if let selected, !library.profiles.contains(where: { $0 == selected }) {
                    Section("Route profile") {
                        profileLabel(selected)
                        Text("This route retains its selected profile. Select a library profile to update the route.")
                            .font(.footnote).foregroundStyle(.secondary)
                    }
                }
                Section {
                    ForEach(library.profiles) { profile in
                        Button { select(profile); dismiss() } label: {
                            HStack {
                                profileLabel(profile)
                                Spacer()
                                if selected == profile { Image(systemName: "checkmark") }
                            }
                        }
                        .contextMenu {
                            Button("Edit profile", systemImage: "pencil") { editing = profile }
                            Button("Remove from library", systemImage: "trash", role: .destructive) {
                                Task { do { try await library.remove(profile.id) } catch { failure = error.localizedDescription } }
                            }
                        }
                    }
                    Button("New vehicle profile", systemImage: "plus") { creating = true }
                    Button("Import profiles…", systemImage: "square.and.arrow.down") { importing = true }
                    Button("Export profiles…", systemImage: "square.and.arrow.up") {
                        do {
                            exportFile = MissionReviewFile(json: try PlanningCodec.encode(VehicleProfileDocument(profiles: library.profiles)))
                            exporting = true
                        } catch { failure = error.localizedDescription }
                    }.disabled(library.profiles.isEmpty)
                } footer: { Text("Profiles are saved on this device. Import and export use Pilotage vehicle profile JSON.") }
            }
            .navigationTitle("Vehicles")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } } }
            .sheet(isPresented: $creating) { VehicleProfileEditor(library: library) { profile in select(profile); dismiss() } }
            .sheet(item: $editing) { profile in VehicleProfileEditor(library: library, original: profile) { _ in } }
            .sheet(isPresented: Binding(get: { imported != nil }, set: { if !$0 { imported = nil } })) {
                if let imported { importReview(imported) }
            }
            .fileImporter(isPresented: $importing, allowedContentTypes: [.json]) { result in
                Task {
                    do {
                        let url = try result.get()
                        let access = url.startAccessingSecurityScopedResource()
                        defer { if access { url.stopAccessingSecurityScopedResource() } }
                        imported = try await Task.detached { try VehicleProfileStore.read(url) }.value
                    } catch { failure = error.localizedDescription }
                }
            }
            .fileExporter(isPresented: $exporting, document: exportFile, contentType: .json,
                defaultFilename: "Pilotage-vehicles.json") { result in
                    if case .failure(let error) = result { failure = error.localizedDescription }
                }
        }
    }

    private func profileLabel(_ profile: VehicleProfile) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(profile.title).foregroundStyle(.primary)
            Text(String(format: "%.0f %@", profile.cruiseSpeedKnots, profile.speedLabel))
                .font(.caption).foregroundStyle(.secondary)
        }
    }

    private func importReview(_ document: VehicleProfileDocument) -> some View {
        NavigationStack {
            List {
                ForEach(document.profiles) { profileLabel($0) }
                if let failure { Text(failure).foregroundStyle(.red) }
            }
            .navigationTitle("Import \(document.profiles.count) profiles")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { imported = nil } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Import") {
                        Task {
                            do { try await library.importProfiles(document); imported = nil; failure = nil }
                            catch { failure = error.localizedDescription }
                        }
                    }.disabled(library.busy || document.profiles.isEmpty)
                }
            }
        }
    }
}

struct VehicleProfileEditor: View {
    @ObservedObject var library: VehicleProfileLibrary
    var original: VehicleProfile?
    let saved: (VehicleProfile) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var registration = ""
    @State private var kind = "aircraft"
    @State private var speedReference = "true_airspeed"
    @State private var speedText = ""
    @State private var source = ""
    @State private var failure: String?

    private var candidate: VehicleProfile? {
        let value = speedText.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: Locale.current.decimalSeparator ?? ".", with: ".")
        guard let speed = Double(value), speed.isFinite else { return nil }
        return VehicleProfile(id: original?.id ?? UUID().uuidString, revision: original?.revision ?? 1,
            name: name.trimmingCharacters(in: .whitespacesAndNewlines), registration: optional(registration), kind: kind,
            cruiseSpeedKnots: speed, speedReference: speedReference,
            source: optional(source))
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Profile name", text: $name)
                    TextField("Registration or vehicle ID", text: $registration).textInputAutocapitalization(.characters)
                    Picker("Vehicle type", selection: Binding(get: { kind }, set: { value in
                        kind = value
                        speedReference = ["ground", "marine"].contains(value) ? "ground_speed" : "true_airspeed"
                    })) {
                        ForEach(["aircraft", "rotorcraft", "drone", "ground", "marine"], id: \.self) { Text($0.capitalized).tag($0) }
                    }
                }
                Section {
                    Picker("Speed reference", selection: $speedReference) {
                        Text("True airspeed").tag("true_airspeed")
                        Text("Ground speed").tag("ground_speed")
                    }
                    TextField(speedReference == "ground_speed" ? "Cruise speed (kt)" : "Cruise true airspeed (kt)",
                        text: $speedText).keyboardType(.decimalPad)
                    TextField("Performance source (optional)", text: $source)
                } header: { Text("Cruise performance") } footer: {
                    Text("Enter the cruise value for your vehicle and operating condition. Aircraft time assumes still air. Climb, descent, and fuel are not calculated.")
                }
                if let failure { Text(failure).foregroundStyle(.red) }
            }
            .navigationTitle(original == nil ? "New vehicle" : "Edit vehicle")
            .navigationBarTitleDisplayMode(.inline)
            .onAppear {
                if let original { name = original.name; registration = original.registration ?? ""; kind = original.kind
                    speedText = original.cruiseSpeedKnots.formatted(.number.grouping(.never))
                    speedReference = original.speedReference; source = original.source ?? "" }
            }
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        guard let candidate else { return }
                        Task {
                            do {
                                try await library.save(candidate)
                                if let stored = library.profiles.first(where: { $0.id == candidate.id }) { saved(stored) }
                                dismiss()
                            } catch { failure = error.localizedDescription }
                        }
                    }.disabled(candidate == nil || name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || library.busy)
                }
            }
        }
    }

    private func optional(_ value: String) -> String? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}
