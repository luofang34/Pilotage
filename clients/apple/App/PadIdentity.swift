// Naming a controller so the shared registry can recognise it.
//
// The registry keys on a USB vendor/product pair, which GameController does
// not expose. All this device can offer is what the pad calls itself, so that
// is what crosses — and the registry matches a name when no pair is given.

import GameController

/// What this device can say about a pad, for the registry to match on.
///
/// GameController exposes no USB vendor/product pair, so the registry has
/// to recognise the pad by name. Both names it offers are sent, because
/// the same controller is reported as "DualSense" by one and "DualSense
/// Wireless Controller" by the other depending on how it paired.
func padIdentity(_ controller: GCController) -> String {
    [controller.vendorName, controller.productCategory]
        .compactMap { $0 }
        .filter { !$0.isEmpty }
        .joined(separator: " ")
}
