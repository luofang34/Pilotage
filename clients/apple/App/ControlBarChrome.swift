import SwiftUI

/// The arm order telegraph: a two-position lever the operator sets and
/// a lamp only the flight controller's own report moves. Amber between
/// order and answer; the lever never re-sends on its own.
struct ArmTelegraphControl: View {
    @ObservedObject var model: HostLinkModel
    /// Whether the operator holds the authority to move the levers.
    /// Without it the telegraph is a gauge: it shows the flight
    /// controller's own armed state, at reduced emphasis, in the same
    /// place it will be a control once authority is granted.
    let interactive: Bool

    var body: some View {
        // The levers carry the whole story: pressing one is the
        // operator's intent, amber is an order the flight controller
        // has not answered, green is the controller confirming that
        // state. No separate lamp — the answer lives where the order
        // was given.
        HStack(spacing: 0) {
            lever("SAFE", ordersArmed: false)
            lever("ARM", ordersArmed: true)
        }
        .background(Capsule().fill(Color(white: 0.18)))
        .opacity(interactive ? 1.0 : 0.55)
    }

    /// One lever width for both positions: SAFE and ARM must read as
    /// the two ends of one control, and the capsule must not change
    /// shape when the weight of the selected title differs.
    private static let leverWidth: CGFloat = 56

    private func lever(_ title: String, ordersArmed: Bool) -> some View {
        // Under authority the highlight tracks the operator's ORDER;
        // as a gauge it tracks the flight controller's ANSWER — an
        // observer sees the vehicle's state, never a stale order.
        let selected = interactive
            ? model.armOrdered == ordersArmed
            : model.armConfirmed == (ordersArmed ? 2 : 1)
        return Button {
            if ordersArmed { model.arm() } else { model.disarm() }
        } label: {
            Text(title)
                .font(.callout.weight(selected ? .bold : .regular))
                .lineLimit(1)
                .frame(width: Self.leverWidth)
                .padding(.vertical, 5)
                .background(
                    Capsule().fill(selected ? leverTint(ordersArmed: ordersArmed) : .clear)
                )
        }
        .buttonStyle(.plain)
        .disabled(!interactive)
    }

    private func leverTint(ordersArmed: Bool) -> Color {
        // Amber is an unanswered order; green is the flight
        // controller's own report confirming the ordered state; gray
        // is a selection the controller has not (or no longer)
        // confirmed. Red stays reserved for what has actually gone
        // wrong.
        if interactive && model.armPhase == 1 { return .orange }
        let confirmed = model.armConfirmed == (ordersArmed ? 2 : 1)
        return confirmed ? .green : Color(white: 0.35)
    }

}

/// A tile slot this build cannot fill, saying why in place. It never
/// paints a picture that implies the data exists.
struct UnavailableTile: View {
    let title: String
    let reason: String

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 8)
                .fill(Color(white: 0.12))
            VStack(spacing: 6) {
                Text(title)
                    .font(.headline)
                Text(reason)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            .foregroundStyle(.white)
            .padding(8)
        }
    }
}

/// One glanceable statement of where the link stands. Tapping opens the
/// connection sheet — the chip is the door to the flow, not the flow.
struct ConnectionChip: View {
    let phase: HostLinkModel.Phase
    let open: () -> Void

    var body: some View {
        Button(action: open) {
            HStack(spacing: 6) {
                Circle().fill(tint).frame(width: 8, height: 8)
                Text(label)
                    .font(.footnote.weight(.medium))
                    .lineLimit(1)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Capsule().fill(Color(white: 0.18)))
        }
        .buttonStyle(.plain)
        .foregroundStyle(.white)
    }

    private var label: String {
        // One word each: the chip is a glance, and the sheet carries the
        // host and scope in full.
        switch phase {
        case .idle: "Connect"
        case .connecting: "Connecting…"
        case .observing: "Observing"
        case .controlling: "Controlling"
        case .reconnecting: "Reconnecting…"
        case .stopped: "Stopped"
        }
    }

    private var tint: Color {
        switch phase {
        case .idle: .gray
        case .connecting, .reconnecting: .yellow
        case .observing: .green
        case .controlling: .blue
        case .stopped: .red
        }
    }
}

/// The session's own log in a standard sheet: one selectable text
/// block (a single selection copies any number of lines), timestamps
/// leading, newest last — the status line above shows the tail, this
/// shows the whole tape.
struct StatusLogSheet: View {
    @ObservedObject var model: HostLinkModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                Text(joined)
                    .font(.caption.monospaced())
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding()
            }
            .navigationTitle("Session log")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }

    private var joined: String {
        model.statusLog
            .map { "\(Self.clock.string(from: $0.at))  \($0.text)" }
            .joined(separator: "\n")
    }

    private static let clock: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "HH:mm:ss"
        return formatter
    }()
}
