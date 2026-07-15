# Fixture review record with two entries

This file exercises SPECIFIC-entry resolution: it holds one fully complete entry
and one still-pending entry. A review that names the pending entry must fail even
though a complete entry exists in the same file.

<a id="rec-complete"></a>
### rec-complete — a fully completed review entry

- reviewer: J. Doe
- date: 2026-07-14
- disposition: APPROVED
- covers: AIR-HAZ-012

<a id="rec-pending"></a>
### rec-pending — the entry this review names, still pending

- reviewer: PENDING
- date: PENDING
- disposition: PENDING
- covers: AIR-HAZ-012
