# SIM / NOT FOR FLIGHT — self-test fixture matrix with a mismatched authority status
#
# STD-061's "Authority status" cell says `requires authority agreement` while
# registry-good.toml records it as `authority-accepted`. The guard must FAIL on
# the exact-match status comparison.

| ID | Standard / reference | Selected revision | Authority status | Rationale |
| --- | --- | --- | --- | --- |
| STD-061 | [FAA AC 20-167B](https://www.faa.gov/regulations_policies/advisory_circulars/index.cfm/go/document.information/documentID/1044323) — EVS/EFVS/CVS | AC 20-167B (guidance) | `requires authority agreement` | Wrong authority status. |
| STD-062 | [FAA AC 20-185A](https://www.faa.gov/regulations_policies/advisory_circulars/index.cfm/go/document.information/documentID/1039337) — SVS/SVGS/ASA-SVS | AC 20-185A (guidance) | `authority-accepted` | AC 20-185A supersedes AC 20-185. |
| STD-066 | [RTCA DO-407](https://www.rtca.org/news/new-rtca-technical-products-address-global-aviation-functions-and-performance/) / [EUROCAE ED-326](https://www.eurocae.net/product/ed-326-masps-for-svs-svgs-cvs/) — MASPS | DO-407 / ED-326 | `latest engineering baseline` | Released MASPS; FAA recognition unresolved. |
