import SwiftUI

struct MissionParticipantEditor: View {
    @ObservedObject var plan: MissionPlanModel
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var organization = ""

    var body: some View {
        NavigationStack {
            Form {
                TextField("Participant name", text: $name)
                TextField("Organization (optional)", text: $organization)
            }
            .navigationTitle("Add participant")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Add") {
                        let person = name.trimmingCharacters(in: .whitespacesAndNewlines)
                        let org = organization.trimmingCharacters(in: .whitespacesAndNewlines)
                        plan.change { $0.participants.append(MissionParticipant(id: UUID().uuidString, name: person,
                            organization: org.isEmpty ? nil : org)) }
                        dismiss()
                    }.disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
    }
}

struct MissionAssignmentView: View {
    @ObservedObject var plan: MissionPlanModel
    let assignmentId: String

    private var assignment: MissionAssignment? { plan.draft.assignments.first { $0.id == assignmentId } }

    var body: some View {
        Form {
            if let assignment {
                TextField("Assignment name", text: Binding(get: { assignment.name }, set: { value in update { $0.name = value } }))
                Picker("Responsible participant", selection: Binding(get: { assignment.participantId }, set: { value in
                    update { $0.participantId = value }
                })) {
                    ForEach(plan.draft.participants) { Text($0.name).tag($0.id) }
                }
                TextField("Vehicle reference (optional)", text: Binding(get: { assignment.vehicleReference ?? "" }, set: { value in
                    update { $0.vehicleReference = value.isEmpty ? nil : value }
                }))
                Button("Select this route") { plan.selectedAssignmentId = assignmentId }
                Text("A vehicle reference identifies the planned vehicle. A live connection must be assigned separately.")
                    .font(.footnote).foregroundStyle(.secondary)
            }
            if let error = plan.errorMessage { Text(error).foregroundStyle(.red) }
        }
        .navigationTitle(assignment?.name ?? "Assignment")
    }

    private func update(_ change: (inout MissionAssignment) -> Void) {
        plan.change { draft in
            guard let index = draft.assignments.firstIndex(where: { $0.id == assignmentId }) else { return }
            change(&draft.assignments[index])
        }
    }
}

struct MissionSwarmEditor: View {
    @ObservedObject var plan: MissionPlanModel
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var members = Set<String>()

    var body: some View {
        NavigationStack {
            Form {
                TextField("Swarm name", text: $name)
                ForEach(plan.draft.assignments) { assignment in
                    Toggle(assignment.name, isOn: Binding(get: { members.contains(assignment.id) }, set: { value in
                        if value { members.insert(assignment.id) } else { members.remove(assignment.id) }
                    }))
                }
            }
            .navigationTitle("Add swarm")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Add") {
                        plan.change { $0.swarms.append(MissionSwarm(id: UUID().uuidString,
                            name: name.trimmingCharacters(in: .whitespacesAndNewlines), assignmentIds: members.sorted())) }
                        dismiss()
                    }.disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || members.isEmpty)
                }
            }
        }
    }
}
