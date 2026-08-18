import SwiftUI

/// The first sidebar level's sections: what an operator works on. The
/// second level shows the selected section's content, in the two-level
/// idiom of Mail's sidebar.
enum OperatorSection: String, CaseIterable, Identifiable {
    case instruments
    case mission

    var id: String { rawValue }

    var title: String {
        switch self {
        case .instruments: "Instruments"
        case .mission: "Mission"
        }
    }

    var symbol: String {
        switch self {
        case .instruments: "rectangle.stack"
        case .mission: "point.topleft.down.to.point.bottomright.curvepath"
        }
    }
}

/// One token of the route string, in the taxonomy a flight-plan editor
/// speaks: airports, published waypoints, custom coordinates,
/// procedures, and an airport joined to its approach. Each kind wears
/// its own color, as chips in the route row.
struct RouteToken: Identifiable {
    let id = UUID()
    let label: String
    let kind: Kind

    enum Kind {
        case airport
        case customWaypoint
        case waypoint
        case procedure
        case airportApproach

        var tint: Color {
            switch self {
            case .airport: .blue
            case .customWaypoint: Color(white: 0.55)
            case .waypoint: .purple
            case .procedure: .teal
            case .airportApproach: .indigo
            }
        }
    }

    /// The sample route, one of each kind the editor will speak.
    static let sample: [RouteToken] = [
        RouteToken(label: "KTTN", kind: .airport),
        RouteToken(label: "40.28°N/74.66°W", kind: .customWaypoint),
        RouteToken(label: "ARD", kind: .waypoint),
        RouteToken(label: "TENNI", kind: .waypoint),
        RouteToken(label: "TENNI ILS 22L", kind: .procedure),
        RouteToken(label: "KJFK Rwy 22L", kind: .airportApproach),
    ]
}

/// One chip of the route row.
struct RouteTokenChip: View {
    let token: RouteToken

    var body: some View {
        Text(token.label)
            .font(.callout.monospaced().weight(.semibold))
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Capsule().fill(token.kind.tint.opacity(0.28)))
            .overlay(Capsule().stroke(token.kind.tint.opacity(0.5), lineWidth: 1))
    }
}

/// One leg of the preview flight plan. Sample data only: the planner is
/// a placeholder for the mission module and commands nothing.
struct PreviewWaypoint: Identifiable {
    let id = UUID()
    /// Published identifier (airport, fix, or navaid).
    let ident: String
    /// What kind of point this is, for the row's glyph.
    let kind: Kind
    /// Desired track to this point, degrees magnetic.
    let dtkDegrees: Int?
    /// Leg distance, nautical miles.
    let distanceNm: Double?
    /// Crossing altitude, feet.
    let altitudeFt: Int?

    enum Kind {
        case airport
        case navaid
        case fix

        var symbol: String {
            switch self {
            case .airport: "airplane.circle"
            case .navaid: "dot.radiowaves.left.and.right"
            case .fix: "triangle"
            }
        }
    }

    /// The sample route the placeholder shows until the mission module
    /// exists.
    static let sample: [PreviewWaypoint] = [
        PreviewWaypoint(ident: "KTTN", kind: .airport, dtkDegrees: nil, distanceNm: nil, altitudeFt: nil),
        PreviewWaypoint(ident: "ARD", kind: .navaid, dtkDegrees: 47, distanceNm: 6.1, altitudeFt: 2000),
        PreviewWaypoint(ident: "TENNI", kind: .fix, dtkDegrees: 71, distanceNm: 18.4, altitudeFt: 3000),
        PreviewWaypoint(ident: "KJFK", kind: .airport, dtkDegrees: 64, distanceNm: 16.8, altitudeFt: nil),
    ]
}

/// The mission planner's collapsed face: one floating bar over the
/// bottom of the map, in the idiom of a search bar, stating the route.
/// Tapping opens the plan sheet. Nothing here is wired: the bar is the
/// placeholder for the mission module (flight plan, procedures,
/// performance, and upload), and it says so.
struct MissionPlannerBar: View {
    @ObservedObject var model: HostLinkModel
    @ObservedObject var plan: MissionPlanModel
    @State private var planPresented = false

    var body: some View {
        Button {
            planPresented = true
        } label: {
            HStack(spacing: 10) {
                Image(systemName: "point.topleft.down.to.point.bottomright.curvepath")
                    .foregroundStyle(.cyan)
                // Collapsed, the bar answers the glance questions only:
                // where from, where to, how far, how long.
                Text(plan.summary.endpoints)
                    .font(.body.monospaced().weight(.semibold))
                    .lineLimit(1)
                Text(plan.summary.detail)
                    .font(.footnote.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Spacer(minLength: 0)
                Text("Preview")
                    .font(.caption2.weight(.semibold))
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(.orange.opacity(0.25)))
                    .foregroundStyle(.orange)
                Image(systemName: "chevron.up")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 18)
            .padding(.vertical, 12)
            .frame(maxWidth: 440)
            .glassEffect(.regular, in: Capsule())
        }
        .buttonStyle(.plain)
        // The pill is the planner's collapsed face; while the planner
        // itself is up there is no second face to show.
        .opacity(planPresented ? 0 : 1)
        .allowsHitTesting(!planPresented)
        .animation(.easeInOut(duration: 0.15), value: planPresented)
        .sheet(isPresented: $planPresented) {
            NavigationStack {
                MissionPlannerView(
                    controllable: model.catalog?.offersFlightControl == true,
                    plan: plan
                )
                    .navigationTitle("Mission Planner")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItemGroup(placement: .topBarLeading) {
                            Button("Proc") {}.disabled(true)
                            Button {} label: { Image(systemName: "plus") }.disabled(true)
                            Button("D→") {}.disabled(true)
                        }
                        ToolbarItem(placement: .topBarTrailing) {
                            // Garmin's .fpl is the lingua franca between
                            // planners and panels; the exporter arrives
                            // with the mission wire-up.
                            Button {} label: {
                                Label("Garmin FPL", systemImage: "square.and.arrow.up")
                            }
                            .disabled(true)
                        }
                        ToolbarItem(placement: .confirmationAction) {
                            Button("Done") { planPresented = false }
                        }
                    }
            }
            .presentationSizing(.page)
            // Three working heights: the pill (collapsed), THIS middle
            // state — just the route workspace over a usable map — and
            // the full page with the log.
            .presentationDetents([.height(400), .large])
            .presentationBackgroundInteraction(.enabled(upThrough: .height(400)))
        }
    }
}
