// The control-feel selector: which laws it offers, and which one it says the
// vehicle is on.
//
// The two are separate questions and the module keeps them separate. What is
// OFFERED comes from the vehicle's advertisement, so a control never asks for
// a law nobody qualified. What is SHOWN comes from what the vehicle confirmed,
// never from what the reader last pressed — a refused request that left the
// selector where the reader put it would have the display assert a law the
// vehicle declined.

import { feelModeAdvertised } from "./typed-command.js";

/**
 * Wires one `<select>` to the laws a vehicle advertises.
 *
 * `advertisement` is read on every refresh rather than captured, because the
 * scopes a vehicle advertises change during a session.
 */
export function feelModeControl(control, advertisement) {
  // The law the VEHICLE last confirmed. Null until it confirms one: the
  // adapter boots on the compatibility law and says so only when asked, so
  // naming a law before then would state one nobody has stood behind.
  let confirmed = null;

  /// Offers only the laws this vehicle advertises.
  ///
  /// A control that asks for a law the vehicle has not qualified is a control
  /// that always fails, so each option is enabled by the advertisement rather
  /// than by hope, and the whole control stays disabled until one arrives.
  const refresh = () => {
    if (!control) return 0;
    const { scopes, vehicleId, scope } = advertisement();
    let offered = 0;
    for (const option of control.options) {
      if (option.value === "") continue;
      const target = Number(option.value);
      const advertised = feelModeAdvertised(scopes, vehicleId, scope, target);
      option.disabled = !advertised;
      if (advertised) offered += 1;
    }
    control.disabled = offered === 0;
    // A disabled option still displays when it is the selected one, so a law
    // this scope does not offer would keep showing as current. Fall back to
    // stating none rather than naming one the vehicle will refuse.
    if (control.selectedOptions[0]?.disabled) {
      control.value = "";
      confirmed = null;
    }
    control.title = control.disabled
      ? "This vehicle advertises no control-feel modes"
      : "How the demand is shaped on its way to the vehicle";
    return offered;
  };

  /// Shows the law the vehicle is on.
  ///
  /// `null` means the vehicle did not accept the last request, so the control
  /// returns to the law still installed rather than keeping one the vehicle
  /// refused.
  ///
  /// Before the vehicle has confirmed any law there is nothing to return to,
  /// and the control states none. Leaving it alone in that case would keep the
  /// reader's refused choice on screen for the FIRST refusal — the very case
  /// this exists to close, and the one a session opens in.
  const show = (feelTarget) => {
    if (!control) return;
    if (feelTarget === null) {
      control.value = confirmed ? String(confirmed) : "";
      return;
    }
    confirmed = feelTarget;
    control.value = String(feelTarget);
  };

  return { refresh, show, confirmed: () => confirmed };
}
