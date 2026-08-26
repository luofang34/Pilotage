// The operator's control-feel choice: what a client may offer, and what it
// puts on the wire when a reader chooses.
//
// Run: node clients/web/feel-mode.test.mjs

import assert from "node:assert/strict";

import { feelModeAdvertised } from "./typed-command.js";
import { CONTROL_ACTION, FEEL_TARGET, encodeControlActionCommandEnvelope } from "./wire.js";

const SCOPE = "vehicle.motion";
const VEHICLE = 7n;

const scopesAdvertising = (feelTargets) => [
  {
    vehicleId: VEHICLE,
    scope: SCOPE,
    intents: [],
    actions: [
      { action: CONTROL_ACTION.arm, modeTargets: [], feelTargets: [] },
      { action: CONTROL_ACTION.feelModeRequest, modeTargets: [], feelTargets },
    ],
  },
];

function testAClientOffersOnlyTheLawsTheVehicleQualified() {
  // A control that asks for a law the vehicle has not qualified is a control
  // that always fails. The offer follows the advertisement, never the page.
  const advertised = scopesAdvertising([FEEL_TARGET.balanced, FEEL_TARGET.agile]);
  assert.ok(feelModeAdvertised(advertised, VEHICLE, SCOPE, FEEL_TARGET.balanced));
  assert.ok(feelModeAdvertised(advertised, VEHICLE, SCOPE, FEEL_TARGET.agile));
  assert.ok(
    !feelModeAdvertised(advertised, VEHICLE, SCOPE, FEEL_TARGET.precision),
    "a law the vehicle did not advertise is not offered",
  );

  // A vehicle that advertises the action with no targets offers nothing.
  assert.ok(!feelModeAdvertised(scopesAdvertising([]), VEHICLE, SCOPE, FEEL_TARGET.balanced));
}
testAClientOffersOnlyTheLawsTheVehicleQualified();
console.log("ok - testAClientOffersOnlyTheLawsTheVehicleQualified");

function testAVehicleWithNoFeelActionOffersNothing() {
  // Silence is not consent: a vehicle that never mentions the action has not
  // qualified any law, and the control stays closed.
  const silent = [
    {
      vehicleId: VEHICLE,
      scope: SCOPE,
      intents: [],
      actions: [{ action: CONTROL_ACTION.arm, modeTargets: [], feelTargets: [] }],
    },
  ];
  for (const target of Object.values(FEEL_TARGET)) {
    assert.ok(!feelModeAdvertised(silent, VEHICLE, SCOPE, target));
  }
}
testAVehicleWithNoFeelActionOffersNothing();
console.log("ok - testAVehicleWithNoFeelActionOffersNothing");

/** Reads protobuf fields out of one message body. */
function fieldsOf(bytes) {
  const found = new Map();
  let index = 0;
  const varint = () => {
    let value = 0;
    let shift = 0;
    for (; index < bytes.length; index += 1) {
      const byte = bytes[index];
      value += (byte & 0x7f) * 2 ** shift;
      shift += 7;
      if ((byte & 0x80) === 0) {
        index += 1;
        return value;
      }
    }
    return value;
  };
  while (index < bytes.length) {
    const tag = varint();
    const number = tag >>> 3;
    const wire = tag & 0x07;
    if (wire === 0) {
      found.set(number, varint());
    } else if (wire === 2) {
      const length = varint();
      found.set(number, bytes.slice(index, index + length));
      index += length;
    } else {
      break;
    }
  }
  return found;
}

function testTheRequestReachesTheWireWithItsTarget() {
  // The client never decodes its own command, so the bytes are parsed here.
  // The target rides field 4 of the nested ControlActionRequest; a feel
  // request arriving with a flight-mode target in field 2 instead would be a
  // sender and a receiver disagreeing about what was asked for, and the host
  // refuses that rather than reading past it.
  const request = (feelTarget) => {
    const envelope = encodeControlActionCommandEnvelope({
      sessionId: 11n,
      vehicleId: VEHICLE,
      scope: SCOPE,
      generation: 3n,
      activationRevision: 2,
      action: CONTROL_ACTION.feelModeRequest,
      feelTarget,
      actionId: 42,
    });
    // The envelope wraps the command, which carries the request at field 6.
    const command = [...fieldsOf(envelope).values()].find((value) => value instanceof Uint8Array);
    return fieldsOf(fieldsOf(command).get(6));
  };

  for (const [name, target] of Object.entries(FEEL_TARGET)) {
    const fields = request(target);
    assert.equal(fields.get(1), CONTROL_ACTION.feelModeRequest, `${name} action`);
    assert.equal(fields.get(4), target, `${name} reaches the wire`);
    assert.equal(fields.get(2), undefined, `${name} carries no flight-mode target`);
    assert.equal(fields.get(3), 42, `${name} keeps its correlation id`);
  }

  // A request with no target states none rather than defaulting to one: the
  // vehicle refuses an unspecified target instead of guessing which law to
  // install.
  assert.equal(request(undefined).get(4), undefined);
}
testTheRequestReachesTheWireWithItsTarget();
console.log("ok - testTheRequestReachesTheWireWithItsTarget");

function testTheThreeLawsAreDistinctAndStable() {
  // The wire values are a contract with the vehicle. Renumbering one would
  // silently install a different law than the reader chose.
  assert.deepEqual(
    [FEEL_TARGET.precision, FEEL_TARGET.balanced, FEEL_TARGET.agile],
    [1, 2, 3],
  );
  assert.equal(CONTROL_ACTION.feelModeRequest, 8);
}
testTheThreeLawsAreDistinctAndStable();
console.log("ok - testTheThreeLawsAreDistinctAndStable");

console.log("\nall control-feel mode checks passed");
