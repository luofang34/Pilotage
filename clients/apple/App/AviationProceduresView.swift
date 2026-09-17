import PDFKit
import SwiftUI

struct AviationProceduresView: View {
    @ObservedObject var model: AviationDataModel
    let installed: InstalledAviationRelease
    @State private var catalog: AviationProcedureCatalog?
    @State private var search = ""

    var body: some View {
        List {
            Section {
                Text("\(installed.release.edition) · \(installed.release.validityLabel())")
                Text(installed.release.coverage.name).font(.footnote)
                if installed.release.channel == "development" {
                    Text("Development sample · Partial coverage").foregroundStyle(.orange)
                }
            }
            if let catalog {
                ForEach(catalog.charts.filter {
                    search.isEmpty || "\($0.airport) \($0.name)".localizedCaseInsensitiveContains(search)
                }) { chart in
                    NavigationLink {
                        AviationProcedurePDFView(catalog: catalog, chart: chart)
                    } label: {
                        VStack(alignment: .leading) {
                            Text(chart.name)
                            Text("\(chart.airport) · \(chart.chartCode)").font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
            } else if model.busy {
                ProgressView("Opening installed procedures…")
            } else if let error = model.errorMessage {
                Text(error).foregroundStyle(.orange)
                Button("Try again") { Task { catalog = await model.openProcedures(installed) } }
            }
        }
        .navigationTitle("Procedures")
        .searchable(text: $search, prompt: "Airport or procedure")
        .task { if catalog == nil { catalog = await model.openProcedures(installed) } }
    }
}

struct AviationProcedurePDFView: View {
    let catalog: AviationProcedureCatalog
    let chart: AviationProcedureChart
    @State private var document: PDFDocument?
    @State private var loaded = false

    var body: some View {
        VStack(spacing: 0) {
            Text("\(chart.airport) · \(catalog.installed.release.edition) · \(catalog.installed.release.validityLabel())")
                .font(.caption).padding(6)
            if let document {
                InstalledPDF(document: document)
            } else if loaded {
                ContentUnavailableView("The chart is unavailable", systemImage: "doc")
            } else {
                ProgressView("Opening chart…")
            }
        }
        .navigationTitle(chart.name)
        .navigationBarTitleDisplayMode(.inline)
        .task {
            if let url = try? catalog.url(for: chart) { document = PDFDocument(url: url) }
            loaded = true
        }
    }
}

private struct InstalledPDF: UIViewRepresentable {
    let document: PDFDocument

    func makeUIView(context: Context) -> PDFView {
        let view = PDFView()
        view.autoScales = true
        view.displayMode = .singlePageContinuous
        view.document = document
        return view
    }

    func updateUIView(_ view: PDFView, context: Context) {}
}
