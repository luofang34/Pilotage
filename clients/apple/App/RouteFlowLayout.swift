import SwiftUI

/// Reorders the dragged chip as it passes over its neighbors, the
/// standard flowing drag-to-reorder choreography.
struct ChipReorderDelegate: DropDelegate {
    let target: RouteToken.ID
    @Binding var tokens: [RouteToken]
    @Binding var dragged: RouteToken.ID?

    func dropEntered(info: DropInfo) {
        guard let dragged,
              dragged != target,
              let from = tokens.firstIndex(where: { $0.id == dragged }),
              let to = tokens.firstIndex(where: { $0.id == target })
        else { return }
        withAnimation {
            tokens.move(
                fromOffsets: IndexSet(integer: from),
                toOffset: to > from ? to + 1 : to
            )
        }
    }

    func dropUpdated(info: DropInfo) -> DropProposal? {
        DropProposal(operation: .move)
    }

    func performDrop(info: DropInfo) -> Bool {
        dragged = nil
        return true
    }
}

/// Ends the drag when the drop lands on the workspace but not on a
/// chip, so nothing is left half-lifted.
struct ChipDropEndDelegate: DropDelegate {
    @Binding var dragged: RouteToken.ID?

    func performDrop(info: DropInfo) -> Bool {
        dragged = nil
        return true
    }
}

/// A minimal wrapping layout: rows fill left to right and break where
/// the width ends, like words in a paragraph.
struct FlowLayout: Layout {
    var spacing: CGFloat = 6

    func placeSubviews(
        in bounds: CGRect,
        proposal: ProposedViewSize,
        subviews: Subviews,
        cache: inout ()
    ) {
        // The container's actual bounds govern the wrap: the proposal
        // that sized the layout is not always the width it was given.
        let arrangement = arrange(width: bounds.width, subviews: subviews)
        for (subview, position) in zip(subviews, arrangement.positions) {
            subview.place(
                at: CGPoint(x: bounds.minX + position.x, y: bounds.minY + position.y),
                proposal: .unspecified
            )
        }
    }

    func sizeThatFits(
        proposal: ProposedViewSize,
        subviews: Subviews,
        cache: inout ()
    ) -> CGSize {
        arrange(width: proposal.width ?? .infinity, subviews: subviews).size
    }

    private func arrange(
        width limit: CGFloat,
        subviews: Subviews
    ) -> (positions: [CGPoint], size: CGSize) {
        var positions: [CGPoint] = []
        var sizes: [CGSize] = []
        var rowStart = 0
        var cursor = CGPoint.zero
        var rowHeight: CGFloat = 0
        var width: CGFloat = 0

        func closeRow(endingAt end: Int) {
            // Chips and the taller entry field share a line; each row's
            // members center on that row's own height.
            for index in rowStart..<end {
                positions[index].y += (rowHeight - sizes[index].height) / 2
            }
        }

        for (index, subview) in subviews.enumerated() {
            let size = subview.sizeThatFits(.unspecified)
            if cursor.x > 0, cursor.x + size.width > limit {
                closeRow(endingAt: index)
                cursor.x = 0
                cursor.y += rowHeight + spacing
                rowHeight = 0
                rowStart = index
            }
            positions.append(cursor)
            sizes.append(size)
            cursor.x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
            width = max(width, cursor.x - spacing)
        }
        closeRow(endingAt: subviews.count)
        return (positions, CGSize(width: width, height: cursor.y + rowHeight))
    }
}
