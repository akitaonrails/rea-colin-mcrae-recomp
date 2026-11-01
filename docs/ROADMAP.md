# Reconstruction roadmap

The project advances by evidence-backed vertical slices. A subsystem is not
“done” because pseudocode exists; it is done when a clean implementation has a
repeatable compatibility check at the strongest practical boundary.

## Architecture direction

Keep recovered game behavior independent of the host platform:

```text
user-owned installation
        |
        v
verified importer ---> ignored runtime-data/
                            |
                            v
formats/resources --> game simulation and state
                            |
              +-------------+-------------+
              |                           |
      compatibility backend         native host / modern backend
      (recovery oracle)              (SDL3; GPU choice benchmarked)
```

The compatibility backend exists to establish the original contract. The
modern backend is allowed to differ internally, not observably where the
compatibility specification applies.

### Auxiliary PS1 reference lane

The PlayStation release may be analyzed as a separately identified reference
edition using PS1-specific tooling. It is not a second compatibility target and
does not replace Windows evidence. The selected lane is: `dumpsxiso` for disc
layout, Ghidra plus `ghidra_psx_ldr` and PsyQ signatures for static recovery,
Splat for executable/overlay accounting, local `m2c`/`maspsx` matching only for
selected questions, and PCSX-Redux for explicitly scoped runtime observations.
PSXRecomp is an optional alpha proof of concept, not an evidence authority.

Cross-edition findings must preserve separate artifact/tool identities and be
promoted only when an independent Windows observation supports the same
contract. The staged procedure and gates are in
[PSX_REFERENCE_PLAN.md](PSX_REFERENCE_PLAN.md). This auxiliary lane does not
change the completion counters in [DECOMP_MAP.md](DECOMP_MAP.md).

Current bounded result: P0 identifies the USA `SCUS_94474` four-track disc and
pins the local static tools; P1 has validated the PS-X EXE loader and accounted
for the main executable with a conservative Splat map, but trustworthy
code/data boundaries and SDK separation still need the Ghidra-derived seeds.
The first P2 comparison found the same replay-object architecture at a high
level and corrected the Windows model's final two bytes from unnamed padding to
a neutral trailing-field contract. Exact field semantics remain unknown.

## Milestone 0 — provenance and safe workspace

Deliverables:

- exact hashes, sizes, PE identities, and analysis-tool versions;
- read-only inventory of installation contents and patches;
- asset exclusion rules and importer design;
- evidence ledger format and initial unknowns;
- subsystem map and ordered recovery backlog.

Exit gate:

- another developer can reproduce the inventories without copying source media
  into Git;
- observations are attributable to exact artifact hashes;
- reference edition/region ambiguity is explicit.

Reference policy: use the official EU updater payload as the canonical clean
code artifact. Keep the installed EU-derived executable and both supplied
xdelta outputs as comparison variants. Treat the official US payload as a
separate regional target; its differences are too broad to model as resources
alone. This policy may expand when original-disc evidence becomes available.

## Milestone 1 — executable map and data contracts

Recover both executables before assigning their roles. Build a function map from
entry points, imports, strings, references, and call clusters. Prioritize
boundaries that reveal stable contracts:

1. startup, command line, registry/configuration, and executable handoff;
2. filesystem lookup, case/separator rules, gzip/`CMPR` unwrapping, and
   archive/resource loading;
3. frontend state, localization, save/config persistence;
4. timing, input, rendering, audio, CD/music, and networking;
5. car/track loading, physics state, AI, race rules, replay, and results.

Initial static evidence identifies `Game.exe` as a video/launcher frontend that
starts `Rally.exe`. Reconstruct it as a separable launcher/presentation concern;
do not let its DirectShow-era lifecycle contaminate the game core.

Deliverables:

- versioned function/symbol map linked to REA Evidence IDs;
- format specifications with bounded parsers and synthetic fixtures;
- compatibility interfaces for clock, input, video, audio, storage, and network;
- reviewed reconstruction obligations with owners and verification methods.

Exit gate:

- each high-priority boundary has an observation, a bounded inference, or a
  named unknown with a next query;
- no format parser depends on game payloads in the test suite.

## Milestone 2 — first compatible vertical slice

Implement the smallest path that exercises real contracts end to end:

- locate and validate user data;
- parse configuration and localization;
- create a window and deterministic clock/input loop;
- render one frontend state using imported resources;
- exit cleanly while preserving expected configuration behavior.

Preferred implementation language is Rust unless recovered ABI/layout evidence
makes a narrow C/C++ module materially safer. Do not translate entire decompiler
listings mechanically; recover named domain behavior and explicit data layouts.
Every completed recovered behavior lands with deterministic unit tests in the
same milestone. Shared backend contracts supplement, rather than replace,
focused boundary and failure tests; no later subsystem may depend on an
untested compatibility rule.

Current implemented portion: reviewed asset-store loading now reaches startup
art and fixed-width frontend/game localization tables; configuration has an
opaque exact-size load model and a tested caller-owned write boundary; indexed
startup presentation runs against the deterministic software backend. The
first/root frontend state, fixed-layout font parsing, byte-text layout, input,
window-loop, timing, and root-menu visual rules are pure tested contracts. All
16 font styles and the four present root-menu artwork rows resolve through the
verified import; the loader's distinct Italian row is explicitly absent from
the reviewed corpus. Root bitmaps now reach fixed-slot indexed surfaces without
backend mutation, and both persistent font and root-bitmap pools now use a
tested shared source store without monopolizing the backend. Transaction,
fixed-slot, and mixed-pool tests cover 33 coexisting sources, optional upload
holes, isolated required rollback, repeated-frame reuse, and global teardown.
Host clock/input, native window creation, and a host adapter for the recovered
presentation executor remain before the vertical-slice exit gate. Complete
root frames can now be composed and captured by the deterministic headless
backend, without assigning native display semantics to that test oracle.
Root-footer text reaches a tested software back-buffer pixel through
localization, the shared store, and the persistent font pool. Root visual
transcripts name fixed bitmap slots directly, and the bitmap executor now
submits their exact opaque/keyed plans with checked geometry and no per-frame
uploads. A sequential frame dispatcher preserves bitmap/footer/C3D/scene/
presentation order and typed failure boundaries. It stages one owned prepared
C3D plan at the root command, then consumes it once inside the terminal scene.
The scene runs even without a selected object and now owns checked viewport
setup, conditional depth clear, Begin/End cleanup, world reset, and the opaque
scene tail. The software backend now executes the bounded branch-zero plan into
the indexed back buffer with persistent bindings and depth, then atomically
captures an owned indexed frame and palette at the terminal hook. The separate
native presentation executor now preserves sample-before-Flip invocation and
the windowed vblank/fallback sequence behind a typed adapter. The recovered
deferred-palette state and depth-8 commit are tested and integrated, including
runtime group mapping, both software caches, sentinel behavior, typed failure,
and exact scene/cleanup→palette→presentation order. The prepared pipeline sets
the selected root visibility latch after successful planning and clears all
six after every terminal scene attempt. Native host adaptation remains a
boundary. A bounded static search found no steady-frame windowed copy owner in
the shipped build, so a working portable windowed copy must be explicit host
policy rather than attributed legacy behavior.
The software compatibility path now has a production, callback-scoped root-menu
session assembler. It consumes an initialized startup display and verified
import, applies the recovered zero-RGB then pending `disk.pal` bootstrap,
retains every root resource and mutable frame state across repeated renders,
and tears down persistent sources before returning the backend. Native window,
event-loop, and host presentation integration remain separate later work.
The six optional root-menu C3D files now load through fixed slots and a bounded,
byte-preserving parser. All three counted groups expose proven framing: group
0's raw `u16` values; group 1's typed, borrowed 32/24-byte entry views; and
group 2's raw label fields plus opaque length-delimited blobs. The 32-byte view
decodes eight signed little-endian Q16.16 components in their recovered groups.
The 24-byte view exposes its three local entry ordinals, signed resource
ordinal, three byte-exact corner tuples, and two still-unknown scalars. Checked
borrowed resolvers now implement the proven group-0→group-1, entry24
`+2/+4/+6`→local entry32, and group-1-entry24→group-2 numeric joins, including
the latter's `-1` sentinel, without making reference validity a structural
parse requirement. A separate checked topology planner now partitions only
consecutive equal prepared selectors, deduplicates local entry ordinals in
first-use order, and emits three compact indices per source entry. It retains
borrowed typed source views rather than pretending to produce render-ready
vertices. Runtime affine conversion, selector/resource preparation, associated
side-vector materialization, and resource-derived mapping-coordinate scaling
are now recovered below; general/non-root material execution and
backend-integrated pixels remain unrecovered.
Group-2 records now also classify complete
`BM` signatures as opaque bitmap bytes and expose strictly framed, borrowed
`ANIM` entries with their resource ordinals, raw advance thresholds, and
separate legacy low-byte behavior. Bitmap pixels remain undecoded here because
the general embedded resource set is not limited to the standalone 8-bit
subset. A selected animation record can now resolve each frame ordinal against
the same file's group-2 table through an exact-size borrowed iterator; an
invalid frame remains a typed result and does not hide later frames.
For the root-only corpus-backed subset, all six objects of either visual variant
can now prepare into an owned fixed-slot resource set. Every embedded bitmap is
strictly decoded while preserving source indices and its full embedded palette;
the exact mode-2 path installs no surface color key. Bounded-label/dimensions/pixels
matches share arena-scoped stable prepared IDs, entry24 selectors retain source
order and `-1` remains an explicit missing selector, and one bad optional object
rolls back only its new resources before later slots continue. Palette
attachment and backend upload remain boundaries for alternate hardware paths;
the software root path retains its sampled bindings directly. The prepared resource
also retains the raw low words of its originating DIB dimensions because the
legacy surface-layout path consumes those words rather than normalized decoded
extents.
Prepared selectors now feed the checked consecutive-run topology planner through
a combined constructor tied to the exact borrowed asset set. The resulting
fixed-slot plans preserve group order, source views, missing selectors, compact
first-use vertex ordinals, and preparation diagnostics without exposing an
unsafe arbitrary source/preparation join. Group-0 references now retain their
owning record and checked group-1 target, and the pre-scale mode-2 materializer
reproduces wrapping Q16 affine arithmetic, f32 field layout, the persistent
pass-through latch, opaque pre-mutation side vectors, source-order last-writer
corner mutation, missing selectors, and batch compaction. All reviewed root
records force pass-through. Per-slot loader state keeps the unresolved tuple
gate explicit. A second materializer derives the legacy surface layout from the
first interned resource occurrence, including wrapping width alignment,
quarter/half gates, and asymmetric square expansion. It applies vertical then
horizontal mapping-coordinate factors with one f32 store, shares first-resource
claims by source vertex ordinal across selector runs, preserves same-entry
duplicate multiplication, and only then compacts batches. The exact root corpus
has square resources and therefore takes the proven zero-padding/no-multiply
case under normal or quarter preparation. The software branch-zero backend now
owns sampled bindings and active-palette composition; alternate material
profiles and the unresolved tuple-source gate remain C3D boundaries.
The root transform oracle is now portable and deterministic. A source-owned
quarter-wave table reconstructs the exact 4,096-entry legacy trig table, while
square-root values use equivalent integer arithmetic. The fixed-point row
normalizer preserves wrapping sums, low-word products, signed shifts, the
complete-raw-angle zero branch, and the distinction between raw angles `0` and
`0x1000`. Root world matrices retain the recovered Q16 translation and convert
to exact f32 words; identity view, frontend projection, and checked viewport
words are available without a platform graphics dependency. The software root
path now consumes them through scene traversal, resource binding, rasterization,
and indexed back-buffer composition; other renderer paths remain separate.
Topology and both materialized runtime stages now also carry the same opaque,
unforgeable prepared-arena provenance. The planner can therefore reject data
mixed across independent preparations even when no concrete resource ID is
available to expose the mismatch.
The root-only frame planner now performs that join transactionally. It checks
the exact one-level root shape, resource selectors, compact batch payloads,
viewport, slot presence, variant, and provenance before changing an angle, then
owns source-ordered submission snapshots and the exact recovered transform and
numeric draw words. Concrete software binding, depth/scene execution, the pixel
oracle, and a headless terminal capture are now joined. The selected one-frame
visibility latch is now set transactionally and all six root latches are
cleared after the scene attempt. The depth-gated deferred-palette commit now
runs between that cleanup and presentation. Native presentation adaptation
remains next.
Prepared resources now map once into an arena-scoped opaque binding catalog.
The root batch executor consumes a plan, preflights the complete catalog join,
sets world state, and preserves the recovered resource-binding and persistent
selector-cache order across source batches and frames. Branch-specific
material/texture work remains inside the opaque backend binding. Bindings are
released in reverse order, and external render paths can explicitly invalidate
the cache. The software backend implements the bounded branch-zero binding and
headless frame capture and visibility-latch cleanup; native presentation
adaptation, alternate profiles, and a portable GPU implementation remain
separate.
The championship-car path now preflights a single-use actor plan against the
exact frontend binding catalog before any scene effect, then executes the
shared terminal C3D bracket with actor-scoped worlds, resource/default-zero
selector transitions, and Type-2 draw commands. The full championship terminal
owner now preserves inline callback order, coordinates root/car binding-cache
ownership, clears all 20 frontend visibility latches, commits the requested
palette, and presents last. Static recovery now separates the process-global
optional Type-2 word-5 pre-draw mutation from caller flag `0x08`, which affects
only a different world/spatial renderer in resolved code. A pure owned-copy
mutation contract models the retained gate, thresholds, distance/center tuple,
fixed truncating x87 qword conversion, and exact fixed-square-root lookup.
Each car frame now prepares fresh owned Type-2 words before terminal effects,
submits them after the recovered selector-binding decision even on cache hits,
routes the active mesh's exact all-`-0x10000` origin sentinel to the per-vertex
distance arm, then conditionally overwrites word-4 RGB while preserving alpha.
Frame planning records the renderer's list-head RGB owner for each sibling list;
the standalone session requires explicit source-ordered per-car actor tables
because it cannot inherit arbitrary process state. Count, ownership, and
profile/state disagreement are rejected before effects. The software backend
admits all four word-5/actor-RGB gate combinations for the otherwise identical
unified-indexed COPY/no-blend profile, copies texture indices directly, and keeps
missing selectors distinct as reviewed-car index zero. A public synthetic
asset-to-scene test pins resource/default/resource execution and exact indexed
color/depth output. Car-present scenes now invoke a backend-specific effect-free
preflight before any terminal pixel or state effect; an end-to-end software
car/palette/presentation test renders the car, commits a discriminating group-4
request, and captures the same indices under the new palette. A callback-scoped
software championship session now loads and retains all 14 car topology/runtime
pairs, explicit raster-profile provenance, compact car/FCHE palette layout,
shared bindings, and frame state; synthetic and reviewed-import tests cover
hidden-to-visible rendering, car switching, Next, presentation, and teardown.
The original device branch, alpha, word-5 mutation, and actor-RGB gates are
independent retained inputs. The alternate branch now has exact 16/24/32
selection priority and a bounded canonical-RGB MODULATE oracle, while its
runtime texture descriptors and framebuffer masks now have exact raw retention,
candidate classification, retained non-alpha selection, and separate role-safe
source-owned mask utilities. Canonical RGB can be converted into logical masked
framebuffer words without asserting byte order or pitch. A nominal source-owned
conversion stage now retains Palette8 storage or materializes RGB555/RGB565
logical words before the alternate oracle filters and modulates them. Driver
texture `Load` equivalence remains separate; a terminal oracle now packs only
depth-passing results through independent logical framebuffer masks and leaves
uncovered words unchanged. Native surface layout, effective filter, dithering,
alpha pixels, full-session integration, and presentation remain separate. A
standalone headless split backend now owns the logical target/depth lifecycle,
transactionally retains converted bindings, preflights every resolved batch
before effects, validates retained-cache coherence, and executes repeated
frames. Native layout, effective filter, dithering, alpha, indexed 2D
composition, alternate-session ownership, and presentation remain open. A
source-owned bridge can now expand a completed indexed frame through an
explicit palette into that logical target without replaying keyed/clipped 2D
work. It remains a deterministic integration oracle rather than a substitute
for source-by-source conversion. Focused static evidence now establishes the
ordinary `0x81` 8-bit-BMP path at a 16-bit destination: each source uses its
own embedded palette, discriminator byte `5` selects RGB555 while every other
value selects RGB565, the complete aligned stored rows are converted, and the
source key is packed embedded-palette entry zero. A tested nominal
`ordinary_bitmap_rgb16` implementation now preserves that complete logical
result and rejects malformed source state before returning output. DirectDraw
stretch, active-device format selection, and full source-ordered session
integration remain open. Matching-format fast opaque/keyed logical blits now
have effect-free preflight, bit-exact word copy, nonzero packed-key handling,
aligned-row bounds, and owner-scoped inactive-backend execution. Source-ordered
alternate-session integration remains separate. Alternate fonts are explicitly
separate: their initialized path prepares a `0x90` source, may invoke driver
`Load`, and queues a colored primitive. The exact no-driver-`Load` ARGB4444
decoder subset now has one persistent 16-style owner retaining aligned rows,
palette provenance, parsed glyph metadata, literal-zero keying, and exact-black
alpha selection. Queue-consumer inspection also pins three 2,047-entry lanes,
52-byte records, ordered lane flushing, primitive submission versus terminal
bit-`0x40` fallback, and the championship tuple/fade arithmetic. The setup
model now retains the raw four-byte source tuple and signed Q16 fade, including
the recovered terminal/inactive/active row selection, and a source-neutral
font preflight emits lane-neutral semantic glyph-producer inputs in
command/glyph order against the persistent ARGB4444 owner. A separate semantic
router now owns three independent typed lanes, accepts exactly 2,047 records
per lane, accounts for later silent drops, retains FIFO order, and applies the
recovered byte-3 gate after acceptance. That gate preserves the caller byte
only when effective renderer mode is not 7 and the recovered alpha-texture
capability bit is present; otherwise it forces `0xff`. Session callers select
the lane explicitly because ordinary championship lane 2 remains a strong
inference. Native queue packing/flush execution, driver conversion, tuple
channel meanings, blend semantics, and final pixels remain unresolved
alongside native layout and dithering. R_INFO, the 11
unique logo assets, and all 15 Chequered frames now also have one transactional
27-source logical-RGB16 owner built directly from decoded user assets. Stable
identity/path diagnostics and per-bitmap palette provenance are pinned. One
shared ordered kernel now prepares either rich indexed payloads or a
lightweight alternate transcript from exact source extents, with tests proving
full public-command parity across car, gear, and Next cold/warm frames. The
alternate adapter derives aligned extents from the 27-source RGB16 pool and an
optional nominal converted Selection source without indexed uploads. A
two-pass alternate executor rejects stale cache snapshots, preflights every
supported Fast bitmap plus Selection Stretch before
effects, and composes them in transcript order while returning text, NoOp, and
other deferred boundaries as exact typed outcomes. Selection uses the recovered
literal-zero key/call geometry and an explicitly source-owned center-nearest
policy because the legacy DirectDraw sampling kernel is not present in the
executable. The cache commit proves only that bounded ordinary-bitmap pass;
the alternate terminal now adds complete common-scene/car-batch preflight,
coordinates transform/latch/palette state inline, and runs the selected-car or
empty Next scene after setup on the same target. It explicitly skips the
non-indexed palette commit while preserving the pending request. Standalone
alternate session ownership now builds this complete retained slice directly
from a verified user import without an indexed backend or uploaded sources. It
loads and converts Selection once, executes it through the named source-owned
Stretch policy, validates caller-explicit RGB and raster policies before
callback entry, and preserves target/depth, binding cache, preview/transform/
latch/route/palette state across repeated car and Next frames. The session now
also loads and owns the selected frontend table and all 16 ARGB4444 font
sources, prepares every non-deferred setup Text command before terminal
effects, routes that slice from initially empty lanes through caller-explicit
lane and resolved byte-3 policy, and attaches the result only to successful
outcomes. It does not yet merge later deferred info/next-preview producers.
The publication guarantee covers the returned value, not rollback of setup or
scene effects after a later failure. Static follow-up now also resolves the
native record's optional viewport, angle/pivot, lane, tuple, and terminal fields;
the current championship producer supplies zero angle/pivots and no viewport.
A backend-neutral flush plan orders accepted records by lanes 3, 2, 1 with exact
lane depths and per-record textured/fallback dispatch. Pure preflight now
resolves every source and constructs the bounded zero-transform/no-viewport
quad with exact corner order, depth, RHW, diffuse/specular carriers, and
renderer-mode UV rule; optional backend validation still completes before any
frame effect. Fresh owner evidence places the unconditional dispatch inside the
common scene after identity-world reset and before its opaque tail/EndScene,
including an empty Next scene. The alternate terminal executes that semantic
dispatch and reports per-terminal counts, but no font pixel backend exists yet.
Native record packing/count clearing, optional transforms/UV swaps, device
state, filtering, blending, fallback pixels, driver-exact Stretch comparison,
native presentation, and the other deferred pixels are the next integration
seams.

The workspace now also has its first application executable. `cmr-app`
requires a caller-selected imported runtime tree, embeds only reviewed profile
metadata, and renders one fixed official-EU English/Normal root frame through
the production software session. A versioned digest contract binds the complete
indexed plane and raw palette, while the CLI emits only path-free counters and
checksums. The application, existing video vertical test, and their failure
tests share one dependency-neutral generated import fixture. This completes
runtime-data discovery and the headless executable part of the vertical slice;
window creation, an event/input loop, SDL presentation, audio, and GPU work
remain. Before the SDL step, expose the completed software frame through a
narrow presenter consumer so the proven startup/session/render orchestration is
reused rather than copied.

Exit gate:

- clean build from source on the declared toolchain;
- importer succeeds for a recognized source and fails safely for wrong,
  incomplete, or modified sources;
- deterministic tests cover the vertical slice;
- a documented controlled comparison bounds remaining differences.

## Milestone 3 — playable compatibility baseline

Add subsystems in dependency order:

1. menus, profiles, saves, options, and localization;
2. resource decoding, renderer, audio mixer, and input mapping;
3. car and track scene loading;
4. fixed-step physics and collision;
5. race sequencing, timing, penalties, AI, co-driver, replay, and results;
6. multiplayer only after its original protocol and security boundary are known.

Current G03 recovery has isolated the signed force-candidate selector and the
pure arithmetic behind the nonzero control `+0x420` correction gate inside
`FUN_0040d810`. Both lookup-free candidate branches and the combined
lookup-dependent branch are reconstructed. The latter uses neutral
`signed_ratio_angle` and `derive_lookup_dependent_candidates` contracts with
the observed asymmetric zero, swap/sign, exact-ratio index-512 adjacent-zero,
and wrapping arithmetic. Zero gates precede validation: `(0,INT_MIN)` returns
`0`, `(INT_MIN,0)` returns `0x400`, and `INT_MIN` is rejected only when both
operands are nonzero. `0x400` denotes a
quarter-period and `0x1000` a full wrapping phase; this is not exposed as a
conventional atan2 API. `FUN_0048fba0` proves all five tables are generated
math rather than asset payloads, including arcsine `DAT_00831380` (canonical
hash `ddf0d95f...e02745b`) used by `FUN_0040fbe0`. The exact source-owned
`signed_arcsine_phase` contract preserves sign restoration, clamp/index
behavior, the exact-negative-one adjacent-zero sentinel, and safe `INT_MIN`
rejection. `apply_geometric_lane_postprocess` reconstructs the four-lane
two-pass qualification, legacy normalization, signed wrapping alignment bound,
shared scalar, and wrapping accumulator adds behind an atomic API whose only
fallible domain is the unsafe arcsine `INT_MIN` lookup.
`apply_lane_accumulation` now composes the subsequent `FUN_0040d810` tail in
its observed order: selected-scalar overwrite, gated first direction add,
second projection against that updated accumulator and optional add, then the
independent surface projection/add. The other `legacy_math` helpers centralize
the recovered consumers while mathematical `Fixed16::sqrt` stays distinct.
The upstream direction producer is now reconstructed as a neutral contract:
reject/normalize sources at physics `+0x550`, `+0x544`, and `+0x5d4` against
axis `+0x324`; form the fourth from the ordered raw cross of `+0x5d4` and
`+0x264` before rejection; pack A/B as `[P,P,S,S]`/`[R,R,T,T]`; preserve the
pre-mode A-valid flags; and apply caller-resolved Preserve, Exchange, or Rebuild
packing. Rebuild leaves C as the raw doubled-projection rejection and normalizes
only U, so exact unit/orthogonal claims are explicitly excluded. Eight focused
tests bring `vehicle_dynamics` to 71; `legacy_math` remains at 13. The
translational part of `FUN_0040a4a0` is now reconstructed separately as
`apply_translation_step`: cached prior speed scales old velocity into the
damping subtraction, acceleration advances velocity by `0x3333`, and position
adds the new velocity minus the Q16 `0x8000` half-delta. Nine focused tests,
including final-position wrapping, bring `vehicle_dynamics` to 80. This also
corrects the state map to position
`+0xc8..+0xd0`, velocity `+0xd4..+0xdc`, prior velocity `+0xe0..+0xe8`, and
cached prior speed `+0xec`; `+0x3cc..+0x3d4` is an additive
acceleration-like term and `+0x3d8..+0x3e0` belongs to the rotational chain.
The callback/global mode resolver, semantic vector/mode names, integration into
the simplified race-state harness, and a distinct suspension-damper identity
remain. The table-provenance inventory has now disproved the singular runtime
loader premise: two adjacent 120-word surface/material coefficient families are
PE-backed and can be extracted from a positively identified official-EU
executable, the gear/car values are emitted by fourteen compiled initializer
cases, and the `DAT_0050e170` family is BSS/runtime geometry state. The
source-owned `pe_surface` parser plus the profile-gated, hash-first
`extract-surface-tables` command establishes a safe local 960-byte extraction
under ignored `runtime-data/` without vendoring values. The fail-closed
`SurfaceTableStore` now validates the supported state generation, exact
reviewed-profile identity/digest and source metadata, and output path/size/hash
before owning both decoded families without retaining a filesystem path. Its
persistent-symlink checks assume a stable caller-owned directory during
`open`, not protection from concurrent adversarial mutation. A checked core
adapter now consumes the first `&[i32; 120]` family through the recovered
lane-scalar rule: it bounds-checks before override handling and preserves the
selected raw signed Q16.16 value. The adjacent second family remains
store-only and unknown; this milestone does not wire the store into the app or
session and does not conflate it with FFB `DAT_004edb70`, car initializer
`FUN_00404440`, or BSS `DAT_0050e170`. The compiled initializer now has a
payload-free schema contract: fourteen unsigned selectors share an exact
eight-state/twenty-three-control-word destination layout, invalid selectors
preserve those destinations, and a separate helper reconstructs the later
eight-slot reciprocal/copy family with signed division and zero handling. Nine
synthetic tests bring `cmr-core` to 1,104 while `vehicle_dynamics` remains 85.
No official tuning values are tracked. G03-P now completes the narrow
user-owned extraction boundary for those direct case words. The shared pure
PE32 parser proves the reviewed seed, unsigned dispatch, fourteen-entry jump
table, non-overlapping case bodies, and exact constant-store grammar before
returning all fourteen records. The metadata-only official profile and
`extract-compiled-car-initializer` command hash the bounded source before
parsing, then atomically publish the versioned 1,760-byte `CMRCCASE` container
and path-free state beneath an explicit ignored `runtime-data/` destination.
`CompiledInitializerStore` revalidates every identity/shape field, exact size
and hash, and owns the decoded records without retaining a path. G03-Q adds the
narrow `CompiledDirectCaseSet` core consumer: fixed array types prove per-record
word counts, while construction explicitly checks selector count and exact
neutral offset identity, copies all signed words in selector order, and feeds only the
existing unsigned direct-case selection contract. Two synthetic tests bring
`cmr-core` to 1,106. This is not application/session/gameplay wiring and does
not extend the recovered behavior beyond the direct stores. G03-R recovers the
next bounded common-path step, official-EU `FUN_00404160`: it reads and rewrites
only control `+0x1d4..+0x1e8`, computes all five wrapping adjacent differences
before mutation, selects one of five Q16 scale/first-word-bias policies from the
low three option bits, then rebuilds the six words with signed arithmetic-shift
products and cumulative wrapping addition. The payload-free
`transform_compiled_control_run` and eight-word
`transform_compiled_control_family` adapter preserve the outer words and add
six synthetic tests, bringing `cmr-core` to 1,112. The signed config-record
lookup/domain remains outside that transform. G03-S recovers the preceding
`FUN_00404100` helper: after every valid or invalid direct-case selector has
joined the common path, it reads config-record byte `+0x95`, masks its low three
bits, and always writes center drive split `control+0x170` as Q16 `0x4000` for
mode `0`, `0x8000` for mode `1`, or `0xc000` for modes `2..7`. This field is
not a direct-case destination. The existing `FUN_0040d810` evidence consumes
it and its `0x10000-split` complement in both coast and driven front/rear
distribution. The total `compiled_center_split_q16` helper plus two tests,
including all 256 byte inputs, brings `cmr-core` to 1,114. The signed config
lookup/domain remains outside that helper. G03-T recovers `FUN_00407270` as a
separate 22-dword auxiliary control patch. Its exact ascending destinations are
`+0xd8,+0xdc,+0xe0,+0xe4,+0xe8,+0xec,+0xf0,+0xf4,+0xf8,+0xfc,+0x100`,
`+0x104,+0x108,+0x10c,+0x110,+0x118,+0x11c,+0x120,+0x124,+0x128,+0x12c`,
and `+0x13c`: seven policy immediates, nine common immediates, and six derived
words. A force bit bypasses config lookup; otherwise low-three-bit modes `1`,
`2`, and `0,3..7` choose policy one, policy two, and default. The derived words
use signed Q16 multiplication, signed `state+0x48 >> 3`, and three guarded
signed `2^32/x` reciprocals. `AuxControlConstants` keeps every withheld
function literal caller-owned; `derive_aux_control_patch` implements only the
payload-free policy/arithmetic and a typed atomic overflow. Six tests bring
`cmr-core` to 1,120. The patch is disjoint from direct-case destinations. At
the G03-T boundary, a reviewed user-owned constants extractor/store, signed
config lookup/domain, full composition with the other initializer stages and later tail,
application/session/gameplay wiring, meanings/units, gameplay index invariants,
other editions, runtime geometry/BSS consumers, and full
`FUN_0040d810`/`FUN_0040a4a0` closure remain open.

G03-U completes that narrow user-owned auxiliary-constant extraction/storage
boundary. `cmr-formats::pe_aux_control_initializer` semantically decodes the
canonical official-EU `FUN_00407270` instruction and control-flow grammar rather
than embedding a normalized byte template, returning a neutral 38-word shape
(nine common words, two Q16 multipliers, and three seven-word/two-divisor
policies). Its independently versioned `CMRAUXC` v1 codec is exactly 184 bytes:
a 32-byte magic/version/count header followed by 38 signed little-endian words.
The exact-digest metadata-only official profile and explicit
`extract-aux-control-constants` command identify and hash the bounded user-owned
executable before parsing, then self-check and atomically publish fixed flat
data/state files beneath an explicit ignored `runtime-data/` destination.
`AuxControlConstantsStore` rejects mismatched identity/shape/hash, linked or
special files, truncation/trailing bytes, and malformed codecs before owning the
decoded words without retaining a path. The 149 format tests, 76 assets-library
tests, eight CLI tests, three audit tests, and opt-in ignored official smoke
cover the semantic decoder and storage boundary without tracking or exposing an
official value. A checked format/store-to-core adapter, safe signed config-record
resolver and domain, composition with the direct cases/center split/six-word
transform/auxiliary patch/later tail, semantics and units, application/session
wiring, and other editions remain open.

G03-V (`824acde`) adds the checked core ownership boundary for those
authenticated words.
`AuxControlConstantSet` accepts an `AuxControlConstantsStore`, uses fixed array
types to pin record cardinalities, verifies the exact format/core order of all
22 output, nine common, and seven policy offsets, and copies the complete
38-word signed shape unchanged into core-owned records. The value survives
dropping both the store and its backing root. Its narrow `derive` method only
delegates to `derive_aux_control_patch`, including that helper's typed reciprocal
overflow. Three synthetic tests cover cross-layer shape/order invariants, signed
ownership and every policy after drop, and overflow propagation, bringing
`cmr-core` to 1,123. The signed config-record resolver, composition with direct
cases, center split, six-word transform, later reciprocal/copy tail, application
or session/gameplay state, meanings, units, and other editions remain open.

G03-W (`f9e3b1f`) closes the unsafe shared option-lookup boundary without
pretending the surrounding configuration block is a homogeneous table. Static
evidence traces the authoritative `state+0x738` store through every resolved direct constructor:
ordinary construction supplies `0..=7`, while the alternate path supplies only
zero or one. All three helpers sign-extend that byte and use stride `0x9c` to
read raw options at record-relative `+0x93`, `+0x95`, and `+0x99`; the auxiliary
helper separately checks unindexed `config+0x26 & 2` to force its default. The
config pointer comes from the persistent root and the initialized block is
`0x6c4` bytes, but its persistence layout is heterogeneous: only slots zero and
one belong to the directly authored/persisted two-stride window, while later
helper-visible addresses overlap other copied regions. Thus eight is the bound
for every resolved explicit state constructor, not a homogeneous record count.
The derived `resolve_compiled_config_options` accepts a caller-owned byte view, rejects
negative and `>=8` signed slots, validates the highest required address before
reading, preserves all raw option bits for the existing arithmetic helpers, and
uses only the unindexed force bit. Five synthetic tests cover error precedence,
exact truncation boundaries, record selection without mutation, slot seven, and
force-flag addressing, bringing `cmr-core` to 1,128. Composition with direct
cases, center split, the six-word transform, auxiliary patch, and reciprocal
tail remains separate; config loading, application/session/gameplay wiring,
meanings, units, and other editions remain open.

G03-X (`f26520d`) composes those recovered stages while preserving the gaps established by
static dataflow. From the direct-case join to the first helper, the omitted
common prefix always adjusts direct control `+0x148`, can Q16-scale direct
control `+0x148/+0x15c` and all eight direct state words under config flags,
and derives state `+0x48/+0x4c`; it does not touch the
eight-word family `+0x1d0..+0x1ec`, center split `+0x170`, or auxiliary patch
region. The observed later sequence is center split, six-word family transform,
auxiliary patch, and the separate `FUN_00486090` write to state `+0x758`. The
also-omitted `0x406324..0x406459` interval then writes state `+0xec`; control
dwords `+0x144,+0x14c,+0x150,+0x154,+0x160,+0x174,+0x178,+0x17c`,
`+0x180,+0x184,+0x188,+0x18c,+0x1a8,+0x1b4,+0x408`; control byte `+0x440`;
and control word `+0x3e4`
before the reciprocal/copy tail. Its guarded reciprocal from `+0x15c` to
`+0x160` can fault first. `RecoveredCompiledStageEffects` therefore keeps the raw
direct-case outcome, center scalar, auxiliary patch, and four post-tail family
arrays separate rather than inventing a complete state/control snapshot.
At G03-X, `compose_recovered_compiled_stage_effects` resolved the shared config
once, accepted caller-supplied post-prefix `state_word_48`, and validated in
config-before-aux-before-family order only among represented stages. Typed
failure returned no partial bundle, an explicit safe reconstruction policy
rather than shipped in-place fault semantics; success did not prove the shipped
full path reached or published the tail. Five synthetic tests covered a selected
case, invalid-selector common
effects, exact destination relationships, option routing and tail output, and
typed error precedence, bringing `cmr-core` to 1,133. The common prefix,
`FUN_00486090`, the post-helper interval, merged object application,
application/session/gameplay wiring,
meanings/units, and other editions remain open.

G03-Y (`414e90f`) supersedes the G03-X caller-supplied-prefix interface. Static
instruction verification pins exact interval `[0x4060d3,0x4062ba)`: wrapped
`control+0x148-0x51e`; optional raw-config-`+0x25` bit-`0x02` Q16 stores to
`+0x15c` and adjusted `+0x148`; optional bit-`0x04` Q16 halving of all eight
direct state words; wrapped `state+0x48=control+0x140+0x780000`; and guarded
signed `2^32/+0x48` into state `+0x4c`. The exact-write patch preserves absent
conditional stores as `None`; divisors `-1`, `1`, and `2` return a typed atomic
error. This no-result error boundary is reconstruction policy because the
shipped in-place path had already written earlier destinations before `IDIV`.
The config resolver now returns raw unindexed `+0x25`, and the stage composition
derives the prefix for selected and invalid-selector-preserved words, routes its
derived `+0x48` into the auxiliary helper, and reports represented failures in
config-before-prefix-before-aux-before-family order. Eight new synthetic tests
plus strengthened existing composition tests cover the exact write set,
individual and combined flags, unrelated bits, signed/wrapping arithmetic,
fault boundaries, resolver addressing, invalid selectors, destination
disjointness, and precedence, bringing `cmr-core` to 1,141.
`FUN_00486090`, interval `0x406324..0x406459`, complete object application,
runtime reachability, meanings/units, other editions, and application wiring
remain open.

G03-Z (`bcda59c`) reconstructs and integrates the exact no-call interval
`[0x406324,0x40645f)`. A mixed-width exact-write patch carries fifteen control
dwords, state dword `+0xec`, control byte `+0x440`, and control word `+0x3e4`.
It consumes effective post-prefix control `+0x148/+0x15c`; `+0x14c` copies the
former and `+0x160` is zero-to-`-1` or signed `2^32/+0x15c`. Divisors
`-1,1,2` return a typed atomic error, unlike the shipped in-place path where
eight earlier writes are already visible. Composition selects scaled or raw
`+0x15c` correctly and validates represented stages
config-before-prefix-before-aux-before-post-helper-before-family. Five new
tests plus strengthened existing composition tests cover the complete schema,
widths, values, signed boundaries, routing, disjointness, invalid selectors,
and precedence, bringing `cmr-core` to 1,146. The preceding `FUN_00486090`
state `+0x758` effect, complete object application, semantics/units, runtime
reachability, application wiring, and other editions remain open.

G03-AA reconstructs `FUN_00486090` plus its initializer caller selection while
keeping the fixed-address table caller-owned. Context zero and the gated
selection-byte region choose `-1`; other bytes select `5*row+column`, reaching
indices `0..=13,15..=18`. The helper stores non-sentinel dwords verbatim to
state `+0x758`; `-1` resolves checked record-relative config `+0x91 & 7`.
Composition conditionally reads unindexed config `+0x6c0`, returns the state
effect separately, and validates config-before-prefix-before-aux-before-state-
selection-before-post-helper-before-family. Four new tests plus strengthened
tests bring `cmr-core` to 1,150. Static review found the second direct caller at
`0x46d113` and a `+0x738` save/restore store, narrowing the older claim to the
initializer being the only observed originating constructor assignment. Table
values/extent, semantic meanings, generic second-caller provenance, runtime
reachability, complete object application, wiring, and other editions remain
open.

G03-AB reconstructs the bounded immediate post-family no-call interval
`[0x406655,0x4066dd)`. It signed-Q16 multiplies effective post-prefix control
`+0x15c` by reciprocal-family slot 6 (`+0x208`), keeps product bits 16..47 as
`p`, and uses arithmetic `d=p>>2`. Control `+0x164` receives `-1` when `d` is
zero or signed `2^32/d`; exact quotient-fault divisors `-1,1,2` become a typed
safe error. The shipped path writes `+0x164` first, then reloads and recomputes
the product before writing `p` to `+0x168`. The pure helper returns the pair
atomically under stable-memory assumptions, without claiming whole-initializer
atomicity or concurrency equivalence. Composition runs after family success and
uses `control_families.reciprocal[6]`, extending represented failure order with
the product pair last. Four new tests plus strengthened composition tests bring
`cmr-core` to 1,154. Field semantics, complete writers/consumers, runtime
reachability, merged object application, application wiring, and other editions
remain open.

G03-AC maps the remaining instruction-aligned initializer suffix
`[0x4066dd,0x407231)` into bounded arithmetic, reset, call, pointer-backed,
RNG, snapshot, and terminal regions; `0x406d00` is explicitly rejected as a
slice boundary because it bisects an instruction. The first recovered slice,
`[0x4066dd,0x406831)`, derives state `+0x44` from the wrapping Q16 square sum of
arithmetic-halved effective `+0x20/+0x24/+0x28` via the existing exact
`sqrt_q16`, then writes raw integer seeds `[1,2,1]` at `+0x50/+0x54/+0x58`
or `[2,4,2]` when unindexed config `+0x25 & 0x04` is set. Composition routes
prefix-scaled or raw selected/prior state words into the separate effect. Three
new tests plus strengthened schema/composition coverage bring `cmr-core` to
1,157. The sequential reciprocal trio beginning at `0x406831`, later calls,
pointer-backed writes, resets, RNG, snapshot, terminal blocks, semantic names,
runtime reachability, complete object application, and other editions remain
open.

G03-AD reconstructs the sequential state reciprocal interval
`[0x406831,0x4068dc)`. Effective post-prefix state `+0x20/+0x24/+0x28`
produces `+0x38/+0x3c/+0x40` in order through zero-to-`-1` or signed
`2^32/input`; exact fault divisors are `-1,1,2`. The typed safe error records
the failing index/divisor and earlier stores the shipped in-place sequence had
already published, while the pure aggregate returns no effect. Composition
runs the trio after suffix defaults and ProductPair. Three new tests plus
strengthened schema/composition coverage bring `cmr-core` to 1,160. The next
control-clear loop and all later mapped suffix regions remain open.

G03-AE reconstructs the control-clear loop `[0x4068dc,0x4068ff)`. The
containing path carries zero in `EDI`; eight iterations publish sixteen dword
zeros as interleaved pairs across control `+0x31c..+0x338` and
`+0x33c..+0x358`. The pure effect preserves observed store order and is added
to stage composition only after reciprocal-trio success. One new focused test
plus strengthened schema and composition coverage bring `cmr-core` to 1,161.
The repeatedly loaded native control pointer, partial memory-fault visibility,
field meanings, and the half/corner state region from `0x4068ff` remain open.

G03-AF corrects two provisional suffix boundaries and reconstructs the complete
direct object-write effect of no-call interval `[0x4068ff,0x406aaf)`. Effective
state `+0x20/+0x24/+0x28` is arithmetic-halved and expanded into 27 ordered
half/sign dwords. The overlapping reset setup then clears ten control dwords
and four state u16s while raw unindexed config `+0x26 & 0x02` selects four state
dwords as zero or `-0x8000`. Three new focused tests plus strengthened resolver,
schema, and composition coverage bring `cmr-core` to 1,164. Native pointer/config
rebinding, partial publication, the outgoing `ESI=0`/`EBX=2` handoff, meanings,
and the call-bearing region beginning at `0x406aaf` remain open.

G03-AG maps `[0x406aaf,0x406b00)` as an ordered call-spine contract:
state-basis call, eight control-u16 clears, three global seeds, eight-lane
mutation call, then global state transform. Because every call materially
mutates state, the direct local effects are intentionally not appended to the
composed initializer. One focused test brings `cmr-core` to 1,165. Recovery of
`FUN_00409c10` with transitive `FUN_00418120` is the next gate.

G03-AH reconstructs `FUN_00409c10` with its sole helper `FUN_00418120` as a
fixed-shape stable-snapshot contract. The pair derives four primary sign vectors
and four center reflections in state `+0xfc..+0x158`; the reusable helper adjusts
only the primaries from two vectors and three scalars. Six focused tests bring
`cmr-core` to 1,171. Native progressive matrix reads, aliases/store prefixes,
and call-spine integration remain open while `FUN_00419a40`/`FUN_00446c40` and
`FUN_00408fe0` are unresolved.

G03-AI separates the exact caller-side portion of `FUN_00419a40` from its
geometry-dependent `FUN_00446c40` dispatcher. Eight per-lane post-dispatch snapshots
feed tested gate/drive, type-switch, first-four subtraction, accumulator/latch,
and per-lane config-override rules. Six focused tests bring `cmr-core` to 1,177.
The dispatcher and its global geometry/spatial tables remain an injected open
boundary, so this aggregate is not yet composed into the initializer.

G03-AJ reconstructs the `FUN_00446c40` wrapper state machine behind a narrow
geometry-provider interface. The typed controller preserves redundant negative
handle normalization, validate/search/build routing, fallback behavior, ordered
handle-write prefixes, provider-owned output/type writes, and failure phase.
Three focused tests bring `cmr-core` to 1,180. The provider's transitive
geometry, tables, arithmetic faults, and partial internal writes remain open.

G03-AK1 reconstructs `FUN_00446fa0` arithmetic over decoded caller-owned
triangle data. It pins type/raw-vector/final-vector store order, exact Q16
normalization and cross orientation, fallback behavior, and a typed final
division overflow carrying already-published effects. Five focused tests bring
`cmr-core` to 1,185. Raw record/vector lookup and spatial search remain open.

G03-AK2 reconstructs the inner ordered candidate chooser `FUN_00447540` over
an explicit signed count and bounded handle list. It preserves lazy
validate/decode gating, wrapped signed-average scoring, first-winner and strict
replacement behavior, tie stability, and the sole final handle publication.
Five focused tests bring `cmr-core` to 1,190. The outer `FUN_004473f0` spatial
hierarchy/list-offset decoder and raw geometry-table access remain open.

G03-AK3 reconstructs outer hierarchy resolver `FUN_004473f0` over decoded
eight-byte cell views. Wrapping origin subtraction, signed coarse bounds,
row-major indexing, five two-bit refinements, exact sentinel/depth behavior,
and lazy reached-cell failures are covered by four focused tests, bringing
`cmr-core` to 1,194. Candidate-pool adaptation, raw table decoding, and
composition into the dispatcher provider remain open.

G03-AK4 composes the decoded hierarchy, candidate-word pool, and AK2 chooser
without eager full-range validation. Three focused tests preserve hierarchy,
pool-start, provider, and per-ordinal list-error precedence, bringing
`cmr-core` to 1,197. Raw official table acquisition and a concrete geometry
provider that also validates/decodes/builds remain open.

G03-AK5 reconstructs the post-decode `FUN_00446d00` X/Z validator with signed
inclusive bounds and exact wrapping, independently projected Q16 edge scores.
Six focused tests cover orientation, edges, degeneracy, bounds, rounding, and
overflow, bringing `cmr-core` to 1,203. Raw record/vertex decoding and concrete
provider integration remain open.

G03-AK6 reconstructs exact `FUN_00446e80` record/vertex lookup as a safe
type-free decoder with ordered bounds errors. Three focused tests bring
`cmr-core` to 1,206.

G03-AK7 composes the decoded hierarchy, candidate pool, record/vertex tables,
validator, chooser, and builder into a concrete stable-table geometry provider.
The build performs its checked type +6 read only after a fresh vertex decode;
nested failures retain the wrapper/search/build phase and publication prefix.
Five focused tests bring `cmr-core` to 1,211. Raw official table acquisition,
native rebinding/alias/fault behavior, and the eight-lane application remain
open.

G03-AK8 binds that provider boundary to the eight iterations of
`FUN_00419a40`. `execute_compiled_eight_lanes` performs each dispatch and its
caller postprocess before advancing, applies provider type/vector writes before
the post-call type branch, and reloads config through a late fallible provider.
Typed dispatch/config failures retain the exact reconstructed state prefix and
completed-lane trace at the represented wrapper/caller boundary; provider-
internal partial publication remains in the nested typed error. Four focused
tests, including a concrete eight-provider decoded-table run, bring `cmr-core`
to 1,215. Raw official table acquisition, native pointer aliases/rebinding/
machine-fault prefixes, `FUN_00408fe0`, and full call-spine composition remain
open.

G03-AK9 resolves the table provenance and implements the decoded format
boundary: every geometry view consumed by AK3-AK8 comes from the decoded
payload of the stage's `.hpc` sidecar, loaded through the existing CMPR path.
`cmr-formats::hpc::parse_hpc` preserves reserved words and safely validates the
five real hierarchy arrays, vertices, records, candidate handles, dimensions,
and every reachable relationship. A core adapter feeds the parsed snapshot to
the decoded provider while replacing the native sixth-slot vertex-pointer alias
with an unreachable empty slice. The decoded format also retains any opaque
tail the unchecked native view constructor leaves unread. Eight synthetic
tests plus an opt-in owned-media corpus test bring `cmr-formats` to 158 tests;
`cmr-core` remains at 1,215.
G03-AK10 adds the authenticated runtime owner without changing the recovered
14-section render bundle. `StageCompiledGeometry::load` accepts an explicit
profile-relative stage base, resolves and re-verifies its `.hpc` through the
existing `AssetStore`, parses it into an owned snapshot, and exposes eight
providers borrowing that same immutable payload. Seven focused tests cover raw
and CMPR imports, case-normalized lookup, owned lifetime, tamper-before-parse
precedence, authenticated malformed payloads, unsafe/missing identities,
path-free parser diagnostics, and a complete eight-lane execution. `cmr-core` reaches
1,222 tests. No new extractor or media profile is needed: the reviewed generic
profile already authenticates all 76 installed-EU `.hpc` files.
Live application identity/input wiring, the adjacent `.srf`/`.shp` sidecars,
`FUN_00408fe0`, native pointer fault/rebinding behavior, and full call-spine
composition remain open.

G03-AK11 reconstructs post-lane `FUN_00408fe0` over a stable snapshot. Signed
wrapping pseudo-absolute selector words choose one of three four-point quads
with exact B-then-A-then-C tie priority. Two Q16 face vectors are oriented to a
nonnegative second component and normalized; the first is published, then its
wrapping sum with the second is normalized and overwrites the same output.
`derive_compiled_post_lane_global_normal` exposes both publication layers and
the ignored raw final square-sum return. Five focused tests bring `cmr-core` to
1,227. Native global-pointer rebinding/fault prefixes and composition after the
fallible eight-lane executor remain open.

G03-AK12 adds that orchestration boundary. `execute_compiled_eight_lanes_then`
invokes one typed post stage only after all eight lanes and their late config
reads succeed. Lane/config failure skips it; post failure retains the complete
eight-lane success and leaves progressive post publication in the adapter's
typed error. `CompiledPostLaneGlobalNormal` is the concrete AK11 adapter and
feeds source vectors plus final post-config lane words. Five focused tests
bring `cmr-core` to 1,232. Native pointer rebinding/fault prefixes and the
remaining caller continuation after `0x406b00` remain open.

G03-AK13 reconstructs the call-free post-normal basis rebuild at
`FUN_00404440 [0x406b00,0x406e30)`. The pure contract dots the AK11 final
vector with the prior `+0x258` vector using wrapping Q16 arithmetic, removes
that projection, normalizes through the generated sqrt table, and takes the
cross in exact `u × n` order. The verbatim AK11 vector is stored at `+0x264`
(superseding old selector word `+0x268`), then the rebuilt row at `+0x258`,
then the cross row at `+0x270`; the old rows are write-only.
`CompiledPostLaneBasis` consumes the AK11 final publication only after AK8
success. Ten focused tests bring `cmr-core` to 1,242. The following
`[0x406e30,0x406e9a)` pointer publication, destination aliases, rebinding, and
progressive fault prefixes remain open.

G03-AK14 reconstructs that pointer publication. `publish_compiled_post_lane_basis`
writes the rebuilt prior row, the verbatim AK11 row, and the cross row into an
injected destination at strides `+0x00/+0x10/+0x20`, three dwords each,
preserving the padding dwords. `CompiledPostLanePublishedBasis` composes AK11,
AK13, and publication through the AK12 barrier. The following call to the
already reconstructed `FUN_00409c10(state)` is documented; `0x406ea5` begins
the separate state-adjustment phase. Three focused tests bring `cmr-core` to
1,245. Destination aliases, pointer rebinding, and progressive fault prefixes
remain open.

G03-AK15 opens that adjustment phase. `derive_compiled_state_adjustment_cc`
reproduces the wrapping accumulation `state+0xcc += state+0x2e4 - state+0x100`,
and `publish_compiled_state_adjustment` writes `+0xc8`, the updated `+0xcc`,
and `+0xd0` at destination `+0x30/+0x34/+0x38`, preserving the following
dword. The subsequent call order is pinned as `FUN_00492770(state+8,
state+0xc8)`, `FUN_00409c10(state)`, `FUN_004126b0()`; only `FUN_00409c10` is
already reconstructed. Two focused tests bring `cmr-core` to 1,247. The other
two callees and destination alias/fault behavior remain open.

G03-AK16 reconstructs the following reset block: six contiguous state dwords
zeroed at `+0x3cc..+0x3e0`, then `+0xbc = 0`, `+0xc0 = 0x10000`, `+0xc4 = 0`.
The descending control loop clears exactly eight dwords at `0x3b0..0x3cc` in
native order. `derive_compiled_adjustment_reset` pins the constant effects.
Two focused tests bring `cmr-core` to 1,249. The config-dependent flag bytes,
copy loops, and final state writes remain open.

G03-AK17 reconstructs the config-selected control flag (`+0x3f0` is `2` when
config bit `0x10` is clear, else `1`) and the surrounding constant writes:
control dwords `+0x3f4/+0x3f8/+0x3fc/+0x3ec` and bytes `+0x43c/+0x43d/+0x43e`
zeroed, control `+0x37c = -0x4ccc` and `+0x41c = 0`, state dwords
`+0x684/+0x610/+0x678/+0x6e4` and u16 words `+0x60c/+0x614` zeroed, and three
dwords copied from `+0x258` to `+0x5d4`. Two focused tests bring `cmr-core` to
1,251. The `+0x6f8` conditional flag and the `+0x6e8/+0x6ec` words remain open.

G03-AK18 reconstructs the conditional flags: `+0x6f8` is `1` when
`FUN_0046b240()` returns `8` or `FUN_0048d2d0()` returns nonzero, with the
second callee short-circuited, then `+0x6f0 = 1`. The `+0x6e8/+0x6ec` words
are zero when `DAT_00550c08 == 2` or `DAT_00d3ca88 != 0`; otherwise `+0x6e8 =
1` and `+0x6ec` is the signed remainder of the RNG draw by two times ten plus
`0xd`, reachable values `3`, `13`, and `23`. Two focused tests bring
`cmr-core` to 1,253. The copy loops and final state writes remain open.

G03-AK19 closes the direct effects of `FUN_00404440`: copy loops move 24
dwords from `+0xfc` to `+0x1bc` and 9 dwords from `+0x258` down to `+0x21c`;
then `+0x240` receives `+0xc8/+0xcc/+0xd0` and `+0x24c` receives the AK11
final vector. State bytes `+0x74d/+0x74e` and control byte `+0x428` are
zeroed, control `+0x420 = 1`, `DAT_0050c5fc = ((~config_25 & 1) << 0xf) |
0x1999`, and `FUN_0046eda0()` runs only when `DAT_00d3ca88 != 0`. Three
focused tests bring `cmr-core` to 1,255. The remaining callees and full
call-spine composition remain open.

G03-AK20 reconstructs `FUN_0048d2d0`: false at detail `8`; otherwise weather
`3`/`4` yields `DAT_00592efc != 1`, weather `5` yields true, and every other
input yields false. The weather and intensity getters are evaluated only on
their native short-circuit paths. `FUN_0046b240` is confirmed as the
`DAT_00592f00` getter and `FUN_004cfac0` as the already reconstructed LCG. One
focused test brings `cmr-core` to 1,256. The `FUN_00492770` switch stores
remain open.

G03-AK21 reconstructs `FUN_00492770`: the switch at `param_1+0x180` stores the
three source dwords at `+0xc4/+0xc8/+0xcc` for cases `0` and `3`, adds stores
at `+0x10/+0x14/+0x18` for cases `1` and `2`, and stores no vector otherwise.
Every path zeroes `param_1+0x188` and calls `FUN_00492050`. One focused test
brings `cmr-core` to 1,257. `FUN_00492050` remains unreconstructed; the
`FUN_004126b0` lane threshold scan follows.

G03-AK22 reconstructs `FUN_004126b0`: starting from `+0x742 = 0` and
`+0x743 = 1`, it scans lanes `7` down to `0` with the strict signed test
`lane_word + 0x1999 < source_y` (wrapping addition). Passing lanes set their
`+0x73a` flags; failing lanes clear them and `+0x743`, append their indices at
`+0x744 + count`, and increment `+0x742`. Two focused tests bring `cmr-core`
to 1,259. The `FUN_0046eda0` global copy gate follows.

G03-AK23 reconstructs `FUN_0046eda0`: only when `DAT_00550c08 == 1`, it copies
`0x119` dwords from `DAT_005fffc8` to `DAT_0060042c` and `0x1da` dwords from
`DAT_00700300` to `DAT_00700a68`. Each destination begins immediately after
its source range, so the blocks never overlap. One focused test brings
`cmr-core` to 1,260. The copied global tables are runtime data. All direct
effects and callees of the initializer are now contracted; full call-spine
composition follows.

G03-AK24 reconstructs `FUN_00492050`: it follows the node link at `+0x8` and
sets the `+0x170` flag to `1` on every node until the null link, doing nothing
for a null initial pointer. `apply_compiled_92050` reproduces the effect over
an injected flag slice. One focused test brings `cmr-core` to 1,261.

G03-AK25 composes the post-lane spine in native order.
`execute_compiled_spine_post` runs the eight-lane executor (aborting on
failure), AK11, AK13, the AK14 publication, the adjustment and its
publication, `FUN_00492770`, the lane thresholds, the reset, the control
flags, the conditional flags and RNG word, the copies, `DAT_0050c5fc`, and the
final call gate, all through the pure callee contracts. Two focused tests
bring `cmr-core` to 1,263. Progressive faults, pointer rebinding, and the
early initializer stages remain open.

G03-AK26 completes the composition. `compose_compiled_initializer` chains the
early post-dispatch stages, the basis transform, the call-spine locals, and
the eight-lane/post-lane spine into one typed result. An early failure returns
before any lane runs; a lane failure aborts before the post-lane spine. Three
focused tests bring `cmr-core` to 1,266. The exact native dataflow into the
injected inputs remains a separate boundary; wiring the composition to real
imported stage and car/config inputs is next.

G03-AK27 proves the composed initializer over an authenticated snapshot: a
CMPR-wrapped reviewed `.hpc` loads through `StageCompiledGeometry` and drives
`compose_compiled_initializer` to completion with the reviewed synthetic
config. All eight lanes finish with the decoded record type and built output
vector, and the AK11 normal, AK13 basis, and copies are independently
re-derived from the final lane words. One focused test brings `cmr-core` to
1,267. The car config record remains a reviewed synthetic fixture; recovering
its car-data source and live stage identity is next.

G03 continues into the per-wheel force pass. The slip candidate is
`dot * 4 - wheel_speed` (wrapping), the slip force is
`qmul(slip, DAT_0050c600) * 4` clamped symmetrically to the per-wheel limit at
`ctl+0x378`, and the downshift is `5` with the per-wheel flag set, else `2`
when both config dwords are zero, else `3`. `cmr-core::drivetrain` gains
`wheel_slip_candidate`, `slip_sign_disagreement`, `clamp_wheel_slip_force`,
and `wheel_slip_shift`. Four focused tests bring `cmr-core` to 1,271. The
slip-force gate, per-wheel drive application, and wheel-speed integration
remain open.

G03 next recovers the per-wheel drive application (split and wheel constant
added on negative speed, unchanged at zero, subtracted on positive), the slip
gate (front always, rear only without handbrake or with sign disagreement),
the wheel-speed integration `speed + qmul(force, DAT_0050c5c0)`, and the
zeroing gate (sign flip when `gear == 0`, `ctl+0x40c != 0`, or `ctl+0x158`
equals the per-wheel constant; otherwise a gear-ratio sign mismatch).
`cmr-core::drivetrain` gains `apply_wheel_drive`, `apply_wheel_slip`,
`integrate_wheel_speed`, and `gate_wheel_speed`. Four focused tests bring
`cmr-core` to 1,275. The second per-wheel force loop remains open.

G03 recovers the second-loop ratio gate `|dot * 4 - speed| < 0x10000`
(pseudo-absolute, so `i32::MIN` passes), the sign-aware residual saturation
(residual reduced by the force only when they share sign and the force is
smaller in magnitude, zeroed when it is not smaller, unchanged on opposite
signs or zeros), and the gear-ratio gate (residual zeroed when `ctl+0x2ec == 0`
and the ratio is zero or has the opposite sign). `cmr-core::drivetrain` gains
`wheel_ratio_within_unit`, `residual_with_saturation`, and
`gate_residual_by_gear_ratio`. Three focused tests bring `cmr-core` to 1,278.
G03 recovers the second-loop residual computation: `-qmul(qmul(ratio,
0x4000), DAT_0050c5c4)` with an optional `0x147a` factor when the per-wheel
flag is set, and `-qmul(dot, DAT_0050c5c4)` for the third residual, all with
wrapping negation and multiplication. The normalization stage is selected when
`|residual| > 0x10000` (pseudo-absolute, so `i32::MIN` is not large).
`cmr-core::drivetrain` gains `second_loop_residual`,
`second_loop_axis_residual`, and `second_loop_ratio_is_large`. Three focused
G03 recovers the large-ratio normalization branch: `normalizer = |local_30|`
and `reciprocal = 2^32 / normalizer` (signed division, `-1` when zero), the two
direction sources scaled by `qmul(local_2c, reciprocal)` and `qmul(local_30,
reciprocal)` and summed, then normalized with the legacy generated-table rule
(zero square sum yields zero direction). The length is `qmul(square_sum,
normalizer)`. `cmr-core::drivetrain::normalize_second_loop_direction`
G03 recovers the small-ratio branch selection: the normalizer is `|local_30|`
when `|local_2c| < 0x10001` or `|local_2c| < |local_30|`, and `|local_2c|`
otherwise. Both branches scale and sum the direction sources, normalize with
the legacy rule, and compute `length = qmul(square_sum, normalizer)`. Two
G03 recovers the post-normalization arctan index fold: a zero product yields
`0`, a zero denominator with a nonzero product yields `0x400`; otherwise the
ratio is folded into the first octant (`min/max`, never above `1.0`), indexed
by `(numerator << 16) / denominator >> 7`, complemented (`0x400 - index`) when
the original ratio exceeded `1.0`, and negated on opposite signs.
`cmr-core::drivetrain::arctan_ratio_index` reproduces it with the runtime-built
G03 recovers the gated accumulation factor: produced only when the speed
delta and gear ratio share a nonzero sign, `qmul(qmul(delta, 0x4000),
qmul(DAT_0050c5c4, 0x1999))`, with the result replaced by the signed limit
when the per-wheel limit is below its magnitude. `accumulate_direction_force`
reproduces the wrapping `accumulator += qmul(component, factor)` accumulation.
Two focused tests bring `cmr-core` to 1,290. The table-driven final
G03 recovers the final table-driven accumulation factor: no factor when the
direction dot is zero; otherwise `dot >> 2` scaled by
`qmul(DAT_0050c5c4, table_factor)` when the table entry is present, or
`qmul(DAT_0050c5c4, 0x8000)` otherwise, then negated with wrapping.
`final_loop_accumulation_factor` reproduces it. One focused test brings
G03 recovers the force-pass head projection-normalization:
`rejected = state - normal * dot(state, normal)` with wrapping Q16.16
arithmetic, then the legacy generated-table normalization (zero square sum
yields a zero vector). The normal is the AK11 final vector; the two state
vectors are at `+0x550..+0x558` and `+0x544..+0x54c`.
`cmr-core::drivetrain::project_and_normalize` reproduces it. Two focused tests
bring G03 recovers the force-pass head cross-projection: `cross = v x u` with exact
right-handed Q16.16 products and wrapping subtractions, then projected against
the AK11 normal with the same rejection and legacy normalization.
`cmr-core::drivetrain` gains `cross_q16` and `cross_and_project_normalize`.
Two focused tests bring `cmr-core` to 1,295. The config-gated vector selection
G03 recovers the drive factors (`DAT_0050c5c4`, `DAT_0050c600`, and
`DAT_0050c5c0`) and the alternate-branch pair orthogonalization:
`scaled_dot = qmul(qdot(v, w), 0x20000)`, `rejected = w - v * scaled_dot`,
then `result = v - rejected * qdot(v, rejected)`, normalized with the legacy
generated-table rule. `cmr-core::drivetrain` gains `compute_drive_factors` and
G03 recovers the config-gated vector selection: the alternate
pair-orthogonalization branch runs when `DAT_00d3ca80+0x26 & 0x80` is set, or
when `DAT_00d3ca88 == 0` and `FUN_0042a840()` returns nonzero; otherwise the
projection results replace the initial direction sources only when
`DAT_00d3ca80+0x25 & 0x10` is set. `select_head_vector_path` reproduces the
branch selection, and `HEAD_BASIS_STORE_OFFSETS` pins the four basis rows at
`0x380`, `0x38c`, `0x398`, and `0x3a4`. Two focused tests bring `cmr-core` to
G03 composes the first per-wheel iteration: `execute_first_wheel_iteration`
applies the drive, slip candidate and sign-disagreement, the slip gate and
clamped/downshifted slip force (or a zeroed redrive), wheel-speed integration,
and the zeroing gate in native order. Three focused tests cover the full slip
path, the closed-gate redrive and speed flip, and the rear handbrake-off slip
path, bringing `cmr-core` to 1,303. The recovered rules are now a coherent
G03 recovers the second-loop cosine modulation:
`modulated = qmul(coefficient, angle_cos)`, then
`root = legacy_sqrt(qmul(angle_cos, angle_cos) + qmul(modulated, modulated))`,
and finally `qmul(root, config_scale)`, with the cosine supplied by the
injected runtime-built table. `cmr-core::drivetrain::cosine_modulated_component`
G03 composes the second-wheel core: `execute_second_wheel_core` applies the
ratio and residuals, residual saturation and gear-ratio gate, the two
residuals, direction normalization, arctan folding, and cosine modulation in
native order. The independent re-derivation test checks every stage, bringing
G03 completes the second-loop accumulation: the `ctl+0x420`-gated factor
(sign-matched speed delta and gear ratio, scaled and clamped) and the final
table-driven factor (`dot >> 2` scaled by the table factor or `0x8000`,
negated), both accumulating `qmul(component, factor)` into the three force
accumulators with wrapping addition, using the head's first direction source.
The independent re-derivation test additionally checks both factors and the
G03 recovers the axle factor and travel update: the front/rear factors are
`qmul(car_type_constant, travel)` (front `DAT_0050e19c`, rear `DAT_0050e1a0`,
travel `DAT_0050e184`), and the travel update decrements by
`min(0x10000, qmul(excess, 0x8000))` on excess, bumps `DAT_0050e160` by
`min(0x10000, qmul(excess, 0xccc))`, then applies the surface bump
(`0x147`/`0x20` by the surface flag), capped at `0x10000` with the flag
cleared. `cmr-core::drivetrain` gains `derive_axle_factor`,
`update_travel_on_excess`, `apply_surface_bump`, and `surface_bump_value`.
G03 recovers the car-type runtime defaults: `DAT_0050e19c = 0x10000`
(front), `DAT_0050e1a0 = 0x10000` (rear), `DAT_0050e184 = 0x10000` (initial
travel), and `DAT_0050e18c = 0` (per-wheel constant), all written by
`FUN_0041e3f0` for every car type. With these defaults the axle factors are
simply the travel value and the wheel drive adds no per-wheel bias.
`default_car_type_constants` returns them. Two focused tests bring `cmr-core`
G03 derives the gear ratio and auxiliary values:
`ratio = qmul(original_at_0x40, DAT_0050c860)` and
`value = qmul(original_at_0x60, reciprocal)`, with the reciprocal
`0x100000000 / iVar12` or `-1` when `iVar12 == 0`. The original ratio values
are runtime car data. `cmr-core::drivetrain` gains `derive_gear_ratio`,
`derive_gear_aux_value`, and `reciprocal_or_minus_one`. One focused test
G03 wires the gear ratio from the control families: `raw_copy` (block
`0x210`) feeds the final ratio with `DAT_0050c860`, and `reciprocal_copy`
(block `0x230`) feeds the auxiliary value with the reciprocal. Those blocks
come from the recovered initializer's control-family stages, so the gear ratio
wiring is closed without an external loader.
`derive_gear_ratio_and_aux_from_families` composes them. One end-to-end test
G03 recovers the per-surface capability enable counters:
`FUN_00424a40` marks enabled surface ids, then accumulates each enabled id's
per-surface capability byte into the corresponding enable counter (16 counters
over 120 surface ids, duplicate ids enabled once).
`accumulate_effect_enables` reproduces it with a safe-contract out-of-range
rejection. Two focused tests bring `cmr-core` to 1,313. The per-surface
capability tables are game data (not vendored).
G03 composes the initializer from a reviewed case store: a synthetic
CMPR-style store is opened through `CompiledInitializerStore`, converted to a
`CompiledDirectCaseSet`, and run through `compose_compiled_initializer`. The
integration test independently checks the selected direct state/control case
and derives the gear ratio and auxiliary value from the resulting control
families, bringing `cmr-core` to 1,314. Reviewed synthetic inputs prove the
storage-to-consumer contract; they are not a live car-data run, and the
per-surface capability tables remain caller-supplied game data.
G03 establishes the capability-table provenance: fresh `FUN_00424a40`
pseudocode identifies sixteen contiguous PE-backed 120-byte tables at
`0x4ec0f8`; the fifth enable counter instead reads runtime-backed
`DAT_0054f824` and is outside that span.
`cmr-formats::pe_surface::extract_official_eu_surface_capabilities` performs a
bounded 1,920-byte extraction in address order, with two synthetic tests for
exact ordering and malformed spans. `cmr-formats` now has 160 library tests.
Reviewed import/store wiring and the separate runtime-backed table remain open.
G03 adds the reviewed capability import pipeline: a metadata-only official-EU
profile and `cmr-assets extract-surface-capabilities` authenticate the source,
check the PE span, and atomically publish the 1,920 user-derived bytes plus
path-free state beneath an explicit ignored output root. Two library tests and
one CLI test cover profile shape, synthetic bytes, and command behavior;
`cmr-assets` now has 78 library and nine binary tests. An authenticated local
import passed without exposing values. The immutable store/counter wiring and
runtime-backed fifth table remain open.
G03 adds the immutable capability store and counter adapter. The store owns all
sixteen address-ordered tables after authentication and shape checks. The
adapter maps counter indices to PE tables `[1,0,2,3,runtime,4..11,13,12,14]`,
with the separately supplied runtime fifth table; the trailing PE table is not
consumed by `FUN_00424a40`. Two store tests bring `cmr-assets` to 80 library
tests, one adapter test brings `cmr-core` to 1,315, and the opt-in official
extraction/reopen smoke passes. Runtime-table production and application/session
wiring remain open.
G03 establishes the runtime fifth table's startup boundary: all three literal
sites are reads, no direct writer is analyzed, and the address has no file
mapping. The loader-established table is therefore zero.
`ZERO_INITIALIZED_RUNTIME_FIFTH_CAPABILITY_TABLE` exposes that value and one
test brings `cmr-core` to 1,316. Indirect runtime writes remain unknown;
application/session wiring is still separate.
G03 adds `cmr-app::SessionSurfaceInputs`, which owns the reviewed capability
store and its zero-initialized runtime fifth table. Its `effect_enables` method
computes counters only for caller-supplied stage IDs; no stage IDs are invented.
Two tests bring `cmr-app` to 23 library tests, and the opt-in official
session-open smoke passes. Actual stage-ID extraction and weather/render
consumption remain open.
G03 recovers the stage-ID producer: `FUN_00480a10` loads a `.srf` sibling and
passes its decoded bytes and loader byte count directly to `FUN_00424a40`.
`SessionSurfaceInputs::open_stage` reproduces that input through the bounded
CMPR-aware reader, and `stage_effect_enables` computes the counters. One test
brings `cmr-app` to 24 library tests; the real AU1 path passes. Weather/render
consumption remains open.
G03 wires the recovered weather counter identities into `StageWeatherEffects`:
splash index 0, dust index 2, and snow index 5 become boolean gates from the
`.srf`-driven counters. `SessionSurfaceInputs::stage_weather_effects` composes
them. One core test brings `cmr-core` to 1,317; app coverage and the real AU1
path pass. Other counters remain unnamed and particle/render spawning is separate.
E10 applies those gates at particle-type dispatch: dust, snow/snow-cloud, and
splash are suppressed when disabled, while unrelated classes remain unchanged.
`SessionSurfaceInputs::stage_particle_effect` composes the same behavior from
`.srf` inputs. One focused test brings `cmr-core` to 1,318; app composition and
invalid-ID propagation are covered. Actual particle pool/render spawning remains
separate.
E10 reconstructs the 80-byte `FUN_00424f40` spawn record and its eight
100-slot pools: copied vectors/scalars, sprite/tuning type selection, frame and
flags, and modulo-100 advancement. `stage_spawn_particle` applies weather gates
before creating records. Two tests bring `cmr-core` to 1,320; app coverage checks
creation and suppression. Unknown fields keep neutral offset names and render
traversal remains separate.
E10 recovers the common `FUN_00427d20` update traversal: active records integrate
position and `0xf333`-damped velocity, selected vertical velocities gain the
`-0x34` term, selected lifetimes decrement, `word_38` advances, and age/lifetime
rules deactivate records. `advance_common` implements the shared path and
preserves `word_38` on respawn. Two tests bring `cmr-core` to 1,322.
Type-specific physics and final sprite rendering remain separate.
E10 recovers the clod and splash branches: the Q16 polynomial and
ground-relative clod motion/reset/deactivation, plus splash damping,
random-delta adjustment and relative-Y deactivation.
`advance_type_specific` runs them before the common update, and deactivation
now clears the recovered fields and Y sentinel. Two tests bring `cmr-core` to
1,324. Final sprite rendering remains separate.
E10 recovers the bounded `FUN_00429910` render-reference queue: typed resource
identity, depth and sequence order with a 100-entry cap.
`SpriteRenderQueue` rejects the 101st reference. One test brings `cmr-core` to
1,325. Pool-field depth mapping and final draw submission remain separate.
E10 composes active records into render references using camera-relative vector
length and the recovered square-root approximation for depth.
`ParticlePool::render_queue` excludes deactivated records and produces typed
resource/depth/sequence entries. One test brings `cmr-core` to 1,326. Final draw
submission remains separate.
E10 recovers final sprite draw submission: `FUN_00427c50` walks the queue into
`FUN_004a4330`, whose 84-byte records contain positional packet/resource fields,
overlapping kind bytes and the tag. `SpriteDrawSubmissions` reproduces the exact
layout and the 600th single-primitive rejection. One byte-level test brings
`cmr-core` to 1,327. Backend submission remains behind this contract.
G09 now wires the recovered half-delta translation into `race_state`: the
already-computed acceleration drives `apply_translation_step` with zero damping,
then lateral grip is applied. This preserves the harness order while replacing
`position += velocity * dt` with the official position formula. One independent
test pins the formula, bringing `cmr-core` to 1,328. The force model remains an
explicit harness input; drive-torque-to-contact-force wiring remains open.
G09 adds `step_with_first_wheel`: the recovered first-wheel force pass runs
with explicit runtime inputs and its force drives the existing translation.
A two-tick test checks split, slip clamp, integrated speed and vehicle movement,
bringing `cmr-core` to 1,329. Full force production remains open.
G09 adds `step_with_second_wheel`: the second core consumes the first-wheel
speed delta and explicit vector/table inputs, recording residual, angle,
component and accumulators. Its composition test brings `cmr-core` to 1,330.
Full force production and accumulator feedback remain open.
G09 now completes the second-core lane accumulation in native order: selected-scalar
scaling of the normalized direction, gated correction, projection of that updated
accumulator for the second correction, then the independent surface addition.
This corrects the previous zero-initialized composition. The remaining consumer is
the four-lane `FUN_0040fbe0` postprocess, not direct translation. G09 now wires
that postprocess after the second core, retaining every lane accumulator and the
qualification results; invalid lane selection is rejected before tick work. One
composition test brings `cmr-core` to 1,331. G03 now composes per-lane candidate
production through `select_second_wheel_candidate`, covering both lookup paths
and the strict normalization-length selection boundary with one additional test.
The second core now derives that scalar internally from residuals and runtime
bases, with checked lookup failures propagated through both race-state methods.
The downstream order is now pinned through `FUN_0040ce40` -> `FUN_0040d810` ->
`FUN_0040d400` -> `FUN_0040a4a0`. `sum_translation_lane_forces` reproduces the
consumer's base/non-wheel/contacted-wheel net-force sum, with one additional test
bringing `cmr-core` to 1,333. The four-lane race-state path now performs that sum
after postprocessing and translates with the resulting world force in the same
tick; its test checks negative motion from the negative contacted lane.
`apply_contact_vector_update` now reconstructs one `FUN_0040d400` projection,
normalization, extra-length subtraction and distinct threshold/limit clamp, with
one additional test bringing `cmr-core` to 1,334. `sum_contact_moments` now
reproduces transformed-force cross contact-offset accumulation with the same
contact filtering, bringing `cmr-core` to 1,335. `transform_contact_force` and
`transformed_contact_moment` now reproduce the row-wise `FUN_0048e100` transform
before the cross product, with one additional test bringing `cmr-core` to 1,336.
`apply_angular_momentum_step` now scales and combines moments, applies per-axis
inertia, and negatively integrates angular velocity, bringing `cmr-core` to 1,337.
`clamp_angular_velocity` now applies the gated signed `0x3333` special case with
wrapping-absolute semantics, bringing `cmr-core` to 1,338. The generalized
`FUN_004184e0` orientation update now builds each axis-angle matrix, transforms
and normalizes the basis in native `1,2,0` order, bringing `cmr-core` to 1,339.
`orientation_angle_increment` and `integrate_orientation_from_velocities` now
connect the half-delta angular rate through the single `0x28b` conversion to that
sequence, bringing `cmr-core` to 1,340. `apply_control_axis_resets` now covers
both selective angular reset branches and their priority, bringing `cmr-core` to
1,341. `apply_contact_vector_updates` now traverses all eight records in reverse
order while preserving skipped destinations, bringing `cmr-core` to 1,342.
`apply_angular_orientation_pipeline` now composes momentum, clamp, angle
conversion, selective resets and orientation in native order, bringing `cmr-core`
to 1,343. `suppress_low_speed_position_delta` now reproduces the active-contact
`0xccc` gate without changing velocity, bringing `cmr-core` to 1,344. The gate
now runs inside `apply_translation_step`, with four-lane activity derived from
contact flags and ordinary harness paths unsuppressed, bringing `cmr-core` to
1,345. `step_with_all_four_wheels` now runs all first iterations in reverse,
equalizes axles, runs all second iterations in reverse, then postprocesses and
translates, bringing `cmr-core` to 1,346. This is complete composition over
explicit runtime inputs. `drive_stage_with_all_four_wheels` now completes the
vertical stage and reproduces the identical outcome on replay, using actual prior
speed for low-speed gating. One test brings `cmr-core` to 1,347.
`populate_compiled_gear_ratio` now maps the selected compiled control-family word
into both wheel parameter sets with invalid-gear atomicity, bringing `cmr-core`
to 1,348. `derive_force_pass_scales` and
`derive_alternate_force_pass_factor` now recover the three main scale products
and the early factor, bringing `cmr-core` to 1,349. Runtime scalars remain
injected. `populate_force_pass_scales` now applies one derived bundle to both
wheel parameter sets, and the vertical path uses it for every wheel, bringing
`cmr-core` to 1,350. `populate_surface_table_factor` now maps the reviewed first
family into second-core accumulation with checked indices; an authenticated real
store also reopens and feeds the adapter. One synthetic and one opt-in test bring
`cmr-core` to 1,352. `cmr-app::SessionWheelInputs` now exposes the authenticated
coefficient store to second-core callers; its official opt-in smoke passes,
bringing `cmr-app` to 27 library tests. G03 now recovers the strict `0xcccc`
distance gate, count scaling, and four response-word timestep products with three
focused tests, bringing `cmr-core` to 1,355.
`build_contact_basis_vectors` now reconstructs the twelve primary scalar dwords,
mirrored negatives, eight-vector layout and velocity translation from
`FUN_00409f90`, bringing `cmr-core` to 1,356.
`populate_contact_basis_vector` now selects a checked vector into each second-wheel
surface input, with invalid-index atomicity; the four-wheel vertical fixture derives
all four contact inputs through this adapter and still finishes/replays exactly.
One focused test brings `cmr-core` to 1,357. The complete four-wheel step rebuilds
all contact vectors on every tick and validates all lane indices before wheel-state
mutation. `MotionState` now owns the native basis, primary angular velocity and
compiled half-word inputs, replacing the duplicated contact template; its adapter
forms the six angular words and uses actual pre-step velocity. One native-field test
brings `cmr-core` to 1,358. Composition coverage proves velocity provenance,
invalid-index atomicity and deterministic finish/replay. The four-wheel step now also
runs the recovered momentum/clamp/reset/orientation pipeline after translation and
stores basis, angular velocity and scaled prior moment back into `MotionState`, so
the next contact-production tick consumes the updated native state. Current
postprocessed wheel forces are now transformed through that pre-update basis and
crossed with contact offsets, contact-filtered, and added to the explicit base moment
before angular integration. The composition test independently recomputes this complete
moment/orientation result. `populate_compiled_contact_offsets` now maps the recovered
`+0x5c/+0x68/+0x74/+0x80` sign vectors from `CompiledHalfSignState` into all four
wheel inputs, with exact signs and Q16-scaled angular coverage.
`GameplaySession::new_with_four_wheel` now selects the complete producer, and
`try_advance` runs a state-5 step before the FSM frame so checked failures leave the
FSM, checkpoint progress and tick count untouched. One success/error test brings
`cmr-core` to 1,359; the simplified session API remains unchanged.
`SessionWheelInputs::four_wheel_params` now supplies reviewed coefficients, compiled
gear ratios, force scales and compiled offsets, with atomic surface-index rejection.
`run_headless_four_wheel_game_session` drives the complete FSM using those inputs and
checked errors. A synthetic authenticated finish/replay test and an opt-in real-store
smoke bring `cmr-app` to 29 library tests. The remaining scalar tuning and start state
are explicit harness inputs, not a trajectory-equivalence claim. The complete producer
now also runs `apply_contact_vector_updates` after lane postprocessing and before
translation/moment integration, replacing all eight input vectors from the current
contact basis and preserving skipped destinations in `FourWheelState`. The composition
test compares against the independent kernel. The update projection axis now comes
from the compiled AK11 normal/basis, `+0x528` is count-scaled `+0x48`, the extra vector
is reset, wheel enables come from contact flags, and limits/thresholds come from the
compiled control-clear loop. Only the global update gate and non-wheel enable flags
remain explicit in this path. Each four-wheel step now also constructs all four A/B
lane-direction pairs and uses the same outputs for second-wheel accumulation and
geometric postprocessing, preserving independent C vectors and active flags. The
projection axis and `+0x264` cross operand come from `MotionState`.
`resolve_lane_direction_mode` now implements Rebuild-before-Exchange/Preserve, and
`derive_lane_direction_pair` copies S at zero angle or rotates it through the recovered
axis-angle matrix before the raw R cross. Two tests bring `cmr-core` to 1,361.
`stabilize_lane_source_s` now reproduces the rejection, residual-length,
response/timestep blend, final normalization and verbatim `+0x258` fallback, with S
persistent in `MotionState` for the next tick. One normalization/fallback test brings
`cmr-core` to 1,362. Contact-index validation occurs before any S mutation, preserving
rejected-step atomicity. The interactive windowed session now uses the same
authenticated four-wheel constructor and `try_advance`; SDL frame closures stop and
return checked producer errors through `WindowedStageFrameError`. A displayed run is
not verified in this environment. The global contact-update gate now comes from the
transformed projection axis, and all eight enable bytes come from
`derive_compiled_lane_thresholds`; `+0x528` now correctly uses the failing-lane count
rather than active-wheel count. One direct gate test brings `cmr-core` to 1,363.
G09 is complete as a deterministic offline-trace contract: the full four-wheel producer,
checked errors, start/finish/replay and authenticated sessions are covered. Explicit
harness tuning is documented and does not imply trajectory equivalence. G10 is also
complete under its documented harness contract: start, drive, checkpoint finish,
save/replay, the complete producer and application wiring are all covered. Static
`DAT_004eac6c/70/74` are now recovered as `SOLVER_GLOBAL_CONSTANTS` (all `0x10000`)
and drive lane stabilization, angular timestep and force scaling in the native
application path. One constant-evidence test brings `cmr-core` to 1,364.
The second-wheel adapter now derives wheel/state and direction/contact Q16 dot
products and the independent `0x147a` residual enable. Static review corrected the
lane split to 0/1 versus 2/3 and established that `+0x43b` plus `+0x158` controls
this flag without the separate slip-sign disagreement. One focused test brings
`cmr-core` to 1,365. Constructed A directions and current contact vectors now feed
both wheel loops through `populate_wheel_vector_inputs`. The existing canonical
source-owned arctangent table is exposed with its exact-1.0 adjacent zero sentinel,
replacing empty producer/application tables. Two focused tests bring `cmr-core` to
1,367. `reset_wheel_speed_state` now recovers the reset/respawn speed writes and
unconditional `+0x43b=10`; application parameters use that post-reset enable while
zero start speeds remain explicit harness input. One all-branch test brings
`cmr-core` to 1,368. The per-frame residual enable now persists as a countdown:
mode-selected raw speed and prior `state+0xec` drive the strict `-0x1998` branch,
and translation updates that speed through the legacy square root. One focused
boundary/mode test brings `cmr-core` to 1,369. Smoothed speeds now have separate
persistent state and ordinary native tracking: snap below `0x10000`, otherwise step
one Q16 unit, then apply the strict `0x7ae` dead zone. The second loop reads those
values rather than raw speed. One focused test brings `cmr-core` to 1,370; driven
integration and neutral-gear handling remain. The driven gate and integration are
now pure recovered contracts, including mode-dependent throttle/brake mismatches,
raw/sign/asymmetric clamps and the shared dead zone. Two tests bring `cmr-core` to
1,372. The distinct `+0x2c8` force array and global scalar producers must be wired
before enabling this branch in the application. Neutral gear now bypasses tracking
and zeroes all four smoothed speeds, resuming ordinary tracking for a nonzero gear.
One transition test brings `cmr-core` to 1,373. Driven force/scalar production and
the separate engine-speed store remain. Driven scalar recovery now includes the
suspension slip gain, clamped engine load, signed speed limit and the longitudinal/
gear-reciprocal direction gain with native zero and negative-ratio behavior. One
exact-value test brings `cmr-core` to 1,374. Input population and `force_2c8`
production remain next. Smoothed engine `+0x2e8` now comes from the native axle
blend and load-dependent acceleration cap, with neutral copying `+0x2e0`; both
engine fields persist in `FourWheelState`. One formula/cap test brings `cmr-core`
to 1,375. The ordinary engine path now advances `+0x2e0` from raw axle speeds with
native middle-16-bit arithmetic, applies the rev limiter, stores `+0x444/+0x448`,
and then performs smoothing. One fractional-update test brings `cmr-core` to 1,376.
Neutral/idle special cases and `force_2c8` production remain. The ordinary
`state+0x530` force and common neutral/idle update are now wired with guarded
`6e8/6ec` state, over-redline handling, limiter cut and rev limiting. One branch
test brings `cmr-core` to 1,377. Countdown special cases and runtime response/brake
production remain. The `6e8 != 0` reset now clears/reloads at signed countdowns
below two and decrements larger values before rev limiting. One boundary test
brings `cmr-core` to 1,378. Nonzero-idle oscillation and `force_2c8` production
remain. The nonzero-idle oscillation now replaces RPM through the generated sine
phase, decrements its countdown and writes `+0x2d8` at the native threshold.
Persistent response state feeds lane stabilization and engine force. One exact
sine/threshold test brings `cmr-core` to 1,379. `force_2c8` production remains.
G08's four-byte actuator record is now recovered and implemented with digital
`-66` decoding, mode flags, double-XOR current-field semantics and the repeat
counter. Two byte-exact tests plus native vertical trace decoding bring `cmr-core`
to 1,381. Region A is now fully sectioned with checked offsets and an exact
500-object bound; one layout test brings `cmr-core` to 1,382. `.SSR` writing is
explicitly unsupported by the resolved static callers (save-slot/config only),
with indirect-call coverage caveats. Network framing remains. Network replay uses
the same four-byte records with native duplicate suppression; one mask test brings
`cmr-core` to 1,383. G08 is complete, with `.SSR` writing explicitly unsupported
and transport packet envelopes assigned to networking. G07 is complete: the
20-byte display record round-trips, the visual FIFO rotates/evicts/accumulates
with native depth behavior, and audio routing uses replay-car and active-player
counts. Two display/FIFO tests plus count-gate coverage bring `cmr-core` to 1,385.
G06 is complete: replay data slots and simulated cars have separate recovered
counts, and `generate_rival_times` covers tier curves, LCG jitter, exclusions,
remapping and difficulty placement. One setup test and two generator tests bring
`cmr-core` to 1,386. No on-track AI is inferred.
F06 now pins the ordinary post-stage writer and country transition: the native
commit increments progress and the packed stage without rollback, country
advance resets the stage and clears cumulative times, and direct ranking
accesses resolve time at `+0x16c` versus positions at `+0x170`. Two tests bring
`cmr-core` to 1,388. Custom scheduling and record/unlock residuals remain.
Both record tables are now proven to persist inside the same `0x1e20` settings
image saved as `cmRally.cfg`; checked ranges reject invalid inputs and record
bytes survive `SettingsImage`. `FUN_0044da00` establishes dynamic country/
standings-driven rival selection, with its forced-country lookup implemented.
Two tests bring `cmr-core` to 1,390. Custom scheduling remains next.
F06 is complete: sub-mode 1 now honors fixed/variable limits and forced
countries, while sub-mode 2 scans sparse stage masks, applies two-player and
special-country rules, and rejects invalid table indices safely. Three tests
bring `cmr-core` to 1,395. Availability globals and table values remain inputs.
F07 is complete: its integration suite now also proves championship-record bytes
survive repeated settings round trips. Nine integration tests cover persistence
and failure recovery. Original-written differential fixtures are explicitly
unsupported without a supplied original save; the static policy forbids running
the shipped binary to create one.
F05 is complete after correcting its result-slot interpretation: eight position
bytes precede the entry/order flag and tiebreak, with six trailing bytes kept
opaque. `leading_result_slot` now matches the native order-zero scan instead of
claiming a completed-stage counter. One layout test brings `cmr-core` to 1,396.
External `.TIM` content remains data; the Rally-School selector persists in
`cmRally.cfg`, outside the championship save-slot payload.
F01 now has its four-phase frontend transition spine in `frontend_menu`, with
explicit next-screen, quit and save-and-quit actions. Three tests bring
`cmr-core` to 1,399. Descriptor navigation and screen-specific callbacks remain.
F02 is complete: descriptor initialization, vertical navigation, boundary links
and type-3/6 value editing now complement the options/binding/calibration field
models. Three tests bring `cmr-core` to 1,402. F01 still owns screen-specific
callback and graph recovery.
F03 is complete: the load-apply callback restores the 256-byte results region,
six-bit progress and difficulty, and sets the loaded flag. The owned 12-slot
cache retains valid positions and releases together; invalid records cannot
mutate live state. Two tests bring `cmr-core` to 1,404.
E09 is complete: `release_car_data_slots` implements the 16-slot non-null
free-and-clear teardown, while wheel size/scale fields are confirmed as the
existing radius inputs. One test brings `cmr-core` to 1,405. Car variant tables
remain runtime data rather than unresolved code.
E10 is complete: `fog_settings` implements every weather/intensity/country fog
row and low-detail fallback, and `MAX_DUST_SIZE` is the native 2.0 Q16.16 clamp.
One focused test brings `cmr-core` to 1,406; 34 weather tests pass. Surface and
sprite tables remain caller-supplied data.
E11 is complete: the source-owned RGB/tolerance fixtures and imported-stage
benchmark provide representative comparisons, including the documented 0.1%
sampling-tie budget. All 9 image tests and both opt-in GPU conformance tests pass
on this checkpoint; no additional fixture layer is needed.
E07 is complete: transform, clipping, culling, Z-buffer occlusion and real stage
composition are implemented and self-verified. Only legacy differential capture
is unsupported under the static policy; no original binary is run to create it.
E06 is complete: primitive batching/order, Z-buffer behavior, transparency and
blending have a shared submission contract and the concrete E07 renderer. The
capability level, alpha gate and force-draw override are resolved.
E04 is complete: spatial section neighbors, ordered relocation and ancestor dirty
propagation complement the recovered world-transform hierarchy. Two tests bring
`cmr-core` to 1,408; runtime section tables remain inputs.
E03 is complete: truecolor 4/8-bit indexed rows preserve four-pixel alignment
and `ANIM` composites support caller-selected frames with checked targets.
Three video tests cover those paths. Direct 24-bit is explicitly unsupported in
the indexed sampler rather than quantized; animation clocks remain caller input.
E02 is complete: consumer evidence proves every vertex/face field role, on-disk
RGBA order, and that `field_22` is not a rasterizer input. Group-2 binding is
covered by E03; no new duplicate implementation is needed.
B03 is complete: all 16 reachable QMixer imports have recovered contracts and a
software/SDL playback path. The remaining 46 exports are explicitly unreferenced.
Unknown units stay verbatim instead of becoming invented conversion rules.
B02 is complete: `KillVideoAndStartGame` releases video interfaces, constructs
the configured `Rally.exe` path, calls `CreateProcessA`, and reports failure then
quits. Successful process execution remains outside the static evidence boundary.
B01's executable map now records 2,168 procedures, 32,530 symbols and 2,310
defined strings across six memory blocks. Indirect calls and uncarved code stay
explicit unknowns; map coverage is not misrepresented as total discovery.
B09 is complete: the native `WallClock` drives the recovered frame driver,
25 Hz catch-up, render skipping and windowed lifecycle. Busy-wait timing and
message pumping remain host policy rather than unmodeled game behavior.
B10 is complete: SDL keyboard and gamepad polling feed the recovered mapper and
drivetrain with analog dead-zones and hotplug handling. DirectInput is confined
to enumeration, and no legacy input backend is loaded.
B11 is complete: `physical_drawable_size` exposes real pixel dimensions and F11
toggles fullscreen in every SDL loop. An offscreen smoke test proves enter/exit
fullscreen alongside the shared presentation-scaling contract.
B12 is complete: both SDL loops pause on focus loss, resume and restore surfaces
on focus gain, and close through `HostLifecycle`. Fullscreen toggling uses the
same live windows, so no lifecycle path remains unwired.
C05 is complete: explicit roots drive exact-size settings persistence, record
storage and profile paths without registry or absolute-path dependencies. The
remaining opaque field semantics are tracked by C06.
C06 is complete: exact settings/profile formats and their record, progression and
load-apply semantics round-trip safely. Unassigned fields remain explicitly
opaque and preserved, not guessed or dropped.
F01 and F04 are complete: the generic callback router joins the four-phase
transition, root graph, options, profile and championship setup. One test brings
`cmr-core` to 1,409. Menu families share the recovered descriptor engine rather
than invented mode-specific algorithms.
C13 is complete: the real headless session starts and finishes with the recovered
FSM, and the native window lifecycle is wired. Optional official wheel fixtures
need separate derived coefficient files, which does not block startup.

Exit gate:

- a complete offline race can be started, driven, finished, saved, and replayed
  using user-imported data;
- compatibility checks cover deterministic state traces, file round trips, and
  representative rendered/audio outputs with documented tolerances;
- all required obligations for that path are closed or explicitly unsupported.

## Milestone 4 — modern platform backends

Replace legacy boundaries one at a time while retaining the compatibility lane:

| Legacy boundary | Candidate modern backend |
| --- | --- |
| DirectDraw / GDI | First native slice: software oracle presented by SDL_Renderer; provisional native 3D: wgpu, benchmarked against SDL_GPU before lock-in |
| DirectInput / WinMM joystick | Provisional: SDL3 keyboard/joystick; optional mapped gamepad after compatibility |
| QMixer / WinMM audio | Provisional: SDL3 audio with a reconstruction-owned mixer |
| Win32 timing/window loop | Provisional: SDL3 events plus a monotonic host clock |
| DirectPlay | explicitly designed transport after protocol recovery |
| Registry/install paths | portable configuration and data discovery |

The rationale, alternatives, compatibility constraints, and benchmark gates are
tracked in [`MODERN_BACKEND_RESEARCH.md`](MODERN_BACKEND_RESEARCH.md). SDL3 is
the selected host-layer direction; SDL_Renderer is only the first compatibility
presenter, not a mandate that native 3D use SDL_GPU.

Phase 4A is complete: `cmr-video-wgpu` and the Vulkan-only
`cmr-video-sdl-gpu` pass the same exact indexed-palette readback contract on the
locally available discrete NVIDIA and integrated AMD adapters. This establishes
the presentation primitive but does not select a winner. Phase 4B must feed
both candidates the same raw recovered C3D draw packets and benchmark a
caller-selected imported stage before I08 can complete or I09 can begin.

The Phase 4B correctness slice is also complete for the bounded opaque-COPY
stage profile: both candidates consume the same raw recovered C3D packets and
match exact readback on source-owned geometry, with shared tests for culling,
depth ordering, and palette mutation. The persistent prepared-scene
architecture now exists on both candidates: `PersistentRawC3dGpuBackend`
moves all allocation, palette mapping, and upload into a fence-completed
one-time preparation, and the hardware-proven timed path only encodes,
submits, and waits — no resource creation, upload, download, or mapping.

I08's decision lane is the correctness-gated `cmr-gpu-stage-benchmark`
(`cmr-gpu-conformance::stage_benchmark`). It reads one caller-selected imported
stage in place, composes the software stage frame as the oracle, prepares the
persistent scene once per candidate, and ranks only the steady-state
encode/submit/fence-wait interval. The gate is cross-backend byte equality plus
a documented 0.1%-of-frame nearest-neighbour sampling-tie budget against the
software oracle; a fixed-point hardware rasterizer cannot reproduce a float
software rasterizer at UV texel boundaries, and the installed-corpus worst case
is 189 of 307,200 pixels (0.062%) with the candidates identical to each other.
47 of the 60 installed EU stage scenes gate on the RTX 5090 and AMD 610M/RADV
paths; 12 see nothing at the placeholder origin camera and GR4 still hits the
unresolved 32x64 padded-layout surface case. The placeholder camera/palette and the origin
visibility limit are recorded in the evidence ledger. Captured steady-state
timings (median of three 240-frame runs after 32 warmup frames,
encode+submit / submit-to-completion in microseconds) show wgpu ahead on the
draw-heavy recovered stages: MC5 1235/98 vs 3110/91, NZ2 1296/88 vs 2253/72,
AU1 645/78 vs 1210/59 on the RTX 5090 (same ordering on RADV), while SDL_GPU
leads the 97-draw stage (66/75 vs 108/47). The CPU encode/submit interval,
which grows with prepared draw count, dominates. I09 therefore selects
**wgpu**, which also avoids SDL_GPU's external `glslangValidator` runtime
dependency, and wires it behind the shared contracts. The application's stage
and windowed-session commands now render through the persistent wgpu path by
default (`--renderer auto`), with the software compositor as the oracle and
automatic fallback. Layered main/OBJ/SHT/car foreground plus the separate HZN
projection/depth pass are implemented behind the same indexed-frame contract.
The native session now reports the requested and actual renderer and a typed,
path-free setup/frame fallback category, so an automatic software fallback is
no longer silent. The raw candidate benchmark (not the full layered native loop
or SDL presentation) measured per 640x480 frame 79.6 ms
(software) vs 0.75 ms (GPU) on AU1, 67.9 vs 1.07 on NZ2, 101.4 vs 1.11 on
MC5, and 7.2 vs 0.16 on IN8. A zero-copy window presentation that skips the
index readback remains a host optimization.

The native session now boots directly into the stage instead of automatically
leaving the menu after a short display. Four-wheel stepping also adds live
forward force along the car heading before lane-force summation, with a test
proving real displacement. This is a documented playable policy, not recovered
original force production.

Input selection now falls back to the keyboard when a connected gamepad is
idle. Only non-neutral pad axes/buttons select analog driving, so a gamepad
cannot leave the car static by overriding keyboard throttle.

The full windowed-session callback now also receives the existing
edge-triggered `FrontendInputMapper` output from keyboard and gamepad events.
The native loop now consumes those pulses through one `RootInteraction`, renders
the live recovered selection, and permits only the Rally entry to emit the
state-2 leave pulse. The front-end gate is cleared before state 4, so the
explicitly configured stage and car arm state 5 instead of auto-advancing at
boot. Other unreconstructed destinations remain pending.

The common C3D vertex-preparation UV rule is now shared by the software, wgpu,
and SDL_GPU paths before sampler clamp: raw Q16 U is converted to `f32` and
subtracts one exactly once only when strictly greater than one; V adds one in
the native x87 expression before its single final `f32` store. Parser/raw
carriers remain raw, while already-prepared frontend, generated reflection, and
2D UVs are not transformed again. AU1 corpus evidence shows the former missing
V bias collapsed the main scene to texture row zero; the latest controlled
capture confirms coherent stage textures after the correction. This is not a
claim of DX5 raster-exact pixels.

The native game session is a drivable stage harness. Authored `.cat`
checkpoints replace the synthetic line, a real country palette is loaded, and
the `.csp` player-one pose drives the car and first chase camera. The native
car adapter now treats local `+X` as front and local `-Z` as right, maps those
axes into the planar harness while flipping two transverse columns to preserve a
positive determinant, and adapts steering sign only at the explicit
`CarControls -> RaceState::Controls` harness boundary so requested world-side
turns agree for keyboard and analog input. The full suite passes, and a
controlled native wgpu run moved the car geometrically right and advanced to
43 MPH without renderer fallback. A controlled transform A/B confirms the
retained adapter shows the rear; the inverse shows the front. CAT-derived startup is retained
only for stages without CSP. AU1's
layered software composition and CSP-to-checkpoint drive regression pass on the
imported assets. The first layered-GPU prerequisite is complete: wgpu material
textures carry the resolved active-palette slot and a distinct source-key mask,
and keyed fragments discard before color or depth writes. The software oracle
and GPU now agree that later submissions win `LessEqual` depth ties. The
existing main-stage adapter remains explicitly opaque. Combined
stage/OBJ/SHT/car preparation is now implemented as one persistent foreground
upload with exact layer-local namespaces and per-frame car node worlds. A
synthetic Vulkan test matches the software keyed-layer oracle at two camera
poses. The separate horizon pass is now implemented too: HZN clears indexed
color/depth under its far projection, foreground loads color with a fresh depth
buffer, and one readback reaches the host. Authenticated AU1 including the car
passes the established 0.1% hardware/software raster boundary. Full layered
GPU rendering is therefore wired; zero-copy presentation and Windows execution
remain later portability/optimization verification.

The native playable baseline now uses the existing planar dynamics and bounded
harness control strengths instead of feeding live play through incomplete
vertical-force integration. The SDL presenter uses the shared explicit viewport
calculation at the renderer's actual output extent. Imported AU1 integration
checks prove input-driven world/frame changes, steering and checkpoint progress;
controlled Xvfb captures verify visible throttle, steering and coasting. The
caller-selected in-race C3D car is now depth-composed with the stage under its
live pose. The normal single-player race HUD is composed over every live world
frame from the authenticated bitmap/font inputs. Controlled Xvfb release
captures show a visible gauge, timer, gear/speed readout, player car, and
changed world frames after real throttle and steering events. The post-UV
controlled wgpu capture confirms coherent road, sky, scenery, and car textures,
geometric right steering, progression, and no renderer fallback; the latest
user capture withdraws the earlier rear-facing and stable-pacing interpretations. Static `FUN_00480890` evidence passes the
CSP position into the compiled initializer, whose recovered HPC lane-zero step
then applies `center_y += road_y - lower_corner_y`. The native session now
authenticates the car selector, compiled case and stage HPC, owns fresh-process
zero-filled lane state, and publishes only that setup-time Y before the first
camera/render/session frame. Owned AU1 raises the center by `40857` Q16
(~0.6234), and the controlled wgpu capture shows the complete body above the
road without a cosmetic lift. This does not claim complete four-wheel
trajectory fidelity or a verified full-race finish. The subsequent host pacing
fix enforces one simulation tick per 40 ms deadline; world collision/contact
and car/wheel animation remain absent.

The menu-to-race path now presents the selected stage's loading artwork during
state 4. The app reads `.LOA` directly from the caller-selected media root,
handles both raw and gzip/CMPR Windows storage, validates the recovered 640x480
indexed-BMP contract, and keeps missing art non-fatal. It authenticates the
shared `BAR.BMP` and `LINE.BMP` sources through the explicit runtime-data
`AssetStore` rather than inferring an installation root from the stage-local
media path, then composes the progress fill followed by the static 1px rule over
the `.LOA` background. The normal-stage cap is the observed `0x1a` argument at
callsite `0x421fd1`. Because the native harness completes its synchronous setup
before the SDL loop, it presents that completed value for the sole state-4 frame
and advances to state 5 without a simulation tick; it does not synthesize a
progress animation. Exact clipping, order, key-zero behavior, typed failures,
and owned-media BAR/LINE metadata are test-backed. Center-nearest stretching and
active-palette L1 remapping are documented software policy: the ordinary
RGB555/565 packing formula is recovered elsewhere, but the DX5 stretched raster
and an RGB16-to-active-indexed presentation bridge are not. PS1's same-role TIM
format remains a separate reference-only parser boundary. The controlled native
run now reaches a coherent live stage after this frame. The GPU/auto visual gate
has passed for GPU selection and geometric steering, and the authenticated
setup-time spawn-height gate now passes too. The next I14 gate is fixed-step
live authenticated collision/contact response; car and wheel animation remain
explicit follow-ups. The controlled transform A/B confirms the retained chase
view shows the rear of the car.

Static recovery has now placed scenery collision before per-car mechanics at
`FUN_0043d5d0 -> FUN_0043e640 -> FUN_0043d760`, identified `+0x3f0` as the
mutable local contact/lever arm, and established current-pose overlap with prior-position-informed
depenetration rather than swept CCD. Builder recovery identifies OBJ-only OBB
records, exact point/projection helpers, main-C3D grid provenance, native list
prepend order and literal-zero static history identity. A tested provider
boundary owns those 4,096 cell lists, raw materials, sequential accumulator
state and five-ID history. Exact OBB/grid construction, both reciprocal-corner
narrow paths and the response arithmetic are now test-backed. The pure parsed-
scene bridge applies the recovered OBJ admission, shallow traversal, first
group-1 bound, immediate-parent translation, root-local basis/scale and native
cell-prepend rules; an AU1 read-in-place diagnostic accounts for 2,169 records
across 156 occupied cells of the derived 29x26 grid. The pure live-car producer
now captures pre-integration X/Z, rebuilds/deforms the eight-point hull, creates
the horizontal projection cache and expands the strict four-cell neighborhood.
The concrete kernel distinguishes alternate/miss/normal paths, owns one tick-
rotated history and consumes ordered per-normal external results; the app now
retains the parsed scene at stage load. The primary-cell selector now scans the
retained authored XYZ centers with the recovered wrapping Manhattan/divide-by-
three metric and strict first-wins ties, rather than substituting the analytic
XZ grid mapper. Authenticated live response inputs and persistent session
publication remain the next gate. The authenticated compiled setup now
publishes exact collision halves, AK11 projection axis, corrected basis and
typed deformation scalars into/alongside the start state instead of discarding
them. The per-contact response-input producer now derives the final normal and
both response limits after arm finalization, and exact accumulated-correction
damage severity replaces an arbitrary supplied scalar. Canonical contact axes
are now derived from their literal normalized seed; damage branch entry is
retained separately so equality can enter with zero severity while below-
threshold values cannot produce a handle. The exact four-lane distance gate now
publishes setup flags and contact count `+0x674` from authenticated lane/hull
owners. Pending linear correction, angular-moment ownership, response history,
the two persistent recovery flags and deformation scalars now have typed live-car
owners with the recovered consume-clear-reaccumulate lifetime. Translation now
consumes and clears prior linear correction exactly once. Ordinary per-tick
republishing and the concrete damage-object implementation remain open, but a
lazy synchronous owner now replaces the pre-sized per-hit replay FIFO. Atomic session
publication remain open; live deformation/basis evolution is also unresolved.
Render mesh AABBs are not accepted as
compatibility geometry.

The parsed static scene is now part of the live planar state-5 path. After
integration, the session builds the exact authored-cell/hull frame, runs the
recovered kernel, and atomically publishes depenetration, alternate velocity,
next-tick linear/angular corrections, history and persistent response flags;
failures restore the pre-step race before FSM, checkpoint or tick publication.
Synthetic tests cover contact publication and rollback. Production deliberately
uses a no-damage owner until the mutable damage object is recovered. The long
authenticated AU1 input run reached its final steering assertion without a
collision failure and exposed the planar bridge's reversed sign (`left` was
`+40`, expected `-40`); the bridge now preserves recovered right-minus-left,
with keyboard/analog tests pinning the correction. Owned-media contact accuracy,
angular consume/clear, per-tick contact count, four-wheel basis evolution and
car/wheel animation remain the next I14 work.

The angular `+0x3d8` lifecycle is now pinned from the shipped integrator
`FUN_0040a4a0`. The integrator decays the prior moment in place
(`+0x3d8..+0x3e0 <- qmul(component, +0x48)`) and uses the decayed value; its
caller then clears it (see the correction below). The same reading recovered a missing moment-overflow
trigger: while not out of control, a wrapping `|x| + |z|` of the decayed moment
strictly above `0xbb80000` sets `+0x680/+0x688` and records `+0xbc..+0xc4` as the
projection axis `+0x324` in the pre-rotation basis frame, and that tick's angle
conversion already uses the out-of-control form without the half-delta term.
Instruction-level reading also corrected the momentum order to
`qmul(-qmul(combined, inertia), dt)`; the old negate-after form was off by one
LSB whenever the time-step product had low bits, which ONE-timestep tests could
not see. All three are reconstructed and published by the four-wheel step into
the existing `LiveCollisionState` owners. The live windowed session still runs
the planar path and so does not yet decay `+0x3d8`; the recovered angle
deadzones and resting flag `+0x700` from the same function, the four-wheel
transition, per-tick contact count, the `+0xbc` consumer, damage, and car/wheel
animation remain.

The contact-gated tail of the same function is now read and partly applied.
The low-speed position suppression was fed the wrong inputs: the native test
runs after `+0xec` is recomputed from the new velocity, so it reads the new
speed, not the previous tick's cache, and in the four-wheel step it is gated on
the threshold scan's `+0x742 > 0` rather than the wheel flags. Both are
corrected, and the translation outcome now returns the recomputed speed. The
three angle deadzones and the resting flag `+0x700` are reconstructed as pure
tested gates but not wired: the yaw deadzone needs the raw steer magnitudes
`+0x2fc/+0x300`, which the simulation step does not yet receive, and the
control word `+0x404`, whose writer is not the in-race mapper. The same mapper
pass also showed it owns the shipped camera-change input, a rising edge on
device bit `DAT_0055aeb0 >> 16` (default bit `0x40`) that calls the view cycle.

Correction to the two milestones above. The per-car mechanics driver
`FUN_00407690`, which calls `FUN_0040a4a0`, zeroes `+0x3cc..+0x3e0` after the
integrator and the two drivetrain passes, neither of which reads the moment. So
`+0x3d8` *is* cleared every tick; the earlier "never cleared" conclusion came
from reading the integrator without its caller. The per-tick lifecycle is
accumulate-from-zero (collision), consume-decayed (integrator), clear (driver).
This exposed two real defects, now fixed by an end-of-tick clear in every
mechanics-tick method: the four-wheel step wrote the decayed moment back, so the
next collision pass accumulated onto it, and the live planar session never
cleared the moment at all, so angular impulses compounded across ticks. The four-
wheel test that should have caught the first defect used a zero moment; it now
uses a nonzero one. The same driver also yields the writer of `+0x404`, an
upright-cone test on the projection axis in the body frame, now reconstructed,
and a low-speed steer cut (`|+0xec| < 0x147` zeroes `+0x2fc/+0x300`), observed
but not yet reconstructed.

The deadzones and `+0x700` are now wired into the four-wheel step. The steer cut
is confirmed at instruction level and reconstructed; `Controls` carries the raw
steer magnitudes from the input map; the step publishes `+0x404` from the
start-of-tick body axis and `+0x700` from the integrator's recomputed speed; and
the deadzones run inside the angular pipeline between angle conversion and the
control-axis resets, the order the integrator uses. The planar windowed path
still skips this pass, the cut reads the harness speed (recomputed after the
planar lateral grip), and the steering consumer `FUN_0041b5b0` is not
reconstructed, so the cut does not yet reach the derived steering rate.

The four-wheel step now translates through the integrator's own path:
previous-speed damping, inverse mass `+0x4c`, the pending collision correction,
and the recomputed `+0xec`, with no heading-rate steering, harness drag, throttle
push or lateral grip. That resolves the `speed_ec` divergence. A four-wheel car
now moves only through lane and non-wheel forces, so the next step is routing
throttle and brake `+0x304/+0x308` from `Controls` into the drivetrain, and
filling mass, inertia and `+0x4c` from the car's compiled prefix.

The pedal half of the control processor `FUN_0041b5b0` is now recovered and
wired. Raw throttle, brake and handbrake travel from the input map through
`Controls` into its ramps, which produce the throttle response `+0x2d8`, the
brake response `+0x2ec` and the handbrake torque `+0x2f4` each tick; the
four-wheel step hands them, and the raw pedal words, to their recovered
drivetrain consumers. The ramp scalars are still harness inputs, and the
processor's gearbox and steering parts are next.

The four-wheel car constants now come from the recovered initializer stages:
`derive_compiled_four_wheel_constants` yields mass and inverse mass, inertia,
the pedal ramp scalars and the `+0x408` mode from one car's case words, and the
app builder takes that set. Only the headless session, which has no case words,
still passes explicit harness values.

The steering half of the control processor is recovered and wired: digital and
analog routines turn the cut steer magnitudes into the wheel angle `+0x3e0`,
and the four-wheel step rotates the lane source by the per-car base plus that
angle, so steering now reaches the wheel forces. The digital rate and minimum
deflection (`+0x44c..+0x45c`) and the surface/car-type steering tables are not
yet sourced.

The gearbox completes the control processor: automatic, manual and
brake-to-reverse modes, the deferred shift request and the reversing mode are
reconstructed. An optional live gearbox in the four-wheel step now feeds gear,
shift request, drive ratio and reversing mode to the drivetrain and pedals each
tick, and its tables derive from the compiled initializer. The four-wheel car
can now accelerate through the gears, steer and brake under its own recovered
rules; moving the windowed session onto it is next.

A probe of that four-wheel step on the reviewed AU1 data (`obs.au1-four-wheel-probe`)
shows the control processor working but the car not moving at all: the engine
response table `DAT_0050e1ac` is zero, the drive torque split is not wired into
the wheel forces, and no gravity or ground-contact force reaches the lanes. The
windowed session stays planar until those exist; the per-car-type table block
(stride `0xcd0` at `DAT_0050e170..`) and the ground part of `FUN_0040d810` are
the next recovery targets.

That block is now recovered: it is the per-player car-setup block, reset by car
setup and derived from the setup sliders (and damage) at race start, and the
four-wheel step reads its engine response, steering factor, wheel constants,
base angle and gear-2 skip. The engine force now runs before the wheel passes
and is split into the four drive slots each tick, as the original does. On the
owned AU1 data the engine now revs against the car's real redline, but the
wheels still do not turn: the remaining first/second-wheel inputs and ground
contact are next.

The wheel-force producer's brake torques and its grounded/airborne and
per-lane contact gates are now recovered and wired. On the owned AU1 data the
car is grounded and upright, its driven wheels spin up under the engine and the
rev limiter cuts in, but no lane force reaches the body yet: the second loop's
tyre inputs (transition and alternate gains and bases, correction gate,
accumulation limit) are still harness zeros and are the next target.

The per-car ground pass `FUN_00410a60` is now reconstructed and run after each
four-wheel tick: probes are rebuilt and the stage geometry is queried every
tick, the car aligns to the ground normal, penetration lifts it, and
into-ground velocity is removed. The contact flags and threshold scan now come
from live ground state instead of the setup snapshot (handoff item 3).
Correction to the first version of this paragraph: the four-wheel car does not
yet drive. It stays on the surface and upright with live contacts, but every
lane force is still zero, so the motion seen on AU1 was gravity sliding it
downhill (the checkpoint cursor never advances). The second loop's tyre inputs
are the blocker, and the windowed session waits for them.

Those tyre inputs are now produced: `cmr-core::tyre_load` reconstructs the
driver's `DAT_004f777c` terms and `FUN_004095e0`'s per-wheel loads and gains,
the reviewed tyre store keeps each surface record's first dword, and the
four-wheel step feeds the gains, bases and slip limits to both wheel loops
(`impl.tyre-gains-and-live-force-head`). The same path derives the slip shift
and `DAT_0050c5c4` per tick and uses the recovered unit step for `DAT_0050c5c0`.
On AU1 every lane now produces force and the gearbox shifts under throttle, but
the summed lane force alternates sign each tick and the car stays below the
position gate, so it still does not advance.

That blocker is fixed (`impl.second-loop-inputs-and-contact-mirror`): the
second loop now scales the slip and the axis dot as the native does, its
small-residual normalization keeps the raw square sum, and the mirrored contact
vectors are whole opposite corners. On AU1 the four-wheel car now accelerates
along its body axis through the gears under throttle. Steering it along the
road (the checkpoint cursor) is the next gate.

The compiled spawn rows now face the road (`impl.spawn-rows-face-the-road`,
an inference: the native rolls along `+0x258`, and the planar adaptation had
row 0 reversed). Under throttle alone the four-wheel car now drives down the
AU1 road and passes 21 checkpoints before it leaves the ground and tumbles,
which needs the out-of-control path `FUN_004115d0`.

Steering and the drive split are now sourced (`impl.steering-and-drive-split`):
the steering words come from `FUN_0041b160`, the lane source turns about the up
row through the native axis-angle matrix, and every wheel reads the compiled
`+0x158` split, so all four wheels drive. The windowed session now runs the
four-wheel producer with the ground pass and draws the car from its basis.

The orientation update now uses the native row-vector transform
(`impl.orientation-row-vector-transform`); the transposed version had turned
every rotation backwards, which the spawn half-turn and the harness `+0x800`
heading were compensating. Rendered frames of the windowed loop on AU1 now show
the car following the road around a right bend under right steer.

The out-of-control path `FUN_004115d0` is reconstructed (`impl.tumbling-ground-pass`):
a car that loses control keeps touching the ground, damps its spin, counts roof
and side time toward the righting animation and the totalled latch, and recovers
when settled upright.

The integrator head is reconstructed (`impl.integrator-head`): the grip assist
`FUN_0040b240`, the rollover trigger `FUN_0040bfe0` and the native force and
moment sum over the touching probes. With them the AU1 bank-slide blow-up is gone.

The car reset is reconstructed (`impl.car-reset`): once a car is wrecked, sunk or
stuck (latch `+0x67c`), the screen fades out, the car is placed back on the
last eligible `.cat` node facing along the road, and the fade comes back in.
The suspension and wheel animation routines are presentation-only.

The HUD's rev, speed and gear now come from the car through the recovered
preparation `FUN_00464130` (`impl.race-hud-prepare`).

The `.dat` trigger walk `FUN_00403a00` runs every tick (`impl.trigger-walk-and-barriers`):
barrier walls push the car back and bounce it, kill volumes reset it, type-1
records move the checkpoint cursor and co-driver calls fire. The earlier labels
(split, finish, sound) for types 0, 1 and 2 were wrong.

The co-driver now speaks (`impl.codriver-speech`): the walk's calls feed the
recovered sequencer `FUN_00444450`, which plays the 40-wave bank one sample
per frame.

A finished stage now returns to the menu and a new stage starts from its setup
(`impl.gameplay-script-and-stage-restart`), instead of the app stopping.

The wheels now spin, steer, follow the suspension and vibrate on rough surfaces
(`impl.wheel-animation`).

The car now sounds: the recovered engine tick crossfades the per-car rev-band
loops, adds the whine, overrun pops, blow-off, anti-lag and gear clunk, and
balances front/rear emitters from the camera (`impl.engine-sound`). The band
tables come from the user's executable through a reviewed extraction.

Escape now pauses the race. The recovered pause menu resumes (Escape or the
default item), restarts or quits, and its sound submenu sets the effects and
speech volumes (`impl.pause-menu`). The menu draws its title, items and the
darkened panel with the game's own fonts and strings (`impl.pause-menu-draw`).

The car now rolls and skids audibly on each surface, thumps on landings, and
barrier hits play their impact sounds (`impl.surface-and-prang-sound`).

The front end now runs outside the game state machine, as `FUN_004c1e00`
does: confirming a race starts a race session, and the state machine seeds
from state 1, which chooses the event (championship, single rally or
selected stages). After each stage the hub runs the recovered event advance;
when the event is over the session ends and the front end comes back
(`impl.event-flow`). The race hand-off commits the cmRally.cfg record, and the
stage load follows the event, so the root's Championship, Rally and Time
Trial entries run their events stage after stage. The setup, name-entry and
stage-select screens now run as the native menu flow and write the record
(`impl.frontend-setup-flow`); all of them are drawn, without the preview
panels, 3D cars, stage thumbnails and flag animations yet; the stage load
attaches the chosen car.

The build and verification infrastructure is complete. Linux reviewed-import
and offscreen SDL checks pass; the portable Windows target checks cleanly; CI
covers portable/cross-target/full-workspace jobs. Native Windows and Actions
provisioning remain explicitly unrun. Comparison/deviation matrices and the
README describe the current 117 complete / 2 partial roadmap state.

A10 is bounded to the reviewed official-EU/installed-EU profile. Other editions
and language packs are explicitly unsupported; official-US and xdelta artifacts
remain comparison variants, not proof of compatibility.

E16 closes the championship boundary; E08 remains partial. Stage loading now
consumes `.c3d`, `.dat`, `.obj`, `.sht`, `.ars`, `.hzn`, `.tsc`, `.csp`, and
`.cat`, with layered software presentation, authored starts, bounded
trigger-volume decoding, and authored trackside-camera records. `.moj`, `.spl`,
`.anm`, `.rsf`, and `.fuz` remain unintegrated. The native race path composes the
player car and normal race HUD and enters the configured race through the
recovered Rally root selection. The country, stages and car now come from the
setup screens (the `--car-code` car only drives the window's first stage),
and DAT dispatch remains open.
The normal single-player HUD now has a pure, test-backed command plan for the
pinned common gauge core including all 15 strict-threshold rev segments,
speed/gear text, progress marker, and late timer
overlay. Native setup authenticates, decodes, bounds-parses, and retains all six
source-owned HUD bitmap/font inputs. The indexed software compositor preserves
command order, color-keying, diffuse/tinted blending, nearest stretching, and
the recovered direct-index font behavior, then quantizes blended RGB through a
deterministic full-palette oracle. The live adapter uses recovered speed/timer
arithmetic; first gear, zero revs, and CAT-cursor progress remain explicit
harness inputs until their runtime producers are connected.

DAT trigger geometry is now reconstructed over the native current/prior
eight-point world-space car bounds, including directional gating, thickness,
open-Y span, and evaluate-before-snapshot ordering. The exact type-to-spatial
gate switch, outer player-bit check (including type-1 ordinal-byte behavior),
whole-table evaluation, and single post-table snapshot are now test-backed
(the native walk is per cell; see `rea.official-eu.trigger-walk`).
Live mode predicates/side effects, the car-bound producer attachment, and
first-frame prior-array initialization remain open.

E01 is closed at its deterministic indexed boundary. Supported BM, palette,
row, composition, and queued-font paths are implemented; native padding,
24-bit, driver-exact filtering, device-owned formats, and alternate-font final
pixels are explicitly unsupported rather than approximated.

D14/D15 now close the supported type-1 boundary: geometry/resources and the
flat/COPY renderer are implemented. General lighting and alternate
Gouraud/modulate output are explicitly unsupported and are not replaced by the
type-2 model.

D16/D18 now close final presentation and native root interaction: owned indexed
frames, checked RGBA expansion, render-route selection, descriptor menus, and
focus/pause/restore/quit lifecycle all share the SDL path. Native flip timing
and the live display server remain backend boundaries.

H09's reachable DirectPlay scope now includes per-player record messages and
state transitions, not just the session lifecycle. Positional words are
preserved and accumulation wraps natively. Application/provider GUIDs stay
runtime-negotiated; H10 remains a one-peer backend, not full synchronization.

H08 now includes the final DirectInput update wrappers: sample-period and
periodic-gain `SetParameters` layouts, including exact flags and the observed
two-byte type payload. Runtime surface/car tables remain external data.

H05–H07 are closed by the existing PCM music and portable video contracts.
The native frontend drives looping/fading PCM through `AudioHost`; video has a
tested open/play/pause/skip/close interface. No CD music or command-line skip is
added. Replay trigger, focus suppression, clip suffix/order, and a concrete
video decoder remain explicit unsupported boundaries.

H03 now includes the exact collision impact bucket and both volume formulas,
plus the strict listener-curve threshold. Vendor post-scale units and 3D
coordinates remain explicit unsupported boundaries.

H01 is closed by B03's complete 16-import ABI scope. H02 now records both
`QSWaveMixOpenWave` forms exactly (`8` named PCM, `0x10` managed); unsupported
vendor priority/lifetime and memory-versus-file meanings remain explicit rather
than inferred from those constants.

G11's camera controller is complete through the unified `ViewController`: all
recovered placements, cycling, trackside/reset behavior, and dual-slot crossfade
are wired into the native session. Runtime camera data and the uncovered helmet
overlay/remap/director pieces remain explicit unsupported boundaries.

G05 is closed at the evidenced timing/results boundary. Checkpoint deltas now
use the recovered normalizing pre-transform, best times use the checked settings
record backend, and result rows have the recovered 18x20-byte/TTL lifecycle.
The spawn-position writer is explicitly unsupported by current static evidence;
no camera/object routine is relabeled to fill that gap.

G04 now includes the signed out-of-bounds countdown and the crash reset's two
orientation modes, not just impact/damage arithmetic, plus the dynamic
trackside moving objects (`.obj` admission `FUN_00482140`, cell bucketing
`FUN_004622b0`, car-object OBB collision and two-body impulse/torque response
`FUN_0043f790`/`FUN_0043faf0`/`FUN_00440d90`, per-tick gravity/ground/damping
and cone settle `FUN_00450d40`, rally-school `DAT_00552f68`/`DAT_00558fb0`
contact flags, and software+GPU dynamic `.obj` node-local rendering), the night
headlight, brake/tail/pod lamp, electrics flicker, pod breakage `FUN_0044a770`,
and night road light pool `FUN_004a8750`/`FUN_004a9490` pipeline
(`cmr-core::car_headlights`), and the car-vs-car narrow phase, 50/50
depenetration, inelastic impulse/torque, and crash-damage response
(`FUN_00442740`/`FUN_004438a0`/`FUN_00442860`/`FUN_00441f30` in
`cmr-core::car_vs_car_collision`). All 14 stage sibling sections (`.c3d`,
`.dat`, `.obj`, `.moj`, `.sht`, `.ars`, `.hzn`, `.tsc`, `.csp`, `.cat`, `.spl`,
`.anm`, `.rsf`, and `.fuz`) are integrated in `cmr-formats::stage::StageBundle`
(closing E08), and `.moj` kinematic object motion (`FUN_0045bde0`..`FUN_0045c9c0`)
is reconstructed in `cmr-core::stage_moj_motion`. Replay snapshot keyframes
(`FUN_0046c7d0`/`FUN_0046c910`/`FUN_0046ca70`), Region-A initial-state seed
views and partial-preserve restores (`FUN_0046d370`/`FUN_0046d460`), RLE
input-trace playback (`FUN_0046dc20`), `.SSR`/`.RSP` file parsing and pointer
relocation (`FUN_0046ddc0`/`FUN_0046de30`), `CarState`/`RaceState` Region-A
seeding, and Rally School `intro.rsp`/`exit.rsp` playback (`FUN_0046de70`,
`FUN_00437ef0`, `FUN_004384a0`, `FUN_0046edd0`) are reconstructed in
`cmr-core::replay`, `cmr-core::race_state`, `cmr-core::gameplay_session`, and
`cmr-app`. Bound `.moj` kinematic scene-object slots (`FUN_0045bde0`..`FUN_0045c9c0`)
are wired end to end through `StageRenderResources::build_moj_slots` and
`GameplaySession`, and `FUN_0046f920` briefing `.avi`/`.wav` path formatting
(`"%d"` + `".avi"`, `[80, 32, 320, 208]`) is pinned in `cmr-core::intro_video`,
completing `I14` (`119 / 119` items complete). Headless `gamescope` differential
verification against the original game further pinned and wired C3D Group-1
`flag20` cylindrical Y-billboarding (`0x04`, `FUN_004a18e0`) and two-sided
scenery (`0x01`, `FUN_004a2460`) across software and GPU renderers, the 12
decorative HUD border/progress-rail bitmaps (`FUN_0045dac0`, `FUN_0045f530`),
HUD font palette quantization/tinting against the stage palette, uppercase
default hostname player names, case-insensitive stage sibling lookup, and
clean State-4 stage-loading interstitial presentation. Interactive gameplay
verification further reconstructed `FUN_00435d10` gameplay viewport projection
(`c3d_camera::gameplay_projection_default`, fixing vertical car squishing),
`FUN_00419e50`/`FUN_0041a3e0` chase-camera basis easing and Gram-Schmidt
orthonormalization before local-offset rotation (`GameplayCamera::update`,
keeping the car centered while turning), planar shadow camera-pull scaling
(`plan_shadow`, keeping the shadow beneath the car), full gamepad navigation
across all sub-menus/pause/results plus D-pad driving fallback
(`cmr-platform-sdl3::input`), zero-clone no-zwrite software shadow/sprite
blending (`rasterize_c3d_fragments_untextured_no_zwrite`), and native 40 Hz
(`25 ms`) front-end menu pacing (`FRONTEND_SESSION_TICK_INTERVAL`).
Stage start-sequence and four-wheel physics calibration further reconstructed
the Mode-8 start-line camera orbit (`FUN_00467630`..`FUN_00467c20`,
`gameplay_camera::start_orbit_camera`), the 5-4-3-2-1-GO! HUD countdown overlay
(`FUN_00423570`/`FUN_00423700`, `cntdown2.bmp` and `go!.bmp`) and co-driver
countdown voice cues (`FUN_0046af10`), the 3-phase pre-race state machine
(`FUN_00439b50` + `FUN_00407590`, revving in neutral with brakes locked before
launching in 1st gear at `GO!`), static+dynamic quarter-car wheel normal loads
(`FUN_00417cf0`, `wheel_animation::compute_wheel_loads_417cf0`), compiled
control constants (`control_word_144 = 0x83`, `control_word_160`,
`control_word_170 = 0x8000` in `FUN_004079f0`), the `FUN_0040a4a0` `probe < 4`
wheel-vs-hull force/moment split in `integrator_head::contact_force_and_moment`,
and the column-major `R^T` angular-velocity contact basis projection in
`FUN_00409f90` (`vehicle_dynamics::build_contact_basis_vectors`), restoring
authentic rally acceleration and slide recovery.
Stage checkpoint/split/classification HUD and 1:1 physics fidelity further
reconstructed `FUN_004620f0`/`FUN_00462230`/`FUN_00462b70` `.cat` checkpoint
gate progression (`StageProgress` ignoring the `CAT[0]` start-line split bit,
testing forward crossing before regression, and finishing only at `cursor >=
nodes.len()`, eliminating premature stage finishes at Split 7), `FUN_00464930`
top-left 4-row driver classification bars/ranks/3-letter codes and 14 exact
64x16 car/make thumbnails (`white.bmp`, `placenum.fon`, `result.fon`, and the
`osdnew/High/common` car bitmaps), `FUN_00463080` right progress-rail sector bars
and split tick marks, `FUN_0047fe50` bottom-left latched split time and
top-center split delta (`plan_race_checkpoint_hud`), `FUN_004088c0` forward-axis
`driven_smoothing.scalars.longitudinal` (`dot(velocity, basis[0])` instead of
`basis[2]`), and `FUN_00404440`/`FUN_004b98b0` live player car setup options
(`PlayerCarSetupOptions`, defaults `[0, 1, 1, 1, 2]`, passing `common_flags_025`
to `derive_compiled_four_wheel_constants` and `derive_compiled_gearbox_tables`
and `compiled_center_split_q16(brake_balance_095)` to `control_word_170` so the
Player car setup options (`PlayerCarSetupOptions`, `live[0x91, 0x93, 0x95, 0x97, 0x99]`, default `[0, 1, 1, 1, 2]`, plus `common_flags_025` at `live[0x25]`) are now wired into `SessionCompiledPose` and `SessionAuxControl` (with `common_flags_025` driving `derive_compiled_four_wheel_constants` and `derive_compiled_gearbox_tables` and `compiled_center_split_q16(brake_balance_095)` driving `control_word_170` so the engine rev limiter, automatic gearbox upshift thresholds, and normal `0x9999` gravity stay aligned).
Post-race and GPU visual fidelity are now aligned with the original D3D path: `RaceAudio::mute` silences looping engine/surface/wet/ambience/speech channels when leaving State 5 driving (Stage Results, Pause, Restart, and Quit) and `SpeechBank::frame` gates co-driver word sequencing on `speech_active` (`advanced.stepped && state_id == 5`), clears the ring buffer on `speech_reset` (`!matches!(state_id, 5 | 0xa)`), and scales `PACENOTE_CHANNEL_VOLUME` by `speech_master_4f6538 / 100` (`FUN_0047eba0`); `FrontendSession::resume_to_root` and `InteractiveRootMenuSession::resume_to_root` restore the interactive root menu (`active_menu = 0`, `menu_band_open = true`) after quitting or finishing a race; `StageSceneFrame::blend_shadow_triangles` and `blend_tyre_mark_strips` depth-test against the stage depth buffer (`zwrite = false`, `D3DCMP_LESSEQUAL`) with GPU car-layer depth masking (`PreparedRawStageLayer::with_car_layer`, `StageSceneFrame::mask_car_depths_from_rgba8_alpha` / `restore_masked_depths`, and the full `1.0 m` `FUN_0047bee0` `camera_pull`) so neither the car's own depth buffer nor dust/smoke sprites punch holes in the planar shadow; `cmr-gpu-stage` + `cmr-video-wgpu` render direct `Rgba8Unorm` with per-vertex Gouraud diffuse lighting (`C3dEntry24.corner_tuples()` on main-track `.c3d` meshes, `FUN_004a2b70` `mode == 0`) and color-keyed/tinted HUD and pause overlays; `wheel_animation::step_body_roll_416760` (`FUN_00416760`, including the `0x4171ee..0x417ca7` visual body roll/heave/pitch integrator `visual_roll_state_130` and `visual_heave_y`), `wheel_animation::step_visual_chassis_415df0` (`FUN_00415df0` lagged visual chassis follower `visual_chassis_basis`, ground-clearance corner lift `visual_chassis_lift_y`, and C3D **Node 5 `[car + 0x04]`** body posing in `WheelRig`), `wheel_animation::interpolate_matrix_435400` (`FUN_00435400` sub-tick pose interpolation across `prev_world_2a0 -> world_264`, Node 5 body, Nodes 1..4 wheels, chase camera, shadow, lamps, and `.moj` objects at `75.0 Hz` via `run_windowed_game_session_with_subticks`), live `FUN_0040fbe0` (`apply_geometric_lane_postprocess`), `drivetrain::select_second_wheel_candidate` (`0x40f5e0..0x40f5e8` keeping proportional `normalization.length` below the static friction threshold `alternate` and dropping to kinetic sliding friction `transition` only above `alternate`), and `FUN_00407690` (`contact_updates` zeroing, `flags_73a[i] == 0` gating on `FUN_0040d400`, start-of-tick `advance_residual_enable`, `upshift_hint_420` seeding, and `Loop 7` orientation integration before `Loop 8` wheel smoothing) complete dynamic braking/cornering weight transfer, proportional static grip, tangent-plane wheel force projection, and high-frequency car body bump-shake animation; and `cmr-platform-sdl3` uses hybrid coarse sleep + `< 2 ms` sub-millisecond poll pacing for exact `40.000 ms` (`25.000 Hz`) simulation ticks with `75.0 Hz` (`3 × 25 Hz`) sub-tick interpolated presentation.
The reset trigger remains an explicit input, and no unsupported position-snap,
time penalty, or damage-performance mapping is claimed.

G03 is complete at the safe portable-contract boundary. The authenticated HPC
provider runs the complete compiled-initializer spine, and the four-wheel
producer includes contact/force/postprocess, translation, angular and smoothing
stages. Cached-speed damping, the semi-implicit position term, and direction
selection are implemented. Runtime car/surface values and native pointer-fault
behavior remain explicit inputs/limitations rather than unrecovered formulas.

G02's driven smoothing now consumes the actual per-wheel force produced by the
first wheel loop. `DrivenWheelUpdateParams` separates recovered formulas from
runtime suspension/longitudinal/response data; the shared scalars also feed the
smoothed engine-speed cap. Neutral and ordinary branches remain covered, and
`cmr-core` has 1,410 tests.

G01's fixed-step world update is wired end to end through the shared
`GameplaySession`: the recovered FSM and race-flow tables, state-5 simulation,
checkpoint finish tracking, and render routing drive both headless and native
SDL sessions. The synthetic checkpoints, drive response, and scripted cinematic
flags remain explicit harness inputs owned by later gameplay items.

The rectangular C3D surface boundary is now explicit: DirectDraw owns the
initial contents of padding introduced by square expansion, and the recovered
copy loops never initialize it. The deterministic indexed sampler therefore
continues to reject GR4's 32x64-to-64x64 case rather than invent a fill. This is
an evidenced unsupported native-driver case, not a remaining formula to guess;
driver-exact Stretch and alternate-font final pixels remain bounded E01 work.

An original-compatible audio lane can begin with a narrow shim for the 16
`QSWaveMix*` APIs the game imports. The shipped DLL's complete 62-export surface
is an ABI oracle, but unreferenced exports are not automatically reconstruction
requirements.

Exit gate:

- modern backends pass the same contract suite;
- no original DLL is required for the supported portable build;
- deviations from legacy behavior are intentional and documented.

## Verification budget

Use the least duplicated evidence that can establish each claim:

| Claim | Primary evidence owner | Minimum proof |
| --- | --- | --- |
| artifact identity | provenance tooling | size + SHA-256 + parsed PE metadata |
| file format | format module | bounds tests + synthetic fixtures + local corpus scan |
| state transition | game subsystem | deterministic trace comparison |
| persistence | storage subsystem | byte/field round trip and negative cases |
| rendering | renderer | scene/state trace, then image comparison with tolerance |
| audio | mixer | event trace, then deterministic sample/hash window where possible |
| backend parity | platform interface | shared contract suite on both backends |
| playable path | integration owner | controlled end-to-end run with imported data |

Static analysis cannot prove runtime behavior. Runtime comparisons will be
introduced only when their environment, side effects, reset procedure, and
capture authority are explicit.

