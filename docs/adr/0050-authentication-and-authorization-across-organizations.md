# ADR-0050: Authentication and authorization across units and organizations

- Status: Proposed
- Date: 2026-09-23

## Context

ADR-0049 keeps shared records and authority ready for collaboration. It says
that a host verifies a foreign issuer against trust anchors, and that a vehicle
owner grants rights to a foreign principal. It does not define the trust
anchor, the grant, or how roles relate to scopes.

These records already apply:

- A passkey authenticates a person. A short-lived session capability binds a
  principal to a session and to the scopes that the principal can request
  (ADR-0006).
- A lease gives real-time authority for one scope, with a fencing generation.
  `AuthorityClass` names the class of a principal: operator, supervisor,
  administrator, or automation (ADR-0006, ADR-0010). The authority engine
  records the class. It does not enforce an order of classes.
- The coordination server is optional. It gives identity and admission with
  passkeys, rendezvous, and entitlements. It never carries session data
  (ADR-0027).

An operator terminal can control vehicles of other units, or of other
organizations, within the rights that the owner grants. A person can belong to
more than one organization.

## Decision

### 1. Identities

- A **principal** is an issuer and a subject at that issuer. People, hosts, and
  agents are principals.
- People authenticate with passkeys at their own organization's issuer.
  Hosts and agents authenticate with keys that their organization issues.
- An **organization** is identified by its issuer. An individual with no
  organization is a principal with a personal issuer.
- An organization's issuer can be its coordination server (ADR-0027). The
  server then issues identities only inside that issuer. It cannot add an
  issuer to the policy of a host.
- Session-local integers stay on the real-time wire. The session maps them to
  issuer-qualified identities (ADR-0049 rule 1). This mapping is a dependency of
  this record.

### 2. Trust anchors

- A **trust anchor** record names a foreign issuer, its public key, and the
  validity period of the key. The host operator installs it out of band.
- The **host policy** pins the set of issuers that the host trusts. It lists
  the installed anchors. An administrator of the organization that owns the
  host signs the host policy.
- There is no automatic discovery and no transitive trust. A host trusts an
  issuer only when its own policy lists that issuer.
- Key rotation installs the new key before the old key expires. Removing an
  anchor revokes every grant that depends on it at the next check.

### 3. Scope grants

A **scope grant** is the record that authorizes a foreign or local principal
before any lease exists. It is a record in `pilotage-authority`, next to the
authority engine:

| Field | Meaning |
|---|---|
| `grantor` | The owning organization and the principal that signs the grant |
| `grantee` | A principal, a role in an organization, or an organization |
| `vehicles` | One vehicle, a named fleet, or a coordinator's aggregate scope (ADR-0028) |
| `scopes` | The scopes that the grantee can request |
| `permitted_classes` | The `AuthorityClass` values that the grantee can hold |
| `validity` | Start and end times |
| `release_filter` | The releasability that the grantee's session receives (ADR-0049 rule 2) |
| `delegable` | Whether the grantee can grant a subset to another principal, and to what depth |
| `revision` | Revision and previous-revision digest, as for other shared records |

The host issues a session capability only inside an active grant. Leases,
fencing, handover, and override (ADR-0006, ADR-0010) then apply unchanged. A
grant never gives a lease directly.

### 4. Roles map to scopes

An organization defines roles, such as "pursuit coordinator" or "observer". A
role is a named set of scopes and a set of permitted classes. A grant can name a role.
The host expands the role to scopes when it issues the capability. The wire and
the authority engine see only scopes. Fencing does not change.

### 5. Shared records: the owner filters

The owner filters a shared record by its releasability before the record
leaves the owner's organization (ADR-0049 rule 2). A receiver never evaluates
the owner's policy, and no reader-side access list exists. A receiver can
forward a record only when the record's releasability allows it.

### 6. Revocation and link loss

- A session capability is short-lived. A revoked grant stops new capabilities
  immediately and ends existing capabilities at their expiry.
- An urgent revocation also ends the grantee's leases. The generation advances,
  so stale frames are refused (ADR-0006).
- A host that loses contact with the grantor's issuer keeps enforcing its local
  grants until they expire. It does not extend them.

### 7. Audit

Each grant, capability, lease change, and intent is a session event (ADR-0012)
with issuer-qualified identities. An organization can audit what a foreign
principal did on its vehicle.

### 8. Interoperability with military symbology and scenario tools

Shared marks and target tracks carry a symbol identification code (SIDC) of
MIL-STD-2525 or APP-6. Tools such as ORBAT Mapper and TacTrace use this
vocabulary.

- The TacTrace application has no license and no public source. Pilotage does
  not use its code or its `.tactrace` file format.
- These libraries are published under the MIT license: `milsymbol`,
  `@orbat-mapper/control-measures`, `@orbat-mapper/tactical-draw`,
  `@orbat-mapper/tactical-draw-adapter-maplibre`, `@orbat-mapper/msdllib`, and
  the ORBAT Mapper application. The dependencies of `msdllib` use MIT or
  BSD-3-Clause licenses. The web client can use these packages with their
  license notices.
- Native clients implement symbol rendering from the SIDC themselves, or render
  sprites that the web libraries make.
- Exchange with these tools uses open formats: GeoJSON features with SIDC
  properties, and the Military Scenario Definition Language (MSDL). The native
  records stay authoritative. Import and export are adapters.

## Consequences

- Cross-organization control needs no new authority primitive. A grant limits
  what a principal can request, and the existing lease model controls what it
  holds.
- A host decides trust from its own signed policy. A compromised coordination
  server cannot add an issuer to a host.
- The trust-anchor record, the scope-grant record, role expansion, and the
  GeoJSON and MSDL adapters are deferred implementation work. They follow the
  identity mapping of ADR-0049.
