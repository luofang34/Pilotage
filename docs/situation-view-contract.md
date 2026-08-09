# SituationView V1 contract

This contract defines the read-only `SituationView` request and result.
It also defines the conformance corpus for each implementation.

## Scope

`SituationView` reads immutable domain snapshot handles.
It does not collect source data.
It does not decode source protocols.
It does not own mutable domain state.

The contract is source-neutral.
The contract does not use a provider type or a link-service type.
A domain core does not depend on this contract.

The Rust types are in `pilotage-situation-view`.
`SITUATION_VIEW_SCHEMA_VERSION` identifies the request and result schema.
V1 uses schema version `1`.

## Time query

The caller supplies one time axis and one UTC value.
The valid-time axis selects data that is in effect at the UTC value.
The knowledge-time axis selects data that the system held at the UTC value.
The caller must not use one UTC value for both axes.

The caller selects each required domain and snapshot subject.
The caller can also supply freshness requirements.
The caller can also supply coherence requirements.

The composition host attaches one monotonic evaluation stamp.
The stamp contains the host clock identity and a nanosecond value.
The result repeats the time axis, the UTC value, and the host stamp.

The caller uses `SituationViewQueryV1`.
The host uses `SituationViewRequestV1::attach` to make the request.

## Snapshot capture

The host calls the snapshot source one time for each selected domain subject.
The source returns one retainable immutable handle or one missing-data reason.
The host keeps the selection order in the result.

A valid-time source selects a handle for data that is in effect.
It can use a supplied fixed handle.
It can also capture its best available handle.

A knowledge-time source uses a retained handle ring or replay data.
It must not use a current handle as a fallback.
It returns `knowledge_state_unavailable` when the selected state is not
available.

## Consistency guarantee

V1 uses `best_available_non_atomic` consistency.
The host captures one immutable handle from each selected domain.
The host does not use one transaction across domains.
Two domain handles can represent different update times.

Each available domain result contains these values:

| Value | Requirement |
|---|---|
| Domain schema version | Identifies the complete domain record schema. |
| Producer instance ID | Identifies one continuous domain producer. |
| Snapshot revision | Orders states for one subject and producer. |
| Domain identity | Contains domain-specific identity when required. |
| Fields | Contains values and contributor evidence. |
| Clock correspondences | Maps contributor clocks to the host clock. |

A Navdata result uses the `navdata` domain identity.
It contains the navigation-data cycle, snapshot ID, and snapshot digest.
These values do not replace the producer instance ID or snapshot revision.

A missing domain result contains the domain, subject, and missing-data reason.
It does not make a producer ID or revision.

## Field evidence

Each field contains a present value or a missing-data reason.
The value uses JSON only as a source-neutral exchange value.
The domain schema defines the value meaning.

Each field contains a bounded contributor list.
The domain schema defines the contributor bound.
Each contributor contains these items:

- source identity;
- source epoch;
- local monotonic ingress time;
- source observation or product time;
- source time quality;
- validity interval;
- domain-owned data quality; and
- source time uncertainty.

Each item uses `EvidenceV1`.
`EvidenceV1` contains a value or an explicit missing-data reason.
The contract does not use a sentinel value for missing evidence.
A domain can supply a stable domain-specific reason code.

A source restart changes the source epoch.
It does not change the domain producer instance ID.
A domain producer restart changes the producer instance ID.

## Clock correspondence

`ClockCorrespondenceV1` maps one source monotonic clock to one target
monotonic clock.
It contains these values:

- source clock identity;
- target clock identity;
- signed nanosecond offset;
- symmetric uncertainty in nanoseconds; and
- an inclusive valid interval on the source clock.

The mapping is:

```text
target stamp = source stamp + offset
```

The host uses the mapping only when the source stamp is in the valid interval.
One source stamp must have no more than one valid mapping to the host clock.
An implementation can use short valid intervals to limit clock drift.
It can publish a new mapping when the offset or uncertainty changes.

Stamps on the same clock do not need a correspondence.
Their mapping uncertainty is zero.

The ingress age is:

```text
host evaluation stamp - mapped ingress stamp
```

The observation age is:

```text
evaluation UTC - source observation time
```

The result reports the two ages separately.
A known age contains its nanosecond value and uncertainty evidence.

The result reports ingress age as unknown in these conditions:

- the ingress stamp is missing;
- a required correspondence is missing;
- the source stamp is outside the valid interval;
- more than one valid correspondence applies;
- the mapping cannot make a target stamp; or
- the mapped ingress stamp is after the host evaluation stamp.

The result reports observation age as unknown in these conditions:

- the source time is missing;
- the source time quality is missing or untrusted;
- a UTC value is invalid; or
- the source time is after the evaluation UTC.

The view does not compare raw stamps from different clocks.

## Freshness requirements

A freshness requirement selects one field and one age type.
It also supplies a maximum age.

The view checks all contributors for the selected field.
The view adds the age uncertainty to the age value.
The requirement is satisfied only when each upper bound is not more than the
maximum age.

The requirement is not satisfied when a domain or field is missing.
The requirement is not satisfied when an upper bound is more than the maximum.
The result is indeterminate when an age or its uncertainty is unknown.

## Coherence requirements

V1 defines three coherence rules.

`maximum_ingress_age_spread` limits the possible age spread for selected
fields.
The assessment includes each contributor uncertainty.

`equal_field_values` requires equal JSON values for selected fields.
This rule can require one navigation-data cycle in a rendered composition.

`exact_snapshots` requires stated producer instance IDs and revisions.
The rule does not compare revisions from different producer instances.

Each requirement has a caller-owned ID.
The result contains one assessment for each ID.
The result keeps request order.

## Conformance corpus

The shared corpus is
`crates/pilotage-situation-view/corpus/situation-view-v1.json`.
`SITUATION_VIEW_CORPUS_VERSION` identifies its schema.
V1 uses corpus version `1`.

The corpus contains these exact cases:

| Case | Required behavior |
|---|---|
| `mixed_age_inputs` | Report different ingress and observation ages. Assess failed freshness and coherence limits. |
| `unknown_age` | Report unknown ingress and observation ages when time evidence is not valid. |
| `missing_domain` | Keep the selected domain and report an explicit reason. |
| `source_restart` | Keep one producer identity and two source epochs. |
| `clock_correspondence_uncertainty` | Apply the valid clock mapping and include its uncertainty in freshness. |

Each case contains a host-attached request, domain capture states, and one exact
required result.
An implementation test installs the capture states and runs the request.
It calls `verify_corpus_v1` to compare the complete result.

CI runs the corpus for the reference composer.
Each additional implementation must add a test that calls the same corpus
runner.
