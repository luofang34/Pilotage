import SwiftUI

struct MissionPlannerView: View {
    @ObservedObject var plan: MissionPlanModel
    @ObservedObject var search: NavigationSearchModel
    @ObservedObject var aviationData: AviationDataModel
    @State private var page = "route"
    @State private var coordinating = false
    @State private var choosingVehicle = false
    @State private var settingDeparture = false
    @State private var showingProcedures = false

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Picker("Planner page", selection: $page) {
                    Text("Route").tag("route")
                    Text("NavLog").tag("log")
                }.pickerStyle(.segmented)
                Button("Team & Timing", systemImage: "person.2") { coordinating = true }
                    .labelStyle(.iconOnly).buttonStyle(.glass).buttonBorderShape(.circle)
            }
            .padding(.horizontal).padding(.top, 8)
            if plan.draft.assignments.count > 1 {
                Picker("Vehicle assignment", selection: $plan.selectedAssignmentId) {
                    ForEach(plan.draft.assignments) { Text($0.name).tag($0.id) }
                }.pickerStyle(.menu).padding(.horizontal)
            }
            if page == "route" { editor } else { navlog }
        }
        .sheet(isPresented: $coordinating) {
            NavigationStack {
                CoordinatedMissionView(plan: plan)
                    .navigationTitle("Coordination").navigationBarTitleDisplayMode(.inline)
                    .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { coordinating = false } } }
            }
        }
        .sheet(isPresented: $choosingVehicle) {
            VehicleProfilesView(library: plan.vehicles, selected: plan.route.vehicleProfile) { plan.selectVehicle($0) }
        }
        .sheet(isPresented: $settingDeparture) { departureEditor }
        .sheet(isPresented: $showingProcedures) {
            MissionProcedureChartsView(model: aviationData, airports: Set(plan.tokens.filter {
                $0.point.kind == "airport"
            }.map(\.label)))
                .presentationSizing(.page)
                .presentationDetents([.large])
        }
        .sheet(item: $plan.routeUpdate) { RouteUpdateReview(plan: plan, proposal: $0) }
    }

    private var editor: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Button { choosingVehicle = true } label: {
                        Label(plan.route.vehicleProfile?.title ?? "Select vehicle", systemImage: "airplane")
                            .lineLimit(1)
                    }.accessibilityIdentifier("select-vehicle")
                    Spacer(minLength: 8)
                    Button { settingDeparture = true } label: {
                        Label(departureLabel, systemImage: "clock")
                    }
                }.font(.subheadline)
                RouteEntryEditor(plan: plan, search: search)
                HStack(spacing: 16) {
                    Button { plan.routeOverviewRequest &+= 1 } label: {
                        Label(plan.estimate.map { String(format: "%.1f NM", $0.distanceNm) } ?? "— NM", systemImage: "arrow.up.left.and.arrow.down.right")
                    }.accessibilityLabel("Show route on map").disabled(plan.tokens.isEmpty)
                    Label(MissionPlanModel.duration(plan.estimate?.durationSeconds), systemImage: "clock")
                    Spacer()
                    if plan.route.vehicleProfile?.speedReference == "true_airspeed" && plan.route.groundspeedKnots == nil {
                        Text("Still air").foregroundStyle(.secondary)
                    }
                }.font(.footnote.monospacedDigit())
                if let message = plan.errorMessage { Text(message).foregroundStyle(.red).font(.footnote) }
                navigationStatus
                Button("Procedure charts", systemImage: "doc.text") { showingProcedures = true }
                    .font(.subheadline).disabled(aviationData.busy)
                    .accessibilityIdentifier("mission-procedures")
                DisclosureGroup("Details") {
                    VStack(alignment: .leading, spacing: 10) {
                        if let profile = plan.route.vehicleProfile {
                            Text(String(format: "%@ · %.0f %@", profile.name, profile.cruiseSpeedKnots, profile.speedLabel))
                            if let source = profile.source { Text(source).foregroundStyle(.secondary) }
                        }
                        if let override = plan.route.groundspeedKnots {
                            HStack {
                                Text(String(format: "Ground-speed override: %.0f kt", override))
                                Spacer()
                                Button("Clear override") { plan.changeRoute { $0.groundspeedKnots = nil } }
                            }
                        }
                        Text("Route time uses cruise speed. Wind, climb, descent, and fuel are not calculated.")
                            .foregroundStyle(.secondary)
                        ForEach(plan.estimate?.issues ?? [], id: \.self) { Text($0) }
                        Text(plan.saveState).foregroundStyle(.secondary)
                    }.font(.footnote).frame(maxWidth: .infinity, alignment: .leading).padding(.top, 8)
                }.font(.subheadline)
            }.padding()
        }
    }

    private var departureLabel: String {
        guard let departure = plan.route.departureUtc else { return "Departure" }
        return Date(timeIntervalSince1970: Double(departure))
            .formatted(Date.FormatStyle(date: .omitted, time: .shortened, timeZone: .gmt)) + " UTC"
    }

    private var departureEditor: some View {
        NavigationStack {
            Form {
                Toggle("Set departure time", isOn: Binding(get: { plan.route.departureUtc != nil }, set: { value in
                    plan.changeRoute { $0.departureUtc = value ? Int64(Date().timeIntervalSince1970) : nil }
                }))
                if let departure = plan.route.departureUtc {
                    DatePicker("Departure (UTC)", selection: Binding(get: { Date(timeIntervalSince1970: Double(departure)) }, set: { value in
                        plan.changeRoute { $0.departureUtc = Int64(value.timeIntervalSince1970) }
                    })).environment(\.timeZone, .gmt)
                }
            }
            .navigationTitle("Departure").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { settingDeparture = false } } }
        }
    }

    @ViewBuilder
    private var navigationStatus: some View {
        if let progress = plan.currentProgress {
            HStack {
                if let destination = progress.destination {
                    Label("Current: " + destination.label, systemImage: "location.fill").foregroundStyle(.pink)
                    Spacer()
                    Button("Mark reached") { plan.advanceCurrentLeg() }
                } else { Label("Route complete", systemImage: "checkmark") }
                Button("Stop") { plan.navigationProgress = nil }
            }.font(.subheadline)
            Text("Navigation on this iPad · Blue: planned · Magenta: current · Orange: past")
                .font(.caption).foregroundStyle(.secondary)
        } else if plan.tokens.count > 1 {
            Button("Navigate on this iPad", systemImage: "location") { plan.setCurrentLeg(to: plan.tokens[1].id) }
                .font(.subheadline)
        }
    }

    private var navlog: some View {
        List {
            ForEach(Array((plan.estimate?.legs ?? []).enumerated()), id: \.element.id) { index, leg in
                if let point = plan.tokens.first(where: { $0.id == leg.waypointId }) {
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Text(point.label).font(.headline.monospaced())
                            if index > 0 {
                                let role = plan.legRole(destinationIndex: index, assignmentID: plan.assignment?.id ?? "")
                                Text(role.label).font(.caption).foregroundStyle(role == .current ? Color.pink : role == .past ? .orange : .blue)
                            }
                            Spacer()
                            if let arrival = leg.arrivalUtc {
                                Text(Date(timeIntervalSince1970: arrival), format: .dateTime.hour().minute().timeZone())
                            }
                        }
                        HStack {
                            Text(leg.trackTrueDeg.map { String(format: "TRK %.0f°T", $0) } ?? "TRK —")
                            Spacer()
                            Text(String(format: "LEG %.1f NM", leg.distanceNm))
                            Spacer()
                            Text(MissionPlanModel.duration(leg.elapsedSeconds))
                        }.font(.caption.monospacedDigit())
                        Text(String(format: "Remaining %.1f NM", leg.remainingNm)).font(.caption)
                    }
                    .contextMenu { if index > 0 { Button("Set current leg") { plan.setCurrentLeg(to: point.id) } } }
                }
            }
        }
        .environment(\.timeZone, .gmt)
        .overlay { if plan.tokens.isEmpty { ContentUnavailableView("No route", systemImage: "point.topleft.down.to.point.bottomright.curvepath") } }
    }
}
