//! Stage/track bundle and cross-file reference formats (item E08).
//!
//! # Provenance
//!
//! A rally stage is **not a single container file and not a CMPR bundle**. It is
//! a set of sibling files that share one base path and differ only by extension
//! (the master loader `FUN_004228d0` walks a 14-entry extension table at
//! `0x4eb92c` and loads `<stagebase><ext>` for each through the shared,
//! CMPR-capable loader `FUN_00495ed0`). There is no stage-level magic, header, or
//! offset directory: the "directory" is the filesystem directory plus the
//! extension convention, and each *section file* may independently be CMPR
//! wrapped. This module therefore decodes the individual section payloads whose
//! on-disk layout was confidently recovered, and records the bundle structure as
//! metadata.
//!
//! Recovered, decodable payloads:
//! - The **`.cat` node spline** (`FUN_00466350` -> `DAT_0057ae6c`): a header-less
//!   flat array of `0x1C`-byte records; the element count is `filesize / 0x1C`.
//!   This is the timing/split line (not an AI racing line).
//! - The **`{e,i,n}.cjp` split-time reference**: a block of 13 dwords selected by
//!   difficulty (novice/intermediate/easy).
//! - The **trigger-volume record** (`0x30` bytes; walked by `FUN_00403a00`,
//!   linked by the fix-up `FUN_004904e0`): its containing section file was not
//!   confirmed, so only the record decode is reconstructed here.
//!
//! The semantic classification of a trigger's `raw_type` byte lives in
//! `cmr-core::collision` (`TriggerType`/`surface_switch_index`); this crate
//! stays a pure byte decoder and preserves the raw value. Unknown bytes are
//! preserved rather than dropped. Only format metadata is encoded - no game
//! payload bytes are vendored; fixtures are tiny synthetic records.
//!
//! See evidence `rea.official-eu.stage-bundle`.

use std::fmt;

use crate::c3d::{C3dError, C3dFile, C3dLimits, parse_c3d};
use crate::school_rsf::{RsfError, SchoolReference, parse_rsf};

/// Stride of one `.cat` node record (7 dwords).
pub const CAT_NODE_STRIDE: usize = 0x1c;

/// Expected number of split nodes in a well-formed stage ("should be 7, not
/// including start/finish" - a developer diagnostic, enforced only as a check).
pub const EXPECTED_SPLIT_NODES: usize = 7;

/// Number of dwords the loader copies from a `.cjp` split-time file.
pub const CJP_DWORD_COUNT: usize = 13;

/// Minimum `.cjp` length (the 13 dwords the loader reads).
pub const CJP_RECORD_LEN: usize = CJP_DWORD_COUNT * 4;

/// Stride of one trigger-volume record.
pub const TRIGGER_VOLUME_STRIDE: usize = 0x30;

/// The 14 stage section extensions, in the loader's order (`0x4eb92c` table).
pub const STAGE_EXTENSIONS: [&str; 14] = [
    ".c3d", ".dat", ".obj", ".moj", ".sht", ".ars", ".hzn", ".tsc", ".csp", ".cat", ".spl", ".anm",
    ".rsf", ".fuz",
];

/// Extension of the node-spline section.
pub const CAT_EXTENSION: &str = ".cat";
/// Extension of the 3D world-mesh section.
pub const C3D_EXTENSION: &str = ".c3d";
/// Extension of the stage trigger/collision-volume stream.
pub const DAT_EXTENSION: &str = ".dat";
/// Extension of the horizon/background scene. The shipped loader sends this
/// payload through the same C3D parser as the world scene (`FUN_00480d80`),
/// with render mode 4 and lit vertices.
pub const HZN_EXTENSION: &str = ".hzn";
/// Extension of the four-word stage projection/horizon settings record.
pub const ARS_EXTENSION: &str = ".ars";
/// Bytes consumed by the shipped `.ars` loader (`FUN_00480960`).
pub const ARS_SETTINGS_LEN: usize = 16;
/// Extension of the primary object/scenery C3D graph (parser mode 2).
pub const OBJ_EXTENSION: &str = ".obj";
/// Extension of the scripted moving/animated track-object records (`FUN_0045bde0`).
pub const MOJ_EXTENSION: &str = ".moj";
/// Size of one `.moj` record (`0x30` bytes = 12 dwords).
pub const MOJ_RECORD_STRIDE: usize = 0x30;
/// Capacity of the runtime `.moj` active table (`DAT_00579b68..0x57ae28`).
pub const MOJ_MAX_ACTIVE_RECORDS: usize = 100;
/// Extension of the secondary object/scenery C3D graph (parser mode 3).
pub const SHT_EXTENSION: &str = ".sht";
/// Extension of the authored car-start records.
pub const CSP_EXTENSION: &str = ".csp";
/// One normal or mirrored `.csp` block: two positions and two rotations.
pub const CSP_BLOCK_LEN: usize = 0x28;
/// Extension of the authored trackside/cinematic camera records.
pub const TSC_EXTENSION: &str = ".tsc";
/// Size of one `.tsc` camera record.
pub const TSC_CAMERA_STRIDE: usize = 0x10;
/// Extension of the split/checkpoint timing flag section (`FUN_0047fe10`).
pub const SPL_EXTENSION: &str = ".spl";
/// Extension of the animated-texture name table (`FUN_00482cc0`).
pub const ANM_EXTENSION: &str = ".anm";
/// Size of one `.anm` texture-name record (`0x0d` = 13 bytes).
pub const ANM_RECORD_STRIDE: usize = 0x0d;
/// Extension of the Rally-School reference run section (`FUN_004716b0` -> `FUN_00473140`).
pub const RSF_EXTENSION: &str = ".rsf";
/// Extension of the compiled-out `.fuz` stub section (`FUN_00443a40` / `FUN_00443ae0`).
pub const FUZ_EXTENSION: &str = ".fuz";

/// Rally-School stage directory template (`%s` = track name, `%d` = task index).
pub const STAGE_DIR_TEMPLATE: &str = r"RALLSCHL\TRACKS\%s\TASK%d\";

/// The per-difficulty `.cjp` split-reference sub-extension (`FUN_00466350`
/// selects by `(cfg & 0x18) >> 3`): 0 -> novice, 1 -> intermediate, 2 -> easy.
pub fn cjp_extension(difficulty_index: u8) -> Option<&'static str> {
    match difficulty_index {
        0 => Some("n.cjp"),
        1 => Some("i.cjp"),
        2 => Some("e.cjp"),
        _ => None,
    }
}

/// Build a stage section file path by appending an extension to the shared base
/// path, as every section loader does in place.
pub fn stage_section_path(base: &str, extension: &str) -> String {
    format!("{base}{extension}")
}

/// Errors from decoding a stage section payload, each carrying the offending
/// length/offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageError {
    /// A `.cat` payload whose length is not a whole number of `0x1C` records.
    CatNotNodeAligned { len: usize, stride: usize },
    /// A `.cjp` payload shorter than the 13 dwords the loader copies.
    CjpTooShort { len: usize, needed: usize },
    /// A trigger-volume blob whose length is not a whole number of `0x30`
    /// records.
    TriggerNotAligned { len: usize, stride: usize },
    /// A single trigger-volume slice of the wrong length.
    TriggerRecordLen { len: usize, expected: usize },
    /// An `.ars` payload shorter than the four dwords consumed by the loader.
    ArsTooShort { len: usize, needed: usize },
    /// A present `.csp` payload was neither one complete normal block nor a
    /// complete normal+mirrored pair.
    CspInvalidLength { len: usize },
    /// A `.tsc` payload whose length is not a whole number of camera records.
    TscNotCameraAligned { len: usize, stride: usize },
    /// A `.moj` payload whose length is not a whole number of `0x30` records.
    MojNotRecordAligned { len: usize, stride: usize },
    /// An `.anm` payload whose length is not a whole number of `0x0d` records.
    AnmNotRecordAligned { len: usize, stride: usize },
}

impl fmt::Display for StageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StageError::CatNotNodeAligned { len, stride } => write!(
                f,
                "cat node table length {len} is not a multiple of the {stride}-byte node stride"
            ),
            StageError::CjpTooShort { len, needed } => {
                write!(
                    f,
                    "cjp split table length {len} is shorter than {needed} bytes"
                )
            }
            StageError::TriggerNotAligned { len, stride } => write!(
                f,
                "trigger-volume blob length {len} is not a multiple of the {stride}-byte stride"
            ),
            StageError::TriggerRecordLen { len, expected } => {
                write!(
                    f,
                    "trigger-volume record length {len} is not {expected} bytes"
                )
            }
            StageError::ArsTooShort { len, needed } => {
                write!(
                    f,
                    "ars settings length {len} is shorter than {needed} bytes"
                )
            }
            StageError::CspInvalidLength { len } => write!(
                f,
                "csp start-position length {len} is not {CSP_BLOCK_LEN} bytes or at least {} bytes",
                CSP_BLOCK_LEN * 2
            ),
            StageError::TscNotCameraAligned { len, stride } => write!(
                f,
                "tsc camera table length {len} is not a multiple of the {stride}-byte record stride"
            ),
            StageError::MojNotRecordAligned { len, stride } => write!(
                f,
                "moj moving-object table length {len} is not a multiple of the {stride}-byte record stride"
            ),
            StageError::AnmNotRecordAligned { len, stride } => write!(
                f,
                "anm animated-texture table length {len} is not a multiple of the {stride}-byte record stride"
            ),
        }
    }
}

impl std::error::Error for StageError {}

/// Supplies the CMPR-decoded bytes of a stage's sibling section files, keyed by
/// extension (`.c3d`, `.cat`, ...).
///
/// Keeping the byte source abstract keeps this a pure decoder: the base-path
/// resolution ([`stage_section_path`]), file IO, and the shared CMPR decode
/// (`FUN_00495ed0`, handled upstream in `cmr-formats::stored_asset`) all live in
/// the caller, which reads original media in place and hands the already-decoded
/// section payloads here.
pub trait StageSectionSource {
    /// The decoded bytes of the section with this extension, or `None` when the
    /// stage has no such sibling file.
    fn section(&self, extension: &str) -> Option<&[u8]>;
}

/// The decoded sections of one stage (`FUN_004228d0` walks all 14 sibling
/// extensions in [`STAGE_EXTENSIONS`]): the mandatory `.c3d` world scene and
/// the 13 optional sibling sections.
#[derive(Debug)]
pub struct StageBundle {
    scene: C3dFile,
    objects: Option<C3dFile>,
    moving_object_paths: Option<Vec<MojRecord>>,
    scenery: Option<C3dFile>,
    horizon: Option<C3dFile>,
    ars: Option<ArsSettings>,
    starts: Option<CspStarts>,
    trackside_cameras: Option<Vec<TscCameraRecord>>,
    nodes: Option<Vec<CatNode>>,
    triggers: Option<Vec<TriggerVolume>>,
    split_section: Option<SplSection>,
    animated_textures: Option<Vec<AnmRecord>>,
    school_reference: Option<SchoolReference>,
    fuz_section: Option<FuzSection>,
}

impl StageBundle {
    /// The parsed `.c3d` scene (group-0 node records + group-1 mesh records).
    #[must_use]
    pub fn scene(&self) -> &C3dFile {
        &self.scene
    }

    /// The parsed `.obj` object/scenery graph, if present.
    #[must_use]
    pub fn objects(&self) -> Option<&C3dFile> {
        self.objects.as_ref()
    }

    /// The parsed `.moj` scripted moving/animated object records, if present.
    #[must_use]
    pub fn moving_object_paths(&self) -> Option<&[MojRecord]> {
        self.moving_object_paths.as_deref()
    }

    /// The parsed `.sht` secondary scenery graph, if present.
    #[must_use]
    pub fn scenery(&self) -> Option<&C3dFile> {
        self.scenery.as_ref()
    }

    /// The parsed `.hzn` background scene, if the stage carried one.
    #[must_use]
    pub fn horizon(&self) -> Option<&C3dFile> {
        self.horizon.as_ref()
    }

    /// The parsed `.ars` stage projection/horizon settings, if present.
    #[must_use]
    pub fn ars(&self) -> Option<&ArsSettings> {
        self.ars.as_ref()
    }

    /// Authored normal/mirrored car-start records from `.csp`, if present.
    #[must_use]
    pub fn starts(&self) -> Option<&CspStarts> {
        self.starts.as_ref()
    }

    /// Parsed `.tsc` trackside/cinematic camera records, if present.
    #[must_use]
    pub fn trackside_cameras(&self) -> Option<&[TscCameraRecord]> {
        self.trackside_cameras.as_deref()
    }

    /// The parsed `.cat` node spline, if the stage carried a `.cat` sibling.
    #[must_use]
    pub fn nodes(&self) -> Option<&[CatNode]> {
        self.nodes.as_deref()
    }

    /// The parsed `.dat` trigger/collision volumes, if present.
    #[must_use]
    pub fn triggers(&self) -> Option<&[TriggerVolume]> {
        self.triggers.as_deref()
    }

    /// The `.spl` split/checkpoint section state, if a `.spl` sibling was supplied.
    #[must_use]
    pub fn split_section(&self) -> Option<&SplSection> {
        self.split_section.as_ref()
    }

    /// Parsed `.anm` animated-texture name records, if present.
    #[must_use]
    pub fn animated_textures(&self) -> Option<&[AnmRecord]> {
        self.animated_textures.as_deref()
    }

    /// Parsed `.rsf` Rally-School reference run, if present.
    #[must_use]
    pub fn school_reference(&self) -> Option<&SchoolReference> {
        self.school_reference.as_ref()
    }

    /// The `.fuz` stub section state, if a `.fuz` sibling was supplied.
    #[must_use]
    pub fn fuz_section(&self) -> Option<&FuzSection> {
        self.fuz_section.as_ref()
    }
}

/// Failure to load a stage bundle from its section source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StageLoadError {
    /// The stage has no `.c3d` section (the scene is mandatory).
    MissingScene,
    /// The `.c3d` scene payload failed to parse.
    Scene(C3dError),
    /// The optional `.hzn` background scene payload failed to parse.
    Horizon(C3dError),
    /// The optional `.obj` scene payload failed to parse.
    Objects(C3dError),
    /// The optional `.moj` moving-object path table failed to decode.
    MovingObjectPaths(StageError),
    /// The optional `.sht` scene payload failed to parse.
    Scenery(C3dError),
    /// The optional `.ars` settings payload failed to decode.
    Ars(StageError),
    /// The optional `.csp` start-position payload failed to decode.
    Starts(StageError),
    /// The optional `.tsc` trackside-camera payload failed to decode.
    TracksideCameras(StageError),
    /// The `.cat` node-spline payload failed to decode.
    Nodes(StageError),
    /// The optional `.dat` trigger-volume payload failed to decode.
    Triggers(StageError),
    /// The optional `.anm` animated-texture table failed to decode.
    AnimatedTextures(StageError),
    /// The optional `.rsf` Rally-School reference run failed to decode.
    SchoolReference(RsfError),
}

impl fmt::Display for StageLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StageLoadError::MissingScene => write!(f, "stage has no .c3d scene section"),
            StageLoadError::Scene(error) => write!(f, "stage .c3d scene: {error}"),
            StageLoadError::Horizon(error) => write!(f, "stage .hzn horizon: {error}"),
            StageLoadError::Objects(error) => write!(f, "stage .obj objects: {error}"),
            StageLoadError::MovingObjectPaths(error) => {
                write!(f, "stage .moj moving-object paths: {error}")
            }
            StageLoadError::Scenery(error) => write!(f, "stage .sht scenery: {error}"),
            StageLoadError::Ars(error) => write!(f, "stage .ars settings: {error}"),
            StageLoadError::Starts(error) => write!(f, "stage .csp starts: {error}"),
            StageLoadError::TracksideCameras(error) => {
                write!(f, "stage .tsc trackside cameras: {error}")
            }
            StageLoadError::Nodes(error) => write!(f, "stage .cat nodes: {error}"),
            StageLoadError::Triggers(error) => write!(f, "stage .dat triggers: {error}"),
            StageLoadError::AnimatedTextures(error) => {
                write!(f, "stage .anm animated textures: {error}")
            }
            StageLoadError::SchoolReference(error) => {
                write!(f, "stage .rsf school reference: {error}")
            }
        }
    }
}

impl std::error::Error for StageLoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StageLoadError::Scene(error) => Some(error),
            StageLoadError::Horizon(error) => Some(error),
            StageLoadError::Objects(error) => Some(error),
            StageLoadError::MovingObjectPaths(error) => Some(error),
            StageLoadError::Scenery(error) => Some(error),
            StageLoadError::Ars(error) => Some(error),
            StageLoadError::Starts(error) => Some(error),
            StageLoadError::TracksideCameras(error) => Some(error),
            StageLoadError::Nodes(error) => Some(error),
            StageLoadError::Triggers(error) => Some(error),
            StageLoadError::AnimatedTextures(error) => Some(error),
            StageLoadError::SchoolReference(error) => Some(error),
            StageLoadError::MissingScene => None,
        }
    }
}

/// Loads a stage bundle from its section source, parsing the mandatory `.c3d`
/// scene (with `c3d_limits`) and all present sibling sections from the
/// 14-extension master-loader table (`FUN_004228d0`).
pub fn load_stage(
    source: &impl StageSectionSource,
    c3d_limits: C3dLimits,
) -> Result<StageBundle, StageLoadError> {
    let scene_bytes = source
        .section(C3D_EXTENSION)
        .ok_or(StageLoadError::MissingScene)?;
    let scene = parse_c3d(scene_bytes, c3d_limits).map_err(StageLoadError::Scene)?;
    let objects = source
        .section(OBJ_EXTENSION)
        .map(|bytes| parse_c3d(bytes, c3d_limits))
        .transpose()
        .map_err(StageLoadError::Objects)?;
    let moving_object_paths = source
        .section(MOJ_EXTENSION)
        .map(parse_moj_records)
        .transpose()
        .map_err(StageLoadError::MovingObjectPaths)?;
    let scenery = source
        .section(SHT_EXTENSION)
        .map(|bytes| parse_c3d(bytes, c3d_limits))
        .transpose()
        .map_err(StageLoadError::Scenery)?;
    let horizon = source
        .section(HZN_EXTENSION)
        .map(|bytes| parse_c3d(bytes, c3d_limits))
        .transpose()
        .map_err(StageLoadError::Horizon)?;
    let ars = source
        .section(ARS_EXTENSION)
        .map(parse_ars_settings)
        .transpose()
        .map_err(StageLoadError::Ars)?;
    let starts = source
        .section(CSP_EXTENSION)
        .map(parse_csp_starts)
        .transpose()
        .map_err(StageLoadError::Starts)?;
    let trackside_cameras = source
        .section(TSC_EXTENSION)
        .map(parse_tsc_cameras)
        .transpose()
        .map_err(StageLoadError::TracksideCameras)?;
    let nodes = source
        .section(CAT_EXTENSION)
        .map(parse_cat_nodes)
        .transpose()
        .map_err(StageLoadError::Nodes)?;
    let triggers = source
        .section(DAT_EXTENSION)
        .map(parse_trigger_volumes)
        .transpose()
        .map_err(StageLoadError::Triggers)?;
    let split_section = source.section(SPL_EXTENSION).map(parse_spl_section);
    let animated_textures = source
        .section(ANM_EXTENSION)
        .map(parse_anm_records)
        .transpose()
        .map_err(StageLoadError::AnimatedTextures)?;
    let school_reference = source
        .section(RSF_EXTENSION)
        .map(parse_rsf)
        .transpose()
        .map_err(StageLoadError::SchoolReference)?;
    let fuz_section = source.section(FUZ_EXTENSION).map(parse_fuz_section);
    Ok(StageBundle {
        scene,
        objects,
        moving_object_paths,
        scenery,
        horizon,
        ars,
        starts,
        trackside_cameras,
        nodes,
        triggers,
        split_section,
        animated_textures,
        school_reference,
        fuz_section,
    })
}

/// One `0x30`-byte (12-dword) scripted moving/animated track-object record
/// loaded from `<stage>.moj` by `FUN_0045bde0` into `DAT_00579b68` and stepped
/// by `FUN_0045be90` / `FUN_0045c1b0` / `FUN_0045c9c0`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MojRecord {
    /// `+0x00` (`dword[0]`): nonzero snaps the object's Y coordinate to ground
    /// height via `FUN_00446c40` during `FUN_0045c9c0`.
    pub terrain_snap_flag: u32,
    /// `+0x04`: initial 12-bit BAM angle (`& 0xfff` into trig table `0x82d380`)
    /// for circular orbit modes `1` and `2`.
    pub initial_angle_bam: u16,
    /// `+0x06`: upper 16 bits of `dword[1]`, preserved verbatim.
    pub angle_padding: u16,
    /// `+0x08` (`dword[2]`): Q16.16 orbit diameter (`idiv 0x20000` -> radius)
    /// for circular orbit modes `1` and `2`.
    pub orbit_diameter_q16: i32,
    /// `+0x0c` (`dword[3]`): preserved dword with no consumer in EU v1.5.
    pub reserved_0c: u32,
    /// `+0x10..+0x1c` (`dwords[4..=6]`): Q16.16 `(X, Y, Z)` coordinates used to
    /// match a scene node by `(X, Z)` in `FUN_0045be90` and as the translation
    /// target in linear modes `3` and `4`.
    pub position_or_target_q16: [i32; 3],
    /// `+0x1c` (`dword[7]`): translation/orbit period in seconds (multiplied by
    /// 25 or 50 ticks/s into runtime `+0xe4`).
    pub translation_period_seconds: i32,
    /// `+0x20..+0x2c` (`dwords[8..=10]`): X/Y/Z rotation periods in seconds
    /// (multiplied by 25 or 50 ticks/s into runtime `+0xe8`, `+0xec`, `+0xf0`).
    pub rotation_period_seconds: [i32; 3],
    /// `+0x2c` (`dword[11]`): motion mode (`0` = inactive/skipped by
    /// `FUN_0045bde0`; `1` = XZ circle, `2` = vertical circle, `3` = one-shot
    /// linear, `4` = ping-pong linear).
    pub motion_mode: u32,
}

impl MojRecord {
    /// Whether `FUN_0045bde0` admits this record into `DAT_00579b68`
    /// (`dword[11] != 0`).
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.motion_mode != 0
    }
}

/// Decode a flat `.moj` moving-object record table (`0x30`-byte stride).
///
/// All records (including inactive `motion_mode == 0` entries) are returned so
/// the on-disk payload is represented losslessly; call
/// [`filter_active_moj_records`] for `FUN_0045bde0`'s filtered `DAT_00579b68`
/// table.
pub fn parse_moj_records(bytes: &[u8]) -> Result<Vec<MojRecord>, StageError> {
    if !bytes.len().is_multiple_of(MOJ_RECORD_STRIDE) {
        return Err(StageError::MojNotRecordAligned {
            len: bytes.len(),
            stride: MOJ_RECORD_STRIDE,
        });
    }
    let (records, tail) = bytes.as_chunks::<MOJ_RECORD_STRIDE>();
    debug_assert!(tail.is_empty());
    Ok(records
        .iter()
        .map(|record| MojRecord {
            terrain_snap_flag: read_i32(record, 0x00) as u32,
            initial_angle_bam: read_u16(record, 0x04),
            angle_padding: read_u16(record, 0x06),
            orbit_diameter_q16: read_i32(record, 0x08),
            reserved_0c: read_i32(record, 0x0c) as u32,
            position_or_target_q16: [
                read_i32(record, 0x10),
                read_i32(record, 0x14),
                read_i32(record, 0x18),
            ],
            translation_period_seconds: read_i32(record, 0x1c),
            rotation_period_seconds: [
                read_i32(record, 0x20),
                read_i32(record, 0x24),
                read_i32(record, 0x28),
            ],
            motion_mode: read_i32(record, 0x2c) as u32,
        })
        .collect())
}

/// Reproduce `FUN_0045bde0`'s load-time filtering into `DAT_00579b68`
/// (`DAT_0057ae28` count): keep only records with `motion_mode != 0`, up to
/// [`MOJ_MAX_ACTIVE_RECORDS`].
#[must_use]
pub fn filter_active_moj_records(records: &[MojRecord]) -> Vec<MojRecord> {
    records
        .iter()
        .copied()
        .filter(MojRecord::is_active)
        .take(MOJ_MAX_ACTIVE_RECORDS)
        .collect()
}

/// Decoded representation of `<stage>.spl`.
///
/// In Windows EU v1.5 `Rally.exe`, `FUN_0047fe10` (`0x47fe10..0x47fe1c`) is a
/// two-instruction stub (`mov dword ptr [0x5ffbf8], 1; ret 4`) that never opens
/// or reads the `.spl` file. Any supplied bytes are preserved verbatim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SplSection {
    /// Value written to `DAT_005ffbf8` by `FUN_0047fe10` (`true`).
    pub split_display_enabled: bool,
    unconsumed_bytes: Vec<u8>,
}

impl SplSection {
    /// Any payload bytes supplied for `.spl` (unconsumed by `FUN_0047fe10`).
    #[must_use]
    pub fn unconsumed_bytes(&self) -> &[u8] {
        &self.unconsumed_bytes
    }
}

/// Capture a `.spl` section under `FUN_0047fe10`'s stub contract.
#[must_use]
pub fn parse_spl_section(bytes: &[u8]) -> SplSection {
    SplSection {
        split_display_enabled: true,
        unconsumed_bytes: bytes.to_vec(),
    }
}

/// One `0x0d`-byte (13-byte) animated-texture name record from `<stage>.anm`
/// (`FUN_00482cc0` -> `FUN_004acd30`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnmRecord {
    /// Raw 13-byte record on disk (a NUL-terminated 8.3 texture name plus any
    /// trailing pad bytes).
    pub raw_bytes: [u8; ANM_RECORD_STRIDE],
}

impl AnmRecord {
    /// The NUL-terminated name slice (`FUN_004acd30` compares from `+0x00` up
    /// to the first NUL byte against loaded stage texture names at `0xbb95b8`).
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        let end = self
            .raw_bytes
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(ANM_RECORD_STRIDE);
        &self.raw_bytes[..end]
    }

    /// The NUL-terminated ASCII/UTF-8 texture name, if valid UTF-8.
    #[must_use]
    pub fn name_str(&self) -> Option<&str> {
        std::str::from_utf8(self.name_bytes()).ok()
    }
}

/// Decode a flat `.anm` animated-texture table (`0x0d`-byte stride).
pub fn parse_anm_records(bytes: &[u8]) -> Result<Vec<AnmRecord>, StageError> {
    if !bytes.len().is_multiple_of(ANM_RECORD_STRIDE) {
        return Err(StageError::AnmNotRecordAligned {
            len: bytes.len(),
            stride: ANM_RECORD_STRIDE,
        });
    }
    let (records, tail) = bytes.as_chunks::<ANM_RECORD_STRIDE>();
    debug_assert!(tail.is_empty());
    Ok(records
        .iter()
        .copied()
        .map(|raw_bytes| AnmRecord { raw_bytes })
        .collect())
}

/// Decoded representation of `<stage>.fuz`.
///
/// In Windows EU v1.5 `Rally.exe`, `FUN_00443a40` zeroes `DAT_00559030` and
/// `DAT_00559034` and returns `1`, and `FUN_00443ae0` (`0x443ae0..0x443ae4`) is
/// `xor eax, eax; ret 4` (`return 0`), never opening or reading `<stage>.fuz`.
/// Any supplied bytes are preserved verbatim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FuzSection {
    /// Value written to `DAT_00559030` by `FUN_00443a40` (`0`).
    pub state_559030: u32,
    /// Value written to `DAT_00559034` by `FUN_00443a40` (`0`).
    pub state_559034: u32,
    unconsumed_bytes: Vec<u8>,
}

impl FuzSection {
    /// Any payload bytes supplied for `.fuz` (unconsumed by `FUN_00443ae0`).
    #[must_use]
    pub fn unconsumed_bytes(&self) -> &[u8] {
        &self.unconsumed_bytes
    }
}

/// Capture a `.fuz` section under `FUN_00443a40` / `FUN_00443ae0`'s stub contract.
#[must_use]
pub fn parse_fuz_section(bytes: &[u8]) -> FuzSection {
    FuzSection {
        state_559030: 0,
        state_559034: 0,
        unconsumed_bytes: bytes.to_vec(),
    }
}

/// One authored `.tsc` trackside/cinematic camera record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TscCameraRecord {
    /// Q16.16 world position (`+0x00..+0x0b`).
    pub position: [i32; 3],
    /// Camera behavior selector (`+0x0c`).
    pub mode: u8,
    /// Preserved byte at `+0x0d`; no semantic consumer is currently proven.
    pub reserved: u8,
    /// CAT-node ordinal at `+0x0e`, initialized by the shipped loader for mode 2.
    pub cat_node_ordinal: u16,
}

/// Decode a flat `.tsc` camera table.
///
/// The shipped loader retains the payload and derives its record count with
/// `size >> 4`. Reconstruction rejects a non-multiple instead of silently
/// ignoring the truncated tail.
pub fn parse_tsc_cameras(bytes: &[u8]) -> Result<Vec<TscCameraRecord>, StageError> {
    if !bytes.len().is_multiple_of(TSC_CAMERA_STRIDE) {
        return Err(StageError::TscNotCameraAligned {
            len: bytes.len(),
            stride: TSC_CAMERA_STRIDE,
        });
    }
    let (records, tail) = bytes.as_chunks::<TSC_CAMERA_STRIDE>();
    debug_assert!(tail.is_empty());
    Ok(records
        .iter()
        .map(|record| TscCameraRecord {
            position: [
                read_i32(record, 0x00),
                read_i32(record, 0x04),
                read_i32(record, 0x08),
            ],
            mode: record[0x0c],
            reserved: record[0x0d],
            cat_node_ordinal: read_u16(record, 0x0e),
        })
        .collect())
}

/// Four little-endian Q16.16 dwords consumed from a stage `.ars` payload.
///
/// Word zero is observed as the horizon Y offset. The remaining words feed the
/// projection/culling setup, but their exact semantic names are not yet proven,
/// so they remain indexed. Trailing bytes are preserved for later recovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArsSettings {
    words: [i32; 4],
    trailing: Vec<u8>,
}

impl ArsSettings {
    #[must_use]
    pub const fn words(&self) -> &[i32; 4] {
        &self.words
    }

    #[must_use]
    pub const fn horizon_y_offset_q16(&self) -> i32 {
        self.words[0]
    }

    #[must_use]
    pub fn trailing(&self) -> &[u8] {
        &self.trailing
    }
}

/// Decodes the four dwords the shipped `.ars` loader consumes and preserves
/// any unconsumed tail. Missing data is handled by the bundle as `None`; a
/// present but truncated record fails closed with its observed length.
pub fn parse_ars_settings(bytes: &[u8]) -> Result<ArsSettings, StageError> {
    if bytes.len() < ARS_SETTINGS_LEN {
        return Err(StageError::ArsTooShort {
            len: bytes.len(),
            needed: ARS_SETTINGS_LEN,
        });
    }
    let mut words = [0i32; 4];
    for (index, word) in words.iter_mut().enumerate() {
        *word = read_i32(bytes, index * 4);
    }
    Ok(ArsSettings {
        words,
        trailing: bytes[ARS_SETTINGS_LEN..].to_vec(),
    })
}

/// One authored car start: Q16.16 position and three 12-bit BAM rotations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CspStart {
    pub position: [i32; 3],
    pub rotation_bam: [u16; 3],
    pub reserved: u16,
}

/// The two player starts for normal direction and, when present, reverse.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CspStarts {
    normal: [CspStart; 2],
    mirrored: Option<[CspStart; 2]>,
    trailing: Vec<u8>,
}

impl CspStarts {
    #[must_use]
    pub const fn normal(&self) -> &[CspStart; 2] {
        &self.normal
    }

    #[must_use]
    pub const fn mirrored(&self) -> Option<&[CspStart; 2]> {
        self.mirrored.as_ref()
    }

    #[must_use]
    pub fn trailing(&self) -> &[u8] {
        &self.trailing
    }
}

fn parse_csp_block(bytes: &[u8]) -> [CspStart; 2] {
    std::array::from_fn(|player| {
        let position_offset = player * 12;
        let rotation_offset = 24 + player * 8;
        CspStart {
            position: std::array::from_fn(|axis| read_i32(bytes, position_offset + axis * 4)),
            rotation_bam: std::array::from_fn(|axis| read_u16(bytes, rotation_offset + axis * 2)),
            reserved: read_u16(bytes, rotation_offset + 6),
        }
    })
}

/// Decodes the native `.csp` layout selected by `FUN_00486110`: one 0x28-byte
/// block is the normal direction; any longer form must contain the second
/// 0x28-byte mirrored block. Extra bytes beyond those blocks are preserved.
pub fn parse_csp_starts(bytes: &[u8]) -> Result<CspStarts, StageError> {
    if bytes.len() != CSP_BLOCK_LEN && bytes.len() < CSP_BLOCK_LEN * 2 {
        return Err(StageError::CspInvalidLength { len: bytes.len() });
    }
    let normal = parse_csp_block(&bytes[..CSP_BLOCK_LEN]);
    let mirrored = (bytes.len() >= CSP_BLOCK_LEN * 2)
        .then(|| parse_csp_block(&bytes[CSP_BLOCK_LEN..CSP_BLOCK_LEN * 2]));
    let consumed = if mirrored.is_some() {
        CSP_BLOCK_LEN * 2
    } else {
        CSP_BLOCK_LEN
    };
    Ok(CspStarts {
        normal,
        mirrored,
        trailing: bytes[consumed..].to_vec(),
    })
}

/// One `.cat` node: a position, a tangent/direction, a split flag, a reset
/// exclusion byte, and a sequence index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatNode {
    /// Position `(x, y, z)` at `+0x00/+0x04/+0x08`.
    pub position: [i32; 3],
    /// Tangent/direction `(x, y, z)` at `+0x0C/+0x10/+0x14` (negated on mirror).
    pub direction: [i32; 3],
    /// Split/checkpoint flag at `+0x18` (nonzero = split node).
    pub split_flag: u8,
    /// Byte at `+0x19`: nonzero excludes the node as a reset target (the
    /// dispatcher `FUN_00467210` skips back over such nodes).
    pub reserved: u8,
    /// Node sequence index at `+0x1A` (recomputed on mirror).
    pub seq_index: u16,
}

impl CatNode {
    /// Whether this node is a split/checkpoint node.
    pub fn is_split(self) -> bool {
        self.split_flag != 0
    }
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

/// Parse the header-less `.cat` node spline. The count is the file length
/// divided by the `0x1C` stride; a length that is not a whole number of records
/// is rejected with its length and the stride.
pub fn parse_cat_nodes(input: &[u8]) -> Result<Vec<CatNode>, StageError> {
    if !input.len().is_multiple_of(CAT_NODE_STRIDE) {
        return Err(StageError::CatNotNodeAligned {
            len: input.len(),
            stride: CAT_NODE_STRIDE,
        });
    }
    let nodes = input
        .as_chunks::<CAT_NODE_STRIDE>()
        .0
        .iter()
        .map(|r| CatNode {
            position: [read_i32(r, 0x00), read_i32(r, 0x04), read_i32(r, 0x08)],
            direction: [read_i32(r, 0x0c), read_i32(r, 0x10), read_i32(r, 0x14)],
            split_flag: r[0x18],
            reserved: r[0x19],
            seq_index: read_u16(r, 0x1a),
        })
        .collect();
    Ok(nodes)
}

/// Apply the recovered mirror/reverse transform (`FUN_00466350` when the
/// mirror-stage config bit is set): reverse the node order, negate each
/// direction vector, and recompute each sequence index as `count - 1 - index`.
pub fn mirror_nodes(nodes: &[CatNode]) -> Vec<CatNode> {
    let last = nodes.len().saturating_sub(1);
    nodes
        .iter()
        .rev()
        .map(|n| CatNode {
            position: n.position,
            direction: [-n.direction[0], -n.direction[1], -n.direction[2]],
            split_flag: n.split_flag,
            reserved: n.reserved,
            seq_index: (last.saturating_sub(n.seq_index as usize)) as u16,
        })
        .collect()
}

/// Count the split/checkpoint nodes (compare with [`EXPECTED_SPLIT_NODES`]).
pub fn count_split_nodes(nodes: &[CatNode]) -> usize {
    nodes.iter().filter(|n| n.is_split()).count()
}

/// A `.cjp` per-difficulty split-time reference: the 13 dwords the loader
/// copies, plus any trailing byte count beyond them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CjpSplitTable {
    /// The 13 recovered dwords.
    pub values: [i32; CJP_DWORD_COUNT],
    /// Bytes present beyond the 13 dwords (preserved as a count).
    pub trailing: usize,
}

impl CjpSplitTable {
    /// The first reference value (`+0x00`, the "6000" reference).
    pub fn reference_a(self) -> i32 {
        self.values[0]
    }
    /// The second reference value (`+0x04`, the "12000" reference).
    pub fn reference_b(self) -> i32 {
        self.values[1]
    }
    /// The target split-time entries (the 11 dwords after the two references;
    /// the recovered code wires nine of them to the active split globals).
    pub fn split_times(&self) -> &[i32] {
        &self.values[2..]
    }
}

/// Parse a `.cjp` split-time reference. The loader copies 13 dwords, so a
/// shorter payload is rejected; trailing bytes are preserved as a count.
pub fn parse_cjp(input: &[u8]) -> Result<CjpSplitTable, StageError> {
    if input.len() < CJP_RECORD_LEN {
        return Err(StageError::CjpTooShort {
            len: input.len(),
            needed: CJP_RECORD_LEN,
        });
    }
    let mut values = [0i32; CJP_DWORD_COUNT];
    for (i, value) in values.iter_mut().enumerate() {
        *value = read_i32(input, i * 4);
    }
    Ok(CjpSplitTable {
        values,
        trailing: input.len() - CJP_RECORD_LEN,
    })
}

/// One trigger volume (`0x30` bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerVolume {
    /// First point of the trigger segment (`+0x00..+0x0b`).
    pub point_a: [i32; 3],
    /// Second point of the trigger segment (`+0x0c..+0x17`).
    pub point_b: [i32; 3],
    /// Q16.16 plane/edge normal (`+0x18..+0x23`).
    pub normal_q16: [i32; 3],
    /// `+0x24` payload (pacenote id / sound id / resource / surface arg).
    pub payload: i32,
    /// `+0x28` spatial-node index on disk (rewritten to a list pointer during fix-up).
    pub spatial_node_index: i32,
    /// `+0x2C` type byte (classify with `cmr-core::collision::TriggerType`).
    pub raw_type: u8,
    /// `+0x2D` event flag byte.
    pub event_flag: u8,
    /// `+0x2E` raw on-disk byte. Native load resets it, then uses it as either
    /// a type-1 ordinal or a per-player consumed mask.
    pub raw_state: u8,
    /// `+0x2F` preserved byte with no observed consumer.
    pub unknown_2f: u8,
}

/// Decode one `0x30`-byte trigger-volume record.
pub fn parse_trigger_volume(record: &[u8]) -> Result<TriggerVolume, StageError> {
    if record.len() != TRIGGER_VOLUME_STRIDE {
        return Err(StageError::TriggerRecordLen {
            len: record.len(),
            expected: TRIGGER_VOLUME_STRIDE,
        });
    }
    Ok(TriggerVolume {
        point_a: [
            read_i32(record, 0x00),
            read_i32(record, 0x04),
            read_i32(record, 0x08),
        ],
        point_b: [
            read_i32(record, 0x0c),
            read_i32(record, 0x10),
            read_i32(record, 0x14),
        ],
        normal_q16: [
            read_i32(record, 0x18),
            read_i32(record, 0x1c),
            read_i32(record, 0x20),
        ],
        payload: read_i32(record, 0x24),
        spatial_node_index: read_i32(record, 0x28),
        raw_type: record[0x2c],
        event_flag: record[0x2d],
        raw_state: record[0x2e],
        unknown_2f: record[0x2f],
    })
}

/// Decode a flat array of trigger-volume records (stride `0x30`).
pub fn parse_trigger_volumes(input: &[u8]) -> Result<Vec<TriggerVolume>, StageError> {
    if !input.len().is_multiple_of(TRIGGER_VOLUME_STRIDE) {
        return Err(StageError::TriggerNotAligned {
            len: input.len(),
            stride: TRIGGER_VOLUME_STRIDE,
        });
    }
    input
        .as_chunks::<TRIGGER_VOLUME_STRIDE>()
        .0
        .iter()
        .map(|r| parse_trigger_volume(r))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build one synthetic `.cat` node record.
    fn cat_record(pos: [i32; 3], dir: [i32; 3], split: u8, reserved: u8, seq: u16) -> Vec<u8> {
        let mut r = Vec::with_capacity(CAT_NODE_STRIDE);
        for v in pos.into_iter().chain(dir) {
            r.extend_from_slice(&v.to_le_bytes());
        }
        r.push(split);
        r.push(reserved);
        r.extend_from_slice(&seq.to_le_bytes());
        assert_eq!(r.len(), CAT_NODE_STRIDE);
        r
    }

    #[test]
    fn parses_cat_nodes_and_counts_from_file_size() {
        let mut buf = cat_record([1, 2, 3], [10, 20, 30], 0, 0x7, 0);
        buf.extend(cat_record([4, 5, 6], [-7, -8, -9], 1, 0, 1));
        let nodes = parse_cat_nodes(&buf).unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].position, [1, 2, 3]);
        assert_eq!(nodes[0].direction, [10, 20, 30]);
        assert_eq!(nodes[0].reserved, 0x7);
        assert!(!nodes[0].is_split());
        assert_eq!(nodes[1].direction, [-7, -8, -9]);
        assert!(nodes[1].is_split());
        assert_eq!(nodes[1].seq_index, 1);
    }

    #[test]
    fn empty_cat_is_zero_nodes_misaligned_is_rejected() {
        assert_eq!(parse_cat_nodes(&[]).unwrap().len(), 0);
        let err = parse_cat_nodes(&[0u8; CAT_NODE_STRIDE + 1]).unwrap_err();
        assert_eq!(
            err,
            StageError::CatNotNodeAligned {
                len: CAT_NODE_STRIDE + 1,
                stride: CAT_NODE_STRIDE,
            }
        );
    }

    #[test]
    fn mirror_reverses_negates_and_renumbers() {
        let mut buf = cat_record([0, 0, 0], [1, 2, 3], 0, 0, 0);
        buf.extend(cat_record([9, 9, 9], [4, 5, 6], 1, 0, 1));
        buf.extend(cat_record([7, 7, 7], [7, 8, 9], 0, 0, 2));
        let nodes = parse_cat_nodes(&buf).unwrap();
        let mirrored = mirror_nodes(&nodes);
        assert_eq!(mirrored.len(), 3);
        // First mirrored node is the original last, direction negated, seq 2->0.
        assert_eq!(mirrored[0].position, [7, 7, 7]);
        assert_eq!(mirrored[0].direction, [-7, -8, -9]);
        assert_eq!(mirrored[0].seq_index, 0);
        // Last mirrored node is the original first, seq 0->2.
        assert_eq!(mirrored[2].position, [0, 0, 0]);
        assert_eq!(mirrored[2].direction, [-1, -2, -3]);
        assert_eq!(mirrored[2].seq_index, 2);
        // The split node is preserved through the mirror.
        assert_eq!(count_split_nodes(&mirrored), 1);
    }

    #[test]
    fn parses_cjp_references_and_splits() {
        let mut buf = Vec::new();
        for i in 0..CJP_DWORD_COUNT as i32 {
            buf.extend_from_slice(&(i * 100).to_le_bytes());
        }
        buf.extend_from_slice(&[0xaa, 0xbb]); // trailing
        let table = parse_cjp(&buf).unwrap();
        assert_eq!(table.reference_a(), 0);
        assert_eq!(table.reference_b(), 100);
        assert_eq!(table.split_times().len(), 11);
        assert_eq!(table.split_times()[0], 200);
        assert_eq!(table.trailing, 2);
    }

    #[test]
    fn cjp_too_short_is_rejected() {
        let err = parse_cjp(&[0u8; CJP_RECORD_LEN - 1]).unwrap_err();
        assert_eq!(
            err,
            StageError::CjpTooShort {
                len: CJP_RECORD_LEN - 1,
                needed: CJP_RECORD_LEN,
            }
        );
    }

    #[test]
    fn parses_trigger_volume_record() {
        let mut r = vec![0u8; TRIGGER_VOLUME_STRIDE];
        for (offset, value) in [
            (0x00, 11i32),
            (0x04, 12),
            (0x08, 13),
            (0x0c, 21),
            (0x10, 22),
            (0x14, 23),
            (0x18, 0x10000),
            (0x1c, -0x10000),
            (0x20, 0),
        ] {
            r[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        r[0x24..0x28].copy_from_slice(&42i32.to_le_bytes());
        r[0x28..0x2c].copy_from_slice(&5i32.to_le_bytes());
        r[0x2c] = 0x15; // out-of-bounds type
        r[0x2d] = 1;
        r[0x2e] = 0x02;
        r[0x2f] = 0x01;
        let v = parse_trigger_volume(&r).unwrap();
        assert_eq!(v.point_a, [11, 12, 13]);
        assert_eq!(v.point_b, [21, 22, 23]);
        assert_eq!(v.normal_q16, [0x10000, -0x10000, 0]);
        assert_eq!(v.payload, 42);
        assert_eq!(v.spatial_node_index, 5);
        assert_eq!(v.raw_type, 0x15);
        assert_eq!(v.event_flag, 1);
        assert_eq!(v.raw_state, 0x02);
        assert_eq!(v.unknown_2f, 0x01);
    }

    #[test]
    fn trigger_volume_length_errors() {
        assert_eq!(
            parse_trigger_volume(&[0u8; 10]).unwrap_err(),
            StageError::TriggerRecordLen {
                len: 10,
                expected: TRIGGER_VOLUME_STRIDE,
            }
        );
        assert_eq!(
            parse_trigger_volumes(&[0u8; TRIGGER_VOLUME_STRIDE + 3]).unwrap_err(),
            StageError::TriggerNotAligned {
                len: TRIGGER_VOLUME_STRIDE + 3,
                stride: TRIGGER_VOLUME_STRIDE,
            }
        );
        let two = vec![0u8; TRIGGER_VOLUME_STRIDE * 2];
        assert_eq!(parse_trigger_volumes(&two).unwrap().len(), 2);
    }

    #[test]
    fn bundle_metadata_matches_recovery() {
        assert_eq!(STAGE_EXTENSIONS.len(), 14);
        assert_eq!(STAGE_EXTENSIONS[0], ".c3d");
        assert_eq!(STAGE_EXTENSIONS[9], ".cat");
        assert_eq!(STAGE_EXTENSIONS[13], ".fuz");
        assert_eq!(
            stage_section_path(r"TRACKS\MONACO\TASK1\stage", CAT_EXTENSION),
            r"TRACKS\MONACO\TASK1\stage.cat"
        );
        assert_eq!(cjp_extension(0), Some("n.cjp"));
        assert_eq!(cjp_extension(1), Some("i.cjp"));
        assert_eq!(cjp_extension(2), Some("e.cjp"));
        assert_eq!(cjp_extension(3), None);
    }

    /// A stage section source backed by an in-memory extension -> bytes map.
    struct MapSource(std::collections::HashMap<&'static str, Vec<u8>>);

    impl StageSectionSource for MapSource {
        fn section(&self, extension: &str) -> Option<&[u8]> {
            self.0.get(extension).map(Vec::as_slice)
        }
    }

    /// A minimal valid header-only CM3D payload (no records).
    fn empty_scene_bytes() -> Vec<u8> {
        let mut buf = vec![0u8; 32];
        buf[0..4].copy_from_slice(b"CM3D");
        buf[4..8].copy_from_slice(&100u32.to_le_bytes()); // version
        buf[20..24].copy_from_slice(&32u32.to_le_bytes()); // group0 at end
        buf[24..28].copy_from_slice(&32u32.to_le_bytes()); // group1 at end
        buf[28..32].copy_from_slice(&32u32.to_le_bytes()); // blob at end
        buf
    }

    #[test]
    fn load_stage_parses_the_scene_and_optional_cat_nodes() {
        let mut sections = std::collections::HashMap::new();
        sections.insert(C3D_EXTENSION, empty_scene_bytes());
        sections.insert(CAT_EXTENSION, vec![0u8; CAT_NODE_STRIDE * 2]); // two nodes
        let source = MapSource(sections);

        let bundle = load_stage(&source, C3dLimits::default()).unwrap();
        assert_eq!(bundle.scene().group0_records().count(), 0);
        assert!(bundle.objects().is_none());
        assert!(bundle.scenery().is_none());
        assert!(bundle.horizon().is_none());
        assert!(bundle.ars().is_none());
        assert!(bundle.starts().is_none());
        assert!(bundle.trackside_cameras().is_none());
        assert_eq!(bundle.nodes().unwrap().len(), 2);
        assert!(bundle.triggers().is_none());
    }

    fn tsc_record(position: [i32; 3], mode: u8, reserved: u8, ordinal: u16) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(TSC_CAMERA_STRIDE);
        for component in position {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
        bytes.push(mode);
        bytes.push(reserved);
        bytes.extend_from_slice(&ordinal.to_le_bytes());
        bytes
    }

    #[test]
    fn tsc_camera_parser_accepts_empty_and_multiple_records() {
        assert!(parse_tsc_cameras(&[]).unwrap().is_empty());

        let mut bytes = tsc_record([1, -2, 3], 2, 0xaa, 0x1234);
        bytes.extend_from_slice(&tsc_record([4, 5, -6], 7, 0xbb, 0xffff));
        let cameras = parse_tsc_cameras(&bytes).unwrap();
        assert_eq!(
            cameras,
            [
                TscCameraRecord {
                    position: [1, -2, 3],
                    mode: 2,
                    reserved: 0xaa,
                    cat_node_ordinal: 0x1234,
                },
                TscCameraRecord {
                    position: [4, 5, -6],
                    mode: 7,
                    reserved: 0xbb,
                    cat_node_ordinal: 0xffff,
                },
            ]
        );
    }

    #[test]
    fn tsc_camera_parser_rejects_a_truncated_tail() {
        assert_eq!(
            parse_tsc_cameras(&[0; TSC_CAMERA_STRIDE + 1]).unwrap_err(),
            StageError::TscNotCameraAligned {
                len: TSC_CAMERA_STRIDE + 1,
                stride: TSC_CAMERA_STRIDE,
            }
        );
    }

    #[test]
    fn load_stage_parses_optional_trackside_cameras_and_types_misalignment() {
        let mut sections = std::collections::HashMap::new();
        sections.insert(C3D_EXTENSION, empty_scene_bytes());
        sections.insert(TSC_EXTENSION, tsc_record([10, 20, 30], 2, 0, 7));
        let bundle = load_stage(&MapSource(sections), C3dLimits::default()).unwrap();
        assert_eq!(bundle.trackside_cameras().unwrap()[0].cat_node_ordinal, 7);

        let mut malformed = std::collections::HashMap::new();
        malformed.insert(C3D_EXTENSION, empty_scene_bytes());
        malformed.insert(TSC_EXTENSION, vec![0; TSC_CAMERA_STRIDE - 1]);
        assert_eq!(
            load_stage(&MapSource(malformed), C3dLimits::default()).unwrap_err(),
            StageLoadError::TracksideCameras(StageError::TscNotCameraAligned {
                len: TSC_CAMERA_STRIDE - 1,
                stride: TSC_CAMERA_STRIDE,
            })
        );
    }

    #[test]
    fn load_stage_parses_optional_trigger_volumes() {
        let mut sections = std::collections::HashMap::new();
        sections.insert(C3D_EXTENSION, empty_scene_bytes());
        sections.insert(DAT_EXTENSION, vec![0u8; TRIGGER_VOLUME_STRIDE * 2]);

        let bundle = load_stage(&MapSource(sections), C3dLimits::default()).unwrap();
        assert_eq!(bundle.triggers().unwrap().len(), 2);
    }

    #[test]
    fn load_stage_rejects_misaligned_trigger_volumes() {
        let mut sections = std::collections::HashMap::new();
        sections.insert(C3D_EXTENSION, empty_scene_bytes());
        sections.insert(DAT_EXTENSION, vec![0u8; TRIGGER_VOLUME_STRIDE + 1]);

        assert_eq!(
            load_stage(&MapSource(sections), C3dLimits::default()).unwrap_err(),
            StageLoadError::Triggers(StageError::TriggerNotAligned {
                len: TRIGGER_VOLUME_STRIDE + 1,
                stride: TRIGGER_VOLUME_STRIDE,
            })
        );
    }

    #[test]
    fn load_stage_parses_an_optional_horizon_as_c3d() {
        let mut sections = std::collections::HashMap::new();
        sections.insert(C3D_EXTENSION, empty_scene_bytes());
        sections.insert(HZN_EXTENSION, empty_scene_bytes());
        let source = MapSource(sections);

        let bundle = load_stage(&source, C3dLimits::default()).unwrap();
        assert_eq!(bundle.horizon().unwrap().group0_records().count(), 0);
    }

    #[test]
    fn load_stage_parses_optional_object_and_scenery_c3d_graphs() {
        let mut sections = std::collections::HashMap::new();
        sections.insert(C3D_EXTENSION, empty_scene_bytes());
        sections.insert(OBJ_EXTENSION, empty_scene_bytes());
        sections.insert(SHT_EXTENSION, empty_scene_bytes());
        let source = MapSource(sections);

        let bundle = load_stage(&source, C3dLimits::default()).unwrap();
        assert_eq!(bundle.objects().unwrap().group0_records().count(), 0);
        assert_eq!(bundle.scenery().unwrap().group0_records().count(), 0);
    }

    #[test]
    fn load_stage_identifies_the_malformed_optional_c3d_graph() {
        for (extension, is_expected) in [
            (
                OBJ_EXTENSION,
                StageLoadError::Objects as fn(C3dError) -> StageLoadError,
            ),
            (
                SHT_EXTENSION,
                StageLoadError::Scenery as fn(C3dError) -> StageLoadError,
            ),
        ] {
            let mut sections = std::collections::HashMap::new();
            sections.insert(C3D_EXTENSION, empty_scene_bytes());
            sections.insert(extension, b"short".to_vec());
            let error = load_stage(&MapSource(sections), C3dLimits::default()).unwrap_err();
            assert_eq!(
                error,
                is_expected(C3dError::TruncatedHeader {
                    actual: 5,
                    required: 32,
                })
            );
        }
    }

    #[test]
    fn load_stage_propagates_a_malformed_horizon_section() {
        let mut sections = std::collections::HashMap::new();
        sections.insert(C3D_EXTENSION, empty_scene_bytes());
        sections.insert(HZN_EXTENSION, b"not a c3d".to_vec());
        let source = MapSource(sections);

        assert!(matches!(
            load_stage(&source, C3dLimits::default()).unwrap_err(),
            StageLoadError::Horizon(C3dError::TruncatedHeader { .. })
        ));
    }

    #[test]
    fn ars_settings_decode_four_words_and_preserve_the_tail() {
        let mut bytes = Vec::new();
        for word in [-0x20000i32, 0x960000, 0xc80000, 0x640000] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.extend_from_slice(&[0xaa, 0xbb]);
        let settings = parse_ars_settings(&bytes).unwrap();
        assert_eq!(settings.words(), &[-0x20000, 0x960000, 0xc80000, 0x640000]);
        assert_eq!(settings.horizon_y_offset_q16(), -0x20000);
        assert_eq!(settings.trailing(), &[0xaa, 0xbb]);
    }

    #[test]
    fn load_stage_rejects_a_present_but_truncated_ars_section() {
        let mut sections = std::collections::HashMap::new();
        sections.insert(C3D_EXTENSION, empty_scene_bytes());
        sections.insert(ARS_EXTENSION, vec![0u8; ARS_SETTINGS_LEN - 1]);
        let source = MapSource(sections);
        assert_eq!(
            load_stage(&source, C3dLimits::default()).unwrap_err(),
            StageLoadError::Ars(StageError::ArsTooShort {
                len: ARS_SETTINGS_LEN - 1,
                needed: ARS_SETTINGS_LEN,
            })
        );
    }

    fn csp_block(base: i32) -> Vec<u8> {
        let mut bytes = Vec::new();
        for player in 0..2 {
            for axis in 0..3 {
                bytes.extend_from_slice(&(base + player * 10 + axis).to_le_bytes());
            }
        }
        for player in 0..2u16 {
            for axis in 0..3u16 {
                bytes.extend_from_slice(&(0x100 + player * 10 + axis).to_le_bytes());
            }
            bytes.extend_from_slice(&(0x9000 + player).to_le_bytes());
        }
        bytes
    }

    #[test]
    fn csp_decodes_two_normal_and_optional_mirrored_starts() {
        let normal = csp_block(100);
        let starts = parse_csp_starts(&normal).unwrap();
        assert_eq!(starts.normal()[0].position, [100, 101, 102]);
        assert_eq!(starts.normal()[1].position, [110, 111, 112]);
        assert_eq!(starts.normal()[0].rotation_bam, [0x100, 0x101, 0x102]);
        assert_eq!(starts.normal()[1].reserved, 0x9001);
        assert!(starts.mirrored().is_none());

        let mut both = normal;
        both.extend_from_slice(&csp_block(200));
        both.extend_from_slice(&[0xaa, 0xbb]);
        let starts = parse_csp_starts(&both).unwrap();
        assert_eq!(starts.mirrored().unwrap()[0].position, [200, 201, 202]);
        assert_eq!(starts.trailing(), &[0xaa, 0xbb]);
    }

    #[test]
    fn csp_rejects_truncated_second_block() {
        assert_eq!(
            parse_csp_starts(&[0; CSP_BLOCK_LEN + 1]).unwrap_err(),
            StageError::CspInvalidLength {
                len: CSP_BLOCK_LEN + 1
            }
        );
    }

    #[test]
    fn load_stage_without_a_cat_sibling_has_no_nodes() {
        let mut sections = std::collections::HashMap::new();
        sections.insert(C3D_EXTENSION, empty_scene_bytes());
        let source = MapSource(sections);

        let bundle = load_stage(&source, C3dLimits::default()).unwrap();
        assert!(bundle.nodes().is_none());
    }

    #[test]
    fn load_stage_requires_the_c3d_scene() {
        let source = MapSource(std::collections::HashMap::new());
        assert_eq!(
            load_stage(&source, C3dLimits::default()).unwrap_err(),
            StageLoadError::MissingScene
        );
    }

    #[test]
    fn load_stage_propagates_a_malformed_cat_section() {
        let mut sections = std::collections::HashMap::new();
        sections.insert(C3D_EXTENSION, empty_scene_bytes());
        sections.insert(CAT_EXTENSION, vec![0u8; CAT_NODE_STRIDE + 3]); // misaligned
        let source = MapSource(sections);

        assert_eq!(
            load_stage(&source, C3dLimits::default()).unwrap_err(),
            StageLoadError::Nodes(StageError::CatNotNodeAligned {
                len: CAT_NODE_STRIDE + 3,
                stride: CAT_NODE_STRIDE,
            })
        );
    }

    fn moj_record_bytes(mode: u32, pos: [i32; 3], snap: u32, angle: u16, diameter: i32) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MOJ_RECORD_STRIDE);
        bytes.extend_from_slice(&snap.to_le_bytes());
        bytes.extend_from_slice(&angle.to_le_bytes());
        bytes.extend_from_slice(&0x1234u16.to_le_bytes());
        bytes.extend_from_slice(&diameter.to_le_bytes());
        bytes.extend_from_slice(&0xdead_beefu32.to_le_bytes());
        for c in pos {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        bytes.extend_from_slice(&12i32.to_le_bytes()); // translation period
        for rot in [3i32, 4, 5] {
            bytes.extend_from_slice(&rot.to_le_bytes());
        }
        bytes.extend_from_slice(&mode.to_le_bytes());
        assert_eq!(bytes.len(), MOJ_RECORD_STRIDE);
        bytes
    }

    #[test]
    fn moj_records_parse_and_filter_active_entries() {
        assert!(parse_moj_records(&[]).unwrap().is_empty());

        let mut bytes = moj_record_bytes(0, [10, 20, 30], 0, 0x100, 0x20_0000);
        bytes.extend_from_slice(&moj_record_bytes(1, [40, 50, 60], 1, 0x400, 0x40_0000));
        let records = parse_moj_records(&bytes).unwrap();
        assert_eq!(records.len(), 2);
        assert!(!records[0].is_active());
        assert!(records[1].is_active());
        assert_eq!(records[1].terrain_snap_flag, 1);
        assert_eq!(records[1].initial_angle_bam, 0x400);
        assert_eq!(records[1].angle_padding, 0x1234);
        assert_eq!(records[1].orbit_diameter_q16, 0x40_0000);
        assert_eq!(records[1].reserved_0c, 0xdead_beef);
        assert_eq!(records[1].position_or_target_q16, [40, 50, 60]);
        assert_eq!(records[1].translation_period_seconds, 12);
        assert_eq!(records[1].rotation_period_seconds, [3, 4, 5]);
        assert_eq!(records[1].motion_mode, 1);

        let active = filter_active_moj_records(&records);
        assert_eq!(active, vec![records[1]]);

        assert_eq!(
            parse_moj_records(&[0u8; MOJ_RECORD_STRIDE + 1]).unwrap_err(),
            StageError::MojNotRecordAligned {
                len: MOJ_RECORD_STRIDE + 1,
                stride: MOJ_RECORD_STRIDE,
            }
        );
    }

    #[test]
    fn anm_records_parse_13_byte_names_and_reject_misalignment() {
        assert!(parse_anm_records(&[]).unwrap().is_empty());

        let mut rec0 = [0u8; ANM_RECORD_STRIDE];
        rec0[..9].copy_from_slice(b"WATER.BMP");
        rec0[10] = 0xaa;
        let mut rec1 = [b'X'; ANM_RECORD_STRIDE];
        let mut bytes = Vec::from(rec0);
        bytes.extend_from_slice(&rec1);

        let parsed = parse_anm_records(&bytes).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name_str(), Some("WATER.BMP"));
        assert_eq!(parsed[0].raw_bytes[10], 0xaa);
        assert_eq!(parsed[1].name_bytes().len(), ANM_RECORD_STRIDE);
        rec1[0] = 0xff;
        let non_utf8 = AnmRecord { raw_bytes: rec1 };
        assert_eq!(non_utf8.name_str(), None);

        assert_eq!(
            parse_anm_records(&[0u8; ANM_RECORD_STRIDE - 1]).unwrap_err(),
            StageError::AnmNotRecordAligned {
                len: ANM_RECORD_STRIDE - 1,
                stride: ANM_RECORD_STRIDE,
            }
        );
    }

    #[test]
    fn spl_and_fuz_sections_capture_stub_states_and_preserve_unconsumed_bytes() {
        let spl = parse_spl_section(&[1, 2, 3]);
        assert!(spl.split_display_enabled);
        assert_eq!(spl.unconsumed_bytes(), &[1, 2, 3]);

        let fuz = parse_fuz_section(&[4, 5]);
        assert_eq!(fuz.state_559030, 0);
        assert_eq!(fuz.state_559034, 0);
        assert_eq!(fuz.unconsumed_bytes(), &[4, 5]);
    }

    #[test]
    fn load_stage_integrates_all_14_master_loader_sibling_extensions() {
        let mut sections = std::collections::HashMap::new();
        sections.insert(C3D_EXTENSION, empty_scene_bytes());
        sections.insert(DAT_EXTENSION, vec![0u8; TRIGGER_VOLUME_STRIDE]);
        sections.insert(OBJ_EXTENSION, empty_scene_bytes());
        sections.insert(
            MOJ_EXTENSION,
            moj_record_bytes(4, [1, 2, 3], 1, 0x200, 0x10_0000),
        );
        sections.insert(SHT_EXTENSION, empty_scene_bytes());
        sections.insert(ARS_EXTENSION, vec![0u8; ARS_SETTINGS_LEN]);
        sections.insert(HZN_EXTENSION, empty_scene_bytes());
        sections.insert(TSC_EXTENSION, vec![0u8; TSC_CAMERA_STRIDE]);
        sections.insert(CSP_EXTENSION, vec![0u8; CSP_BLOCK_LEN]);
        sections.insert(CAT_EXTENSION, vec![0u8; CAT_NODE_STRIDE]);
        sections.insert(SPL_EXTENSION, vec![0x99]);
        sections.insert(ANM_EXTENSION, vec![0u8; ANM_RECORD_STRIDE * 2]);
        sections.insert(RSF_EXTENSION, vec![0u8; crate::school_rsf::RSF_BLOCK_SIZE]);
        sections.insert(FUZ_EXTENSION, vec![0x77, 0x88]);
        assert_eq!(sections.len(), STAGE_EXTENSIONS.len());

        let bundle = load_stage(&MapSource(sections), C3dLimits::default()).unwrap();
        assert!(bundle.objects().is_some());
        assert_eq!(bundle.moving_object_paths().unwrap().len(), 1);
        assert!(bundle.scenery().is_some());
        assert!(bundle.ars().is_some());
        assert!(bundle.horizon().is_some());
        assert_eq!(bundle.trackside_cameras().unwrap().len(), 1);
        assert!(bundle.starts().is_some());
        assert_eq!(bundle.nodes().unwrap().len(), 1);
        assert_eq!(bundle.triggers().unwrap().len(), 1);
        assert!(bundle.split_section().unwrap().split_display_enabled);
        assert_eq!(bundle.animated_textures().unwrap().len(), 2);
        assert_eq!(
            bundle.school_reference().unwrap().records.len(),
            crate::school_rsf::RSF_RECORDS
        );
        assert_eq!(
            bundle.fuz_section().unwrap().unconsumed_bytes(),
            &[0x77, 0x88]
        );
    }
}
