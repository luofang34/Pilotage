import PilotageSituationCore
import SwiftUI

struct TrafficDetailView: View {
    let detail: DisplayTrafficDetail

    var body: some View {
        NavigationStack {
            List {
                Section("Identity") {
                    LabeledContent("Primary", value: detail.primaryIdentity)
                    ForEach(detail.otherIdentities, id: \.self) { identity in
                        LabeledContent("Associated", value: identity)
                    }
                    if let reason = detail.otherIdentitiesAbsenceReason {
                        Text(reason)
                            .foregroundStyle(.secondary)
                    }
                }
                Section("Lifecycle") {
                    LabeledContent("State", value: detail.lifecycle)
                    LabeledContent("Newest observation", value: detail.newestObservationAge)
                }
                Section("Fields") {
                    ForEach(detail.fields, id: \.id) { field in
                        TrafficDetailFieldView(field: field)
                    }
                }
            }
            .navigationTitle(detail.title)
            .navigationBarTitleDisplayMode(.inline)
        }
    }
}

private struct TrafficDetailFieldView: View {
    let field: DisplayTrafficDetailField

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(field.title)
                .font(.headline)
            if let value = field.value {
                Text(value)
            }
            if let reason = field.absenceReason {
                Text(reason)
                    .foregroundStyle(.secondary)
            }
            if let age = field.age {
                Text(age)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            if let source = field.source {
                Text(source)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 2)
    }
}
