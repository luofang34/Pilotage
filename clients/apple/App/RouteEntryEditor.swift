import SwiftUI

struct RouteEntryEditor: View {
    @ObservedObject var plan: MissionPlanModel
    @ObservedObject var search: NavigationSearchModel
    @State private var entry = RouteEntryDraft()
    @State private var loading = false
    @State private var failure: String?
    @State private var suggestions: [NavigationMatch] = []
    @State private var ambiguous: String?
    @State private var searching = false
    @State private var chosen: [String: NavigationMatch] = [:]
    @State private var endEditing: UInt64 = 0
    @State private var pendingSubmit: String?
    @State private var submitAgain = false

    private var words: [String] { entry.text.split(whereSeparator: { $0.isWhitespace }).map(String.init).filter { $0 != "DCT" } }
    private var query: String { ambiguous ?? (entry.wholeRoute == nil ? words.first ?? "" : "") }
    private var reference: NavigationPoint? { entry.reference(in: plan.tokens) }
    private var edited: RouteToken? { plan.tokens.first { $0.id == entry.replacingID } }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .top, spacing: 6) {
                RouteFlowLayout(tokens: entry.visible(plan.tokens), text: $entry.text,
                    insertion: entry.position(in: plan.tokens), focusRequest: entry.focusRequest,
                    endEditingRequest: endEditing,
                    allowsReorder: entry.replacingID == nil && entry.beforeID == nil && entry.text.isEmpty,
                    reorder: { plan.proposeRoute($0); return plan.tokens == $0 }, select: { beginEditing($0) },
                    backspace: backspace, submit: { Task { await resolve() } }, invalidInput: invalidInput,
                    editsWholeRoute: entry.wholeRoute != nil, editingID: entry.replacingID)
                HStack(spacing: 0) {
                    if loading || pendingSubmit != nil { ProgressView().frame(width: 44, height: 44) } else {
                        Button("Add route", systemImage: "return") { Task { await resolve() } }
                            .labelStyle(.iconOnly).frame(width: 44, height: 44)
                            .disabled(entry.text.trimmingCharacters(in: .whitespaces).isEmpty && edited == nil && entry.wholeRoute == nil)
                    }
                    Button("Add waypoint", systemImage: "magnifyingglass") { searching = true }
                        .labelStyle(.iconOnly).frame(width: 44, height: 44)
                    Menu("Route actions", systemImage: "ellipsis") { actions }
                        .labelStyle(.iconOnly).frame(width: 44, height: 44)
                }
            }
            if edited != nil || entry.beforeID != nil || !entry.text.isEmpty || entry.wholeRoute != nil {
                HStack {
                    Text(entry.wholeRoute != nil ? "Edit route text" : edited.map { "Editing " + $0.label } ?? entry.beforeID.flatMap { id in plan.tokens.first { $0.id == id }.map { "Insert before " + $0.label } } ?? "Add to route")
                        .font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    Button("Cancel edit") { reset() }.font(.caption)
                }
            }
            if let failure { Text(failure).font(.footnote).foregroundStyle(.red) }
            if pendingSubmit != nil { Text(search.errorMessage ?? "Opening navigation data…").font(.footnote).foregroundStyle(.secondary) }
            if !query.isEmpty { matches }
            if plan.tokens.isEmpty && entry.text.isEmpty {
                Text("Type identifiers separated by spaces. Press Return, then continue typing. You can also write with Apple Pencil.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .padding(12)
        .background(.quaternary.opacity(0.5), in: .rect(cornerRadius: 12))
        .onChange(of: entry.text) { _, text in
            ambiguous = nil; failure = nil; chosen = [:]
            if pendingSubmit != text { pendingSubmit = nil }
        }
        .onChange(of: search.isReady) { _, ready in
            if ready, pendingSubmit == entry.text { Task { await resolve() } }
        }
        .onChange(of: plan.selectedAssignmentId) { _, _ in reset() }
        .task(id: query + ":" + String(search.revision) + ":" + (reference?.key ?? "")) { await loadSuggestions() }
        .sheet(isPresented: $searching) {
            NavigationSearchView(search: search, plan: plan, replacing: entry.replacingID, reference: reference) { token in
                if entry.wholeRoute != nil { entry.text += " " + token.label } else { commit([token]) }
            }
        }
    }

    private var matches: some View {
        VStack(alignment: .leading, spacing: 0) {
            if !suggestions.isEmpty {
                Text(reference.map { "Matches · distance from " + $0.identifier } ?? "Matches")
                    .font(.caption).foregroundStyle(.secondary).padding(.vertical, 4)
            }
            ForEach(Array(suggestions.prefix(6))) { match in
                Button { Task { await resolve(preferred: match) } } label: {
                    HStack {
                        Text(match.point.identifier).font(.callout.monospaced().weight(.semibold))
                        Text(match.point.name.isEmpty ? match.point.region : match.point.name)
                            .font(.caption).lineLimit(1).foregroundStyle(.primary)
                        Spacer(minLength: 4)
                        if Date().timeIntervalSince1970 >= Double(match.source.expiresAt) {
                            Text("Expired").font(.caption).foregroundStyle(.orange)
                        }
                        if let distance = NavigationMatchOrder.distance(match.point, from: reference) {
                            Text(String(format: "%.0f NM", distance)).font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                        }
                    }.frame(minHeight: 44).contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("route-match-" + match.point.identifier)
            }
        }
    }

    @ViewBuilder private var actions: some View {
        Button("Edit route text", systemImage: "pencil") { _ = beginWriting() }
        if let edited {
            Button("Show on Map", systemImage: "map") { plan.mapSelection = edited }
            Button("Insert before", systemImage: "arrow.left") { entry.insert(before: edited.id) }
            Button("Insert after", systemImage: "arrow.right") {
                let index = entry.position(in: plan.tokens) + 1
                entry.insert(before: plan.tokens.indices.contains(index) ? plan.tokens[index].id : nil)
            }
            if edited.id != plan.tokens.first?.id {
                Button("Set current leg", systemImage: "location") { plan.setCurrentLeg(to: edited.id); reset() }
            }
            Button("Remove waypoint", systemImage: "trash", role: .destructive) { commit([]) }
        }
        if let undo = plan.routeUndo, undo.assignmentID == plan.assignment?.id, undo.before == plan.tokens {
            Button("Undo route edit", systemImage: "arrow.uturn.backward") { plan.undoRouteEdit(); reset() }
        }
        if !plan.tokens.isEmpty {
            Button("Copy route", systemImage: "doc.on.doc") { UIPasteboard.general.string = plan.tokens.map(\.label).joined(separator: " ") }
        }
        Button("Search airports and waypoints", systemImage: "magnifyingglass") { searching = true }
    }

    private func beginEditing(_ token: RouteToken) {
        entry.edit(token)
        if RouteIdentifierInput.normalized(token.label) == nil { entry.text = "" }
    }
    private func backspace() {
        guard let deletion = entry.deletingBackward(in: plan.tokens) else { return }
        plan.proposeRoute(deletion.tokens)
        if plan.tokens == deletion.tokens {
            reset(dismissKeyboard: false); entry.beforeID = deletion.beforeID
        } else if plan.routeUpdate != nil { reset() }
        else { failure = plan.errorMessage }
    }
    private func beginWriting() -> String {
        if entry.wholeRoute != nil { return entry.text }
        var labels = plan.tokens.map(\.label)
        let index = entry.position(in: plan.tokens)
        if entry.replacingID != nil, labels.indices.contains(index) { labels.remove(at: index) }
        if !entry.text.isEmpty { labels.insert(entry.text, at: min(index, labels.count)) }
        entry.editRoute(plan.tokens); entry.text = labels.joined(separator: " ")
        return entry.text
    }
    private func invalidInput() { failure = "Use A–Z, 0–9, and spaces. Use search for names or coordinates." }
    private func reset(dismissKeyboard: Bool = true) {
        entry.reset(); failure = nil; ambiguous = nil; suggestions = []; chosen = [:]; pendingSubmit = nil
        if dismissKeyboard { endEditing &+= 1; submitAgain = false }
    }

    private func loadSuggestions() async {
        guard !query.isEmpty else { suggestions = []; return }
        do {
            let found = try await search.search(query)
            guard !Task.isCancelled else { return }
            suggestions = NavigationMatchOrder.sorted(found, query: query, near: reference)
        } catch { if !Task.isCancelled { suggestions = []; failure = error.localizedDescription } }
    }

    private func resolve(preferred: NavigationMatch? = nil) async {
        guard !loading else { submitAgain = true; return }
        guard RouteIdentifierInput.normalized(entry.text) != nil else { invalidInput(); return }
        guard words.count + (entry.wholeRoute == nil ? plan.tokens.count - (edited == nil ? 0 : 1) : 0) <= 512 else {
            failure = "A route can contain at most 512 waypoints."; return
        }
        if words.isEmpty { if edited != nil || entry.wholeRoute != nil { commit([]) }; return }
        guard search.isReady else { pendingSubmit = entry.text; return }
        pendingSubmit = nil
        guard !search.sources.isEmpty else { failure = search.status; return }
        if let preferred, let word = ambiguous ?? words.first { chosen[word] = preferred }
        loading = true
        defer {
            loading = false
            if submitAgain { submitAgain = false; Task { await resolve() } }
        }
        failure = nil
        let submitted = entry, before = plan.tokens, assignmentID = plan.assignment?.id
        let submittedWords = words
        var resolved: [RouteToken] = []
        do {
            for word in submittedWords {
                let match: NavigationMatch
                if let preferred = chosen[word] { match = preferred }
                else {
                    let foundMatches = try await search.search(word)
                    let exact = foundMatches.filter { $0.point.identifier == word }
                    guard !Task.isCancelled, entry.remainder(after: submitted) != nil, plan.tokens == before, plan.assignment?.id == assignmentID else { return }
                    guard exact.count == 1, let found = exact.first else {
                        ambiguous = word
                        failure = exact.isEmpty ? "No exact match for \(word). Choose a match or change the identifier." : "Choose the location for \(word)."
                        suggestions = NavigationMatchOrder.sorted(foundMatches, query: word, near: reference)
                        return
                    }
                    match = found
                }
                resolved.append(RouteToken(id: UUID().uuidString, point: match.point, source: match.source))
            }
            guard !Task.isCancelled, entry.remainder(after: submitted) != nil, plan.tokens == before, plan.assignment?.id == assignmentID else { return }
            commit(resolved, submitted: submitted)
        } catch { failure = error.localizedDescription }
    }

    private func commit(_ additions: [RouteToken], submitted: RouteEntryDraft? = nil) {
        let submitted = submitted ?? entry
        guard let remainder = entry.remainder(after: submitted), let updated = submitted.applying(additions, to: plan.tokens) else {
            failure = "The route changed. Cancel this edit and try again."; return
        }
        let nextID = submitted.wholeRoute == nil ? submitted.nextID(in: plan.tokens) : nil
        let revision = plan.draft.revision
        plan.proposeRoute(updated)
        if plan.draft.revision != revision || plan.routeUpdate != nil || updated == plan.tokens {
            let review = plan.routeUpdate != nil
            reset(dismissKeyboard: review)
            if !review { entry.text = remainder; entry.beforeID = nextID }
        }
        else { failure = plan.errorMessage }
    }
}
