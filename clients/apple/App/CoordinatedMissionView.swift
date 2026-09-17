import SwiftUI
import UniformTypeIdentifiers
import PilotageCore

struct CoordinatedMissionView: View {
    @ObservedObject var plan: MissionPlanModel
    @State private var addingParticipant = false
    @State private var addingSwarm = false
    @State private var addingTarget = false
    @State private var exporting = false
    @State private var document = MissionReviewFile()
    @State private var exportError: String?

    var body: some View {
        List {
            Section {
                TextField("Mission name", text: Binding(get: { plan.draft.title }, set: { title in plan.change { $0.title = title } }))
            }
            Section {
                ForEach(plan.draft.assignments) { assignment in
                    NavigationLink {
                        MissionAssignmentView(plan: plan, assignmentId: assignment.id)
                    } label: {
                        VStack(alignment: .leading, spacing: 4) {
                            HStack {
                                Image(systemName: "airplane")
                                Text(assignment.name).font(.headline)
                                if assignment.id == plan.assignment?.id { Text("Selected").font(.caption).foregroundStyle(.secondary) }
                            }
                            Text(participantName(assignment.participantId)).font(.subheadline).foregroundStyle(.secondary)
                            Text("\(assignment.route.waypoints.count) \(assignment.route.waypoints.count == 1 ? "waypoint" : "waypoints")").font(.caption)
                        }
                    }
                    .contextMenu {
                        Button("Edit this route") { plan.selectedAssignmentId = assignment.id }
                        Button("Copy assignment") { copy(assignment) }
                        Button("Remove", role: .destructive) { plan.change { $0.assignments.removeAll { $0.id == assignment.id } } }
                            .disabled(plan.draft.assignments.count <= 1)
                    }
                }
                Button("Add vehicle assignment", systemImage: "plus") {
                    guard let participant = plan.draft.participants.first else { return }
                    let id = UUID().uuidString
                    plan.change { $0.assignments.append(MissionAssignment(id: id,
                        name: "Vehicle \($0.assignments.count + 1)", participantId: participant.id, route: PlannedRoute())) }
                    plan.selectedAssignmentId = id
                }
            } header: { Text("Vehicle assignments") } footer: {
                Text("Each assignment has its own route and planning values. Select an assignment to edit its route.")
            }
            Section("Participants") {
                ForEach(plan.draft.participants) { participant in
                    LabeledContent(participant.name, value: participant.organization ?? "Individual")
                }
                Button("Add participant", systemImage: "person.badge.plus") { addingParticipant = true }
            }
            Section("Swarms") {
                ForEach(plan.draft.swarms) { swarm in
                    VStack(alignment: .leading) {
                        Text(swarm.name).font(.headline)
                        Text(swarm.assignmentIds.compactMap { id in plan.draft.assignments.first { $0.id == id }?.name }.joined(separator: ", "))
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    .swipeActions { Button("Remove", role: .destructive) { plan.change { $0.swarms.removeAll { $0.id == swarm.id } } } }
                }
                Button("Add swarm", systemImage: "square.3.layers.3d") { addingSwarm = true }
            }
            Section("Time on target") {
                ForEach(plan.draft.timingTargets) { target in
                    timing(target)
                        .swipeActions { Button("Remove", role: .destructive) { plan.change { $0.timingTargets.removeAll { $0.id == target.id } } } }
                }
                Button("Add time on target", systemImage: "clock.badge") { addingTarget = true }
                    .disabled(!plan.draft.assignments.contains { !$0.route.waypoints.isEmpty })
            }
            if let assessment = plan.assessment, !assessment.issues.isEmpty {
                Section("Schedule conflicts") {
                    ForEach(assessment.issues, id: \.self) { Text($0).foregroundStyle(.orange) }
                }
            }
            Section {
                Text(plan.saveState).font(.caption).foregroundStyle(.secondary)
                if let error = plan.errorMessage ?? exportError { Text(error).foregroundStyle(.red) }
                Button("Export for review", systemImage: "square.and.arrow.up") {
                    do {
                        document = MissionReviewFile(json: try exportCoordinatedPlan(planJson: PlanningCodec.encode(plan.draft)))
                        exporting = true
                    } catch { exportError = error.localizedDescription }
                }
            } footer: {
                Text("This is a local planning draft. Export does not obtain owner acceptance or send vehicle commands.")
            }
        }
        .sheet(isPresented: $addingParticipant) { MissionParticipantEditor(plan: plan) }
        .sheet(isPresented: $addingSwarm) { MissionSwarmEditor(plan: plan) }
        .sheet(isPresented: $addingTarget) { MissionTimingEditor(plan: plan) }
        .fileExporter(isPresented: $exporting, document: document, contentType: .json,
            defaultFilename: "Pilotage-mission-\(plan.draft.revision).json") { result in
                if case .failure(let error) = result { exportError = error.localizedDescription }
            }
    }

    private func participantName(_ id: String) -> String {
        guard let participant = plan.draft.participants.first(where: { $0.id == id }) else { return "Unassigned" }
        return [participant.name, participant.organization].compactMap { $0 }.joined(separator: " · ")
    }

    private func copy(_ assignment: MissionAssignment) {
        var copy = assignment
        copy.id = UUID().uuidString
        copy.name += " copy"
        copy.vehicleReference = nil
        plan.change { $0.assignments.append(copy) }
        plan.selectedAssignmentId = copy.id
    }

    private func timing(_ target: MissionTimingTarget) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(target.name).font(.headline)
            Text("\(Date(timeIntervalSince1970: Double(target.utc)).formatted(Date.FormatStyle(date: .abbreviated, time: .shortened, timeZone: .gmt))) UTC · ±\(target.toleranceSeconds)s")
                .font(.subheadline)
            ForEach(plan.assessment?.timings.filter { $0.targetId == target.id } ?? []) { timing in
                VStack(alignment: .leading, spacing: 3) {
                    HStack {
                        Text(plan.draft.assignments.first { $0.id == timing.assignmentId }?.name ?? timing.assignmentId)
                        Spacer()
                        Text(timing.statusLabel).foregroundStyle(timing.status == "late" || timing.status == "early" ? .orange : .secondary)
                    }
                    if let low = timing.earliestDepartureUtc, let high = timing.latestDepartureUtc {
                        Text("Depart \(utc(low))–\(utc(high)) UTC at planned ground speed")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
        }
        .environment(\.timeZone, .gmt)
    }

    private func utc(_ seconds: Double) -> String {
        Date(timeIntervalSince1970: seconds).formatted(Date.FormatStyle(date: .abbreviated, time: .standard, timeZone: .gmt))
    }
}

struct MissionReviewFile: FileDocument {
    static var readableContentTypes: [UTType] { [.json] }
    var json = "{}"
    init(json: String = "{}") { self.json = json }
    init(configuration: ReadConfiguration) throws {
        guard let data = configuration.file.regularFileContents else { throw CocoaError(.fileReadCorruptFile) }
        json = String(decoding: data, as: UTF8.self)
    }
    func fileWrapper(configuration: WriteConfiguration) throws -> FileWrapper {
        FileWrapper(regularFileWithContents: Data(json.utf8))
    }
}
