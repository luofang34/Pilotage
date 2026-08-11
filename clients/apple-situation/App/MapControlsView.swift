import PilotageMapLibreBinding
import SwiftUI

/// The controls that undo what a reader did to the camera.
///
/// The chrome is the system button style, so the platform decides how a control sits over
/// a map. Hand-drawn material was one release behind the moment it was written.
///
/// Each one appears only when there is something to undo. A compass on a map that faces
/// north, or a level control on a map that already looks straight down, is a control that
/// does nothing: it teaches a reader to ignore that corner, which is the corner where the
/// map says it is no longer oriented the way they assume.
/// Identity the map-modes panel grows out of.
enum MapControlIdentity {
    static let modes = "pilotage.map.modes"
}

struct MapControlsView<ModesContent: View>: View {
    let camera: SituationCamera
    let ownship: OwnshipFix?
    let canLocate: Bool
    let follow: FollowMode
    let namespace: Namespace.ID
    let resetHeading: () -> Void
    let resetPitch: () -> Void
    let cycleFollow: () -> Void
    @Binding var modesPresented: Bool
    @ViewBuilder let modesContent: () -> ModesContent
    @State private var levelling = false
    @State private var pillFlash = false

    var body: some View {
        ZStack(alignment: .topTrailing) {
            if modesPresented {
                // The panel takes the corner the pill was in. It does not sit beside the
                // control that opened it: the control became it.
                modesContent()
                    .transition(
                        .scale(scale: 0.18, anchor: .topTrailing).combined(with: .opacity)
                    )
            } else {
                controls
                    .transition(
                        .scale(scale: 0.9, anchor: .topTrailing).combined(with: .opacity)
                    )
            }
        }
        .animation(.spring(response: 0.34, dampingFraction: 0.82), value: modesPresented)
    }

    /// One glass container holds every map control.
    ///
    /// Clear glass rather than regular: these float over a map a reader is reading, and
    /// the map has to stay legible under them. Interactive glass keeps the press response
    /// a button gives up when it stops using the system button style, which is the price
    /// of choosing the variant.
    ///
    /// A glass identity on each control lets one grow out of the group and shrink back
    /// into it. The group flashes as it takes one back, the way a surface does when
    /// something rejoins it.
    private var controls: some View {
        GlassEffectContainer(spacing: Metrics.controlSpacing) {
            VStack(spacing: Metrics.controlSpacing) {
                VStack(spacing: 0) {
                    control(label: "Map modes") { modesPresented = true } content: {
                        Image(systemName: "globe.americas.fill")
                    }
                    if canLocate {
                        control(label: follow.label, action: cycleFollow) {
                            Image(systemName: follow.symbol)
                        }
                    }
                }
                .glassEffect(.clear.interactive(), in: .capsule)
                .glassEffectUnion(id: "pilotage.map.pill", namespace: namespace)
                .glassEffectID("pilotage.map.pill", in: namespace)
                .brightness(pillFlash ? 0.22 : 0)
                .animation(.easeOut(duration: 0.22), value: pillFlash)

                if camera.isTilted {
                    control(label: "Look straight down") {
                        levelling = true
                        resetPitch()
                    } content: {
                        // The control names the state it is about to reach as it goes, so
                        // the press is acknowledged before the control leaves.
                        Text(levelling ? "3D" : "2D")
                    }
                    .glassEffect(.clear.interactive(), in: .circle)
                    .glassEffectID("pilotage.map.level", in: namespace)
                }
                if camera.isRotated {
                    control(label: "Facing \(CompassRose.spokenHeading(camera.headingDegrees)), turn back to north", action: resetHeading) {
                        CompassRose(headingDegrees: camera.headingDegrees)
                    }
                    .glassEffect(.regular.interactive(), in: .circle)
                    .glassEffectID("pilotage.map.compass", in: namespace)
                }
            }
        }
        .animation(.easeInOut(duration: 0.3), value: camera.isTilted)
        .animation(.easeInOut(duration: 0.3), value: camera.isRotated)
        .animation(.easeInOut(duration: 0.3), value: canLocate)
        .onChange(of: camera.isTilted) { _, tilted in
            if !tilted { levelling = false; flashPill() }
        }
        .onChange(of: camera.isRotated) { _, rotated in
            if !rotated { flashPill() }
        }
    }

    /// One control: the same square, the same glyph weight, the same target.
    private func control(
        label: String,
        action: @escaping () -> Void,
        @ViewBuilder content: () -> some View
    ) -> some View {
        Button(action: action) {
            content()
                .font(Metrics.controlGlyph)
                .frame(width: Metrics.control, height: Metrics.control)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }

    /// Mark the group taking a control back.
    private func flashPill() {
        pillFlash = true
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: 220_000_000)
            pillFlash = false
        }
    }
}

/// A compass that names the direction the map faces.
///
/// The letter carries the answer at a glance and the needle carries the precision. A
/// needle alone asks a reader to estimate an angle from a small shape; a letter alone
/// loses everything between the eight points.
struct CompassRose: View {
    let headingDegrees: Double

    var body: some View {
        ZStack {
            // The dial turns with the map. The letter does not, because a reader reads a
            // letter upright or not at all.
            ZStack {
                ForEach(0..<16, id: \.self) { tick in
                    Capsule()
                        .fill(.secondary.opacity(tick.isMultiple(of: 4) ? 0.9 : 0.45))
                        .frame(width: 1.2, height: tick.isMultiple(of: 4) ? 4 : 2.5)
                        .offset(y: -10)
                        .rotationEffect(.degrees(Double(tick) * 22.5))
                }
                // The needle points at north, which is what the control offers to return to.
                Triangle()
                    .fill(.red)
                    .frame(width: 6, height: 5)
                    .offset(y: -10)
            }
            .rotationEffect(.degrees(-headingDegrees))
            Text(Self.cardinal(headingDegrees))
                .font(.system(size: 12, weight: .semibold, design: .rounded))
        }
    }

    /// The compass point the map faces, as a letter a reader reads without thinking.
    static func cardinal(_ headingDegrees: Double) -> String {
        let points = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"]
        let normalised = (headingDegrees.truncatingRemainder(dividingBy: 360) + 360)
            .truncatingRemainder(dividingBy: 360)
        let index = Int((normalised / 45).rounded()) % points.count
        return points[index]
    }

    /// The same direction, said in full for a reader who cannot see it.
    static func spokenHeading(_ headingDegrees: Double) -> String {
        let names = [
            "N": "north", "NE": "north east", "E": "east", "SE": "south east",
            "S": "south", "SW": "south west", "W": "west", "NW": "north west",
        ]
        return names[cardinal(headingDegrees)] ?? "north"
    }
}

/// A north needle.
struct Triangle: Shape {
    func path(in rect: CGRect) -> Path {
        var path = Path()
        path.move(to: CGPoint(x: rect.midX, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.maxY))
        path.addLine(to: CGPoint(x: rect.minX, y: rect.maxY))
        path.closeSubpath()
        return path
    }
}


/// The platform's own treatment for a control that floats over a map.
///
/// The style is the system's, so these controls follow the platform as it changes rather
/// than carrying a copy of one release's material.
extension View {
    /// Apply the platform's treatment for a map control.
    func mapControlButton() -> some View {
        buttonStyle(.glass).clipShape(Circle())
    }
}
