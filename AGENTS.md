# AGENTS Guide for fde-rs

This repository is the standalone Rust 2024 implementation flow for FDE.

## Repository Facts

- Primary product shape: Rust library first, CLI second.
- Primary executable: `fde`.
- Primary frontend assumption: Yosys produces EDIF; this repo consumes EDIF and downstream IR.
- This repo is independent from the historical C++ monolith. Do not reintroduce old mixed-repository assumptions or a single giant pipeline module.
- Determinism matters: fixed seeds should give reproducible output, even if internal work is parallelized.

## Architectural Direction

- Shared typed IR lives in Rust and is reused across all stages.
- Stage logic belongs in focused modules like `map`, `pack`, `place`, `route`, `sta`, `bitgen`, `normalize`, `orchestrator`.
- CLI code should stay thin: argument parsing, file orchestration, report writing, progress output.
- Follow the refactor plan in `docs/refactor-plan.md` when reshaping modules.
- Keep `src/` top-level compact by grouping modules under `app/`, `core/`, `infra/`, and `stages/` instead of adding more root directories.

## Scope Boundaries

- Verilog import is intentionally minimal. Prefer failing clearly and telling the user to run Yosys.
- `bitgen` materializes CIL-backed site SRAM images for supported logic/IO/clock sites and stays within the Rust implementation flow.
- Hardware XML compatibility matters. Reuse established FDE hardware XML conventions and invocation shapes where practical.

## Commands

- Build: `cargo build`
- Check: `cargo check`
- Test: `cargo test`
- CI parity: `cargo fmt --all -- --check && cargo check --locked --all-targets && cargo clippy --locked --all-targets --all-features -- -D warnings && cargo test --locked --quiet`
- CI smoke: `cargo run --locked --quiet --bin fde -- impl --input examples/blinky/blinky.edf --constraints examples/blinky/constraints.xml --resource-root resources/hw_lib --out-dir /tmp/fde-rs-ci-smoke`
- Board EDF dry run: `find examples/board-e2e -mindepth 2 -maxdepth 2 -name '*.edf' | sort | while read -r edf; do case_dir=$(dirname "${edf}"); name=$(basename "${case_dir}"); cargo run --bin fde -- impl --input "${edf}" --constraints "${case_dir}/constraints.xml" --resource-root resources/hw_lib --out-dir "build/board-e2e/${name}"; done`
- Live board run: `python3 scripts/board_e2e.py run`
- Live board single-case run: `python3 scripts/board_e2e.py run sticky16-check`
- Board baseline diff: `python3 scripts/board_diff.py run`
- Random board/model diff: `python3 scripts/random_board_diff.py --count 5 --seed 20260322 --keep-going`
- Slice config diff: `python3 scripts/slice_config_diff.py --packed <02-packed.xml> --sidecar <06-output.bit.txt>`
- Aspen-style Verilog->EDF synthesis: `python3 scripts/synth_yosys_fde.py --top <top> --out-edf build/<top>.edf <sources...>`
- In-repo board probe: `cargo run --manifest-path tools/wave_probe/Cargo.toml -- <bitstream>`
- Main help: `cargo run --bin fde -- --help`
- End-to-end smoke: `cargo run --bin fde -- impl --input examples/blinky/blinky.edf --constraints examples/blinky/constraints.xml --resource-root tests/fixtures/hw_lib --out-dir build/blinky-run`

## Editing Guidance

- Keep ASCII unless the file already requires something else.
- Prefer small stage-focused modules over broad refactors that blur responsibilities.
- Do not silently swallow missing resource/config inputs; either derive a safe default or surface a clear error.
- When adding new tooling, update this file and `README.md` in the same change.
- Keep checked-in board regression netlists in EDF form under `examples/board-e2e/`; do not commit temporary synthesis-only Verilog there.
- Keep board-specific long-cycle probe overrides in `examples/board-e2e/manifest.json` so regressions remain reproducible from the checked-in manifest.
- Keep string handling at parsing and reporting boundaries; do not add new raw string branching in core stage logic when a typed enum or helper can model it.
- Prefer semantic helper modules in `domain/` over repeating `eq_ignore_ascii_case`, `to_ascii_lowercase`, or string literal matches across stage code.

## Active Debug Handoff (2026-03-24)

This section records the current board-debug state so a new contributor can continue
without redoing the same investigation. It is intentionally detailed and should be
updated when the root-cause status changes.

### Scope of this handoff

- The original failures were first surfaced by `case008` and random board/model
  diffs.
- The most actionable current repros are the board cases:
  - `ff-data-check`
  - `ff-slot1-check`
  - `ff-shift2-check`
  - `ff-clock-check`
- The current situation is:
  - some real root causes have already been fixed in Rust;
  - some plausible hardware theories have already been ruled out with board A/Bs;
  - the focused FF board cases now match the sibling C++ implementation once the
    checked-in constraints are corrected to use `P77`;
  - remaining live-board work should move on to other cases or random regressions.

### Update (2026-04-01)

The previously focused FF board cases:

- `ff-data-check`
- `ff-slot1-check`
- `ff-shift2-check`
- `ff-clock-check`

now match the sibling `../FDE-Source` C++ probe logs on hardware.

What changed:

- physical LUT normalization now prefers raw `init` and expands to full site width;
- dual LUT/FF pair slot assignment now preserves cluster pair order;
- the checked-in clock constraints for those four cases now use `P77` instead of
  `P150`.

Verification status:

- Rust live-board outputs match the C++ probe logs for all four cases.
- `examples/board-e2e/manifest.json` has been updated to those confirmed sequences.

### Update (2026-04-03) - random board diff `01352633`

The current random failing case under focused investigation is:

- `build/random-board-diff/random-diff-000-01352633/`

New conclusions from this debug round:

#### 1) The failing `01352633` implementation is logically correct in routed simulation

This was verified on the Rust-placed / C++-routed design:

- placed:
  - `build/random-stage-compare-01352633/61-current-place-cpp-tail/03-placed.xml`
- routed verilog:
  - `build/random-stage-compare-01352633/61-current-place-cpp-tail/rtv/rt.v`

Commands used:

- generate routed verilog:
  - `../FDE-Source/build/bin/route -n build/random-stage-compare-01352633/61-current-place-cpp-tail/03-placed.xml -a resources/hw_lib/fdp3p7_arch.xml -i resources/hw_lib/fdp3p7_cil.xml -o build/random-stage-compare-01352633/61-current-place-cpp-tail/rtv/04-routed.xml -v build/random-stage-compare-01352633/61-current-place-cpp-tail/rtv/rt.v`
- simulate:
  - `iverilog -g2005-sv -o build/random-stage-compare-01352633/61-current-place-cpp-tail/rtv/sim/a.out build/random-cpp-pr-013525e2/ROUTING_SIMLIB_FDP3P7.patched.v build/random-stage-compare-01352633/61-current-place-cpp-tail/rtv/rt.v build/random-board-diff/random-diff-000-01352633/golden_tb.v`
  - `vvp build/random-stage-compare-01352633/61-current-place-cpp-tail/rtv/sim/a.out`

Observed result:

- routed simulation matches the generated golden sequence exactly.

Implication:

- the remaining failure is not "the routed netlist computes the wrong logic" in ordinary functional simulation;
- the mismatch is in hardware-facing semantics after / around physical XML -> programmed bitstream interpretation.

#### 2) Replacing Rust route with C++ route still gives the exact same bad board sequence

This was rechecked by running Rust bitgen on the C++-routed design:

- bitstream:
  - `build/random-stage-compare-01352633/62-current-place-cpp-route-rust-bitgen/06-output.bit`
- probe log:
  - `build/random-stage-compare-01352633/62-current-place-cpp-route-rust-bitgen/wave_probe.log`

Observed result:

- the board sequence is byte-for-byte identical to the current bad sequence from the normal Rust flow.

Implication:

- the active bug is not in the current Rust router's chosen path shape alone;
- route changes that preserve the same higher-level placed / packed semantics do not fix the board.

#### 3) Moving the suspicious S1 combinational-only slices did not change the board at all

Targeted A/Bs performed:

- move only `iSlice__8__`:
  - `build/random-stage-compare-01352633/63-move-islice8/`
- move both `iSlice__4__` and `iSlice__8__`:
  - `build/random-stage-compare-01352633/64-move-both-s1comb/`

In both A/Bs:

- C++ route regenerated a different routed design;
- routed simulation stayed correct;
- hardware probe output stayed exactly the same wrong sequence.

Implication:

- the failure is not explained by a specific same-tile coupling or specific route geometry for those two S1 combinational sites.

#### 4) Narrow-LUT semantics matter, but there is no single global "zero-extend everything" fix

Evidence:

- Rust bitgen currently imports narrow routed LUT functions as logical `LUT2` / `LUT3` values such as:
  - `0x1`
  - `0x6`
  - `0x9`
  - `0x14`
  - `0x96`
  - `0xBE`
- then expands them back to site width by repetition, yielding values like:
  - `0x1111`
  - `0x6666`
  - `0x9999`
  - `0x1414`
  - `0x9696`
  - `0xBEBE`

This differs from the routed simulation Verilog emitted by C++ route, which uses narrow `16'h...` literals such as:

- `16'h1`
- `16'h6`
- `16'h9`
- `16'h14`
- `16'h96`
- `16'hbe`

Global experiment:

- temporarily changed Rust bitgen expansion to zero-extend instead of repeat.

Result:

- board behavior changed dramatically but became much worse, not better.

Implication:

- narrow-LUT semantics are definitely involved in the hardware result;
- but the answer is not "all narrow LUTs should be zero-extended."

#### 5) Per-LUT `16'h....` A/B search shows only a few narrow LUTs materially affect the mismatch

Useful artifacts:

- per-cell search summary:
  - `build/random-stage-compare-01352633/search-results.json`
- pair search summary:
  - `build/random-stage-compare-01352633/pairscan-results.json`
- selected combo search summary:
  - `build/random-stage-compare-01352633/combo-results.json`
  - `build/random-stage-compare-01352633/triplescan-results.json`

Most informative observations:

- forcing only `iSlice__5__.F` to import / program as `16'h0001` reduced mismatch count from 13 to 10;
- forcing `iSlice__5__.F` plus `iSlice__7__.G` to `16'h0001` and `16'h0009` reduced mismatch count from 13 to 9;
- many other single-LUT zero-extend experiments either did nothing useful or made the result much worse.

Important interpretation:

- the remaining bug is likely tied to a small subset of narrow LUT hardware semantics, not all LUTs globally;
- current evidence points more toward a specific import / programming convention mismatch for some routed narrow LUT cases than toward general route geometry.

#### 6) Most actionable next step after this round

Do not keep poking placement or route geometry first.

Instead, focus on the narrow-LUT path:

1. trace how routed `F` / `G` config expressions become logical `lut_init` + inferred LUT width in:
   - `src/infra/io/xml/lut_expr.rs`
   - `src/infra/io/xml/physical_import.rs`
2. compare that against how hardware actually behaves for the specific influential cells:
   - `iSlice__5__.F`
   - `iSlice__7__.G`
   - and then nearby candidates such as:
     - `iSlice__6__.F`
     - `iSlice__0__.G`
3. determine whether the real issue is:
   - wrong inferred logical width,
   - wrong repetition rule for a subset of cases,
   - or missing explicit handling for certain narrow local-LUT / routed-LUT situations.

### Landed fixes already in the Rust implementation

#### 1) Legacy LUT init semantics were wrong and have been fixed

Files:

- `src/stages/bitgen/config_image/literal.rs`
- `src/stages/bitgen/config_image/requests.rs`

What changed:

- Added `parse_compact_hex_digit_literal()`.
- `normalized_lut_function_name()` now:
  - prefers the cell property `init`;
  - falls back to `lut_init` only when `init` is absent.

Why this matters:

- Legacy C++ accepted bare `init` values with non-canonical semantics
  (`hex2expr -> Exp2LUT` style behavior), while Rust had been treating them as
  ordinary LUT truth-table literals.
- That mismatch directly produced wrong LUT SRAM images on hardware.

Concrete parity examples after the fix:

- `init="10"` -> `0x1010`
- `init="12"` -> `0x1212`
- `init="15"` -> `0x1515`

Useful source anchors:

- `src/stages/bitgen/config_image/literal.rs:23`
- `src/stages/bitgen/config_image/requests.rs:277`
- `src/stages/bitgen/config_image/requests.rs:291`

Useful tests:

- `cargo test config_image --lib`
- `src/stages/bitgen/config_image/requests.rs:495`

#### 2) Dual LUT/FF pair slot assignment now preserves cluster pair order

Files:

- `src/stages/bitgen/device/lowering/cells.rs`
- `src/infra/io/xml/physical.rs`

What changed:

- `assign_cluster_bels()` and physical XML slice binding now keep the first paired
  LUT/FF lane in slot 0 and the second paired lane in slot 1.

Why this matters:

- The previous Rust logic reversed the two paired lanes, which swapped F/G and X/Y
  ownership relative to C++ on the focused FF cases.

Useful source anchors:

- `src/stages/bitgen/device/lowering/cells.rs:102`
- `src/stages/bitgen/device/lowering/cells.rs:200`
- `src/stages/bitgen/device/lowering/cells.rs:239`

Useful tests:

- `cargo test dual_lut_ff_pairs_preserve_cluster_pair_order`
- `cargo test single_lut_ff_pair_stays_in_slot_zero`

### Theories that were investigated and ruled out

These were not guesses only; they were tested with targeted A/B bitstreams and did
not change the live-board outcome.

#### A) `SYNCX` / `SYNCY` are not the current main root cause

Observation:

- Legacy C++ output commonly contains `SYNC_ATTR=ASYNC`.
- Rust had additionally been forcing:
  - `SYNCX=ASYNC`
  - `SYNCY=ASYNC`

Test:

- Removed the explicit Rust-side `SYNCX/SYNCY` emission while keeping
  `SYNC_ATTR=ASYNC`.

Command used:

- `python3 scripts/board_e2e.py run ff-data-check ff-slot1-check --resource-root resources/hw_lib --out-root build/board-ab1-nosyncxy`

Result:

- Board behavior did not change.

Conclusion:

- `SYNCX/SYNCY` are not the active functional blocker.

#### B) Missing explicit "safe default" slice mux writes are not the current main root cause

Observation from CIL defaults:

- `DXMUX` defaults to `BX`
- `DYMUX` defaults to `BY`
- `BXMUX` defaults to `1`
- `BYMUX` defaults to `1`
- `CEMUX` defaults to `1`

Concern:

- "Rust does not emit a config" is not equivalent to "hardware gets a harmless off
  state".

Test:

- Explicitly forced supposedly safe values such as:
  - `BXMUX=1`
  - `BYMUX=1`
  - `CEMUX=1`
  - `SRMUX=1`
  - `SRFFMUX=#OFF`
  - `REVUSED=#OFF`

Command used:

- `python3 scripts/board_e2e.py run ff-data-check ff-slot1-check --resource-root resources/hw_lib --out-root build/board-ab2-safe-mux`

Result:

- Board behavior did not change.

Conclusion:

- Missing explicit safe-mux writes are not the active functional blocker either.

### Current unresolved problem

The earlier FF-focused live-board mismatch is no longer unresolved for the checked-in
P77 versions of:

- `ff-data-check`
- `ff-slot1-check`
- `ff-shift2-check`
- `ff-clock-check`

If new live-board mismatches remain, treat them as fresh cases and compare against a
fresh C++ probe first instead of assuming they share the old FF root cause.

### Update (2026-04-03): `logic-mesh` Rust-only route bug fixed

After re-filtering against a freshly runnable current C++ full chain, the only
remaining true "C++ correct, Rust wrong" board case was:

- `logic-mesh`

Fresh current C++ invocation details that worked:

- `map` must include `-y`
- `pack` must include `-g resources/hw_lib/fdp3_config.xml`

Root cause:

- Rust route occupancy only reserved exact `(x, y, wire)` nodes.
- That was insufficient for inter-tile stitched wires, where multiple tile-local
  names are really the same physical conductor.
- In `logic-mesh`, Rust allowed `net_Buf-pad-a` and `net_Buf-pad-y1` to both claim
  the same stitched LLH component even though their tile-local nodes differed.
- The decisive aliases on the real arch were on the same stitched LLH component,
  for example:
  - `RIGHT_LLH3 @ (4,53)`
  - `LLH6 @ (4,24)`
  - `LLH0 @ (4,5)`

What changed in Rust:

- `StitchedComponentDb` now records a canonical representative per stitched
  component in addition to bounds.
- Route-node occupancy now reserves and checks that canonical stitched-component
  key instead of only the exact tile-local node.

Verification:

- `cargo test --lib all_route_nodes_are_reserved_across_nets -- --nocapture`
- `cargo test --lib real_arch_stitched_component_occupancy_blocks_shared_llh_track_across_tiles -- --nocapture`
- `python3 scripts/board_e2e.py run logic-mesh --out-root build/board-e2e-logic-mesh-stitched-occ-20260403`

Result:

- `logic-mesh` now matches the correct board sequence:
  `0x8,0xc,0xd,0xc,0x8,0xb,0x8,0xc,0xd,0xc,0xc,0xd,0xc,0x8`
- In the fixed Rust route, `net_Buf-pad-y1` stayed on its prior path, while
  `net_Buf-pad-a` was rerouted off the conflicting stitched LLH resource.
- At this checkpoint, the previously filtered set of "C++ correct, Rust wrong"
  board cases is empty.

### `add16-folded-check` root cause that has now been confirmed

`add16-folded-check` was not a slice-config reconstruction bug.

The actual failing mechanism was route-order starvation in the current Rust router:

- the short two-sink net `$abc$366$new_n59` needs a scarce local escape from the
  source slice at `4,4`;
- a higher-fanout net such as `acc[2]` could claim that escape first;
- once that happened, `$abc$366$new_n59` became unroutable on the full design even
  though it still routed successfully in isolation on the exact same placement.

The decisive isolation was:

- route the full placed design -> `$abc$366$new_n59` failed with
  `could not find a Rust route`;
- route the same placed design with only `$abc$366$new_n59` kept -> the net routed
  successfully;
- move `$abc$366$new_n59` earlier in net order on the same placed design -> the
  full design matched the confirmed hardware sequence.

That means the responsible layer is route ordering / scarce local-exit ownership,
not bitgen guessing.

The current Rust fix is to route lower-fanout / shorter generic nets before wider
fanout generic nets so local escape arcs are claimed by the more constrained nets
first.

### Important caveat: manifest expected outputs were stale for the focused FF cases

The checked-in `expected_outputs` for the four focused FF cases were previously stale
because those cases were still constrained to `P150`.

That has now been corrected:

- the constraints for `ff-data-check`, `ff-slot1-check`, `ff-shift2-check`, and
  `ff-clock-check` now use `P77`;
- `manifest.json` now matches the freshly confirmed C++ probe sequences for those
  four cases.

For other cases, still confirm the true hardware baseline before treating
`manifest.json` as an oracle.

### Most useful artifacts already on disk

Live-board current outputs:

- `build/board-live-verify-2/ff-data-check/wave_probe.log`
- `build/board-live-verify-2/ff-slot1-check/wave_probe.log`

Earlier baseline logs:

- existing paired probe logs for `ff-data-check`
- existing paired probe logs for `ff-slot1-check`
- existing paired probe logs for `ff-shift2-check`
- existing paired probe logs for `ff-clock-check`

Useful case008-era debug artifacts:

- `build/debug-case008-state-03/`
- `build/case008-stateprobe-0-3/`
- `build/case008-stateprobe-4-7/`
- `build/case008-stateprobe-8-11/`
- `build/case008-stateprobe-12-15/`
- `build/random-board-diff-debug-case008-rerun/`

Useful compare/debug scripts:

- `scripts/board_e2e.py`
- `scripts/board_diff.py`
- `scripts/random_board_diff.py`
- `scripts/slice_config_diff.py`
- `scripts/fde_board.py`

Historical upstream source locations worth reading:

- the packing sources in the sibling `FDE-Source` tree
- the bitgen expression sources in the sibling `FDE-Source` tree

### Reproduction and continuation checklist

#### 1) Re-run the focused Rust board repros

Single case:

- `python3 scripts/board_e2e.py run ff-data-check`

Two key cases:

- `python3 scripts/board_e2e.py run ff-data-check ff-slot1-check --resource-root resources/hw_lib --out-root build/board-live-verify-2`

#### 2) Re-check the already-landed logic fixes locally

- `cargo test config_image --lib`
- `cargo test dual_lut_ff_pairs_assign_reverse_bel_order`
- `cargo test single_lut_ff_pair_stays_in_slot_zero`

#### 3) Compare Rust slice config against a known C++ packed result

- `python3 scripts/slice_config_diff.py --packed <02-packed.xml> --sidecar <06-output.bit.txt>`

This is still useful, but do not stop here if the signatures look aligned. The live
board bug may still be in route/site-route encoding.

#### 4) Compare against board baselines

List comparable cases:

- `python3 scripts/board_diff.py list`

Run the diff:

- `python3 scripts/board_diff.py run`

Important caution:

- `scripts/board_diff.py` now probes every discoverable baseline candidate
  bitstream directly and only treats the case as comparable when those
  candidates agree on one output sequence. If multiple baseline candidates
  disagree, treat that as a baseline ambiguity that must be resolved before
  using the script output as a golden baseline.

#### 5) Stress with random regressions

- `python3 scripts/random_board_diff.py --count 5 --seed 20260322 --keep-going`

Use this only after a focused fix hypothesis exists; do not use random runs as the
first debugging loop.

### Recommended next steps for the next contributor

1. If new board mismatches appear, first re-run the same filter discipline:
   - freshly probe current C++ on the exact case;
   - keep only cases where current C++ is correct and Rust is wrong.
2. For any new Rust-only route bug, check stitched-wire ownership early instead of
   only exact route-node ownership.
3. Use the known-good current C++ full-chain invocation:
   - `map -y`
   - `pack ... -g resources/hw_lib/fdp3_config.xml`
4. Do not spend more time on `SYNCX/SYNCY` or safe-default slice mux theories unless
   new evidence appears.
5. Only update `examples/board-e2e/manifest.json` after the C++ baseline is freshly
   confirmed.

### Working style for this debug thread

- Keep changes small and targeted.
- Do not attempt a broad refactor while the current root cause is still open.
- Preserve generated debug artifacts under `build/` when they are useful for the next
  comparison.
- When a new theory is tested, record:
  - the exact code delta;
  - the exact command used;
  - and whether the live board changed.
- That prevents repeating dead-end experiments.
