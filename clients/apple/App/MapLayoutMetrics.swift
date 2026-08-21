import SwiftUI

/// The measurements every floating map control shares.
///
/// One place, because the alternative is what it replaced: a control that was wider than
/// the one under it because each carried its own numbers. A reader reads a column of
/// controls as one thing only when they are one width.
enum Metrics {
    /// Side of a single round or capsule control.
    ///
    /// Forty-four points is the platform's smallest comfortable target, and a map is used
    /// with a thumb while the aircraft moves.
    static let control: CGFloat = 52

    /// Size of the glyph inside a control.
    static let controlGlyph: Font = .system(size: 19, weight: .semibold)

    /// Box the compass dial is drawn in, inside a control.
    static let controlGlyphBox: CGFloat = 30

    /// Gap between controls in a stack.
    ///
    /// Also the blend distance of the glass container that holds them, and the two are
    /// the same number on purpose. A container blends only the shapes that sit within its
    /// spacing, so a distance below this gap leaves each control its own island: it stops
    /// growing out of the group and starts arriving from nowhere.
    static let controlSpacing: CGFloat = 10

    /// Distance from a control to the edge of the safe area.
    ///
    /// The map runs under the status bar and the home indicator, and the controls do not.
    /// This is the margin between the two.
    static let edgeInset: CGFloat = 12

    /// How tall the system's own bar is, where it is drawn over this application.
    ///
    /// Asked of the scene rather than taken from the safe area, because the two do not
    /// agree: a window carries a safe area with no bar in it, and a full screen carries a
    /// bar that a control has to clear rather than merely start beneath.
    static var systemBarHeight: CGFloat {
        UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .first?
            .statusBarManager?
            .statusBarFrame.height ?? 0
    }

    /// Room the window's own controls take at the top of the leading edge.
    ///
    /// A windowed application is given a grabber and a close control there, and they are
    /// drawn over whatever the application puts underneath. The system does not always
    /// reserve the space in the safe area, so anything the application floats in that one
    /// corner keeps clear of it by hand. Only that corner: reserving it across the width
    /// would push the far side down for a control that is not there.
    static let windowControlAllowance: CGFloat = 44

    /// Corner radius of a floating panel.
    static let panelCorner: CGFloat = 26

    /// Padding inside a floating panel.
    static let panelPadding: CGFloat = 18

    /// Width of a floating panel.
    static let panelWidth: CGFloat = 320

    /// Height of the system navigation band the neighboring panes draw
    /// their bar buttons in. The map pane has no bar of its own, but
    /// its floating top controls sit ON THE SAME LINE as the sidebar's
    /// bar buttons — one shared row across every pane, the way the
    /// platform's own split applications align theirs.
    static let navigationBand: CGFloat = 50
}

extension View {
    /// Place a floating control against the window, clear of whatever the system reserves.
    ///
    /// The margin is the larger of the standard one and the system's own inset, not the
    /// sum. Added to the inset instead, a control ends up further from the top edge than
    /// from the side by exactly the height of the status bar, which reads as the corner
    /// being weighted rather than as a margin. Taking the larger keeps the control clear
    /// of the status bar where there is one, and square to the corner where there is not.
    ///
    /// Declared once rather than as a padding value repeated at each corner, so a control
    /// added later cannot sit at a different distance from the edge than the rest.
    func mapControlPlacement(_ alignment: Alignment) -> some View {
        // The window's controls sit in the top leading corner and are drawn over the
        // application. Only what shares that corner gives way to them.
        let sharesTheWindowControls = alignment == .topLeading
        return GeometryReader { proxy in
            let safe = proxy.safeAreaInsets
            // Under a system bar the margin goes below it, so a control clears the bar
            // rather than beginning where it ends. Without one the plain margin applies,
            // which is what keeps the corner square where nothing is reserved.
            let bar = Metrics.systemBarHeight
            // Top-edge controls center on the navigation band beside
            // them rather than hanging a margin below the status bar:
            // the sidebar's bar buttons and the map's floating controls
            // read as one row across the window.
            let bandTop = max(bar, safe.top)
                + (Metrics.navigationBand - Metrics.control) / 2
            let top = max(Metrics.edgeInset, bandTop)
            frame(maxWidth: .infinity, maxHeight: .infinity, alignment: alignment)
                .padding(
                    .top,
                    sharesTheWindowControls ? top + Metrics.windowControlAllowance : top
                )
                .padding(.bottom, max(Metrics.edgeInset, safe.bottom))
                .padding(.leading, max(Metrics.edgeInset, safe.leading))
                .padding(.trailing, max(Metrics.edgeInset, safe.trailing))
        }
        .ignoresSafeArea()
    }
}
