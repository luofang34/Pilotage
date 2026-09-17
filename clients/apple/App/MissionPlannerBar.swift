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

struct MissionPlannerBar: View {
    @ObservedObject var plan: MissionPlanModel
    @ObservedObject var search: NavigationSearchModel
    @ObservedObject var aviationData: AviationDataModel
    @State private var planPresented = LaunchRequest.openMission
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Button {
            planPresented = true
        } label: {
            HStack(spacing: 10) {
                Image(systemName: "point.topleft.down.to.point.bottomright.curvepath")
                    .foregroundStyle(.primary)
                // Collapsed, the bar answers the glance questions only:
                // where from, where to, how far, how long.
                Text(plan.summary.endpoints)
                    .font(.body.monospaced().weight(.semibold))
                    .lineLimit(1)
                Text(plan.summary.detail)
                    .font(.footnote.monospaced())
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                Spacer(minLength: 0)
                Text(plan.currentProgress == nil ? "Draft" : "Navigation")
                    .font(.caption2.weight(.semibold))
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(.quaternary, in: .capsule)
                    .foregroundStyle(.primary)
                Image(systemName: "chevron.up")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.primary)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .frame(maxWidth: 440)
        }
        .buttonStyle(.glass)
        .buttonBorderShape(.capsule)
        .foregroundStyle(.primary)
        // The pill is the planner's collapsed face; while the planner
        // itself is up there is no second face to show.
        .opacity(planPresented ? 0 : 1)
        .allowsHitTesting(!planPresented)
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.15), value: planPresented)
        .sheet(isPresented: $planPresented) {
            NavigationStack {
                MissionPlannerView(
                    plan: plan, search: search, aviationData: aviationData
                )
                    .navigationTitle(plan.draft.assignments.count == 1 ? "Route Plan" : "Mission Planner")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
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
