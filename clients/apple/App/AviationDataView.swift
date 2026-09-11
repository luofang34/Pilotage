import SwiftUI

struct AviationDataView: View {
    @ObservedObject var model: AviationDataModel

    var body: some View {
        List {
            Section {
                if model.busy {
                    ProgressView(model.status)
                    if let progress = model.progress {
                        ProgressView(value: Double(progress.received), total: Double(max(1, progress.total)))
                        Text(progress.path).font(.caption).foregroundStyle(.secondary)
                        Button("Pause download") { model.cancel() }
                    }
                } else {
                    Text(model.status).foregroundStyle(.secondary)
                }
                if let error = model.errorMessage { Text(error).foregroundStyle(.orange) }
                if model.updatesConfigured {
                    Button("Check for updates") { Task { await model.refresh() } }.disabled(model.busy)
                } else {
                    Text("Updates are not available for this development build.")
                        .font(.footnote).foregroundStyle(.secondary)
                }
            }
            Section("On this iPad") {
                if model.snapshot.installed.isEmpty {
                    Text("No data package is installed.").foregroundStyle(.secondary)
                }
                ForEach(model.snapshot.installed) { installed in
                    NavigationLink {
                        AviationReleaseView(model: model, installed: installed)
                    } label: {
                        releaseRow(installed.release, retained: model.retainedReason(installed))
                    }
                }
            }
            if !downloads.isEmpty {
                Section("Available to download") {
                    ForEach(downloads) { release in
                        VStack(alignment: .leading, spacing: 8) {
                            releaseRow(release, retained: nil)
                            Button("Download") { Task { await model.download(release) } }.disabled(model.busy)
                        }
                    }
                }
            }
        }
        .navigationTitle("Data")
        .navigationBarTitleDisplayMode(.inline)
    }

    private var downloads: [AviationRelease] {
        model.available.filter { release in !model.snapshot.installed.contains { $0.id == release.id } }
    }

    private func releaseRow(_ release: AviationRelease, retained: String?) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(release.product.title).font(.headline)
                Spacer()
                Text(release.validityLabel()).font(.caption)
                    .foregroundStyle(release.validityLabel() == "Expired" ? .orange : .secondary)
            }
            Text("\(release.authority) · \(release.edition)").font(.subheadline)
            Text(release.coverage.name).font(.caption).foregroundStyle(.secondary)
            HStack {
                if release.channel == "development" { Text("Development sample") }
                if let retained { Text(retained) }
                Spacer()
                Text(ByteCountFormatter.string(fromByteCount: release.bytes, countStyle: .file))
            }
            .font(.caption).foregroundStyle(.secondary)
        }
        .padding(.vertical, 4)
    }
}

private struct AviationReleaseView: View {
    @ObservedObject var model: AviationDataModel
    let installed: InstalledAviationRelease
    @Environment(\.dismiss) private var dismiss
    private var release: AviationRelease { installed.release }

    var body: some View {
        List {
            Section("Edition") {
                LabeledContent("Provider", value: release.authority)
                LabeledContent("Edition", value: release.edition)
                LabeledContent("State", value: release.validityLabel())
                if release.channel == "development" { Text("Development sample").foregroundStyle(.orange) }
                if let validity = release.validity {
                    LabeledContent("Effective (UTC)", value: utc(validity.effectiveAt))
                    LabeledContent("Expires (UTC)", value: utc(validity.expiresAt))
                }
            }
            Section("Coverage") {
                Text(release.coverage.name)
                if !release.coverage.complete { Text("Partial coverage").foregroundStyle(.orange) }
                ForEach(release.coverage.exclusions, id: \.self) { Text($0).font(.footnote) }
            }
            Section {
                Button("Verify files") { Task { await model.verify(installed) } }.disabled(model.busy)
                if let reason = model.retainedReason(installed) {
                    Text(reason).foregroundStyle(.secondary)
                } else {
                    Button("Remove from this iPad", role: .destructive) {
                        Task {
                            await model.remove(installed)
                            if !model.snapshot.installed.contains(where: { $0.id == installed.id }) { dismiss() }
                        }
                    }.disabled(model.busy)
                }
                if let error = model.errorMessage { Text(error).foregroundStyle(.orange) }
                if model.busy { ProgressView(model.status) } else { Text(model.status).font(.footnote) }
            }
            Section("Sources") {
                ForEach(release.attributions, id: \.self) { Text($0).font(.footnote) }
            }
        }
        .navigationTitle(release.product.title)
    }

    private func utc(_ date: Date) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = .gmt
        formatter.dateFormat = "yyyy-MM-dd HH:mm 'UTC'"
        return formatter.string(from: date)
    }
}
