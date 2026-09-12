import PilotageMapLibreBinding
import SwiftUI

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
    let togglePitch: () -> Void
    let cycleFollow: () -> Void
    @Binding var modesPresented: Bool
    let modesGrowFromControls: Bool
    @ViewBuilder let modesContent: () -> ModesContent
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        GlassEffectContainer(spacing: Metrics.controlSpacing) {
            ZStack(alignment: .topTrailing) {
                if modesPresented {
                    if modesGrowFromControls {
                        modesContent()
                            .glassEffectID(MapControlIdentity.modes, in: namespace)
                            .glassEffectTransition(.matchedGeometry)
                    }
                } else {
                    controls
                }
            }
        }
        .animation(reduceMotion ? nil : .default, value: modesPresented)
        .animation(reduceMotion ? nil : .default, value: camera.isTilted)
        .animation(reduceMotion ? nil : .default, value: camera.isRotated)
        .animation(reduceMotion ? nil : .default, value: camera.canTilt)
        .animation(reduceMotion ? nil : .default, value: canLocate)
    }

    private var controls: some View {
        VStack(spacing: Metrics.controlSpacing) {
            MapControlPill {
                MapPillButton(label: "Map modes", action: { modesPresented = true }) {
                    Image(systemName: "globe.americas.fill")
                }
                if canLocate {
                    MapPillButton(label: follow.label, action: cycleFollow) {
                        Image(systemName: follow.symbol)
                    }
                }
            }
            .glassEffectID(MapControlIdentity.modes, in: namespace)
            .glassEffectTransition(.matchedGeometry)
            if camera.isRotated {
                MapControlButton(action: resetHeading) {
                    CompassRose(headingDegrees: camera.headingDegrees)
                }
                .accessibilityLabel("Facing \(CompassRose.spokenHeading(camera.headingDegrees)), turn back to north")
                .glassEffectID("pilotage.map.compass", in: namespace)
                .glassEffectTransition(.matchedGeometry)
            }
            if camera.canTilt || camera.isTilted {
                MapControlButton(action: togglePitch) {
                    Text(camera.isTilted ? "2D" : "3D")
                        .contentTransition(.numericText())
                }
                .accessibilityLabel(camera.isTilted ? "Look straight down" : "Show 3D terrain")
                .glassEffectID("pilotage.map.pitch", in: namespace)
                .glassEffectTransition(.matchedGeometry)
            }
        }
    }
}

struct MapControlPill<Content: View>: View {
    @ViewBuilder let content: () -> Content

    var body: some View {
        VStack(spacing: Metrics.controlSpacing, content: content)
            .glassEffect(.regular.interactive(), in: .capsule)
    }
}

struct MapPillButton<Content: View>: View {
    let label: String
    let action: () -> Void
    @ViewBuilder let content: () -> Content

    var body: some View {
        Button(action: action) {
            content()
                .font(Metrics.controlGlyph)
                .frame(width: Metrics.control, height: Metrics.control)
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .foregroundStyle(.primary)
        .accessibilityLabel(label)
    }
}

struct MapControlButton<Content: View>: View {
    let action: () -> Void
    @ViewBuilder let content: () -> Content

    var body: some View {
        Button(action: action) {
            content()
                .font(Metrics.controlGlyph)
                .frame(width: Metrics.controlLabelBox, height: Metrics.controlLabelBox)
        }
        .buttonStyle(.glass)
        .buttonBorderShape(.circle)
        .controlSize(.regular)
        .foregroundStyle(.primary)
    }
}

/// The dial points to north. The letter shows the map heading.
struct CompassRose: View {
    let headingDegrees: Double
    var diameter: CGFloat = Metrics.control

    private var radius: CGFloat { diameter / 2 }

    var body: some View {
        ZStack {
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
                Triangle()
                    .fill(.red)
                    .frame(width: diameter * 0.13, height: diameter * 0.11)
                    .offset(y: -radius * 0.78)
            }
            .rotationEffect(.degrees(-headingDegrees))
            Text(Self.cardinal(headingDegrees))
                .font(.system(size: diameter * 0.375, weight: .semibold, design: .rounded))
        }
        .frame(width: diameter, height: diameter)
    }

    /// The compass point for the map heading.
    static func cardinal(_ headingDegrees: Double) -> String {
        let points = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"]
        let normalised = (headingDegrees.truncatingRemainder(dividingBy: 360) + 360)
            .truncatingRemainder(dividingBy: 360)
        let index = Int((normalised / 45).rounded()) % points.count
        return points[index]
    }

    /// The compass point for VoiceOver.
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
