import SwiftUI

struct RouteUpdateProposal: Identifiable {
    let id = UUID()
    let assignmentID: String
    let before: [RouteToken]
    let after: [RouteToken]
    var destinationID: String?
}

struct RouteUpdateReview: View {
    @ObservedObject var plan: MissionPlanModel
    let proposal: RouteUpdateProposal
    @State private var destination = ""
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        NavigationStack {
            Form {
                Section("Current route") { Text(proposal.before.map(\.label).joined(separator: " → ")).font(.body.monospaced()) }
                Section("Updated route") { Text(proposal.after.map(\.label).joined(separator: " → ")).font(.body.monospaced()) }
                Section {
                    Picker("Continue toward", selection: $destination) {
                        Text("Stop local navigation").tag("")
                        ForEach(Array(proposal.after.dropFirst().enumerated()), id: \.element.id) { index, token in
                            Text("\(index + 1). \(token.label)").tag(token.id)
                        }
                    }
                } footer: { Text("The current route stays active until you apply this update.") }
                if let error = plan.errorMessage { Text(error).foregroundStyle(.red) }
            }
            .navigationTitle("Review route update").navigationBarTitleDisplayMode(.inline)
            .onAppear {
                if let id = proposal.destinationID, proposal.after.dropFirst().contains(where: { $0.id == id }) { destination = id }
            }
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { plan.routeUpdate = nil; dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Apply update") { plan.applyRoute(proposal, destinationID: destination.isEmpty ? nil : destination) }
                }
            }
        }
    }
}
