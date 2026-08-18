import PilotageCore
import SwiftUI

/// The flight control unit strip: the autopilot's face, in the idiom of
/// an Airbus FCU — speed, heading, altitude, and vertical speed windows
/// with their engage buttons. A placeholder: every window shows dashes
/// or a static figure and every control is inert. It appears at all
/// only when the connected host offers a CONTROLLABLE flight computer;
/// a plan-input panel (a Garmin-style navigator) never shows one.
struct FlightControlUnit: View {
    /// Closes the strip; the pop-out lives in the rack's control area.
    let dismiss: () -> Void

    var body: some View {
        HStack(spacing: 14) {
            window(label: "SPD", value: "---", unit: "KT")
            window(label: "HDG", value: "---", unit: "°")
            window(label: "ALT", value: "-----", unit: "FT")
            window(label: "V/S", value: "----", unit: "FPM")
            Divider()
                .frame(height: 34)
            engage("AP1")
            engage("A/THR")
            engage("APPR")
            Button(action: dismiss) {
                Image(systemName: "xmark")
                    .font(.caption.weight(.bold))
                    .padding(6)
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(Color(white: 0.08), in: RoundedRectangle(cornerRadius: 12))
        .overlay(
            RoundedRectangle(cornerRadius: 12)
                .stroke(Color(white: 0.25), lineWidth: 1)
        )
        .overlay(alignment: .bottomTrailing) {
            Text("preview — not wired")
                .font(.system(size: 8).weight(.semibold))
                .foregroundStyle(.orange)
                .padding(.trailing, 34)
                .padding(.bottom, 3)
        }
    }

    /// One value window with its selector knob glyph beneath.
    private func window(label: String, value: String, unit: String) -> some View {
        VStack(spacing: 3) {
            Text(label)
                .font(.system(size: 9).weight(.semibold))
                .foregroundStyle(.secondary)
            Text(value)
                .font(.system(.body, design: .monospaced).weight(.bold))
                .foregroundStyle(.orange)
            HStack(spacing: 3) {
                Image(systemName: "dial.medium")
                    .font(.system(size: 9))
                    .foregroundStyle(.secondary)
                Text(unit)
                    .font(.system(size: 8))
                    .foregroundStyle(.secondary)
            }
        }
        .frame(minWidth: 52)
    }

    /// One engage button, dark-cockpit style: unlit until a mode is
    /// real.
    private func engage(_ title: String) -> some View {
        Button {} label: {
            Text(title)
                .font(.system(size: 11, design: .monospaced).weight(.bold))
                .padding(.horizontal, 8)
                .padding(.vertical, 8)
                .background(RoundedRectangle(cornerRadius: 4).fill(Color(white: 0.16)))
                .overlay(
                    RoundedRectangle(cornerRadius: 4)
                        .stroke(Color(white: 0.3), lineWidth: 1)
                )
        }
        .buttonStyle(.plain)
        .foregroundStyle(Color(white: 0.6))
        .disabled(true)
    }
}

extension LinkCatalog {
    /// The `pilotage.v1.IntentFamily` codes that command the VEHICLE:
    /// velocity, position-hold, attitude-thrust, and body-rate. The
    /// gimbal family moves a camera, and zero is the unspecified
    /// sentinel; neither makes a host controllable.
    private static let vehicleCommandFamilies: ClosedRange<Int32> = 1...4

    /// Whether this host commands a flight computer, rather than only
    /// accepting a plan: some scope advertises a typed motion intent.
    /// The FCU exists exactly when this holds.
    var offersFlightControl: Bool {
        vehicles.contains { vehicle in
            vehicle.scopes.contains { scope in
                scope.intents.contains { intent in
                    Self.vehicleCommandFamilies.contains(intent.family)
                }
            }
        }
    }
}
