# Progress

## 2026-09-25
- Researched anchors, testkit, host; drafted plan 051; indexed. Not committed.
- Committed 9ae931a3 (plan) + d68369d6 (notes); created issue #119; plan links it. Not implemented per user.

## 2026-09-25 implementation (thread t-0054)
- S0 done: scripts/testing/resolve-desktop-anchors.mjs -> tests/upgrade-matrix/anchors.json (13 anchors, 14 releases) + expectations.json (457 cells). Re-run check = no drift.
- S1: scripts/testing/build-upgrade-anchors.sh caches per rev under <target>/upgrade-anchors/<rev>/bin; build dir reclaimed after copy.
  Measured: a13 29s (warm sccache); fresh anchors 2-5.5 min each; binary ~170MB.
- Host API layering: 7 capability features (host-features.json), no patches needed so far.
- a01/a02 cannot disable default N0 relay/pkarr at startup (no with_test_relay_fallback) -> host refuses start (local_network_unavailable) -> skip per plan risk row.
- S2/S3 harness written (tests/upgrade-matrix crate); runner group upgrade-matrix; summarize script; rc15 flow switched to build script (a07).
