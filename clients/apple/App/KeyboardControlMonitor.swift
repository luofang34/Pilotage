import GameController
import PilotageCore
import UIKit

/// Feeds hardware-keyboard transitions to the shared control runtime,
/// the way the browser shell feeds `keydown`/`keyup`. The monitor owns
/// no key table — names are canonicalized by `KeyboardKeyCanon` and
/// what a key DOES lives in the shared keyboard profile — and it owns
/// one safety rule: keys never fly the vehicle while the operator is
/// typing into a text field, and everything held is dropped whenever
/// capture ends, so a key released out of sight cannot keep a demand
/// alive.
@MainActor
final class KeyboardControlMonitor {
    /// One canonical key transition that should reach the runtime.
    private var onKey: ((String, Bool) -> Void)?
    /// Every held key must be dropped (focus left, scene left, or the
    /// keyboard detached).
    private var onClear: (() -> Void)?

    /// Whether the last event was swallowed by a focused text field;
    /// the transition into typing clears the held set exactly once.
    private var typing = false

    func start(onKey: @escaping (String, Bool) -> Void, onClear: @escaping () -> Void) {
        self.onKey = onKey
        self.onClear = onClear
        if let keyboard = GCKeyboard.coalesced {
            attach(keyboard)
        }
        NotificationCenter.default.addObserver(
            forName: .GCKeyboardDidConnect, object: nil, queue: .main
        ) { [weak self] _ in
            // The keyboard object may not cross the isolation hop;
            // the coalesced accessor re-resolves it on the actor.
            Task { @MainActor in
                if let keyboard = GCKeyboard.coalesced { self?.attach(keyboard) }
            }
        }
        NotificationCenter.default.addObserver(
            forName: .GCKeyboardDidDisconnect, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor in self?.onClear?() }
        }
        NotificationCenter.default.addObserver(
            forName: UIApplication.willResignActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor in self?.onClear?() }
        }
    }

    private func attach(_ keyboard: GCKeyboard) {
        keyboard.keyboardInput?.keyChangedHandler = { [weak self] _, _, code, pressed in
            Task { @MainActor in self?.handle(code: code, pressed: pressed) }
        }
    }

    private func handle(code: GCKeyCode, pressed: Bool) {
        // A focused text field owns the keyboard: nothing typed there
        // may double as a flight input, and whatever was already held
        // must let go the moment typing starts.
        if Self.textInputHasFocus() {
            if !typing {
                typing = true
                onClear?()
            }
            return
        }
        typing = false
        guard let name = KeyboardKeyCanon.canonicalName(for: code) else { return }
        onKey?(name, pressed)
    }

    /// Whether any window's first responder is a text input right now.
    private static func textInputHasFocus() -> Bool {
        UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows)
            .contains { firstResponder(in: $0) is UITextInput }
    }

    private static func firstResponder(in view: UIView) -> UIResponder? {
        if view.isFirstResponder {
            return view
        }
        for subview in view.subviews {
            if let responder = firstResponder(in: subview) {
                return responder
            }
        }
        return nil
    }
}
