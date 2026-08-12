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
    /// Whether the panel grows out of this group or arrives from the edge of the screen.
    let modesGrowFromControls: Bool
    @ViewBuilder let modesContent: () -> ModesContent
    @State private var levelLabelShown = false

    var body: some View {
        ZStack(alignment: .topTrailing) {
            if modesPresented, modesGrowFromControls {
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
        .animation(
            .spring(response: 0.34, dampingFraction: 0.82),
            value: modesPresented && modesGrowFromControls
        )
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
                .glassEffectID("pilotage.map.pill", in: namespace)

                if camera.isTilted {
                    // The label sits over the control rather than inside it. A button
                    // hands its label to its style, which is free to rebuild it, and a
                    // view rebuilt by something else does not keep the transition it was
                    // given. Over the top, the label answers to nothing but its own state.
                    control(label: "Look straight down", action: resetPitch) {
                        Color.clear
                    }
                    .overlay {
                        risingLabel
                    }
                    .glassEffect(.clear.interactive(), in: .circle)
                    .glassEffectID("pilotage.map.level", in: namespace)
                    .glassEffectTransition(.matchedGeometry)
                    // A child inserted with its parent has no transition of its own to
                    // run: the parent's covers the whole subtree. Delaying the animation
                    // curve does not escape that, because the state still changes in the
                    // cycle that inserts the parent and is folded into it. Waiting first
                    // puts the change in a later cycle, where it is an insertion of its
                    // own and the label's own transition runs.
                    .task {
                        // The state outlives the control, which comes and goes with the
                        // camera, so a run starts from hidden rather than from whatever
                        // the last one left.
                        levelLabelShown = false
                        try? await Task.sleep(for: .milliseconds(180))
                        withAnimation(.easeOut(duration: 0.25)) {
                            levelLabelShown = true
                        }
                    }
                }
                if camera.isRotated {
                    control(
                        label: "Facing \(CompassRose.spokenHeading(camera.headingDegrees)), turn back to north",
                        action: resetHeading
                    ) {
                        CompassRose(headingDegrees: camera.headingDegrees)
                    }
                    .glassEffect(.clear.interactive(), in: .circle)
                    .glassEffectID("pilotage.map.compass", in: namespace)
                    .glassEffectTransition(.matchedGeometry)
                }
            }
        }
        .animation(.easeInOut(duration: 0.3), value: camera.isTilted)
        .animation(.easeInOut(duration: 0.3), value: camera.isRotated)
        .animation(.easeInOut(duration: 0.3), value: canLocate)
    }

    /// The word the level control shows, arriving from under it.
    ///
    /// Clipped to the control it fills, so the word travels from the edge of the shape
    /// rather than from wherever the layout would otherwise have started it. Without the
    /// clip the direction belongs to whatever alignment the surrounding stack happens to
    /// use, which is how a rise becomes a slide from the side.
    private var risingLabel: some View {
        ZStack {
            if levelLabelShown {
                Text("2D")
                    .font(Metrics.controlGlyph)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .frame(width: Metrics.control, height: Metrics.control)
        .clipShape(.circle)
        .allowsHitTesting(false)
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

}

/// A compass that names the direction the map faces.
///
/// The letter carries the answer at a glance and the dial carries the precision. A dial
/// alone asks a reader to estimate an angle from a small shape; a letter alone loses
/// everything between the eight points.
///
/// It is drawn to the control it sits in, not to a glyph box: a dial with a ring of air
/// around it reads as a smaller control beside its neighbours even when the buttons match.
struct CompassRose: View {
    let headingDegrees: Double
    var diameter: CGFloat = Metrics.control

    private var radius: CGFloat { diameter / 2 }

    var body: some View {
        ZStack {
            // The dial turns with the map. The letter does not, because a reader reads a
            // letter upright or not at all.
            ZStack {
                ForEach(0..<16, id: \.self) { tick in
                    Capsule()
                        .fill(.secondary.opacity(tick.isMultiple(of: 4) ? 0.9 : 0.45))
                        .frame(
                            width: diameter * 0.03,
                            height: diameter * (tick.isMultiple(of: 4) ? 0.13 : 0.08)
                        )
                        .offset(y: -radius * 0.74)
                        .rotationEffect(.degrees(Double(tick) * 22.5))
                }
                // The needle points at north, which is what the control offers to return to.
                Triangle()
                    .fill(.red)
                    .frame(width: diameter * 0.13, height: diameter * 0.11)
                    .offset(y: -radius * 0.78)
            }
            .rotationEffect(.degrees(-headingDegrees))
            Text(Self.cardinal(headingDegrees))
                .font(.system(size: diameter * 0.34, weight: .semibold, design: .rounded))
        }
        .frame(width: diameter, height: diameter)
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
