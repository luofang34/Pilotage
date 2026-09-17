import SwiftUI

struct MissionTimingEditor: View {
    @ObservedObject var plan: MissionPlanModel
    @Environment(\.dismiss) private var dismiss
    @State private var name = "Arrival"
    @State private var scope = "individual"
    @State private var assignmentId = ""
    @State private var swarmId = ""
    @State private var target = Date(timeIntervalSince1970: floor(Date().timeIntervalSince1970 / 60) * 60)
    @State private var tolerance: UInt32 = 30
    @State private var waypoints: [String: String] = [:]
    @State private var offsets: [String: Int32] = [:]

    private var memberIds: [String] {
        if scope == "swarm" { return plan.draft.swarms.first { $0.id == swarmId }?.assignmentIds ?? [] }
        return assignmentId.isEmpty ? [] : [assignmentId]
    }

    private var complete: Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !memberIds.isEmpty
            && memberIds.allSatisfy { id in
                plan.draft.assignments.first { $0.id == id }?.route.waypoints.contains { $0.id == waypoints[id] } == true
            }
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Target name", text: $name)
                    Picker("Applies to", selection: $scope) {
                        Text("Individual vehicle").tag("individual")
                        Text("Swarm").tag("swarm")
                    }
                    if scope == "individual" {
                        Picker("Assignment", selection: $assignmentId) {
                            ForEach(plan.draft.assignments) { Text($0.name).tag($0.id) }
                        }
                    } else {
                        Picker("Swarm", selection: $swarmId) {
                            ForEach(plan.draft.swarms) { Text($0.name).tag($0.id) }
                        }
                        if plan.draft.swarms.isEmpty { Text("Add a swarm in Team & Timing first.").foregroundStyle(.secondary) }
                    }
                }
                Section {
                    DatePicker("Time on target (UTC)", selection: $target)
                        .environment(\.timeZone, .gmt)
                    Stepper("Tolerance ±\(tolerance) seconds", value: $tolerance, in: 0...3600, step: 5)
                } footer: { Text("The tolerance applies before and after the target time.") }
                ForEach(memberIds, id: \.self) { id in
                    if let assignment = plan.draft.assignments.first(where: { $0.id == id }) {
                        Section(assignment.name) {
                            if assignment.route.waypoints.isEmpty {
                                Text("Add a route waypoint for this member first.").foregroundStyle(.secondary)
                            } else {
                                Picker("Target waypoint", selection: Binding(get: { waypoints[id] ?? "" }, set: { waypoints[id] = $0 })) {
                                    ForEach(assignment.route.waypoints) { Text($0.label).tag($0.id) }
                                }
                                Stepper("Offset \(offsets[id] ?? 0) seconds", value: Binding(get: { offsets[id] ?? 0 }, set: { offsets[id] = $0 }),
                                    in: -3600...3600, step: 5)
                            }
                        }
                    }
                }
                Section {
                    Text("A positive offset places a member after the common time. Ground speed and departure time determine the arrival estimate.")
                        .font(.footnote).foregroundStyle(.secondary)
                }
                if let error = plan.errorMessage { Text(error).foregroundStyle(.red) }
            }
            .navigationTitle("Time on target")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Add") { save() }.disabled(!complete)
                }
            }
            .onAppear {
                assignmentId = plan.assignment?.id ?? ""
                swarmId = plan.draft.swarms.first?.id ?? ""
                for assignment in plan.draft.assignments {
                    waypoints[assignment.id] = assignment.route.waypoints.last?.id
                }
            }
        }
    }

    private func save() {
        let members = memberIds.compactMap { id -> MissionTimingMember? in
            guard let waypoint = waypoints[id] else { return nil }
            return MissionTimingMember(assignmentId: id, waypointId: waypoint, offsetSeconds: offsets[id] ?? 0)
        }
        let condition = MissionTimingTarget(id: UUID().uuidString, name: name.trimmingCharacters(in: .whitespacesAndNewlines),
            utc: Int64(target.timeIntervalSince1970), toleranceSeconds: tolerance,
            swarmId: scope == "swarm" ? swarmId : nil, members: members)
        let revision = plan.draft.revision
        plan.change { $0.timingTargets.append(condition) }
        if plan.draft.revision != revision { dismiss() }
    }
}
