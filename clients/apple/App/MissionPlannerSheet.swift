import PilotageCore
import SwiftUI
import UniformTypeIdentifiers

/// The one working route, owned above every planner face so the
/// column, the sheet, and the pill all speak about the same plan.
final class MissionPlanModel: ObservableObject {
    @Published var tokens: [RouteToken] = RouteToken.sample

    /// The glance summary: where from, where to, how many in between.
    var summary: (endpoints: String, detail: String) {
        guard let first = tokens.first else {
            return ("No route", "")
        }
        let last = tokens.last?.label ?? ""
        let between = max(tokens.count - 2, 0)
        return (
            tokens.count == 1 ? first.label : "\(first.label) → \(last)",
            between > 0 ? "\(between) between · 41 NM · 0+22" : "41 NM · 0+22"
        )
    }
}

/// The mission planner's full face: the route editor and the navigation
/// log, in a page-wide sheet. Everything is a preview — the editor
/// accepts tokens and the log shows the standard columns, but nothing
/// reaches a vehicle and the exporter is a door not yet opened.
struct MissionPlannerView: View {
    /// Whether the connected host commands a flight computer directly.
    /// Direct control executes a mission itself; a panel-style host
    /// (a Garmin navigator) exchanges plans instead.
    let controllable: Bool
    /// The shared plan every planner face edits.
    @ObservedObject var plan: MissionPlanModel
    /// Which face is up: the editor or the log.
    @State private var page: Page = .edit
    /// The in-progress token entry.
    @State private var entry = ""

    enum Page: String, CaseIterable, Identifiable {
        case edit
        case navlog

        var id: String { rawValue }
        var title: String {
            switch self {
            case .edit: "Edit"
            case .navlog: "NavLog"
            }
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                Picker("Page", selection: $page) {
                    ForEach(Page.allCases) { page in
                        Text(page.title).tag(page)
                    }
                }
                .pickerStyle(.segmented)
                // The plan's exit is the host's shape: direct control
                // executes, a panel exchanges. Both doors wait for the
                // mission wire-up.
                if controllable {
                    Button("Execute") {}
                        .buttonStyle(.borderedProminent)
                        .disabled(true)
                } else {
                    Button("Send to Panel") {}
                        .buttonStyle(.bordered)
                        .disabled(true)
                    Button("Get from Panel") {}
                        .buttonStyle(.bordered)
                        .disabled(true)
                }
            }
            .padding(.horizontal)
            .padding(.top, 8)
            switch page {
            case .edit: editor
            case .navlog: navlog
            }
        }
    }

    // MARK: - Editor

    private var editor: some View {
        // A ScrollView, not a List: a list hoists any drag inside a row
        // onto the whole cell, so the chips could never lift alone nor
        // accept a drop. Plain views keep the drag where it was put.
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                sectionCard("Route") {
                    // ONE workspace: the chips and the cursor share the
                    // same flowing line, like words in a sentence —
                    // typing continues where the route ends, and a chip
                    // drags to its new place in the order.
                    RouteWorkspace(tokens: $plan.tokens, entry: $entry)
                    Divider()
                    HStack(spacing: 0) {
                        metric("DIST", "41 nm")
                        metric("ETE", "0h22m")
                        metric("ETA", "—")
                        metric("FUEL", "—")
                        metric("WIND", "—")
                    }
                }
                sectionCard("Procedures") {
                    // The mission module will browse SID, STAR, and
                    // approach with transitions per airport; the rows
                    // hold the door open.
                    LabeledContent("Departure", value: "—")
                    Divider()
                    LabeledContent("Arrival", value: "—")
                    Divider()
                    LabeledContent("Approach", value: "—")
                }
            }
            .padding()
        }
    }

    /// One grouped-list-look card, drawn by hand so its contents keep
    /// their own gestures.
    private func sectionCard(
        _ title: String,
        @ViewBuilder content: () -> some View
    ) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title.uppercased())
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
                .padding(.leading, 4)
            VStack(alignment: .leading, spacing: 12) {
                content()
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: 12)
                    .fill(Color(uiColor: .secondarySystemGroupedBackground))
            )
        }
    }

    // MARK: - NavLog

    /// The standard navigation log: one row per fix with the columns a
    /// paper log carries — desired track, leg and remaining distance,
    /// time, and the planned altitude.
    private var navlog: some View {
        List {
            Section {
                Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 8) {
                    GridRow {
                        Text("WPT")
                        Text("DTK")
                        Text("LEG")
                        Text("REM")
                        Text("ETE")
                        Text("ETA")
                        Text("ALT")
                    }
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(.secondary)
                    Divider()
                    ForEach(Array(navlogRows.enumerated()), id: \.offset) { _, row in
                        GridRow {
                            Text(row.ident)
                                .font(.body.monospaced().weight(.semibold))
                            Text(row.dtk)
                            Text(row.leg)
                            Text(row.remaining)
                            Text(row.ete)
                            Text(row.eta)
                            Text(row.altitude)
                        }
                        .font(.footnote.monospaced())
                    }
                }
            } footer: {
                Text("Totals: 41 nm · 0h22m · fuel — · preview data")
                    .font(.footnote.monospaced())
            }
        }
    }

    private struct NavlogRow {
        let ident: String
        let dtk: String
        let leg: String
        let remaining: String
        let ete: String
        let eta: String
        let altitude: String
    }

    private var navlogRows: [NavlogRow] {
        [
            NavlogRow(ident: "KTTN", dtk: "—", leg: "—", remaining: "41.3", ete: "—", eta: "—", altitude: "—"),
            NavlogRow(ident: "ARD", dtk: "047°", leg: "6.1", remaining: "35.2", ete: "0+03", eta: "—", altitude: "2000"),
            NavlogRow(ident: "TENNI", dtk: "071°", leg: "18.4", remaining: "16.8", ete: "0+10", eta: "—", altitude: "3000"),
            NavlogRow(ident: "KJFK", dtk: "064°", leg: "16.8", remaining: "0.0", ete: "0+09", eta: "—", altitude: "—"),
        ]
    }

    private func metric(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label)
                .font(.caption2.weight(.semibold))
                .foregroundStyle(.secondary)
            Text(value)
                .font(.footnote.monospaced())
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The route workspace: chips and the text entry flowing together,
/// wrapping like a sentence. A tap on a chip opens the waypoint's own
/// menu; a long-press drags it to a new place in the order.
private struct RouteWorkspace: View {
    @Binding var tokens: [RouteToken]
    @Binding var entry: String
    @State private var dragged: RouteToken.ID?

    /// A zero-width space keeps the field one deletion away from
    /// empty: when backspace consumes it, the LAST CHIP is what the
    /// keystroke meant — the token-field grammar every route editor
    /// speaks. The system keyboard has no key events to observe; the
    /// sentinel is what makes the deletion visible.
    private static let sentinel = "\u{200B}"
    /// What an identifier may be made of: ICAO name-codes and ARINC 424
    /// identifiers are A–Z and 0–9 only; the degree-and-minute marks
    /// admit hand-typed coordinate points. Everything else — spaces,
    /// accents, any non-ASCII — never enters the field.
    private static let identifierAlphabet = Set(
        "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789./°'\"+- \u{200B}"
    )

    @FocusState private var entryFocused: Bool

    var body: some View {
        FlowLayout(spacing: 6) {
            ForEach(tokens) { token in
                WaypointChipMenu(token: token) {
                    withAnimation { tokens.removeAll { $0.id == token.id } }
                }
                .onDrag {
                    dragged = token.id
                    return NSItemProvider(object: token.label as NSString)
                } preview: {
                    // The lift is the chip alone, never the whole row a
                    // list cell would volunteer.
                    RouteTokenChip(token: token)
                }
                .onDrop(
                    of: [.text],
                    delegate: ChipReorderDelegate(
                        target: token.id,
                        tokens: $tokens,
                        dragged: $dragged
                    )
                )
            }
            TextField("", text: $entry)
                .font(.body.monospaced())
                .autocorrectionDisabled()
                .textInputAutocapitalization(.characters)
                .keyboardType(.asciiCapable)
                .focused($entryFocused)
                .submitLabel(.next)
                .frame(minWidth: 150, maxWidth: 240)
                .overlay(alignment: .leading) {
                    if entry == Self.sentinel || entry.isEmpty {
                        Text("Add waypoint…")
                            .font(.body.monospaced())
                            .foregroundStyle(.tertiary)
                            .allowsHitTesting(false)
                    }
                }
                .onAppear {
                    if entry.isEmpty { entry = Self.sentinel }
                }
                .onChange(of: entry) { previous, value in
                    if value.isEmpty {
                        // Only the backspace that ate the sentinel means
                        // "delete the last chip"; clearing typed text
                        // (select-all, cut) only empties the field.
                        if previous == Self.sentinel, !tokens.isEmpty {
                            withAnimation { tokens.removeLast() }
                        }
                        entry = Self.sentinel
                        return
                    }
                    // Uppercase as typed and refuse what an identifier
                    // cannot contain, in place.
                    let cleaned = String(
                        value.uppercased().filter { Self.identifierAlphabet.contains($0) }
                    )
                    if cleaned != value {
                        entry = cleaned.isEmpty ? Self.sentinel : cleaned
                    }
                }
                .onKeyPress(.delete) {
                    // A hardware keyboard deletes past the sentinel in
                    // one report; treat an effectively-empty field as
                    // the same order on the last chip.
                    guard entry == Self.sentinel || entry.isEmpty else { return .ignored }
                    guard !tokens.isEmpty else { return .ignored }
                    withAnimation { tokens.removeLast() }
                    return .handled
                }
                .onSubmit {
                    let label = entry
                        .replacingOccurrences(of: Self.sentinel, with: "")
                        .trimmingCharacters(in: .whitespaces)
                        .uppercased()
                    if !label.isEmpty {
                        withAnimation {
                            tokens.append(RouteToken(label: label, kind: .waypoint))
                        }
                    }
                    entry = Self.sentinel
                    // Return means "next waypoint", never "done": the
                    // cursor stays in the sentence.
                    DispatchQueue.main.async { entryFocused = true }
                }
        }
        .onDrop(of: [.text], delegate: ChipDropEndDelegate(dragged: $dragged))
        .contentShape(Rectangle())
        .onTapGesture { entryFocused = true }
    }
}

/// The waypoint's own menu, in the grammar a flight-plan editor speaks.
/// A TAP opens it as a popover anchored to the chip — a `Menu` would
/// claim the long press that belongs to drag-to-reorder. Only removal
/// acts today; every other entry is the door the mission module will
/// walk through.
private struct WaypointChipMenu: View {
    let token: RouteToken
    let remove: () -> Void
    @State private var presented = false

    var body: some View {
        Button {
            presented = true
        } label: {
            RouteTokenChip(token: token)
        }
        .buttonStyle(.plain)
        .popover(isPresented: $presented, arrowEdge: .bottom) {
            VStack(alignment: .leading, spacing: 0) {
                row("Show on Map", "map")
                row("Direct To", "location.north.line")
                Divider()
                row("Replace…", "arrow.triangle.2.circlepath")
                row("Insert Before…", "arrow.left.to.line")
                row("Insert After…", "arrow.right.to.line")
                Divider()
                row("Select Runway…", "road.lanes")
                row("Along-Track Offset…", "point.topleft.down.to.point.bottomright.curvepath")
                row("Hold…", "arrow.triangle.capsulepath")
                row("Set Altitude/Speed…", "gauge.with.needle")
                Divider()
                Button {
                    presented = false
                    remove()
                } label: {
                    Label("Remove from Route", systemImage: "trash")
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 10)
                }
                .foregroundStyle(.red)
            }
            .frame(minWidth: 260)
            .padding(.vertical, 6)
            .presentationCompactAdaptation(.popover)
        }
    }

    private func row(_ title: String, _ symbol: String) -> some View {
        Button {} label: {
            Label(title, systemImage: symbol)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
        }
        .disabled(true)
    }
}
