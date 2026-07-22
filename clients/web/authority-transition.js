const APPLIED_LABEL = {
  grant: ({ generation }) => `granted gen=${generation}`,
  denial: ({ reason }) => `denied (terminal)${reason === undefined ? "" : ` reason=${reason}`}`,
  release: ({ generation }) => `released fence=${generation}`,
  revocation: ({ generation }) => `revoked fence=${generation}`,
  recovery: ({ generation }) => `recovered gen=${generation}`,
  uplinkIdle: () => "needs arm",
  actionResult: ({ detail }) => (detail === 1 ? "armed" : "needs arm"),
};

/** Applies one reliable transition and reports only table-confirmed changes. */
export function applyAuthorityTransition(control, log, scope, kind, details = {}) {
  const disposition = control?.authorityEvent(scope, kind, details) ?? "ignored";
  if (disposition === "stale") {
    log(`authority[${scope}]: STALE ${kind} gen=${details.generation}`);
  } else if (disposition === "applied") {
    const describe = APPLIED_LABEL[kind];
    log(`authority[${scope}]: ${describe ? describe(details) : kind}`);
  }
  return disposition;
}
