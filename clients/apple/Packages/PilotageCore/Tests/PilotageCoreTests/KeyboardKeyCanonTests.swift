import GameController
import Testing

@testable import PilotageCore

@Test("The flight keys translate to the profile's canonical names")
func flightKeysSpeakTheProfileVocabulary() {
    // The shared keyboard profile binds exactly these: WASD on the
    // left stick, arrows on the right, Enter to arm, Backspace to
    // disarm. A drifted name here is a silently dead control.
    #expect(KeyboardKeyCanon.canonicalName(for: .keyW) == "w")
    #expect(KeyboardKeyCanon.canonicalName(for: .keyA) == "a")
    #expect(KeyboardKeyCanon.canonicalName(for: .keyS) == "s")
    #expect(KeyboardKeyCanon.canonicalName(for: .keyD) == "d")
    #expect(KeyboardKeyCanon.canonicalName(for: .upArrow) == "ArrowUp")
    #expect(KeyboardKeyCanon.canonicalName(for: .downArrow) == "ArrowDown")
    #expect(KeyboardKeyCanon.canonicalName(for: .leftArrow) == "ArrowLeft")
    #expect(KeyboardKeyCanon.canonicalName(for: .rightArrow) == "ArrowRight")
    #expect(KeyboardKeyCanon.canonicalName(for: .returnOrEnter) == "Enter")
    #expect(KeyboardKeyCanon.canonicalName(for: .deleteOrBackspace) == "Backspace")
}

@Test("Letters are lower-cased and unnamed keys stay silent")
func lettersLowerCaseAndUnnamedKeysDrop() {
    for (code, expected) in [(GCKeyCode.keyQ, "q"), (.keyZ, "z"), (.one, "1"), (.zero, "0")] {
        #expect(KeyboardKeyCanon.canonicalName(for: code) == expected)
    }
    // Modifier keys have no canonical binding vocabulary today; they
    // must drop rather than reach the runtime under an invented name.
    #expect(KeyboardKeyCanon.canonicalName(for: .leftShift) == nil)
    #expect(KeyboardKeyCanon.canonicalName(for: .escape) == nil)
}
