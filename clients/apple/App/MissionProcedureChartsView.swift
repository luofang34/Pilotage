import SwiftUI

struct MissionProcedureChartsView: View {
    @ObservedObject var model: AviationDataModel
    let airports: Set<String>
    @Environment(\.dismiss) private var dismiss
    @State private var catalogs: [AviationProcedureCatalog] = []
    @State private var search = ""
    @State private var loading = true

    var body: some View {
        NavigationStack {
            List {
                if loading { ProgressView("Opening installed charts…") }
                if let error = model.errorMessage {
                    Text(error).foregroundStyle(.orange)
                    Button("Try again") { Task { await load() } }.disabled(model.busy)
                }
                chartSection("On this route", onRoute: true)
                chartSection("Other airports", onRoute: false)
                if !loading && catalogs.isEmpty {
                    ContentUnavailableView("No procedure charts installed", systemImage: "doc.text",
                        description: Text("Open Aviation Data to download procedure charts."))
                }
            }
            .navigationTitle("Procedure charts").navigationBarTitleDisplayMode(.inline)
            .searchable(text: $search, prompt: "Airport or procedure")
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
            .task { await load() }
        }
    }

    @ViewBuilder private func chartSection(_ title: String, onRoute: Bool) -> some View {
        let entries = catalogs.flatMap { catalog in
            catalog.charts.filter { chart in
                airports.contains(chart.airport) == onRoute && (search.isEmpty ||
                    "\(chart.airport) \(chart.name)".localizedCaseInsensitiveContains(search))
            }.map { (catalog, $0) }
        }
        if !entries.isEmpty {
            Section(title) {
                ForEach(entries, id: \.1.id) { catalog, chart in
                    NavigationLink {
                        AviationProcedurePDFView(catalog: catalog, chart: chart)
                    } label: {
                        VStack(alignment: .leading, spacing: 4) {
                            Text("\(chart.airport) · \(chart.name)")
                            Text("\(catalog.installed.release.edition) · \(catalog.installed.release.validityLabel())")
                                .font(.caption).foregroundStyle(.secondary)
                            if !catalog.installed.release.coverage.complete {
                                Text("Partial coverage").font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }.accessibilityIdentifier("procedure-chart-" + chart.id)
                }
            }
        }
    }

    private func load() async {
        loading = true
        var opened: [AviationProcedureCatalog] = []
        var chartIDs = Set<String>()
        for installed in AviationProcedureCatalog.preferredReleases(model.snapshot, at: Date()) {
            if let catalog = await model.openProcedures(installed) {
                let charts = catalog.charts.filter { chartIDs.insert($0.id).inserted }
                if !charts.isEmpty { opened.append(AviationProcedureCatalog(installed: installed, charts: charts)) }
            }
        }
        catalogs = opened
        loading = false
    }
}
