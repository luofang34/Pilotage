import SwiftUI

struct NavigationSearchView: View {
    @ObservedObject var search: NavigationSearchModel
    @ObservedObject var plan: MissionPlanModel
    let replacing: String?
    var reference: NavigationPoint?
    var selection: ((RouteToken) -> Void)?
    @Environment(\.dismiss) private var dismiss
    @State private var query = ""
    @State private var results: [NavigationMatch] = []
    @State private var loading = false
    @State private var failure: String?
    @State private var coordinatePresented = false

    var body: some View {
        NavigationStack {
            List {
                if let failure = failure ?? search.errorMessage {
                    Section { Text(failure).foregroundStyle(.red) }
                }
                if query.trimmingCharacters(in: .whitespaces).isEmpty {
                    Section {
                        Label("Airports, waypoints, and navigation aids", systemImage: "magnifyingglass")
                        Text(search.status).font(.footnote).foregroundStyle(.secondary)
                        ForEach(search.sources) { installed in
                            Text("\(installed.release.authority) · \(installed.release.edition) · \(installed.release.validityLabel())")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }
                } else if results.isEmpty && !loading && failure == nil {
                    ContentUnavailableView.search(text: query)
                }
                ForEach(results) { result in
                    Button {
                        let token = RouteToken(id: UUID().uuidString, point: result.point, source: result.source)
                        if let selection { selection(token) }
                        else if let updated = RouteEntryDraft(replacingID: replacing).applying([token], to: plan.tokens) { plan.proposeRoute(updated) }
                        dismiss()
                    } label: {
                        HStack(spacing: 12) {
                            Image(systemName: symbol(result.point.kind)).frame(width: 24)
                            VStack(alignment: .leading, spacing: 4) {
                                HStack {
                                    Text(result.point.identifier).font(.headline.monospaced())
                                    Text(result.point.region).font(.caption).foregroundStyle(.secondary)
                                }
                                if !result.point.name.isEmpty { Text(result.point.name).foregroundStyle(.primary) }
                                Text("\(result.source.authority) · \(result.source.edition)")
                                    .font(.caption).foregroundStyle(.secondary)
                                if let distance = NavigationMatchOrder.distance(result.point, from: reference), let reference {
                                    Text(String(format: "%.0f NM from %@", distance, reference.identifier)).font(.caption).foregroundStyle(.secondary)
                                }
                                if search.sources.first(where: { $0.id == result.source.releaseId })?.release.channel == "development" {
                                    Text("Development data").font(.caption).foregroundStyle(.secondary)
                                }
                                if Date().timeIntervalSince1970 >= Double(result.source.expiresAt) {
                                    Text("Expired data").font(.caption).foregroundStyle(.orange)
                                }
                            }
                            Spacer()
                            Image(systemName: "plus.circle").accessibilityHidden(true)
                        }
                    }
                    .accessibilityLabel("Add \(result.point.identifier), \(result.point.name), \(result.point.region), \(result.source.authority)")
                }
            }
            .navigationTitle(replacing == nil ? "Add waypoint" : "Replace waypoint")
            .navigationBarTitleDisplayMode(.inline)
            .searchable(text: $query, prompt: "Airport, waypoint, or name")
            .autocorrectionDisabled()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Coordinate", systemImage: "mappin.and.ellipse") { coordinatePresented = true }
                }
            }
            .overlay(alignment: .bottom) { if loading { ProgressView().padding().glassEffect() } }
            .task(id: query + ":" + String(search.revision)) { await updateResults() }
            .sheet(isPresented: $coordinatePresented) {
                CoordinateWaypointView { point in
                    if let selection { selection(point) }
                    else if let updated = RouteEntryDraft(replacingID: replacing).applying([point], to: plan.tokens) { plan.proposeRoute(updated) }
                    coordinatePresented = false
                    dismiss()
                }
            }
        }
    }

    private func updateResults() async {
        loading = true
        failure = nil
        do {
            let matches = try await search.search(query)
            guard !Task.isCancelled else { return }
            results = NavigationMatchOrder.sorted(matches, query: query, near: reference)
        } catch {
            guard !Task.isCancelled else { return }
            results = []
            failure = error.localizedDescription
        }
        loading = false
    }

    private func symbol(_ kind: String) -> String {
        switch kind {
        case "airport": "airplane.circle"
        case "navaid": "dot.radiowaves.left.and.right"
        default: "mappin"
        }
    }
}

private struct CoordinateWaypointView: View {
    let add: (RouteToken) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var latitude = ""
    @State private var longitude = ""

    private var point: RouteToken? {
        guard let lat = Double(latitude), let lon = Double(longitude),
              (-90...90).contains(lat), (-180...180).contains(lon) else { return nil }
        let id = UUID().uuidString
        let identifier = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard identifier.count <= 64 else { return nil }
        return RouteToken(id: id, point: NavigationPoint(key: id,
            identifier: identifier.isEmpty ? String(format: "%.4f, %.4f", lat, lon) : identifier,
            kind: "waypoint", name: "", region: "", latitudeDeg: lat, longitudeDeg: lon), source: nil)
    }

    var body: some View {
        NavigationStack {
            Form {
                TextField("Name (optional)", text: $name)
                TextField("Latitude (−90 to 90)", text: $latitude).keyboardType(.numbersAndPunctuation)
                TextField("Longitude (−180 to 180)", text: $longitude).keyboardType(.numbersAndPunctuation)
                Text("Use WGS 84 decimal degrees. South and west values are negative.")
                    .font(.footnote).foregroundStyle(.secondary)
            }
            .navigationTitle("Coordinate waypoint")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Add") { if let point { add(point) } }.disabled(point == nil)
                }
            }
        }
    }
}
