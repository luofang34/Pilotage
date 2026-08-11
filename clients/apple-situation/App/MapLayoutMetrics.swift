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

    /// Corner radius of a floating panel.
    static let panelCorner: CGFloat = 26

    /// Padding inside a floating panel.
    static let panelPadding: CGFloat = 18

    /// Width of a floating panel.
    static let panelWidth: CGFloat = 320
}

extension View {
    /// Place a floating control against the safe area, with the standard margin.
    ///
    /// Declared once rather than as a padding value repeated at each corner, so a control
    /// added later cannot sit at a different distance from the edge than the rest.
    func mapControlPlacement(_ alignment: Alignment) -> some View {
        frame(maxWidth: .infinity, maxHeight: .infinity, alignment: alignment)
            .padding(Metrics.edgeInset)
    }
}
