import GameController

/// Translates hardware key codes into the canonical `KeyboardEvent.key`
/// vocabulary the shared control runtime's keyboard profile speaks:
/// single letters lower-cased, arrows and editing keys by their DOM
/// names. The table is the whole translation — which keys DO anything
/// stays in the profile data, so a rebinding never needs shell code.
public enum KeyboardKeyCanon {
    /// The canonical name for one key code, or nil for a key the
    /// canonical vocabulary does not carry.
    public static func canonicalName(for code: GCKeyCode) -> String? {
        names[code]
    }

    private static let names: [GCKeyCode: String] = {
        var names: [GCKeyCode: String] = [
            .upArrow: "ArrowUp",
            .downArrow: "ArrowDown",
            .leftArrow: "ArrowLeft",
            .rightArrow: "ArrowRight",
            .returnOrEnter: "Enter",
            .deleteOrBackspace: "Backspace",
            .spacebar: " ",
        ]
        let letters: [(GCKeyCode, String)] = [
            (.keyA, "a"), (.keyB, "b"), (.keyC, "c"), (.keyD, "d"),
            (.keyE, "e"), (.keyF, "f"), (.keyG, "g"), (.keyH, "h"),
            (.keyI, "i"), (.keyJ, "j"), (.keyK, "k"), (.keyL, "l"),
            (.keyM, "m"), (.keyN, "n"), (.keyO, "o"), (.keyP, "p"),
            (.keyQ, "q"), (.keyR, "r"), (.keyS, "s"), (.keyT, "t"),
            (.keyU, "u"), (.keyV, "v"), (.keyW, "w"), (.keyX, "x"),
            (.keyY, "y"), (.keyZ, "z"),
        ]
        let digits: [(GCKeyCode, String)] = [
            (.one, "1"), (.two, "2"), (.three, "3"), (.four, "4"),
            (.five, "5"), (.six, "6"), (.seven, "7"), (.eight, "8"),
            (.nine, "9"), (.zero, "0"),
        ]
        for (code, name) in letters + digits {
            names[code] = name
        }
        return names
    }()
}
