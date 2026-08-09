# Domain snapshot envelope contract

This contract defines the common envelope for an immutable domain snapshot. It
supplies the immutable snapshot part of migration step 1 in ADR-0036.

Surveillance supplies the reference shape. Its `TrackSnapshot` is an immutable
domain value. Its `TimedField<T>` keeps time, quality, and provenance with each
field. Its `TrackRecord` uses `CURRENT_TRACK_SCHEMA_VERSION` to validate the
exported record before a reader interprets the payload.

This contract keeps that shape. It adds a producer instance ID, a snapshot
revision, a retainable handle, and an explicit reason for an absent value.

## Scope

A domain snapshot is one immutable value that a domain producer publishes. The
domain defines the snapshot payload and its subject identity. For example,
Surveillance uses `TrackSnapshot` and `TrackId` for one track.

A snapshot envelope binds the domain snapshot to the producer instance and the
snapshot revision. A snapshot handle gives a consumer retainable access to one
snapshot envelope.

This contract does not define a domain ingress value. It does not define
`SituationView`, a renderer, or a transport encoding. Each domain owns these
types at its boundary.

## Contract shape

A snapshot envelope has these required elements.

| Element | Requirement |
|---|---|
| Producer instance ID | Identifies one continuous domain producer instance. |
| Snapshot revision | Orders published states of one snapshot subject from that producer instance. |
| Domain snapshot | Contains the immutable, domain-owned payload and its subject identity. |

A versioned domain record has this relation to the envelope:

```text
versioned domain record
├── domain schema version
└── snapshot envelope
    ├── producer instance ID
    ├── snapshot revision
    └── immutable domain snapshot
```

The record and the in-process handle can use different representations. They
must keep the same identity, revision, and snapshot semantics.

## Producer instance ID

A producer instance ID is an opaque equality value. A construction API must
give a new ID to each new producer instance.

The ID stays constant for the life of that producer instance. A restarted or
reconstructed producer is a new producer instance. It must use a new ID. This
rule also applies when the new producer restores equivalent domain state.

Two producer instances must not use the same ID when their handles or exported
records can be compared. The contract does not specify the ID generation
method.

A loss of revision continuity creates a new producer instance. Revision
continuity is lost after a restart, reconstruction, revision rollback, or
revision counter exhaustion. The new producer instance must use a new ID.

A source restart does not change the domain producer instance ID. The source
epoch in field provenance identifies a source restart. A producer instance ID
does not identify a source, a host, a schema, or a domain subject.

A consumer uses producer instance IDs only for equality. A consumer must not
order the IDs or derive time from them.

## Snapshot revision

A snapshot revision is a monotonic counter for one snapshot subject from one
producer instance. A domain that publishes more than one subject must put the
subject identity in its snapshot payload.

The producer increments the revision once for each new published state of that
subject. The producer keeps the revision when a read returns an unchanged
state. The producer also keeps the revision when internal work does not change
the published state.

A change to any published member creates a new published state. This rule
includes a change to a value, time evidence, quality, provenance, or absence
reason.

The first revision value has no special meaning. A consumer must not assume a
start value.

A finite counter must not wrap under the same producer instance ID. The
producer must end the producer instance before the next revision wraps. A Rust
implementation must calculate the next value with `wrapping_add(1)`. It must
detect a wrapped value and must not publish that value. A new producer instance
can publish the state with a new producer instance ID.

A consumer compares revisions only when the producer instance ID and the
domain subject identity are equal. Equal revisions identify the same published
state. A larger revision identifies a newer published state. Revisions from
different producer instances or different subjects have no order.

## Retainable immutable handle

A producer must return a handle that a consumer can retain without a borrow of
the producer's mutable state. The handle can own the envelope or use an
immutable reference-counted representation.

The capture operation must bind the producer instance ID, the snapshot
revision, and the domain snapshot from one published state. A consumer must not
assemble these elements with separate producer calls.

The producer must not change a retained handle. The handle stays unchanged
when the producer accepts input, advances time, publishes another state, or
ends. A repeated capture of an unchanged state can return the same handle or an
equal handle.

The contract does not require one allocation method. The domain can use an
owned value, shared ownership, or another immutable representation.

An issued handle stays valid and unchanged while the consumer retains it. A
producer can reject a new capture when a declared capacity bound is full. It
must not invalidate an issued handle to satisfy that bound.

## Per-field timed provenance

Each source-derived domain field has a present state or an absent state. A
present state uses the `TimedField<T>` pattern from Surveillance. It keeps these
elements together:

- the field value;
- the time evidence for that value;
- the quality evidence for that value;
- the provenance for that value.

The local monotonic ingress stamp is the clock value recorded when data enters
the runtime. Time evidence includes this stamp. It also keeps the source
observation time or product time when the source supplies one.

The time basis states how the producer established the source time. Time
uncertainty states the error bound for a mapped source time. The time evidence
states that the source time is unknown when no valid source time exists.

A clock correspondence maps values between two clocks. The enclosing record or
stream identifies the monotonic clock. It supplies a clock correspondence when
a consumer must translate the stamp to another clock.

Provenance identifies the source and the source epoch. A source epoch identifies
one continuous source instance. The origin states what produced the evidence.
The delivery path states how the evidence reached the runtime. A domain can add
a domain term such as the Surveillance address namespace.

The Surveillance `FieldProvenance` pattern identifies the one observation that
supplies a fused field. A domain can define a bounded set when more than one
source supplies one value. The domain schema must define the set and its bound.

Quality is domain-owned evidence. A domain must keep an unavailable quality
indicator unavailable. It must not make a quality value that the source did
not supply.

A field update must not refresh another field. Each source-derived field keeps
the time, quality, and provenance of the evidence that supplies that field.
Producer-assigned identity and lifecycle fields do not need a `TimedField<T>`
wrapper.

## Absent values

An absent state contains an absence reason instead of a value. A missing member,
`None`, zero, an empty string, `NaN`, or another sentinel is not a complete
absence representation.

Each domain schema must represent at least these absence reasons when they can
apply:

| Reason | Meaning |
|---|---|
| Not observed | The producer has not accepted evidence for the field. |
| Source reported unavailable | A source explicitly reported no value or no data. |
| Expired | An accepted value passed its freshness or validity limit. |
| Rejected | Candidate evidence failed a domain validation or acceptance rule. |
| Not applicable | The field has no meaning for this snapshot subject. |
| Unsupported | The producer cannot interpret or represent the source value. |
| Resource limit | A declared bound prevented retention of the value. |

A domain can add a more specific absence reason. The domain schema defines the
additional reason. A domain must not use an unspecified reason when a listed or
domain-specific reason applies.

The absence reason describes the current published state. It does not replace
an event log or the source recording.

An absent state keeps time and provenance when source evidence or a lifecycle
decision supplies the absence reason. For example, a source-reported
unavailable state identifies the source and the time of its report. An absent
state must not contain a fabricated value, time, quality, or provenance. It
states unknown evidence explicitly when no evidence exists.

## Domain schema version

Each domain owns its exported schema and its schema version. The common
snapshot envelope does not have a separate global schema version.

The domain schema version covers the complete serialized record. This scope
includes the producer instance ID, the snapshot revision, the domain snapshot,
the timed field representation, and the absence representation.

A serialized record must expose a version header that a reader can validate
before it interprets the envelope or payload. A reader must return a typed
version error for an unsupported version. This rule follows the Surveillance
`TrackRecordHeader` pattern.

A domain must increase its schema version when an envelope or payload change
can make an older reader interpret the record incorrectly. A new absence
variant needs a version increase when an older reader can receive that variant.

A schema version identifies a record format. It does not identify a producer
instance or a published state. A producer restart does not by itself change the
schema version. A snapshot publication does not change the schema version.

A composition schema can have its own version. That version does not replace
the schema version of an embedded domain record. Each domain can change its
schema on its own release schedule.

## Conformance boundary

An adopting domain must test the identity, revision, retention, field evidence,
absence, and schema rules at its public boundary.

ADR-0036 migration step 2 defines the `SituationView` contract and the shared
conformance corpus. This contract supplies the domain shape that the corpus
will consume. It does not define corpus fixtures or required composed results.
