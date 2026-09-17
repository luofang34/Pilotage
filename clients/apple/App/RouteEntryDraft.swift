import Foundation
import CoreLocation

enum RouteIdentifierInput {
    static func normalized(_ value: String) -> String? {
        var output = ""
        for scalar in value.unicodeScalars {
            switch scalar.value {
            case 65...90, 48...57: output.unicodeScalars.append(scalar)
            case 97...122: output.append(String(scalar).uppercased())
            case 9, 10, 13, 32: output.append(" ")
            default: return nil
            }
        }
        return output
    }
}

struct RouteEntryDraft {
    var text = ""
    var replacingID: String?
    var beforeID: String?
    var wholeRoute: [RouteToken]?
    var focusRequest: UInt64 = 0
    func visible(_ tokens: [RouteToken]) -> [RouteToken] { tokens.filter { $0.id != replacingID } }
    func position(in tokens: [RouteToken]) -> Int {
        let id = replacingID ?? beforeID
        return tokens.firstIndex { $0.id == id } ?? tokens.count
    }
    func reference(in tokens: [RouteToken]) -> NavigationPoint? {
        if let token = tokens.first(where: { $0.id == replacingID }) { return token.point }
        let index = position(in: tokens)
        return index > 0 ? tokens[index - 1].point : tokens.first?.point
    }
    mutating func edit(_ token: RouteToken) {
        text = token.label; replacingID = token.id; beforeID = nil; focusRequest &+= 1
    }
    mutating func insert(before id: String?) {
        reset(); beforeID = id; focusRequest &+= 1
    }
    mutating func editRoute(_ tokens: [RouteToken]) {
        reset(); wholeRoute = tokens; text = tokens.map(\.label).joined(separator: " "); focusRequest &+= 1
    }
    func deletingBackward(in tokens: [RouteToken]) -> (tokens: [RouteToken], beforeID: String?)? {
        guard text.isEmpty, wholeRoute == nil else { return nil }
        let index = position(in: tokens) - (replacingID == nil ? 1 : 0)
        guard tokens.indices.contains(index) else { return nil }
        var result = tokens
        result.remove(at: index)
        return (result, result.indices.contains(index) ? result[index].id : nil)
    }
    func remainder(after submitted: RouteEntryDraft) -> String? {
        guard replacingID == submitted.replacingID, beforeID == submitted.beforeID, wholeRoute == submitted.wholeRoute,
              text.hasPrefix(submitted.text), wholeRoute == nil || text == submitted.text,
              text == submitted.text || submitted.text.last?.isWhitespace == true else { return nil }
        return String(text.dropFirst(submitted.text.count))
    }
    func nextID(in tokens: [RouteToken]) -> String? {
        let index = position(in: tokens) + (replacingID == nil ? 0 : 1)
        return tokens.indices.contains(index) ? tokens[index].id : nil
    }
    mutating func reset() { text = ""; replacingID = nil; beforeID = nil; wholeRoute = nil }
    func applying(_ additions: [RouteToken], to tokens: [RouteToken]) -> [RouteToken]? {
        if let wholeRoute {
            guard wholeRoute == tokens else { return nil }
            var unused = tokens
            return additions.map { token in
                guard let index = unused.firstIndex(where: { $0.point.key == token.point.key }) else { return token }
                let original = unused.remove(at: index)
                return RouteToken(id: original.id, point: token.point, source: token.source, altitudeMslFt: original.altitudeMslFt)
            }
        }
        if let replacingID, !tokens.contains(where: { $0.id == replacingID }) { return nil }
        if let beforeID, !tokens.contains(where: { $0.id == beforeID }) { return nil }
        let index = position(in: tokens)
        var result = tokens, inserted = additions
        if replacingID != nil {
            let replaced = result.remove(at: index)
            if let first = inserted.first {
                inserted[0] = RouteToken(id: replaced.id, point: first.point, source: first.source, altitudeMslFt: replaced.altitudeMslFt)
            }
        }
        result.insert(contentsOf: inserted, at: index)
        return result
    }
}

enum NavigationMatchOrder {
    static func distance(_ point: NavigationPoint, from reference: NavigationPoint?) -> Double? {
        guard let reference else { return nil }
        return CLLocation(latitude: reference.latitudeDeg, longitude: reference.longitudeDeg)
            .distance(from: CLLocation(latitude: point.latitudeDeg, longitude: point.longitudeDeg)) / 1852
    }
    static func sorted(_ matches: [NavigationMatch], query: String, near reference: NavigationPoint?) -> [NavigationMatch] {
        matches.sorted {
            let a = $0.point.identifier.caseInsensitiveCompare(query) == .orderedSame
            let b = $1.point.identifier.caseInsensitiveCompare(query) == .orderedSame
            if a != b { return a }
            let da = distance($0.point, from: reference) ?? 0, db = distance($1.point, from: reference) ?? 0
            if da != db { return da < db }
            return $0.id < $1.id
        }
    }
}
