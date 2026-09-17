import Foundation
import PilotageCore

struct RouteToken: Codable, Equatable, Identifiable, Sendable {
    let id: String
    var point: NavigationPoint
    var source: NavigationSource?
    var altitudeMslFt: Double?
    var label: String { point.identifier }
}

struct PlannedRoute: Codable, Equatable, Sendable {
    var waypoints: [RouteToken] = []
    var groundspeedKnots: Double?
    var departureUtc: Int64?
    var vehicleProfile: VehicleProfile?
}

struct PlannedRouteLeg: Decodable, Identifiable {
    let waypointId: String
    let distanceNm: Double
    let trackTrueDeg: Double?
    let remainingNm: Double
    let elapsedSeconds: Double?
    let arrivalUtc: Double?
    var id: String { waypointId }
}

struct PlannedRouteSummary: Decodable {
    let distanceNm: Double
    let durationSeconds: Double?
    let legs: [PlannedRouteLeg]
    let issues: [String]
}

actor MissionDraftStore {
    let url: URL
    init(url: URL) { self.url = url }

    func load() throws -> MissionDraft? {
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        let size = try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        guard size <= 16 * 1024 * 1024 else { throw CocoaError(.fileReadTooLarge) }
        let draft = try PlanningCodec.decode(MissionDraft.self, String(contentsOf: url, encoding: .utf8))
        guard draft.schemaVersion == 1 else { throw CocoaError(.coderReadCorrupt) }
        _ = try evaluateCoordinatedPlan(planJson: PlanningCodec.encode(draft))
        return draft
    }

    func save(_ draft: MissionDraft) throws {
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data(PlanningCodec.encode(draft).utf8).write(to: url, options: .atomic)
    }
}

@MainActor
final class MissionPlanModel: ObservableObject {
    let vehicles = VehicleProfileLibrary()
    @Published private(set) var draft = MissionDraft()
    @Published private(set) var assessment: CoordinatedMissionAssessment?
    @Published var selectedAssignmentId = ""
    @Published private(set) var errorMessage: String?
    @Published private(set) var saveState = "Opening draft…"
    @Published var mapSelection: RouteToken?
    @Published var navigationProgress: RouteNavigationProgress?
    @Published var routeUpdate: RouteUpdateProposal?
    @Published var routeUndo: RouteUpdateProposal?
    @Published var routeOverviewRequest: UInt64 = 0
    private var store: MissionDraftStore?
    private var saveTask: Task<Void, Never>?
    private var started = false

    var tokens: [RouteToken] {
        get { route.waypoints }
        set { changeRoute { $0.waypoints = newValue } }
    }
    var assignment: MissionAssignment? { draft.assignments.first { $0.id == selectedAssignmentId } ?? draft.assignments.first }
    var route: PlannedRoute { assignment?.route ?? PlannedRoute() }
    var estimate: PlannedRouteSummary? { assignment.flatMap { assessment?.routes[$0.id] } }
    var retainedSources: [NavigationSource] {
        var ids = Set<String>()
        return draft.assignments.flatMap { $0.route.waypoints.compactMap(\.source) }
            .filter { ids.insert($0.releaseId).inserted }.sorted { $0.releaseId < $1.releaseId }
    }

    func changeRoute(_ update: (inout PlannedRoute) -> Void) {
        guard let id = assignment?.id else { return }
        change { draft in
            guard let index = draft.assignments.firstIndex(where: { $0.id == id }) else { return }
            update(&draft.assignments[index].route)
        }
    }
    var summary: (endpoints: String, detail: String) {
        guard let first = tokens.first else { return ("Plan a mission", "Add airports or waypoints") }
        let endpoints = tokens.count > 1 ? first.label + " → " + (tokens.last?.label ?? "") : first.label
        return (endpoints, estimate.map { String(format: "%.1f NM", $0.distanceNm) } ?? "—")
    }

    func start(url: URL? = nil) async {
        guard !started else { return }
        started = true
        do {
            let destination = try url ?? FileManager.default.url(for: .applicationSupportDirectory,
                in: .userDomainMask, appropriateFor: nil, create: true)
                .appendingPathComponent("Missions/\(LaunchRequest.missionTestDraft.map { "test-" + $0 } ?? "current").json")
            let storage = MissionDraftStore(url: destination)
            await vehicles.start(url: destination.deletingLastPathComponent().appendingPathComponent("vehicles.json"))
            if let saved = try await storage.load() { draft = saved }
            selectedAssignmentId = draft.assignments.first?.id ?? ""
            store = storage
            saveState = "Saved on this device"
            recalculate()
        } catch {
            errorMessage = "The saved mission could not be opened: \(error.localizedDescription)"
            saveState = "Draft could not be opened"
        }
    }

    func add(_ match: NavigationMatch, replacing id: String? = nil) {
        insert(RouteToken(id: UUID().uuidString, point: match.point, source: match.source), replacing: id)
    }

    func insert(_ token: RouteToken, replacing id: String? = nil) {
        changeRoute {
            if let id, let index = $0.waypoints.firstIndex(where: { $0.id == id }) {
                $0.waypoints[index] = RouteToken(id: id, point: token.point, source: token.source,
                    altitudeMslFt: $0.waypoints[index].altitudeMslFt)
            } else { $0.waypoints.append(token) }
        }
        mapSelection = token
    }

    func proposeRoute(_ waypoints: [RouteToken]) {
        guard let assignment, waypoints != tokens else { return }
        let proposal = RouteUpdateProposal(assignmentID: assignment.id, before: tokens, after: waypoints,
            destinationID: currentProgress?.destination?.id)
        if currentProgress != nil { routeUpdate = proposal }
        else { applyRoute(proposal, destinationID: nil) }
    }

    func applyRoute(_ proposal: RouteUpdateProposal, destinationID: String?) {
        guard assignment?.id == proposal.assignmentID, tokens == proposal.before else {
            errorMessage = "The route changed. Review the route and try again."
            routeUpdate = nil
            return
        }
        let revision = draft.revision
        tokens = proposal.after
        guard revision != draft.revision else { return }
        routeUndo = RouteUpdateProposal(assignmentID: proposal.assignmentID, before: proposal.after, after: proposal.before)
        if let destinationID { setCurrentLeg(to: destinationID) }
        routeUpdate = nil
    }

    func undoRouteEdit() {
        guard let routeUndo, routeUndo.assignmentID == assignment?.id, routeUndo.before == tokens else { return }
        proposeRoute(routeUndo.after)
    }

    func change(_ update: (inout MissionDraft) -> Void) {
        guard store != nil else { return }
        var candidate = draft
        update(&candidate)
        do { _ = try evaluateCoordinatedPlan(planJson: PlanningCodec.encode(candidate)) }
        catch { errorMessage = error.localizedDescription; return }
        candidate.revision &+= 1
        if let progress = navigationProgress,
           candidate.assignments.first(where: { $0.id == progress.assignmentID })?.route.waypoints != progress.waypoints {
            navigationProgress = nil
        }
        draft = candidate
        recalculate()
        saveState = "Saving…"
        let previous = saveTask
        let current = draft
        saveTask = Task { [weak self, store] in
            guard let self, let store else { return }
            await previous?.value
            do {
                try Task.checkCancellation()
                try await store.save(current)
                guard current.revision == self.draft.revision else { return }
                self.saveState = "Saved on this device"
            } catch is CancellationError {
            } catch {
                self.errorMessage = error.localizedDescription
                self.saveState = "Could not save draft"
            }
        }
    }

    func flush() async { await saveTask?.value }

    func selectVehicle(_ profile: VehicleProfile?) {
        changeRoute { $0.vehicleProfile = profile; $0.groundspeedKnots = nil }
    }

    private func recalculate() {
        do {
            assessment = try PlanningCodec.decode(CoordinatedMissionAssessment.self,
                evaluateCoordinatedPlan(planJson: PlanningCodec.encode(draft)))
            errorMessage = nil
        } catch {
            assessment = nil
            errorMessage = error.localizedDescription
        }
    }

    static func duration(_ seconds: Double?) -> String {
        guard let seconds, seconds >= 0,
              let minutes = Int(exactly: (seconds / 60).rounded()) else { return "—" }
        return "\(minutes / 60)+\(String(format: "%02d", minutes % 60))"
    }
}
