# SIM / NOT FOR FLIGHT — self-test fixture matrix with the Authority status column REMOVED
#
# The governed rows carry the correct selected revisions and URLs, but the
# "Authority status" column is gone entirely. The guard must FAIL rather than
# pass: it cannot verify that the matrix agrees with the registry's authority
# status when the column is absent.

| ID | Standard / reference | Selected revision | Rationale |
| --- | --- | --- | --- |
| STD-061 | [FAA AC 20-167B](https://www.faa.gov/regulations_policies/advisory_circulars/index.cfm/go/document.information/documentID/1044323) — EVS/EFVS/CVS | AC 20-167B (guidance) | AC 20-167B supersedes AC 20-167A. |
| STD-062 | [FAA AC 20-185A](https://www.faa.gov/regulations_policies/advisory_circulars/index.cfm/go/document.information/documentID/1039337) — SVS/SVGS/ASA-SVS | AC 20-185A (guidance) | AC 20-185A supersedes AC 20-185. |
| STD-066 | [RTCA DO-407](https://www.rtca.org/news/new-rtca-technical-products-address-global-aviation-functions-and-performance/) / [EUROCAE ED-326](https://www.eurocae.net/product/ed-326-masps-for-svs-svgs-cvs/) — MASPS | DO-407 / ED-326 | Released MASPS; FAA recognition unresolved. |
