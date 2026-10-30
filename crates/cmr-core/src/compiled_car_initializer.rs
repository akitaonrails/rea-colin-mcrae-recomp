//! Payload-free contract for the compiled per-selector fields in
//! official-EU `FUN_00404440`.
//!
//! The shipped routine selects one of fourteen immediate-store bodies. Each
//! body writes the same eight state offsets and twenty-three control offsets.
//! The values themselves are game data and are deliberately not present here:
//! callers provide them, and tests use synthetic records only.
//!
//! After intervening initializer calls, the routine derives reciprocal and
//! copy families from eight control words. That later value transformation is
//! exposed separately so this module does not imply that the omitted callees
//! have been reconstructed. Offset names are neutral; physical meanings and
//! selector-to-car names remain unknown.

use std::fmt;

use cmr_assets::{AuxControlConstantsStore, CompiledInitializerStore};
use cmr_formats::hpc::HpcFile;
use cmr_formats::pe_aux_control_initializer::{
    AUX_CONTROL_COMMON_WORD_OFFSETS as FORMAT_AUX_COMMON_WORD_OFFSETS,
    AUX_CONTROL_POLICY_WORD_OFFSETS as FORMAT_AUX_POLICY_WORD_OFFSETS,
    AUX_CONTROL_WORD_OFFSETS as FORMAT_AUX_WORD_OFFSETS,
    AuxControlConstantWords as FormatAuxControlConstantWords,
    AuxControlPolicyWords as FormatAuxControlPolicyWords,
};
use cmr_formats::pe_compiled_initializer::{
    COMPILED_CASE_COUNT as FORMAT_CASE_COUNT,
    COMPILED_CONTROL_WORD_OFFSETS as FORMAT_CONTROL_WORD_OFFSETS,
    COMPILED_STATE_WORD_OFFSETS as FORMAT_STATE_WORD_OFFSETS,
    CompiledCaseWords as FormatCompiledCaseWords,
};

use crate::legacy_math::sqrt_q16;

/// Number of compiled selector bodies accepted by the unsigned dispatch.
pub const COMPILED_CASE_COUNT: usize = 14;

/// Number of direct state-word stores in each compiled body.
pub const COMPILED_STATE_WORD_COUNT: usize = 8;

/// Number of direct control-word stores in each compiled body.
pub const COMPILED_CONTROL_WORD_COUNT: usize = 23;

/// Number of words in the later primary/reciprocal control families.
pub const COMPILED_CONTROL_FAMILY_WORD_COUNT: usize = 8;

/// Number of middle control-family words rewritten by the recovered transform.
pub const COMPILED_CONTROL_RUN_WORD_COUNT: usize = 6;

/// Number of dword destinations written by the auxiliary control patch.
pub const AUX_CONTROL_WORD_COUNT: usize = 22;

/// Stride applied to the live configuration block by the compiled helpers.
pub const COMPILED_CONFIG_RECORD_STRIDE: usize = 0x09c;

/// State-slot domain supplied by every resolved explicit constructor.
///
/// This does not describe eight homogeneous or authored configuration
/// records. Only the lookup index domain is established; later slots alias
/// other live configuration-block fields at the observed stride.
pub const COMPILED_CONFIG_SLOT_COUNT: i8 = 8;

/// Unindexed configuration byte shared by multiple recovered branches.
pub const COMPILED_CONFIG_UNINDEXED_FLAGS_26_OFFSET: usize = 0x026;

/// Compatibility name for the same byte's auxiliary-default use.
pub const COMPILED_CONFIG_AUX_FORCE_FLAG_OFFSET: usize = COMPILED_CONFIG_UNINDEXED_FLAGS_26_OFFSET;

/// Unindexed configuration byte controlling the common-prefix scale branches.
pub const COMPILED_CONFIG_COMMON_FLAGS_OFFSET: usize = 0x025;

/// Record-relative auxiliary-policy option byte.
pub const COMPILED_CONFIG_AUX_OPTION_OFFSET: usize = 0x093;

/// Record-relative center-drive-split option byte.
pub const COMPILED_CONFIG_CENTER_SPLIT_OPTION_OFFSET: usize = 0x095;

/// Record-relative six-word-run option byte.
pub const COMPILED_CONFIG_CONTROL_RUN_OPTION_OFFSET: usize = 0x099;

/// Record-relative fallback byte consumed by `FUN_00486090`.
pub const COMPILED_CONFIG_STATE_758_OPTION_OFFSET: usize = 0x091;
pub const COMPILED_CONFIG_STATE_758_SELECTION_OFFSET: usize = 0x6c0;

const COMPILED_CONFIG_AUX_FORCE_DEFAULT_MASK: u8 = 0x02;
pub const COMPILED_MIXED_RESET_NEGATIVE_VALUE_MASK: u8 = 0x02;

/// Auxiliary control destinations in ascending offset order.
pub const AUX_CONTROL_WORD_OFFSETS: [u16; AUX_CONTROL_WORD_COUNT] = [
    0x0d8, 0x0dc, 0x0e0, 0x0e4, 0x0e8, 0x0ec, 0x0f0, 0x0f4, 0x0f8, 0x0fc, 0x100, 0x104, 0x108,
    0x10c, 0x110, 0x118, 0x11c, 0x120, 0x124, 0x128, 0x12c, 0x13c,
];

/// Policy-owned destinations in [`AuxControlPolicyConstants::words`] order.
pub const AUX_CONTROL_POLICY_WORD_OFFSETS: [u16; 7] =
    [0x0f8, 0x10c, 0x110, 0x120, 0x124, 0x128, 0x13c];

/// Common immediate destinations in [`AuxControlConstants::common_words`] order.
pub const AUX_CONTROL_COMMON_WORD_OFFSETS: [u16; 9] = [
    0x0d8, 0x0dc, 0x0e0, 0x0e4, 0x0e8, 0x0ec, 0x0f0, 0x0fc, 0x104,
];

const _: [(); AUX_CONTROL_WORD_OFFSETS.len()] = [(); FORMAT_AUX_WORD_OFFSETS.len()];
const _: [(); AUX_CONTROL_COMMON_WORD_OFFSETS.len()] = [(); FORMAT_AUX_COMMON_WORD_OFFSETS.len()];
const _: [(); AUX_CONTROL_POLICY_WORD_OFFSETS.len()] = [(); FORMAT_AUX_POLICY_WORD_OFFSETS.len()];

/// State destinations in their observed case-body store order.
pub const COMPILED_STATE_WORD_OFFSETS: [u16; COMPILED_STATE_WORD_COUNT] =
    [0x020, 0x024, 0x028, 0x638, 0x63c, 0x640, 0x648, 0x644];

/// Complete state-destination universe of the recovered common prefix.
pub const COMPILED_COMMON_PREFIX_STATE_OFFSETS: [u16; COMPILED_STATE_WORD_COUNT + 2] = [
    0x020, 0x024, 0x028, 0x638, 0x63c, 0x640, 0x648, 0x644, 0x048, 0x04c,
];

/// Complete control-destination universe of the recovered common prefix.
pub const COMPILED_COMMON_PREFIX_CONTROL_OFFSETS: [u16; 2] = [0x148, 0x15c];

/// Control dword destinations in canonical ascending offset order.
pub const COMPILED_POST_HELPER_CONTROL_WORD_OFFSETS: [u16; 15] = [
    0x144, 0x14c, 0x150, 0x154, 0x160, 0x174, 0x178, 0x17c, 0x180, 0x184, 0x188, 0x18c, 0x1a8,
    0x1b4, 0x408,
];

pub const COMPILED_POST_HELPER_STATE_WORD_OFFSETS: [u16; 1] = [0x0ec];
pub const COMPILED_POST_HELPER_CONTROL_BYTE_OFFSETS: [u16; 1] = [0x440];
pub const COMPILED_POST_HELPER_CONTROL_U16_OFFSETS: [u16; 1] = [0x3e4];

/// Control destinations in the neutral schema order used by
/// [`CompiledCaseWords::control_words`].
///
/// The eight-word family later copied and reciprocated occupies indices
/// `14..22` (`+0x1d0..+0x1ec`).
pub const COMPILED_CONTROL_WORD_OFFSETS: [u16; COMPILED_CONTROL_WORD_COUNT] = [
    0x148, 0x158, 0x16c, 0x15c, 0x194, 0x198, 0x19c, 0x1a0, 0x1a4, 0x1b8, 0x1bc, 0x1c0, 0x1c4,
    0x1c8, 0x1d0, 0x1d4, 0x1d8, 0x1dc, 0x1e0, 0x1e4, 0x1e8, 0x1ec, 0x140,
];

const CONTROL_FAMILY_START: usize = 14;

/// Unindexed configuration flag that scales two direct control destinations.
pub const COMPILED_COMMON_CONTROL_SCALE_MASK: u8 = 0x02;

/// Unindexed configuration flag that scales all eight direct state words.
pub const COMPILED_COMMON_STATE_SCALE_MASK: u8 = 0x04;

/// Caller-supplied words for one compiled selector body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompiledCaseWords {
    pub state_words: [i32; COMPILED_STATE_WORD_COUNT],
    pub control_words: [i32; COMPILED_CONTROL_WORD_COUNT],
}

/// An owned set of the fourteen direct case-body writes.
///
/// Construction proves that the format/store boundary and core contract agree
/// on selector count and neutral offset order. The set contains no result from
/// the omitted common-path helpers and must not be treated as post-processed
/// input to [`derive_compiled_control_families`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledDirectCaseSet {
    cases: [CompiledCaseWords; COMPILED_CASE_COUNT],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompiledDirectCaseSetError {
    SelectorCount {
        core: usize,
        format: usize,
    },
    OffsetOrder {
        object: &'static str,
        index: usize,
        core_offset: u16,
        format_offset: u16,
    },
}

impl fmt::Display for CompiledDirectCaseSetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "compiled direct-case store schema does not match the core contract: {self:?}"
        )
    }
}

impl std::error::Error for CompiledDirectCaseSetError {}

impl CompiledDirectCaseSet {
    /// Copy the verified format-owned direct cases into a core-owned set.
    pub fn from_store(
        store: &CompiledInitializerStore,
    ) -> Result<Self, CompiledDirectCaseSetError> {
        Self::from_format_cases(store.cases())
    }

    fn from_format_cases(
        cases: &[FormatCompiledCaseWords; FORMAT_CASE_COUNT],
    ) -> Result<Self, CompiledDirectCaseSetError> {
        verify_format_schema()?;
        Ok(Self {
            cases: std::array::from_fn(|selector| CompiledCaseWords {
                state_words: cases[selector].state_words,
                control_words: cases[selector].control_words,
            }),
        })
    }

    #[must_use]
    pub const fn cases(&self) -> &[CompiledCaseWords; COMPILED_CASE_COUNT] {
        &self.cases
    }

    /// Apply only the observed unsigned direct-case dispatch.
    #[must_use]
    pub fn apply(&self, prior: CompiledCaseWords, selector: u32) -> CompiledCaseOutcome {
        apply_compiled_case(prior, &self.cases, selector)
    }
}

impl TryFrom<&CompiledInitializerStore> for CompiledDirectCaseSet {
    type Error = CompiledDirectCaseSetError;

    fn try_from(store: &CompiledInitializerStore) -> Result<Self, Self::Error> {
        Self::from_store(store)
    }
}

fn verify_format_schema() -> Result<(), CompiledDirectCaseSetError> {
    if FORMAT_CASE_COUNT != COMPILED_CASE_COUNT {
        return Err(CompiledDirectCaseSetError::SelectorCount {
            core: COMPILED_CASE_COUNT,
            format: FORMAT_CASE_COUNT,
        });
    }
    for (object, core_offsets, format_offsets) in [
        (
            "state",
            COMPILED_STATE_WORD_OFFSETS.as_slice(),
            FORMAT_STATE_WORD_OFFSETS.as_slice(),
        ),
        (
            "control",
            COMPILED_CONTROL_WORD_OFFSETS.as_slice(),
            FORMAT_CONTROL_WORD_OFFSETS.as_slice(),
        ),
    ] {
        for (index, (&core_offset, &format_offset)) in
            core_offsets.iter().zip(format_offsets).enumerate()
        {
            if core_offset != format_offset {
                return Err(CompiledDirectCaseSetError::OffsetOrder {
                    object,
                    index,
                    core_offset,
                    format_offset,
                });
            }
        }
    }
    Ok(())
}

impl CompiledCaseWords {
    /// The eight direct case-body words at `+0x1d0..+0x1ec`, before common-path
    /// helpers such as `FUN_00404160` may transform them.
    #[must_use]
    pub fn control_family(self) -> [i32; COMPILED_CONTROL_FAMILY_WORD_COUNT] {
        std::array::from_fn(|index| self.control_words[CONTROL_FAMILY_START + index])
    }
}

/// Result of the unsigned selector dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledCaseOutcome {
    /// Selected words, or the prior destination words when the selector was
    /// outside the observed unsigned range.
    pub words: CompiledCaseWords,
    /// The selected body. `None` means the case stores were skipped.
    pub selected_case: Option<u8>,
}

/// Apply the observed unsigned fourteen-way case dispatch.
///
/// Values `0..=13` replace all thirty-one case destinations. Every other
/// `u32` preserves the prior destination words; there is no clamp or fallback
/// case. This function covers only the direct case stores, not the common tail.
#[must_use]
pub fn apply_compiled_case(
    prior: CompiledCaseWords,
    cases: &[CompiledCaseWords; COMPILED_CASE_COUNT],
    selector: u32,
) -> CompiledCaseOutcome {
    let selected_case = (selector < COMPILED_CASE_COUNT as u32).then_some(selector as u8);
    let words = selected_case
        .map(|index| cases[index as usize])
        .unwrap_or(prior);
    CompiledCaseOutcome {
        words,
        selected_case,
    }
}

/// Exact writes performed by the observed common prefix.
///
/// `None` means the corresponding flag-controlled destination was not written.
/// This is a bounded patch, not a complete state/control object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledCommonPrefixEffects {
    pub control_word_148: i32,
    pub control_word_15c: Option<i32>,
    pub state_words: Option<[i32; COMPILED_STATE_WORD_COUNT]>,
    pub state_word_48: i32,
    pub state_word_4c: i32,
}

/// A derived state `+0x48` whose signed `2^32 / value` quotient would make the
/// shipped x86 `IDIV` fault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledCommonPrefixReciprocalOverflow {
    pub state_word_48: i32,
}

impl fmt::Display for CompiledCommonPrefixReciprocalOverflow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "compiled common-prefix reciprocal overflows i32 for state +0x48 value {}",
            self.state_word_48
        )
    }
}

impl std::error::Error for CompiledCommonPrefixReciprocalOverflow {}

/// Apply the common prefix reached after both valid and invalid selectors.
///
/// The raw unindexed config byte controls two independent scaling branches.
/// Control `+0x148` is always adjusted before optional scaling; state `+0x48`
/// is then derived from control `+0x140`, and state `+0x4c` is its guarded
/// signed Q16 reciprocal. Every arithmetic store uses the observed 32-bit
/// wrapping/low-word behavior. On reciprocal overflow this pure contract
/// returns no partial effect bundle, an explicit reconstruction-safety policy;
/// the shipped in-place routine had already performed earlier writes.
pub fn derive_compiled_common_prefix(
    words: CompiledCaseWords,
    config_flags: u8,
) -> Result<CompiledCommonPrefixEffects, CompiledCommonPrefixReciprocalOverflow> {
    let adjusted_word_148 = words.control_words[0].wrapping_sub(0x051e);
    let control_scale_enabled = config_flags & COMPILED_COMMON_CONTROL_SCALE_MASK != 0;
    let control_word_148 = if control_scale_enabled {
        common_prefix_q16_mul(adjusted_word_148, 0x2_0000)
    } else {
        adjusted_word_148
    };
    let control_word_15c =
        control_scale_enabled.then(|| common_prefix_q16_mul(words.control_words[3], 0x1_3333));
    let state_words = (config_flags & COMPILED_COMMON_STATE_SCALE_MASK != 0).then(|| {
        words
            .state_words
            .map(|word| common_prefix_q16_mul(word, 0x8000))
    });

    let state_word_48 = words.control_words[22].wrapping_add(0x78_0000);
    let state_word_4c = if state_word_48 == 0 {
        -1
    } else {
        let quotient = (1_i64 << 32) / i64::from(state_word_48);
        i32::try_from(quotient)
            .map_err(|_| CompiledCommonPrefixReciprocalOverflow { state_word_48 })?
    };

    Ok(CompiledCommonPrefixEffects {
        control_word_148,
        control_word_15c,
        state_words,
        state_word_48,
        state_word_4c,
    })
}

fn common_prefix_q16_mul(left: i32, right: i32) -> i32 {
    ((i64::from(left) * i64::from(right)) >> 16) as i32
}

pub const COMPILED_CONTROL_PRODUCT_RECIPROCAL_OFFSET: u16 = 0x164;
pub const COMPILED_CONTROL_PRODUCT_OFFSET: u16 = 0x168;
pub const COMPILED_CONTROL_PRODUCT_OFFSETS: [u16; 2] = [
    COMPILED_CONTROL_PRODUCT_RECIPROCAL_OFFSET,
    COMPILED_CONTROL_PRODUCT_OFFSET,
];

/// State destinations written by the bounded suffix-default interval.
pub const COMPILED_SUFFIX_DEFAULT_STATE_OFFSETS: [u16; 4] = [0x044, 0x050, 0x054, 0x058];
pub const COMPILED_STATE_RECIPROCAL_TRIO_OFFSETS: [u16; 3] = [0x038, 0x03c, 0x040];
pub const COMPILED_CONTROL_CLEAR_LOOP_WORD_COUNT: usize = 16;

/// Control dword destinations in the clear loop's observed store order.
pub const COMPILED_CONTROL_CLEAR_LOOP_OFFSETS: [u16; COMPILED_CONTROL_CLEAR_LOOP_WORD_COUNT] = [
    0x31c, 0x33c, 0x320, 0x340, 0x324, 0x344, 0x328, 0x348, 0x32c, 0x34c, 0x330, 0x350, 0x334,
    0x354, 0x338, 0x358,
];
pub const COMPILED_HALF_SIGN_STATE_WORD_COUNT: usize = 27;

/// State dword destinations in the half/corner interval's observed store order.
pub const COMPILED_HALF_SIGN_STATE_OFFSETS: [u16; COMPILED_HALF_SIGN_STATE_WORD_COUNT] = [
    0x02c, 0x030, 0x034, 0x068, 0x06c, 0x070, 0x05c, 0x060, 0x064, 0x074, 0x078, 0x07c, 0x080,
    0x084, 0x088, 0x098, 0x09c, 0x0a0, 0x08c, 0x090, 0x094, 0x0a4, 0x0a8, 0x0ac, 0x0b0, 0x0b4,
    0x0b8,
];
pub const COMPILED_MIXED_RESET_CONTROL_OFFSETS: [u16; 10] = [
    0x2a8, 0x2b8, 0x2ac, 0x2bc, 0x2b0, 0x2c0, 0x2b4, 0x2c4, 0x250, 0x254,
];
pub const COMPILED_MIXED_RESET_STATE_DWORD_OFFSETS: [u16; 4] = [0x584, 0x588, 0x58c, 0x590];
pub const COMPILED_MIXED_RESET_STATE_U16_OFFSETS: [u16; 4] = [0x57c, 0x57e, 0x580, 0x582];
pub const COMPILED_CALL_SPINE_CONTROL_U16_OFFSETS: [u16; 8] =
    [0x3d0, 0x3d2, 0x3d4, 0x3d6, 0x3d8, 0x3da, 0x3dc, 0x3de];
pub const COMPILED_CALL_SPINE_GLOBAL_ADDRESSES: [u32; 3] = [0x50c5f0, 0x50c5f4, 0x50c5f8];
pub const COMPILED_STATE_BASIS_MATRIX_OFFSETS: [[u16; 3]; 3] =
    [[0x00, 0x04, 0x08], [0x10, 0x14, 0x18], [0x20, 0x24, 0x28]];
pub const COMPILED_STATE_BASIS_VECTOR_BASES: [u16; 8] =
    [0x0fc, 0x108, 0x114, 0x120, 0x12c, 0x138, 0x144, 0x150];
pub const COMPILED_STATE_BASIS_OFFSETS: [u16; 24] = [
    0x0fc, 0x100, 0x104, 0x108, 0x10c, 0x110, 0x114, 0x118, 0x11c, 0x120, 0x124, 0x128, 0x12c,
    0x130, 0x134, 0x138, 0x13c, 0x140, 0x144, 0x148, 0x14c, 0x150, 0x154, 0x158,
];
pub const COMPILED_STATE_BASIS_ADJUSTMENT_STORE_OFFSETS: [u16; 12] = [
    0x12c, 0x130, 0x134, 0x138, 0x13c, 0x140, 0x144, 0x148, 0x14c, 0x154, 0x150, 0x158,
];
pub const COMPILED_LANE_SOURCE_VECTOR_BASES: [u16; 8] =
    [0x0fc, 0x108, 0x114, 0x120, 0x12c, 0x138, 0x144, 0x150];
pub const COMPILED_LANE_OUTPUT_VECTOR_BASES: [u16; 8] =
    [0x33c, 0x348, 0x354, 0x360, 0x36c, 0x378, 0x384, 0x390];
pub const COMPILED_LANE_WORD_OFFSETS: [u16; 8] =
    [0x2e4, 0x2e8, 0x2ec, 0x2f0, 0x2f4, 0x2f8, 0x2fc, 0x300];
pub const COMPILED_LANE_TYPE_U16_OFFSETS: [u16; 8] =
    [0x42a, 0x42c, 0x42e, 0x430, 0x432, 0x434, 0x436, 0x438];
pub const COMPILED_LANE_HANDLE_I16_OFFSETS: [u16; 8] =
    [0x3d0, 0x3d2, 0x3d4, 0x3d6, 0x3d8, 0x3da, 0x3dc, 0x3de];
pub const COMPILED_LANE_GATE_OFFSETS: [u16; 8] =
    [0x69c, 0x6a0, 0x6a4, 0x6a8, 0x6ac, 0x6b0, 0x6b4, 0x6b8];
pub const COMPILED_LANE_DRIVE_OFFSETS: [u16; 8] =
    [0x304, 0x308, 0x30c, 0x310, 0x314, 0x318, 0x31c, 0x320];
pub const COMPILED_LANE_ACCUMULATOR_OFFSETS: [u16; 8] =
    [0x618, 0x61c, 0x620, 0x624, 0x628, 0x62c, 0x630, 0x634];
pub const COMPILED_LANE_FIRST_FOUR_SUBTRACTION_OFFSETS: [u16; 4] = [0x5e8, 0x5f0, 0x5f8, 0x600];
pub const COMPILED_LANE_LATCH_OFFSET: u16 = 0x67c;
pub const COMPILED_LANE_CONFIG_FLAG_OFFSET: u16 = 0x26;
pub const COMPILED_LANE_ACCUMULATOR_LATCH_THRESHOLD: i32 = 0x00c8_9cbc;
pub const COMPILED_POST_LANE_SELECTOR_Y_OFFSETS: [u16; 3] = [0x25c, 0x268, 0x274];
pub const COMPILED_POST_LANE_OUTPUT_OFFSETS: [u16; 3] = [0x324, 0x328, 0x32c];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledSuffixDefaultEffects {
    pub state_word_44: i32,
    pub state_words_50_54_58: [i32; 3],
}

/// Reconstruct initializer interval `[0x4066dd,0x406831)`.
///
/// The first three effective state words are arithmetic-halved into stack
/// temporaries, squared with wrapping signed Q16 projection, and accumulated
/// with wrapping adds before the shared generated-table square root. The
/// halves are not written back. Raw integer seeds `[1, 2, 1]` are written to
/// `+0x50/+0x54/+0x58`; config `+0x25 & 0x04` doubles them through the shipped
/// Q16 multiply.
#[must_use]
pub fn derive_compiled_suffix_defaults(
    effective_state_words_20_24_28: [i32; 3],
    config_flags: u8,
) -> CompiledSuffixDefaultEffects {
    let halves = effective_state_words_20_24_28.map(|word| word >> 1);
    let square_sum = halves.into_iter().fold(0_i32, |sum, word| {
        sum.wrapping_add(common_prefix_q16_mul(word, word))
    });
    let state_word_44 = sqrt_q16(square_sum as u32) as i32;

    let mut state_words_50_54_58 = [1, 2, 1];
    if config_flags & COMPILED_COMMON_STATE_SCALE_MASK != 0 {
        state_words_50_54_58 =
            state_words_50_54_58.map(|word| common_prefix_q16_mul(word, 0x2_0000));
    }

    CompiledSuffixDefaultEffects {
        state_word_44,
        state_words_50_54_58,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledStateReciprocalTrio {
    pub words: [i32; 3],
}

/// A sequential suffix reciprocal whose shipped `IDIV` would fault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledStateReciprocalTrioOverflow {
    pub index: usize,
    pub divisor: i32,
    /// Earlier stores completed by the shipped in-place sequence.
    pub completed: [Option<i32>; 3],
}

impl fmt::Display for CompiledStateReciprocalTrioOverflow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "compiled state reciprocal {} overflows i32 for divisor {}",
            self.index, self.divisor
        )
    }
}

impl std::error::Error for CompiledStateReciprocalTrioOverflow {}

/// Reconstruct sequential interval `[0x406831,0x4068dc)`.
///
/// Each zero input maps to `-1`; otherwise signed `2^32 / input` truncates
/// toward zero. On quotient overflow the safe function returns no effect, but
/// its error records earlier stores already visible in the shipped sequence.
pub fn derive_compiled_state_reciprocal_trio(
    effective_state_words_20_24_28: [i32; 3],
) -> Result<CompiledStateReciprocalTrio, CompiledStateReciprocalTrioOverflow> {
    let mut words = [0; 3];
    for (index, divisor) in effective_state_words_20_24_28.into_iter().enumerate() {
        words[index] = if divisor == 0 {
            -1
        } else {
            let quotient = (1_i64 << 32) / i64::from(divisor);
            i32::try_from(quotient).map_err(|_| {
                let mut completed = [None; 3];
                for completed_index in 0..index {
                    completed[completed_index] = Some(words[completed_index]);
                }
                CompiledStateReciprocalTrioOverflow {
                    index,
                    divisor,
                    completed,
                }
            })?
        };
    }
    Ok(CompiledStateReciprocalTrio { words })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledControlClearLoop {
    pub words: [i32; COMPILED_CONTROL_CLEAR_LOOP_WORD_COUNT],
}

/// Reconstruct the composed initializer effect of `[0x4068dc,0x4068ff)`.
///
/// The loop itself stores the carried `EDI`; the immediately preceding path
/// establishes `EDI = 0` and does not modify it before these stores. The
/// shipped loop reloads the control pointer before each store, so this pure
/// aggregate assumes a stable destination and does not reproduce partial
/// publication from a memory fault or concurrent pointer mutation.
#[must_use]
pub const fn derive_compiled_control_clear_loop() -> CompiledControlClearLoop {
    CompiledControlClearLoop {
        words: [0; COMPILED_CONTROL_CLEAR_LOOP_WORD_COUNT],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledHalfSignState {
    pub words: [i32; COMPILED_HALF_SIGN_STATE_WORD_COUNT],
}

/// Reconstruct state interval `[0x4068ff,0x406a44)`.
///
/// The three effective state inputs are loaded before any destination store,
/// arithmetic-halved, and reused to form eight sign combinations. Negation is
/// wrapping, matching x86 `NEG` for `i32::MIN`. The shipped code repeatedly
/// reloads the state pointer; this pure aggregate assumes one stable object and
/// does not reproduce partial publication from memory faults or pointer races.
#[must_use]
pub fn derive_compiled_half_sign_state(
    effective_state_words_20_24_28: [i32; 3],
) -> CompiledHalfSignState {
    let [x, y, z] = effective_state_words_20_24_28.map(|word| word >> 1);
    let negative_x = x.wrapping_neg();
    let negative_y = y.wrapping_neg();
    let negative_z = z.wrapping_neg();
    CompiledHalfSignState {
        words: [
            x, y, z, x, negative_y, negative_z, x, negative_y, z, negative_x, negative_y, z,
            negative_x, negative_y, negative_z, x, y, negative_z, x, y, z, negative_x, y, z,
            negative_x, y, negative_z,
        ],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledMixedResetEffects {
    pub control_words: [i32; 10],
    pub state_words: [i32; 4],
    pub state_u16_words: [u16; 4],
}

/// Reconstruct the reset effects whose register setup overlaps the preceding
/// state-write interval, completing combined region `[0x4068ff,0x406aaf)`.
#[must_use]
pub fn derive_compiled_mixed_reset_effects(unindexed_flags_26: u8) -> CompiledMixedResetEffects {
    let state_word = if unindexed_flags_26 & COMPILED_MIXED_RESET_NEGATIVE_VALUE_MASK != 0 {
        -0x8000
    } else {
        0
    };
    CompiledMixedResetEffects {
        control_words: [0; 10],
        state_words: [state_word; 4],
        state_u16_words: [0; 4],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompiledCallPointerSource {
    CapturedState,
    CapturedStateAndControl,
    ReloadedGlobalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompiledCallSpineStep {
    Call {
        target_va: u32,
        stack_arg_count: u8,
        pointer_source: CompiledCallPointerSource,
    },
    ControlU16Clear,
    GlobalSeed,
}

pub const COMPILED_CALL_SPINE_STEPS: [CompiledCallSpineStep; 5] = [
    CompiledCallSpineStep::Call {
        target_va: 0x409c10,
        stack_arg_count: 1,
        pointer_source: CompiledCallPointerSource::CapturedState,
    },
    CompiledCallSpineStep::ControlU16Clear,
    CompiledCallSpineStep::GlobalSeed,
    CompiledCallSpineStep::Call {
        target_va: 0x419a40,
        stack_arg_count: 2,
        pointer_source: CompiledCallPointerSource::CapturedStateAndControl,
    },
    CompiledCallSpineStep::Call {
        target_va: 0x408fe0,
        stack_arg_count: 0,
        pointer_source: CompiledCallPointerSource::ReloadedGlobalState,
    },
];

/// Caller-local direct effects between the three mutating call barriers in
/// `[0x406aaf,0x406b00)`. This is not a complete spine result and must not be
/// appended to [`RecoveredCompiledStageEffects`] until the callees are modeled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledCallSpineLocalEffects {
    pub control_u16_words: [u16; 8],
    pub global_words: [i32; 3],
}

#[must_use]
pub const fn derive_compiled_call_spine_local_effects() -> CompiledCallSpineLocalEffects {
    CompiledCallSpineLocalEffects {
        control_u16_words: [0; 8],
        global_words: [0, -0x8000, 0],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledStateBasisInputs {
    pub half_words: [i32; 3],
    pub matrix_rows: [[i32; 3]; 3],
    pub center: [i32; 3],
    pub adjustment_u: [i32; 3],
    pub adjustment_v: [i32; 3],
    pub scale_638: i32,
    pub scale_63c: i32,
    pub scale_640: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledStateBasisEffects {
    /// Final vectors in [`COMPILED_STATE_BASIS_VECTOR_BASES`] order.
    pub vectors: [[i32; 3]; 8],
}

#[must_use]
pub fn apply_compiled_state_basis_adjustment(
    mut primary: [[i32; 3]; 4],
    adjustment_u: [i32; 3],
    adjustment_v: [i32; 3],
    scale_638: i32,
    scale_63c: i32,
    scale_640: i32,
) -> [[i32; 3]; 4] {
    let p = adjustment_u.map(|word| common_prefix_q16_mul(word, scale_638));
    let q = adjustment_v.map(|word| common_prefix_q16_mul(word, scale_640));
    let r = adjustment_u.map(|word| common_prefix_q16_mul(word, scale_63c));
    for component in 0..3 {
        primary[0][component] =
            primary[0][component].wrapping_sub(p[component].wrapping_add(q[component]));
        primary[1][component] =
            primary[1][component].wrapping_add(q[component].wrapping_sub(p[component]));
        primary[2][component] =
            primary[2][component].wrapping_add(r[component].wrapping_sub(q[component]));
        primary[3][component] =
            primary[3][component].wrapping_add(r[component].wrapping_add(q[component]));
    }
    primary
}

/// Pure stable-snapshot contract for `FUN_00409c10` and `FUN_00418120`.
///
/// The native matrix pointer is read progressively and native stores are
/// non-atomic. This aggregate is equivalent only when all sources are stable
/// and the matrix storage does not alias the destination range.
#[must_use]
pub fn derive_compiled_state_basis_effects(
    input: CompiledStateBasisInputs,
) -> CompiledStateBasisEffects {
    let [half_x, half_y, half_z] = input.half_words;
    let a = input.matrix_rows[0].map(|word| common_prefix_q16_mul(word, half_x));
    let b = input.matrix_rows[1].map(|word| common_prefix_q16_mul(word, half_y));
    let c = input.matrix_rows[2].map(|word| common_prefix_q16_mul(word, half_z));

    let combine = |a_sign: bool, b_sign: bool, c_sign: bool| {
        std::array::from_fn(|component| {
            let mut word = 0_i32;
            for (value, positive) in [
                (a[component], a_sign),
                (b[component], b_sign),
                (c[component], c_sign),
            ] {
                word = if positive {
                    word.wrapping_add(value)
                } else {
                    word.wrapping_sub(value)
                };
            }
            word
        })
    };
    let primary: [[i32; 3]; 4] = [
        combine(true, true, true),
        combine(true, true, false),
        combine(false, true, true),
        combine(false, true, false),
    ];
    let reflected = [primary[3], primary[2], primary[1], primary[0]].map(|vector| {
        std::array::from_fn(|component| input.center[component].wrapping_sub(vector[component]))
    });
    let translated_primary = primary.map(|vector| {
        std::array::from_fn(|component| input.center[component].wrapping_add(vector[component]))
    });
    let adjusted = apply_compiled_state_basis_adjustment(
        translated_primary,
        input.adjustment_u,
        input.adjustment_v,
        input.scale_638,
        input.scale_63c,
        input.scale_640,
    );
    CompiledStateBasisEffects {
        vectors: [
            reflected[0],
            reflected[1],
            reflected[2],
            reflected[3],
            adjusted[0],
            adjusted[1],
            adjusted[2],
            adjusted[3],
        ],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledLanePostDispatchSnapshot {
    pub lane_word: i32,
    pub type_word: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledEightLanePostprocessInputs {
    /// EAX plus the type word observed after each successful call and before
    /// that lane's caller-side postprocessing begins.
    pub post_dispatch: [CompiledLanePostDispatchSnapshot; 8],
    pub gate_words: [i32; 8],
    pub drive_words: [i32; 8],
    pub accumulator_words: [i32; 8],
    pub first_four_subtractions: [i32; 4],
    /// Per-lane snapshots model the native global-config reload on every loop.
    pub config_flag_26: [u8; 8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledEightLanePostprocessEffects {
    pub lane_words: [i32; 8],
    pub output_vector_writes: [Option<[i32; 3]>; 8],
    pub type_word_writes: [Option<u16>; 8],
    pub accumulator_writes: [Option<i32>; 8],
    /// `Some(1)` represents the conditional native store; `None` preserves it.
    pub latch_67c: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CompiledSingleLanePostprocessInput {
    post_dispatch: CompiledLanePostDispatchSnapshot,
    gate_word: i32,
    drive_word: i32,
    accumulator_word: i32,
    first_four_subtraction: Option<i32>,
    config_flag_26: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledSingleLanePostprocessEffects {
    pub lane_word: i32,
    pub output_vector_write: Option<[i32; 3]>,
    pub type_word_write: Option<u16>,
    pub accumulator_write: Option<i32>,
    /// `Some(1)` represents the conditional native store; `None` preserves it.
    pub latch_67c: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompiledLaneTypeEffect {
    None,
    ReduceAndResetVector,
    Reduce,
    ReduceAndAccumulate,
}

#[must_use]
pub const fn compiled_lane_type_effect(type_word: u16) -> CompiledLaneTypeEffect {
    // Native sign-extends the word, subtracts 0x3b, then performs an unsigned
    // range check. Matching the six surviving positive values on raw bits is
    // equivalent; every high-bit word necessarily takes the default path.
    match type_word {
        0x3b | 0x3f | 0x40 => CompiledLaneTypeEffect::ReduceAndResetVector,
        0x51 | 0x53 => CompiledLaneTypeEffect::Reduce,
        0x52 => CompiledLaneTypeEffect::ReduceAndAccumulate,
        _ => CompiledLaneTypeEffect::None,
    }
}

fn derive_compiled_single_lane_before_config(
    input: CompiledSingleLanePostprocessInput,
) -> CompiledSingleLanePostprocessEffects {
    let mut lane_word = input.post_dispatch.lane_word;
    let mut output_vector_write = None;
    let mut accumulator_write = None;
    let mut latch_67c = None;

    if input.gate_word != 0 {
        let doubled = common_prefix_q16_mul(input.drive_word, 0x2_0000);
        let upper_clamped = doubled.min(0x1_0000);
        lane_word = lane_word.wrapping_add(common_prefix_q16_mul(upper_clamped, 0x9999));
    }

    match compiled_lane_type_effect(input.post_dispatch.type_word) {
        CompiledLaneTypeEffect::None => {}
        CompiledLaneTypeEffect::ReduceAndResetVector => {
            lane_word = lane_word.wrapping_sub(0x4ccc);
            output_vector_write = Some([0, 0x1_0000, 0]);
        }
        CompiledLaneTypeEffect::Reduce => {
            lane_word = lane_word.wrapping_sub(0x4ccc);
        }
        CompiledLaneTypeEffect::ReduceAndAccumulate => {
            lane_word = lane_word.wrapping_sub(0x4ccc);
            lane_word = lane_word.wrapping_sub(common_prefix_q16_mul(input.accumulator_word, 0xc4));
            let accumulator = input.accumulator_word.wrapping_add(0x1_0000);
            accumulator_write = Some(accumulator);
            if accumulator >= COMPILED_LANE_ACCUMULATOR_LATCH_THRESHOLD {
                latch_67c = Some(1);
            }
        }
    }

    if let Some(subtraction) = input.first_four_subtraction {
        lane_word = lane_word.wrapping_sub(subtraction);
    }

    CompiledSingleLanePostprocessEffects {
        lane_word,
        output_vector_write,
        type_word_write: None,
        accumulator_write,
        latch_67c,
    }
}

fn apply_compiled_single_lane_config(
    effects: &mut CompiledSingleLanePostprocessEffects,
    config_flag_26: u8,
) {
    if config_flag_26 & 0x02 != 0 {
        effects.lane_word = effects.lane_word.wrapping_add(0x1_0000);
        effects.output_vector_write = Some([0, 0x1_0000, 0]);
        effects.type_word_write = Some(0x000a);
    }
}

fn derive_compiled_single_lane_postprocess(
    input: CompiledSingleLanePostprocessInput,
) -> CompiledSingleLanePostprocessEffects {
    let mut effects = derive_compiled_single_lane_before_config(input);
    apply_compiled_single_lane_config(&mut effects, input.config_flag_26);
    effects
}

/// Pure caller-side contract for `FUN_00419a40` after each lane dispatcher
/// returns. EAX and the post-call type word are supplied together as an
/// explicit post-dispatch snapshot.
///
/// The shipped loop publishes progressive per-lane stores and reloads the
/// global config pointer every iteration. This aggregate assumes stable,
/// non-aliasing state/control objects, successful config-pointer dereferences,
/// and no dispatcher-created cross-lane aliases. Dispatcher and postprocessing
/// are interleaved natively, so this total aggregate cannot represent earlier
/// lane or same-lane prefixes surviving a later dispatcher/config fault. It
/// retains a separate config byte for each lane so a captured trace can
/// represent native per-iteration values. Optional fields report only writes
/// performed by this caller; dispatcher-owned passthrough state is omitted.
#[must_use]
pub fn derive_compiled_eight_lane_postprocess(
    input: CompiledEightLanePostprocessInputs,
) -> CompiledEightLanePostprocessEffects {
    let mut lane_words = [0; 8];
    let mut output_vector_writes = [None; 8];
    let mut type_word_writes = [None; 8];
    let mut accumulator_words = input.accumulator_words;
    let mut accumulator_writes = [None; 8];
    let mut latch_67c = None;

    for lane_index in 0..8 {
        let effects = derive_compiled_single_lane_postprocess(CompiledSingleLanePostprocessInput {
            post_dispatch: input.post_dispatch[lane_index],
            gate_word: input.gate_words[lane_index],
            drive_word: input.drive_words[lane_index],
            accumulator_word: accumulator_words[lane_index],
            first_four_subtraction: (lane_index < 4)
                .then_some(input.first_four_subtractions[lane_index.min(3)]),
            config_flag_26: input.config_flag_26[lane_index],
        });
        lane_words[lane_index] = effects.lane_word;
        output_vector_writes[lane_index] = effects.output_vector_write;
        type_word_writes[lane_index] = effects.type_word_write;
        if let Some(accumulator) = effects.accumulator_write {
            accumulator_words[lane_index] = accumulator;
            accumulator_writes[lane_index] = Some(accumulator);
        }
        if effects.latch_67c.is_some() {
            latch_67c = effects.latch_67c;
        }
    }

    CompiledEightLanePostprocessEffects {
        lane_words,
        output_vector_writes,
        type_word_writes,
        accumulator_writes,
        latch_67c,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledLaneDispatchInput {
    pub source_vector: [i32; 3],
    pub initial_handle: i16,
    pub fallback: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompiledLaneBuildCompletion {
    Unavailable,
    Built {
        lane_word: i32,
        type_word: u16,
        output_vector: [i32; 3],
    },
}

pub trait CompiledLaneGeometryProvider {
    type Error;

    fn validate(&mut self, source_vector: [i32; 3], handle: i16) -> Result<bool, Self::Error>;
    fn search(
        &mut self,
        source_vector: [i32; 3],
        fallback: i32,
    ) -> Result<Option<i16>, Self::Error>;
    fn build(
        &mut self,
        source_vector: [i32; 3],
        handle: i16,
        fallback: i32,
    ) -> Result<CompiledLaneBuildCompletion, Self::Error>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompiledLaneHandleWriteTrace {
    pub normalized_to_minus_one: bool,
    pub searched_handle: Option<i16>,
    pub invalidated_after_search_miss: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledLaneDispatchCompletion {
    pub lane_word: i32,
    pub handle_writes: CompiledLaneHandleWriteTrace,
    pub type_word_write: Option<u16>,
    pub output_vector_write: Option<[i32; 3]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompiledLaneDispatchFailurePhase {
    Validate,
    Search,
    Build,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledLaneDispatchFailure<E> {
    pub phase: CompiledLaneDispatchFailurePhase,
    pub handle_writes: CompiledLaneHandleWriteTrace,
    pub provider_error: E,
}

fn finish_compiled_lane_build(
    build: CompiledLaneBuildCompletion,
    handle_writes: CompiledLaneHandleWriteTrace,
) -> CompiledLaneDispatchCompletion {
    match build {
        CompiledLaneBuildCompletion::Unavailable => CompiledLaneDispatchCompletion {
            lane_word: 0,
            handle_writes,
            type_word_write: None,
            output_vector_write: None,
        },
        CompiledLaneBuildCompletion::Built {
            lane_word,
            type_word,
            output_vector,
        } => CompiledLaneDispatchCompletion {
            lane_word,
            handle_writes,
            type_word_write: Some(type_word),
            output_vector_write: Some(output_vector),
        },
    }
}

/// Safe no-fault-provider orchestration contract for `FUN_00446c40`.
///
/// Provider errors retain the completed orchestration-visible handle-write
/// prefix, including a completed search selection. Partial writes internal to
/// the failing provider phase remain owned by the provider error. The
/// typed contract snapshots the source vector and assumes stable, mutually
/// disjoint native arguments. It does not reproduce pointer aliases, machine
/// faults, concurrent mutation, or the open geometry/table arithmetic.
pub fn dispatch_compiled_lane<P: CompiledLaneGeometryProvider>(
    input: CompiledLaneDispatchInput,
    provider: &mut P,
) -> Result<CompiledLaneDispatchCompletion, CompiledLaneDispatchFailure<P::Error>> {
    let mut handle = input.initial_handle;
    let mut handle_writes = CompiledLaneHandleWriteTrace::default();
    if handle < 0 {
        handle = -1;
        handle_writes.normalized_to_minus_one = true;
    }

    let needs_search = if handle == -1 {
        true
    } else {
        match provider.validate(input.source_vector, handle) {
            Ok(valid) => !valid,
            Err(provider_error) => {
                return Err(CompiledLaneDispatchFailure {
                    phase: CompiledLaneDispatchFailurePhase::Validate,
                    handle_writes,
                    provider_error,
                });
            }
        }
    };

    if needs_search {
        match provider.search(input.source_vector, input.fallback) {
            Ok(Some(selected_handle)) => {
                handle = selected_handle;
                handle_writes.searched_handle = Some(selected_handle);
            }
            Ok(None) => {
                if input.initial_handle >= 0 {
                    handle_writes.invalidated_after_search_miss = true;
                }
                return Ok(CompiledLaneDispatchCompletion {
                    lane_word: input.fallback,
                    handle_writes,
                    type_word_write: None,
                    output_vector_write: None,
                });
            }
            Err(provider_error) => {
                return Err(CompiledLaneDispatchFailure {
                    phase: CompiledLaneDispatchFailurePhase::Search,
                    handle_writes,
                    provider_error,
                });
            }
        }
    }

    match provider.build(input.source_vector, handle, input.fallback) {
        Ok(build) => Ok(finish_compiled_lane_build(build, handle_writes)),
        Err(provider_error) => Err(CompiledLaneDispatchFailure {
            phase: CompiledLaneDispatchFailurePhase::Build,
            handle_writes,
            provider_error,
        }),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledEightLaneExecutionInputs {
    pub source_vectors: [[i32; 3]; 8],
    pub gate_words: [i32; 8],
    pub drive_words: [i32; 8],
    pub first_four_subtractions: [i32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledEightLaneExecutionState {
    pub handles: [i16; 8],
    pub type_words: [u16; 8],
    pub output_vectors: [[i32; 3]; 8],
    pub lane_words: [i32; 8],
    pub accumulator_words: [i32; 8],
    pub latch_67c: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledLaneExecutionEffects {
    /// Provider/wrapper writes happen before every caller-owned write below.
    pub dispatch: CompiledLaneDispatchCompletion,
    pub postprocess: CompiledSingleLanePostprocessEffects,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledEightLaneExecutionSuccess {
    pub state: CompiledEightLaneExecutionState,
    pub lanes: [CompiledLaneExecutionEffects; 8],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledEightLaneExecutionProgress {
    pub completed_lanes: usize,
    pub state: CompiledEightLaneExecutionState,
    pub lanes: [Option<CompiledLaneExecutionEffects>; 8],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompiledEightLaneExecutionFailure<ProviderError, ConfigError> {
    Dispatch {
        failed_lane: usize,
        progress: Box<CompiledEightLaneExecutionProgress>,
        failure: CompiledLaneDispatchFailure<ProviderError>,
    },
    Config {
        failed_lane: usize,
        progress: Box<CompiledEightLaneExecutionProgress>,
        dispatch: CompiledLaneDispatchCompletion,
        pre_config_postprocess: CompiledSingleLanePostprocessEffects,
        config_error: ConfigError,
    },
}

/// One post-lane stage invoked only after all eight lanes complete.
pub trait CompiledPostLaneTransform {
    type Completion;
    type Error;

    fn run_after_lanes(
        &mut self,
        input: &CompiledEightLaneExecutionInputs,
        lanes: &CompiledEightLaneExecutionSuccess,
    ) -> Result<Self::Completion, Self::Error>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledLaneAndPostSuccess<PostCompletion> {
    pub lanes: CompiledEightLaneExecutionSuccess,
    pub post_lane: PostCompletion,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompiledLaneAndPostFailure<ProviderError, ConfigError, PostError> {
    Lanes(CompiledEightLaneExecutionFailure<ProviderError, ConfigError>),
    PostLane {
        lanes: Box<CompiledEightLaneExecutionSuccess>,
        post_error: PostError,
    },
}

pub type CompiledLaneAndPostResult<ProviderError, ConfigError, PostCompletion, PostError> = Result<
    CompiledLaneAndPostSuccess<PostCompletion>,
    CompiledLaneAndPostFailure<ProviderError, ConfigError, PostError>,
>;

/// Runs the recovered eight-lane body followed by its post-lane barrier.
///
/// A lane/config failure never invokes `post`. A post failure retains the
/// complete lane success while any progressive post-stage publication remains
/// represented by the post adapter's typed error.
pub fn execute_compiled_eight_lanes_then<P, C, T>(
    input: CompiledEightLaneExecutionInputs,
    state: CompiledEightLaneExecutionState,
    providers: &mut [P; 8],
    config: &mut C,
    post: &mut T,
) -> CompiledLaneAndPostResult<P::Error, C::Error, T::Completion, T::Error>
where
    P: CompiledLaneGeometryProvider,
    C: CompiledLaneConfigProvider,
    T: CompiledPostLaneTransform,
{
    let lanes = execute_compiled_eight_lanes(input, state, providers, config)
        .map_err(CompiledLaneAndPostFailure::Lanes)?;
    let post_lane = match post.run_after_lanes(&input, &lanes) {
        Ok(completion) => completion,
        Err(post_error) => {
            return Err(CompiledLaneAndPostFailure::PostLane {
                lanes: Box::new(lanes),
                post_error,
            });
        }
    };
    Ok(CompiledLaneAndPostSuccess { lanes, post_lane })
}

pub trait CompiledLaneConfigProvider {
    type Error;

    /// Reloads the native global config view late in this lane's iteration.
    fn read_flag_26(&mut self, lane_index: usize) -> Result<u8, Self::Error>;
}

impl<Error, Reader> CompiledLaneConfigProvider for Reader
where
    Reader: FnMut(usize) -> Result<u8, Error>,
{
    type Error = Error;

    fn read_flag_26(&mut self, lane_index: usize) -> Result<u8, Self::Error> {
        self(lane_index)
    }
}

fn apply_compiled_lane_handle_writes(handle: &mut i16, writes: CompiledLaneHandleWriteTrace) {
    if writes.normalized_to_minus_one {
        *handle = -1;
    }
    if let Some(selected) = writes.searched_handle {
        *handle = selected;
    }
    if writes.invalidated_after_search_miss {
        *handle = -1;
    }
}

fn apply_compiled_lane_postprocess_effects(
    state: &mut CompiledEightLaneExecutionState,
    lane_index: usize,
    effects: CompiledSingleLanePostprocessEffects,
) {
    state.lane_words[lane_index] = effects.lane_word;
    if let Some(output_vector) = effects.output_vector_write {
        state.output_vectors[lane_index] = output_vector;
    }
    if let Some(type_word) = effects.type_word_write {
        state.type_words[lane_index] = type_word;
    }
    if let Some(accumulator) = effects.accumulator_write {
        state.accumulator_words[lane_index] = accumulator;
    }
    if let Some(latch) = effects.latch_67c {
        state.latch_67c = latch;
    }
}

/// Executes the recovered `FUN_00419a40` dispatcher/postprocess loop in its
/// native ascending, lane-interleaved order.
///
/// Each lane receives its own provider snapshot because the shipped loop can
/// reload table globals between iterations. The config provider is invoked
/// only after the lane-word/type/accumulator/subtraction writes, allowing its
/// error result to preserve that exact same-lane prefix. Dispatch failures
/// preserve the wrapper's handle trace and all earlier completed lanes; any
/// provider-internal partial publication remains represented by its typed
/// nested error. Stable disjoint state is assumed. Native pointer aliases,
/// within-lane table rebinding, and machine-fault store prefixes remain out of
/// scope for this safe contract.
pub fn execute_compiled_eight_lanes<P, C>(
    input: CompiledEightLaneExecutionInputs,
    mut state: CompiledEightLaneExecutionState,
    providers: &mut [P; 8],
    config: &mut C,
) -> Result<CompiledEightLaneExecutionSuccess, CompiledEightLaneExecutionFailure<P::Error, C::Error>>
where
    P: CompiledLaneGeometryProvider,
    C: CompiledLaneConfigProvider,
{
    let mut lanes = [None; 8];
    execute_compiled_lane_range(input, &mut state, &mut lanes, providers, config, 8)?;
    Ok(CompiledEightLaneExecutionSuccess {
        state,
        lanes: lanes.map(|lane| lane.expect("all eight lanes completed")),
    })
}

/// Height offset of the four derived hull lanes in the runtime query
/// (`FUN_00419c20`, `0x419d24..`): lane `4 + i` is lane `i` minus 5.0.
pub const RUNTIME_HULL_LANE_OFFSET: i32 = 0x5_0000;

/// The runtime four-lane query `FUN_00419c20`: the same per-lane dispatch and
/// postprocess as [`execute_compiled_eight_lanes`] for lanes 0..3 (each with
/// its first-four subtraction), after which lanes 4..7 take only their lane
/// word, `lane_word[i] - 0x50000`; their handles, type words, vectors and
/// accumulators are left as they were.
///
/// # Errors
///
/// The same dispatch and config failures as the eight-lane executor, for lanes
/// 0..3 only.
pub fn execute_runtime_four_lanes<P, C>(
    input: CompiledEightLaneExecutionInputs,
    mut state: CompiledEightLaneExecutionState,
    providers: &mut [P; 8],
    config: &mut C,
) -> Result<CompiledEightLaneExecutionState, CompiledEightLaneExecutionFailure<P::Error, C::Error>>
where
    P: CompiledLaneGeometryProvider,
    C: CompiledLaneConfigProvider,
{
    let mut lanes = [None; 8];
    execute_compiled_lane_range(input, &mut state, &mut lanes, providers, config, 4)?;
    for lane in 0..4 {
        state.lane_words[lane + 4] = state.lane_words[lane].wrapping_sub(RUNTIME_HULL_LANE_OFFSET);
    }
    Ok(state)
}

fn execute_compiled_lane_range<P, C>(
    input: CompiledEightLaneExecutionInputs,
    state: &mut CompiledEightLaneExecutionState,
    lanes: &mut [Option<CompiledLaneExecutionEffects>; 8],
    providers: &mut [P; 8],
    config: &mut C,
    lane_count: usize,
) -> Result<(), CompiledEightLaneExecutionFailure<P::Error, C::Error>>
where
    P: CompiledLaneGeometryProvider,
    C: CompiledLaneConfigProvider,
{
    for lane_index in 0..lane_count {
        let dispatch = match dispatch_compiled_lane(
            CompiledLaneDispatchInput {
                source_vector: input.source_vectors[lane_index],
                initial_handle: state.handles[lane_index],
                fallback: state.lane_words[lane_index],
            },
            &mut providers[lane_index],
        ) {
            Ok(dispatch) => dispatch,
            Err(failure) => {
                apply_compiled_lane_handle_writes(
                    &mut state.handles[lane_index],
                    failure.handle_writes,
                );
                return Err(CompiledEightLaneExecutionFailure::Dispatch {
                    failed_lane: lane_index,
                    progress: Box::new(CompiledEightLaneExecutionProgress {
                        completed_lanes: lane_index,
                        state: *state,
                        lanes: *lanes,
                    }),
                    failure,
                });
            }
        };

        apply_compiled_lane_handle_writes(&mut state.handles[lane_index], dispatch.handle_writes);
        if let Some(type_word) = dispatch.type_word_write {
            state.type_words[lane_index] = type_word;
        }
        if let Some(output_vector) = dispatch.output_vector_write {
            state.output_vectors[lane_index] = output_vector;
        }

        let postprocess_input = CompiledSingleLanePostprocessInput {
            post_dispatch: CompiledLanePostDispatchSnapshot {
                lane_word: dispatch.lane_word,
                type_word: state.type_words[lane_index],
            },
            gate_word: input.gate_words[lane_index],
            drive_word: input.drive_words[lane_index],
            accumulator_word: state.accumulator_words[lane_index],
            first_four_subtraction: (lane_index < 4)
                .then_some(input.first_four_subtractions[lane_index.min(3)]),
            config_flag_26: 0,
        };
        let mut postprocess = derive_compiled_single_lane_before_config(postprocess_input);
        apply_compiled_lane_postprocess_effects(state, lane_index, postprocess);

        let config_flag_26 = match config.read_flag_26(lane_index) {
            Ok(flag) => flag,
            Err(config_error) => {
                return Err(CompiledEightLaneExecutionFailure::Config {
                    failed_lane: lane_index,
                    progress: Box::new(CompiledEightLaneExecutionProgress {
                        completed_lanes: lane_index,
                        state: *state,
                        lanes: *lanes,
                    }),
                    dispatch,
                    pre_config_postprocess: postprocess,
                    config_error,
                });
            }
        };
        apply_compiled_single_lane_config(&mut postprocess, config_flag_26);
        apply_compiled_lane_postprocess_effects(state, lane_index, postprocess);
        lanes[lane_index] = Some(CompiledLaneExecutionEffects {
            dispatch,
            postprocess,
        });
    }

    Ok(())
}

/// Decoded candidate access required by the inner search at `FUN_00447540`.
///
/// The three metric words are the second component of each decoded vertex.
/// `None` preserves the caller's checked decoder-zero branch even though the
/// observed official-EU decoder returns one after its unchecked reads.
pub trait CompiledLaneCandidateProvider {
    type Error;

    fn validate_candidate(
        &mut self,
        source_vector: [i32; 3],
        handle: i16,
    ) -> Result<bool, Self::Error>;
    fn decode_metric_words(&mut self, handle: i16) -> Result<Option<[i32; 3]>, Self::Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompiledLaneCandidateFailurePhase {
    Validate,
    Decode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompiledLaneCandidateFailure<E> {
    CandidateOutOfBounds {
        ordinal: usize,
        requested_count: i16,
        available: usize,
    },
    Provider {
        phase: CompiledLaneCandidateFailurePhase,
        ordinal: usize,
        handle: i16,
        provider_error: E,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompiledLaneCandidateChoice {
    Miss,
    /// The sole caller-visible handle write, performed only after a complete
    /// successful scan.
    Found {
        ordinal: usize,
        handle: i16,
        score: i32,
    },
}

fn compiled_lane_candidate_metric(words: [i32; 3], fallback: i32) -> i32 {
    let average = words.into_iter().fold(0_i32, i32::wrapping_add) / 3;
    fallback.wrapping_sub(average).wrapping_abs()
}

/// Stable decoded-list contract for `FUN_00447540`.
///
/// Candidates are visited in list order. Validation precedes decoding, and
/// rejected candidates do not decode. The first accepted candidate becomes
/// the provisional winner regardless of its signed metric; later candidates
/// replace it only for a strictly smaller signed metric, so ties retain the
/// first handle. The three-word sum and fallback difference wrap as x86
/// dword arithmetic, signed division truncates toward zero, and the native
/// pseudo-absolute-value operation leaves `i32::MIN` negative.
///
/// This safe aggregate receives the decoded signed count and candidate list.
/// A non-positive count is an immediate miss. Positive counts read each list
/// word lazily, so an earlier provider error precedes a later bounds error.
/// Raw hierarchy decoding remains the outer search contract. Errors expose no
/// choice/handle write because the shipped routine publishes its winner only
/// after the whole list has been scanned. The immutable slice also excludes
/// concurrent or aliased mutation between observing and finally reloading the
/// winning list word.
pub fn search_decoded_compiled_lane_candidates<P: CompiledLaneCandidateProvider>(
    source_vector: [i32; 3],
    fallback: i32,
    signed_count: i16,
    candidates: &[i16],
    provider: &mut P,
) -> Result<CompiledLaneCandidateChoice, CompiledLaneCandidateFailure<P::Error>> {
    if signed_count <= 0 {
        return Ok(CompiledLaneCandidateChoice::Miss);
    }

    let mut winner: Option<(usize, i16)> = None;
    let mut winning_metric = 0;

    for ordinal in 0..signed_count as usize {
        let Some(&handle) = candidates.get(ordinal) else {
            return Err(CompiledLaneCandidateFailure::CandidateOutOfBounds {
                ordinal,
                requested_count: signed_count,
                available: candidates.len(),
            });
        };
        let valid =
            provider
                .validate_candidate(source_vector, handle)
                .map_err(|provider_error| CompiledLaneCandidateFailure::Provider {
                    phase: CompiledLaneCandidateFailurePhase::Validate,
                    ordinal,
                    handle,
                    provider_error,
                })?;
        if !valid {
            continue;
        }
        let Some(words) = provider
            .decode_metric_words(handle)
            .map_err(|provider_error| CompiledLaneCandidateFailure::Provider {
                phase: CompiledLaneCandidateFailurePhase::Decode,
                ordinal,
                handle,
                provider_error,
            })?
        else {
            continue;
        };
        let metric = compiled_lane_candidate_metric(words, fallback);
        if winner.is_none() || metric < winning_metric {
            winner = Some((ordinal, handle));
            winning_metric = metric;
        }
    }

    Ok(match winner {
        Some((ordinal, handle)) => CompiledLaneCandidateChoice::Found {
            ordinal,
            handle,
            score: winning_metric,
        },
        None => CompiledLaneCandidateChoice::Miss,
    })
}

pub const COMPILED_LANE_SPATIAL_LEVEL_COUNT: usize = 6;

/// One decoded eight-byte cell from the `FUN_004473f0` hierarchy.
///
/// The two bytes between the signed marker and dword link/offset are retained
/// by the format layer even though the routine does not read them.
pub type DecodedCompiledLaneSpatialCell = cmr_formats::hpc::HpcSpatialCell;

#[derive(Clone, Copy, Debug)]
pub struct DecodedCompiledLaneSpatialHierarchy<'a> {
    pub origin_xz: [i32; 2],
    pub width: i16,
    pub height: i16,
    pub levels: [&'a [DecodedCompiledLaneSpatialCell]; COMPILED_LANE_SPATIAL_LEVEL_COUNT],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompiledLaneSpatialLookup {
    Miss,
    CandidateRegion {
        level: u8,
        cell_index: i32,
        signed_count: i16,
        candidate_offset_words: i32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledLaneSpatialBoundsError {
    pub level: u8,
    pub cell_index: i32,
    pub available_cells: usize,
}

/// Stable decoded-hierarchy contract for outer search `FUN_004473f0`.
///
/// Source X/Z subtract the hierarchy origin with dword wrapping, then their
/// signed top bytes select a row-major level-zero cell. Out-of-grid positions
/// miss before any cell is read. An empty marker (`-1`) descends through at
/// most five 4x4 refinements, consuming successive two-bit X/Z pairs from bits
/// 23..14 and adding them to the wrapping child link. Any other signed marker,
/// including another negative value, terminates with that marker as the signed
/// candidate count and the dword field as a u16-element offset. An empty level
/// five cell misses without consuming its link.
///
/// This contract stops at the candidate region so the inner chooser retains
/// its own lazy list-read/error ordering. Decoded cells are immutable and
/// field-complete; native pointer aliases, concurrent mutation, and faults
/// between the marker and link/offset reads are outside this safe aggregate.
pub fn locate_decoded_compiled_lane_candidate_region(
    source_vector: [i32; 3],
    hierarchy: DecodedCompiledLaneSpatialHierarchy<'_>,
) -> Result<CompiledLaneSpatialLookup, CompiledLaneSpatialBoundsError> {
    let delta_x = source_vector[0].wrapping_sub(hierarchy.origin_xz[0]);
    let delta_z = source_vector[2].wrapping_sub(hierarchy.origin_xz[1]);
    let cell_x = (delta_x >> 24) as i16;
    let cell_z = (delta_z >> 24) as i16;
    if cell_x < 0 || cell_x >= hierarchy.width || cell_z < 0 || cell_z >= hierarchy.height {
        return Ok(CompiledLaneSpatialLookup::Miss);
    }

    let mut level = 0_usize;
    let mut cell_index = i32::from(hierarchy.width)
        .wrapping_mul(i32::from(cell_z))
        .wrapping_add(i32::from(cell_x));
    let mut mask = 0x00ff_ffff_i32;
    let mut shift = 24_u32;

    loop {
        let cell = usize::try_from(cell_index)
            .ok()
            .and_then(|index| hierarchy.levels[level].get(index))
            .copied()
            .ok_or(CompiledLaneSpatialBoundsError {
                level: level as u8,
                cell_index,
                available_cells: hierarchy.levels[level].len(),
            })?;
        if cell.marker != -1 {
            return Ok(CompiledLaneSpatialLookup::CandidateRegion {
                level: level as u8,
                cell_index,
                signed_count: cell.marker,
                candidate_offset_words: cell.link_or_candidate_offset,
            });
        }
        if level + 1 == COMPILED_LANE_SPATIAL_LEVEL_COUNT {
            return Ok(CompiledLaneSpatialLookup::Miss);
        }

        let masked_x = delta_x & mask;
        let masked_z = delta_z & mask;
        mask >>= 2;
        shift -= 2;
        let subcell_x = (masked_x >> shift) as i16;
        let subcell_z = (masked_z >> shift) as i16;
        cell_index = cell
            .link_or_candidate_offset
            .wrapping_add(i32::from(subcell_x))
            .wrapping_add(i32::from(subcell_z).wrapping_mul(4));
        level += 1;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompiledLaneSpatialSearchFailure<E> {
    Hierarchy(CompiledLaneSpatialBoundsError),
    CandidateStartOutOfBounds {
        candidate_offset_words: i32,
        available_words: usize,
    },
    Candidate(CompiledLaneCandidateFailure<E>),
}

/// Safe composition of the decoded outer hierarchy and inner candidate
/// chooser recovered from `FUN_004473f0`/`FUN_00447540`.
///
/// Non-positive leaf counts reach the inner chooser's immediate miss without
/// validating or touching the candidate offset. Positive counts validate only
/// the starting word offset; the remaining count is deliberately not
/// prevalidated so per-ordinal provider calls and later list bounds retain the
/// native lazy order. A one-past-end start is a valid empty tail whose first
/// requested candidate fails through the chooser at ordinal zero.
pub fn search_decoded_compiled_lane_spatial_hierarchy<P: CompiledLaneCandidateProvider>(
    source_vector: [i32; 3],
    fallback: i32,
    hierarchy: DecodedCompiledLaneSpatialHierarchy<'_>,
    candidate_pool: &[i16],
    provider: &mut P,
) -> Result<CompiledLaneCandidateChoice, CompiledLaneSpatialSearchFailure<P::Error>> {
    let region = locate_decoded_compiled_lane_candidate_region(source_vector, hierarchy)
        .map_err(CompiledLaneSpatialSearchFailure::Hierarchy)?;
    let CompiledLaneSpatialLookup::CandidateRegion {
        signed_count,
        candidate_offset_words,
        ..
    } = region
    else {
        return Ok(CompiledLaneCandidateChoice::Miss);
    };

    if signed_count <= 0 {
        return search_decoded_compiled_lane_candidates(
            source_vector,
            fallback,
            signed_count,
            &[],
            provider,
        )
        .map_err(CompiledLaneSpatialSearchFailure::Candidate);
    }

    let candidate_start = usize::try_from(candidate_offset_words)
        .ok()
        .filter(|&start| start <= candidate_pool.len())
        .ok_or(
            CompiledLaneSpatialSearchFailure::CandidateStartOutOfBounds {
                candidate_offset_words,
                available_words: candidate_pool.len(),
            },
        )?;
    search_decoded_compiled_lane_candidates(
        source_vector,
        fallback,
        signed_count,
        &candidate_pool[candidate_start..],
        provider,
    )
    .map_err(CompiledLaneSpatialSearchFailure::Candidate)
}

fn compiled_lane_xz_edge_score(point: [i32; 3], start: [i32; 3], end: [i32; 3]) -> i32 {
    common_prefix_q16_mul(
        point[0].wrapping_sub(start[0]),
        end[2].wrapping_sub(start[2]),
    )
    .wrapping_add(common_prefix_q16_mul(
        point[2].wrapping_sub(start[2]),
        start[0].wrapping_sub(end[0]),
    ))
}

fn compiled_lane_xz_edge_scores(source_vector: [i32; 3], vertices: [[i32; 3]; 3]) -> [i32; 3] {
    [
        compiled_lane_xz_edge_score(source_vector, vertices[0], vertices[1]),
        compiled_lane_xz_edge_score(source_vector, vertices[1], vertices[2]),
        compiled_lane_xz_edge_score(source_vector, vertices[2], vertices[0]),
    ]
}

/// Stable decoded-geometry predicate for `FUN_00446d00` after its decoder
/// succeeds.
///
/// Signed inclusive X/Z axis bounds run before the three winding-sensitive
/// edge tests. Each edge performs its two signed Q16 products independently,
/// then wrapping-adds their projected dwords. Source and vertex Y components
/// are ignored; zero edge scores are accepted. This intentionally preserves
/// degenerate and orientation-sensitive native behavior rather than replacing
/// it with a modern unoriented point-in-triangle test.
#[must_use]
pub fn validate_decoded_compiled_lane_source(
    source_vector: [i32; 3],
    vertices: [[i32; 3]; 3],
) -> bool {
    let source_x = source_vector[0];
    if vertices.iter().all(|vertex| source_x < vertex[0])
        || vertices.iter().all(|vertex| source_x > vertex[0])
    {
        return false;
    }
    let source_z = source_vector[2];
    if vertices.iter().all(|vertex| source_z < vertex[2])
        || vertices.iter().all(|vertex| source_z > vertex[2])
    {
        return false;
    }
    compiled_lane_xz_edge_scores(source_vector, vertices)
        .into_iter()
        .all(|score| score >= 0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodedCompiledLaneTriangle {
    pub vertices: [[i32; 3]; 3],
    pub type_word: u16,
}

pub type DecodedCompiledLaneRecord = cmr_formats::hpc::HpcRecord;

#[derive(Clone, Copy, Debug)]
pub struct DecodedCompiledLaneTables<'a> {
    pub records: &'a [DecodedCompiledLaneRecord],
    pub vertices: &'a [[i32; 3]],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompiledLaneDecodeError {
    HandleOutOfBounds {
        handle: i16,
        available_records: usize,
    },
    VertexOutOfBounds {
        handle: i16,
        slot: u8,
        vertex_index: u16,
        available_vertices: usize,
    },
}

/// Safe stable-table decoder for the record/vertex reads in `FUN_00446e80`.
///
/// Native records are eight bytes (three u16 vertex indices plus a type u16
/// that this helper does not read), and vertices are three signed dwords at
/// stride twelve. The shipped helper performs unchecked progressive reads and returns one; this reconstruction
/// instead reports negative/out-of-range handles and the first invalid vertex
/// slot without converting malformed lookup into geometric rejection.
pub fn decode_compiled_lane_vertices(
    handle: i16,
    tables: DecodedCompiledLaneTables<'_>,
) -> Result<[[i32; 3]; 3], CompiledLaneDecodeError> {
    let vertex_indices = usize::try_from(handle)
        .ok()
        .and_then(|index| tables.records.get(index))
        .map(|record| record.vertex_indices)
        .ok_or(CompiledLaneDecodeError::HandleOutOfBounds {
            handle,
            available_records: tables.records.len(),
        })?;
    let mut vertices = [[0; 3]; 3];
    for (slot, vertex_index) in vertex_indices.into_iter().enumerate() {
        vertices[slot] = tables
            .vertices
            .get(usize::from(vertex_index))
            .copied()
            .ok_or(CompiledLaneDecodeError::VertexOutOfBounds {
                handle,
                slot: slot as u8,
                vertex_index,
                available_vertices: tables.vertices.len(),
            })?;
    }
    Ok(vertices)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledLaneBuildEffects {
    pub lane_word: i32,
    pub type_word: u16,
    pub output_vector: [i32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledLaneBuildDivideOverflow {
    pub numerator: i32,
    pub divisor: i32,
    pub published_type_word: u16,
    pub published_output_vector: [i32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompiledLaneBuildStore {
    TypeWord,
    RawOutputX,
    RawOutputY,
    RawOutputZ,
    FinalOutputX,
    FinalOutputY,
    FinalOutputZ,
}

pub const COMPILED_LANE_BUILD_STORE_ORDER: [CompiledLaneBuildStore; 7] = [
    CompiledLaneBuildStore::TypeWord,
    CompiledLaneBuildStore::RawOutputX,
    CompiledLaneBuildStore::RawOutputY,
    CompiledLaneBuildStore::RawOutputZ,
    CompiledLaneBuildStore::FinalOutputX,
    CompiledLaneBuildStore::FinalOutputY,
    CompiledLaneBuildStore::FinalOutputZ,
];

pub(crate) fn normalize_compiled_lane_vector(vector: [i32; 3]) -> [i32; 3] {
    let square_sum = vector.into_iter().fold(0_i32, |sum, component| {
        sum.wrapping_add(common_prefix_q16_mul(component, component))
    });
    let root = sqrt_q16(square_sum as u32);
    if root == 0 {
        return [0; 3];
    }
    let reciprocal = ((1_u64 << 32) / u64::from(root)) as i32;
    vector.map(|component| common_prefix_q16_mul(component, reciprocal))
}

/// Which four post-lane points `FUN_00408fe0` selects.
///
/// These neutral names reflect selector fields A/B/C. Their physical meaning
/// is not established by static analysis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompiledPostLaneQuad {
    A,
    B,
    C,
}

impl CompiledPostLaneQuad {
    #[must_use]
    pub const fn lane_indices(self) -> [usize; 4] {
        match self {
            Self::A => [0, 1, 4, 5],
            Self::B => [0, 1, 2, 3],
            Self::C => [0, 2, 4, 6],
        }
    }
}

/// Stable-snapshot inputs read by the post-eight-lane global transform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledPostLaneNormalInput {
    /// Eight vectors at state `+0xfc..+0x158`; component 1 is not read.
    pub source_vectors: [[i32; 3]; 8],
    /// The post-`FUN_00419a40` words at state `+0x2e4..+0x300`.
    pub lane_words: [i32; 8],
    /// State words `+0x25c/+0x268/+0x274` used only for quad selection.
    pub selector_words: [i32; 3],
}

/// Stable aggregate of `FUN_00408fe0`'s two publications and ignored return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledPostLaneNormalEffects {
    pub selected_quad: CompiledPostLaneQuad,
    /// Normalized first face, published progressively at `+0x324..+0x32c`.
    pub first_published_normal: [i32; 3],
    /// Normalized second face, retained locally by the shipped function.
    pub second_normal: [i32; 3],
    /// Normalized wrapping sum, which overwrites the first publication.
    pub final_published_normal: [i32; 3],
    /// Wrapping Q16 square sum of the unnormalized final sum, returned in EAX
    /// but ignored by every resolved caller.
    pub final_square_sum: u32,
}

fn compiled_post_lane_pseudo_abs(value: i32) -> i32 {
    if value < 0 {
        value.wrapping_neg()
    } else {
        value
    }
}

fn select_compiled_post_lane_quad(selector_words: [i32; 3]) -> CompiledPostLaneQuad {
    let [a, b, c] = selector_words.map(compiled_post_lane_pseudo_abs);
    if a <= b && b >= c {
        CompiledPostLaneQuad::B
    } else if a > b && a >= c {
        CompiledPostLaneQuad::A
    } else {
        CompiledPostLaneQuad::C
    }
}

pub(crate) fn compiled_post_lane_cross_v_by_u(v: [i32; 3], u: [i32; 3]) -> [i32; 3] {
    [
        common_prefix_q16_mul(v[1], u[2]).wrapping_sub(common_prefix_q16_mul(v[2], u[1])),
        common_prefix_q16_mul(v[2], u[0]).wrapping_sub(common_prefix_q16_mul(v[0], u[2])),
        common_prefix_q16_mul(v[0], u[1]).wrapping_sub(common_prefix_q16_mul(v[1], u[0])),
    ]
}

fn compiled_post_lane_upward_normal(u: [i32; 3], v: [i32; 3]) -> [i32; 3] {
    let mut normal = compiled_post_lane_cross_v_by_u(v, u);
    if normal[1] < 0 {
        normal = normal.map(i32::wrapping_neg);
    }
    normalize_compiled_lane_vector(normal)
}

fn compiled_post_lane_square_sum(vector: [i32; 3]) -> u32 {
    vector.into_iter().fold(0_i32, |sum, component| {
        sum.wrapping_add(common_prefix_q16_mul(component, component))
    }) as u32
}

/// Reconstructs no-argument `FUN_00408fe0` over one stable state snapshot.
///
/// The native function can reload `DAT_0050c5b4` between source capture, its
/// intermediate publication, and its final read/write. This pure contract
/// intentionally collapses that rebinding, progressive machine faults, and
/// concurrent mutation while preserving the exact wrapping arithmetic,
/// selection/tie order, first publication, and final overwrite.
#[must_use]
pub fn derive_compiled_post_lane_global_normal(
    input: CompiledPostLaneNormalInput,
) -> CompiledPostLaneNormalEffects {
    let selected_quad = select_compiled_post_lane_quad(input.selector_words);
    let points = selected_quad.lane_indices().map(|lane| {
        [
            input.source_vectors[lane][0],
            input.lane_words[lane],
            input.source_vectors[lane][2],
        ]
    });
    let [o, p, q, r] = points;
    let first_published_normal = compiled_post_lane_upward_normal(
        std::array::from_fn(|index| p[index].wrapping_sub(o[index])),
        std::array::from_fn(|index| q[index].wrapping_sub(o[index])),
    );
    let second_normal = compiled_post_lane_upward_normal(
        std::array::from_fn(|index| p[index].wrapping_sub(r[index])),
        std::array::from_fn(|index| q[index].wrapping_sub(r[index])),
    );
    let sum = std::array::from_fn(|index| {
        first_published_normal[index].wrapping_add(second_normal[index])
    });
    let final_square_sum = compiled_post_lane_square_sum(sum);
    let final_published_normal = normalize_compiled_lane_vector(sum);
    CompiledPostLaneNormalEffects {
        selected_quad,
        first_published_normal,
        second_normal,
        final_published_normal,
        final_square_sum,
    }
}

/// Concrete post-lane adapter for `FUN_00408fe0`'s stable-snapshot contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledPostLaneGlobalNormal {
    pub selector_words: [i32; 3],
}

impl CompiledPostLaneTransform for CompiledPostLaneGlobalNormal {
    type Completion = CompiledPostLaneNormalEffects;
    type Error = std::convert::Infallible;

    fn run_after_lanes(
        &mut self,
        input: &CompiledEightLaneExecutionInputs,
        lanes: &CompiledEightLaneExecutionSuccess,
    ) -> Result<Self::Completion, Self::Error> {
        Ok(derive_compiled_post_lane_global_normal(
            CompiledPostLaneNormalInput {
                source_vectors: input.source_vectors,
                lane_words: lanes.state.lane_words,
                selector_words: self.selector_words,
            },
        ))
    }
}

/// The post-normal basis rebuild's state vector bases: the prior row `+0x258`,
/// the verbatim AK11 row `+0x264`, the rebuilt cross row `+0x270`, and the AK11
/// final publication `+0x324` read as an input.
pub const COMPILED_POST_LANE_VECTOR_BASES: [u16; 4] = [0x258, 0x264, 0x270, 0x324];

/// Native destination-row store order in `[0x406b00,0x406e30)`: the verbatim
/// AK11 row is stored first, then the rebuilt prior row, then the cross row.
pub const COMPILED_POST_LANE_STORE_BASE_ORDER: [u16; 3] = [0x264, 0x258, 0x270];

/// Stable-snapshot inputs read by the post-normal basis rebuild.
///
/// The old `+0x264`/`+0x270` rows are write-only destinations: their prior
/// contents (including AK11's old selector word at `+0x268`, which the verbatim
/// store supersedes) cannot influence the result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledPostLaneBasisInputs {
    /// Final AK11 vector at state `+0x324/+0x328/+0x32c`.
    pub vector_324: [i32; 3],
    /// Prior vector at state `+0x258/+0x25c/+0x260`.
    pub vector_258: [i32; 3],
}

/// The three destination rows produced by the basis rebuild.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledPostLaneBasisEffects {
    /// Rebuilt prior row `+0x258/+0x25c/+0x260`.
    pub vector_258: [i32; 3],
    /// The AK11 final vector copied verbatim, not renormalized.
    pub vector_264: [i32; 3],
    /// Rebuilt cross row `+0x270/+0x274/+0x278`.
    pub vector_270: [i32; 3],
}

/// Reconstructs the call-free post-normal basis rebuild
/// `FUN_00404440 [0x406b00,0x406e30)` over one stable state snapshot.
///
/// With `n` the AK11 final vector and `s` the prior vector, the exact wrapping
/// Q16 arithmetic is `dot = sum(qmul(n_i, s_i))`, `rejected = s - n*dot`,
/// `u = legacy_normalize(rejected)`, and `v = legacy_normalize(u x n)` in that
/// exact operand order. `legacy_normalize` is the generated-table `sqrt_q16`
/// rule shared with AK11: a zero square sum yields zero, otherwise the
/// reciprocal is `floor(2^32/root)` followed by Q16 component multiplies.
///
/// Native code stores non-atomically and the following interval
/// `[0x406e30,0x406e9a)` publishes these rows through a reloaded pointer; that
/// publication is a separate contract. Aliases, rebinding, and progressive
/// machine faults remain outside this pure aggregate.
#[must_use]
pub fn derive_compiled_post_lane_basis(
    input: CompiledPostLaneBasisInputs,
) -> CompiledPostLaneBasisEffects {
    let n = input.vector_324;
    let s = input.vector_258;
    let dot = (0..3).fold(0_i32, |sum, index| {
        sum.wrapping_add(common_prefix_q16_mul(n[index], s[index]))
    });
    let rejected =
        std::array::from_fn(|index| s[index].wrapping_sub(common_prefix_q16_mul(n[index], dot)));
    let u = normalize_compiled_lane_vector(rejected);
    let v = normalize_compiled_lane_vector(compiled_post_lane_cross_v_by_u(u, n));
    CompiledPostLaneBasisEffects {
        vector_258: u,
        vector_264: n,
        vector_270: v,
    }
}

/// Concrete AK11 -> AK13 post-lane composition: the basis rebuild consumes the
/// AK11 final publication only after the eight-lane barrier has succeeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledPostLaneBasis {
    pub normal: CompiledPostLaneGlobalNormal,
    /// Prior vector at state `+0x258/+0x25c/+0x260`.
    pub prior_vector_258: [i32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledPostLaneBasisCompletion {
    pub normal: CompiledPostLaneNormalEffects,
    pub basis: CompiledPostLaneBasisEffects,
}

impl CompiledPostLaneTransform for CompiledPostLaneBasis {
    type Completion = CompiledPostLaneBasisCompletion;
    type Error = std::convert::Infallible;

    fn run_after_lanes(
        &mut self,
        input: &CompiledEightLaneExecutionInputs,
        lanes: &CompiledEightLaneExecutionSuccess,
    ) -> Result<Self::Completion, Self::Error> {
        let normal = self.normal.run_after_lanes(input, lanes)?;
        Ok(CompiledPostLaneBasisCompletion {
            basis: derive_compiled_post_lane_basis(CompiledPostLaneBasisInputs {
                vector_324: normal.final_published_normal,
                vector_258: self.prior_vector_258,
            }),
            normal,
        })
    }
}

/// Destination-row stride in bytes of the post-basis pointer publication.
pub const COMPILED_POST_LANE_PUBLICATION_ROW_STRIDE: usize = 0x10;

/// Publication source rows in native write order: the rebuilt prior row, the
/// verbatim AK11 row, then the cross row. This differs from the AK13 state
/// store order, which writes the AK11 row first.
pub const COMPILED_POST_LANE_PUBLICATION_SOURCE_BASES: [u16; 3] = [0x258, 0x264, 0x270];

/// Destination byte offsets of the three published rows.
pub const COMPILED_POST_LANE_PUBLICATION_DESTINATION_OFFSETS: [usize; 3] = [0x00, 0x10, 0x20];

/// Publishes the rebuilt basis through the destination pointer reloaded from
/// state `+0x2a0` in `FUN_00404440 [0x406e30,0x406e9a)`.
///
/// The injected destination stands for native memory: the three rows are
/// written at destination strides `+0x00/+0x10/+0x20`, three dwords each,
/// while the padding dwords at `+0x0c/+0x1c/+0x2c` keep their prior contents.
/// Native code reloads the global state and the destination pointer between
/// rows, so destination aliases, pointer rebinding, and progressive machine
/// faults are explicit limitations, not modeled effects. The following
/// `[0x406e9a,0x406ea5)` interval invokes the already reconstructed
/// `FUN_00409c10(state)`; `0x406ea5` starts the separate state-adjustment
/// phase.
pub fn publish_compiled_post_lane_basis(
    destination: &mut [[i32; 4]; 3],
    effects: CompiledPostLaneBasisEffects,
) {
    let rows = [effects.vector_258, effects.vector_264, effects.vector_270];
    for (destination_row, source_row) in destination.iter_mut().zip(rows) {
        destination_row[..3].copy_from_slice(&source_row);
    }
}

/// Concrete AK11 -> AK13 -> publication composition. The destination buffer is
/// injected so pointer ownership stays outside the safe contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledPostLanePublishedBasis {
    pub basis: CompiledPostLaneBasis,
    /// Caller-owned destination rows; the padding dwords are preserved.
    pub destination: [[i32; 4]; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledPostLanePublicationCompletion {
    pub normal: CompiledPostLaneNormalEffects,
    pub basis: CompiledPostLaneBasisEffects,
    /// The published destination rows after the three-dword writes.
    pub destination: [[i32; 4]; 3],
}

impl CompiledPostLaneTransform for CompiledPostLanePublishedBasis {
    type Completion = CompiledPostLanePublicationCompletion;
    type Error = std::convert::Infallible;

    fn run_after_lanes(
        &mut self,
        input: &CompiledEightLaneExecutionInputs,
        lanes: &CompiledEightLaneExecutionSuccess,
    ) -> Result<Self::Completion, Self::Error> {
        let completion = self.basis.run_after_lanes(input, lanes)?;
        let mut destination = self.destination;
        publish_compiled_post_lane_basis(&mut destination, completion.basis);
        Ok(CompiledPostLanePublicationCompletion {
            normal: completion.normal,
            basis: completion.basis,
            destination,
        })
    }
}

/// Destination byte offsets of the published state-adjustment vector.
pub const COMPILED_STATE_ADJUSTMENT_PUBLICATION_OFFSETS: [usize; 3] = [0x30, 0x34, 0x38];

/// Source state bases in publication order.
pub const COMPILED_STATE_ADJUSTMENT_SOURCE_BASES: [u16; 3] = [0xc8, 0xcc, 0xd0];

/// Call targets in native order after the publication:
/// `FUN_00492770(state+8, state+0xc8)`, then `FUN_00409c10(state)`, then
/// `FUN_004126b0()`. The first and last callees are not yet reconstructed;
/// `FUN_00409c10` already has its own stable-snapshot contract.
pub const COMPILED_STATE_ADJUSTMENT_CALL_TARGETS: [u32; 3] = [0x492770, 0x409c10, 0x4126b0];

/// Inputs for the wrapping accumulation that opens the state-adjustment phase
/// at `0x406ea5`, after the second `FUN_00409c10(state)` call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledStateAdjustmentInputs {
    /// Prior value of state `+0xcc`.
    pub prior_cc: i32,
    /// State `+0x2e4`, the minuend of the accumulated difference.
    pub word_2e4: i32,
    /// State `+0x100`, the subtrahend of the accumulated difference.
    pub word_100: i32,
}

/// Reconstructs `state+0xcc += state+0x2e4 - state+0x100` with exact wrapping
/// subtraction and addition.
#[must_use]
pub fn derive_compiled_state_adjustment_cc(input: CompiledStateAdjustmentInputs) -> i32 {
    input
        .prior_cc
        .wrapping_add(input.word_2e4.wrapping_sub(input.word_100))
}

/// Publishes the adjusted vector through the destination pointer reloaded from
/// state `+0x2a0`: `+0x30` receives `+0xc8`, `+0x34` the updated `+0xcc`, and
/// `+0x38` `+0xd0`. The following dword is not written.
///
/// The injected destination stands for native memory. Native code reloads the
/// global state and destination pointer, so aliases, rebinding, and
/// progressive faults remain explicit limitations.
pub fn publish_compiled_state_adjustment(
    destination: &mut [i32; 4],
    word_c8: i32,
    word_cc: i32,
    word_d0: i32,
) {
    destination[..3].copy_from_slice(&[word_c8, word_cc, word_d0]);
}

/// State dwords zeroed after the three adjustment-phase calls: six contiguous
/// dwords at `+0x3cc..+0x3e0`.
pub const COMPILED_ADJUSTMENT_RESET_STATE_ZERO_OFFSETS: [u16; 6] =
    [0x3cc, 0x3d0, 0x3d4, 0x3d8, 0x3dc, 0x3e0];

/// Remaining state writes in native order: `+0xbc = 0`, `+0xc0 = 0x10000`,
/// `+0xc4 = 0`.
pub const COMPILED_ADJUSTMENT_RESET_STATE_WORDS: [(u16, i32); 3] =
    [(0xbc, 0), (0xc0, 0x10000), (0xc4, 0)];

/// Control dwords cleared by the descending loop after the state reset.
///
/// The native loop starts at `0x3cc` and, per iteration, writes zero at
/// `offset-0x10` and `offset`, then subtracts four and continues while the new
/// offset is above `0x3bf`. Iterations therefore run at `0x3cc`, `0x3c8`,
/// `0x3c4`, and `0x3c0`, clearing exactly these eight dwords at
/// `0x3b0..0x3cc` in the listed order.
pub const COMPILED_ADJUSTMENT_CONTROL_CLEAR_OFFSETS: [u16; 8] =
    [0x3bc, 0x3cc, 0x3b8, 0x3c8, 0x3b4, 0x3c4, 0x3b0, 0x3c0];

/// Effects of the post-call state reset and the control clear loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledAdjustmentResetEffects {
    pub state_zero_offsets: [u16; 6],
    pub state_words: [(u16, i32); 3],
    pub control_zero_offsets: [u16; 8],
}

/// Reconstructs the constant reset block following the adjustment-phase calls.
#[must_use]
pub const fn derive_compiled_adjustment_reset() -> CompiledAdjustmentResetEffects {
    CompiledAdjustmentResetEffects {
        state_zero_offsets: COMPILED_ADJUSTMENT_RESET_STATE_ZERO_OFFSETS,
        state_words: COMPILED_ADJUSTMENT_RESET_STATE_WORDS,
        control_zero_offsets: COMPILED_ADJUSTMENT_CONTROL_CLEAR_OFFSETS,
    }
}

/// Control dword offset of the config-selected flag: `2` when the config byte
/// at `DAT_00d3ca80 + 0xb6 + state_738 * 0x9c` has bit `0x10` clear, else `1`.
pub const COMPILED_INITIALIZER_CONTROL_FLAG_OFFSET: u16 = 0x3f0;

/// Reconstructs the config-selected control flag. Only bit `0x10` of the
/// injected config byte participates.
#[must_use]
pub const fn derive_compiled_initializer_control_flag(config_b6: u8) -> i32 {
    if config_b6 & 0x10 == 0 { 2 } else { 1 }
}

/// Constant effects of the flag/reset block following the adjustment reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledInitializerFlagEffects {
    /// Control dwords zeroed in native order.
    pub control_zero_dwords: [u16; 4],
    /// Control bytes zeroed in native order.
    pub control_zero_bytes: [u16; 3],
    /// Remaining control dword writes in native order.
    pub control_constant_dwords: [(u16, i32); 2],
    /// State dwords zeroed in native order.
    pub state_zero_dwords: [u16; 4],
    /// State u16 words zeroed in native order.
    pub state_zero_u16_offsets: [u16; 2],
    /// Three-dword vector copy: source `+0x258` to destination `+0x5d4`.
    pub state_vector_copy: (u16, u16),
}

/// Reconstructs the constant flag/reset writes after the adjustment reset.
#[must_use]
pub const fn derive_compiled_initializer_flag_effects() -> CompiledInitializerFlagEffects {
    CompiledInitializerFlagEffects {
        control_zero_dwords: [0x3f4, 0x3f8, 0x3fc, 0x3ec],
        control_zero_bytes: [0x43c, 0x43d, 0x43e],
        control_constant_dwords: [(0x37c, -0x4ccc), (0x41c, 0)],
        state_zero_dwords: [0x684, 0x610, 0x678, 0x6e4],
        state_zero_u16_offsets: [0x60c, 0x614],
        state_vector_copy: (0x258, 0x5d4),
    }
}

/// State `+0x6f8`: `1` when `FUN_0046b240()` returns `8` or `FUN_0048d2d0()`
/// returns nonzero, else `0`. The second callee is only evaluated when the
/// first result differs from `8`; `state+0x6f0` is then set to `1`.
#[must_use]
pub fn derive_compiled_initializer_6f8(first: i32, second: impl FnOnce() -> i32) -> i32 {
    i32::from(first == 8 || second() != 0)
}

/// Inputs for the `+0x6e8/+0x6ec` words after the `+0x6f0 = 1` store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledInitializer6e8Inputs {
    /// Global `DAT_00550c08`.
    pub global_550c08: i32,
    /// Global `DAT_00d3ca88`.
    pub global_d3ca88: i32,
    /// The `FUN_004cfac0()` result, drawn only when the zero branch is not
    /// taken. It is ignored in the zero branch.
    pub random: u32,
}

/// The `+0x6e8` dword and the `+0x6ec` u16 word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledInitializer6e8Effects {
    pub word_6e8: i32,
    pub word_6ec: i16,
}

/// Reconstructs the `+0x6e8/+0x6ec` words.
///
/// When `DAT_00550c08 == 2` or `DAT_00d3ca88 != 0`, both are zero. Otherwise
/// `+0x6e8 = 1` and `+0x6ec` is `(signed_remainder(random, 2)) * 10 + 0xd`
/// truncated to 16 bits. The native `random & 0x80000001` followed by the
/// negative correction is the C sign-following remainder: for a negative odd
/// value the correction is `(value - 1 | 0xfffffffe) + 1`, all wrapping.
#[must_use]
pub fn derive_compiled_initializer_6e8(
    input: CompiledInitializer6e8Inputs,
) -> CompiledInitializer6e8Effects {
    if input.global_550c08 == 2 || input.global_d3ca88 != 0 {
        return CompiledInitializer6e8Effects {
            word_6e8: 0,
            word_6ec: 0,
        };
    }
    let raw = (input.random & 0x8000_0001) as i32;
    let remainder = if raw < 0 {
        (raw.wrapping_sub(1) | -2).wrapping_add(1)
    } else {
        raw
    };
    CompiledInitializer6e8Effects {
        word_6e8: 1,
        word_6ec: (remainder * 10 + 0xd) as i16,
    }
}

/// The first copy loop: 24 dwords from state `+0xfc` to `+0x1bc`.
pub const COMPILED_INITIALIZER_COPY_FC: (u16, u16, usize) = (0xfc, 0x1bc, 24);

/// The second copy loop: 9 dwords from state `+0x258` to `+0x21c`.
pub const COMPILED_INITIALIZER_COPY_258: (u16, u16, usize) = (0x258, 0x21c, 9);

/// Source state bases copied to `+0x240/+0x244/+0x248` after the second loop.
pub const COMPILED_INITIALIZER_COPY_240_SOURCES: [u16; 3] = [0xc8, 0xcc, 0xd0];

/// Source state base copied to `+0x24c/+0x250/+0x254`: the AK11 final vector.
pub const COMPILED_INITIALIZER_COPY_24C_SOURCE: u16 = 0x324;

/// State bytes zeroed after the copies: `+0x74d` and `+0x74e`.
pub const COMPILED_INITIALIZER_FINAL_ZERO_STATE_BYTES: [u16; 2] = [0x74d, 0x74e];

/// Control byte zeroed after the copies.
pub const COMPILED_INITIALIZER_FINAL_ZERO_CONTROL_BYTE: u16 = 0x428;

/// Control dword set to `1` after the copies.
pub const COMPILED_INITIALIZER_FINAL_CONTROL_ONE_DWORD: u16 = 0x420;

/// Inputs for the two copy loops and the three vector copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledInitializerCopyInputs {
    /// Twenty-four dwords at state `+0xfc..+0x15c`.
    pub source_fc: [i32; 24],
    /// Nine dwords at state `+0x258..+0x27c`.
    pub source_258: [i32; 9],
    /// State `+0xc8/+0xcc/+0xd0` words.
    pub source_c8: i32,
    pub source_cc: i32,
    pub source_d0: i32,
    /// The AK11 final vector at `+0x324/+0x328/+0x32c`.
    pub source_324: [i32; 3],
}

/// Destination blocks produced by the copy loops and vector copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledInitializerCopyEffects {
    /// `+0x1bc..+0x21c`: the 24-dword copy of `+0xfc..+0x15c`.
    pub block_1bc: [i32; 24],
    /// `+0x21c..+0x240`: the 9-dword copy of `+0x258..+0x27c`.
    pub block_21c: [i32; 9],
    /// `+0x240/+0x244/+0x248`: `+0xc8/+0xcc/+0xd0`.
    pub block_240: [i32; 3],
    /// `+0x24c/+0x250/+0x254`: the AK11 final vector.
    pub block_24c: [i32; 3],
}

/// Reconstructs the two copy loops and the three vector copies verbatim. The
/// `+0x240` write follows the second copy loop in native order.
#[must_use]
pub fn derive_compiled_initializer_copies(
    input: CompiledInitializerCopyInputs,
) -> CompiledInitializerCopyEffects {
    CompiledInitializerCopyEffects {
        block_1bc: input.source_fc,
        block_21c: input.source_258,
        block_240: [input.source_c8, input.source_cc, input.source_d0],
        block_24c: input.source_324,
    }
}

/// Reconstructs `DAT_0050c5fc = ((~config_25 & 1) << 0xf) | 0x1999`.
/// Only bit zero of the injected config byte participates, inverted.
#[must_use]
pub const fn derive_compiled_initializer_5fc(config_25: u8) -> u16 {
    (((!config_25 & 1) as u16) << 0xf) | 0x1999
}

/// The closing conditional call edge: `FUN_0046eda0()` runs only when
/// `DAT_00d3ca88 != 0`.
#[must_use]
pub const fn compiled_initializer_final_call_required(global_d3ca88: i32) -> bool {
    global_d3ca88 != 0
}

/// Reconstructs `FUN_0048d2d0` over the three injected global values.
///
/// The result is false when the detail level (`DAT_00592f00`, getter
/// `FUN_0046b240`) is `8`. Otherwise, when the weather type (`DAT_00592ef8`,
/// getter `FUN_0046b220`) is `3` or `4`, the result is `DAT_00592efc != 1`
/// (getter `FUN_0046b230`); when the weather type is `5`, the result is true.
/// Every other input yields false. The weather getter is only evaluated when
/// the detail level differs from `8`, and the intensity getter only when the
/// weather type is `3` or `4`.
#[must_use]
pub const fn derive_compiled_initializer_48d2d0(detail: i32, weather: i32, intensity: i32) -> bool {
    if detail == 8 {
        return false;
    }
    if weather > 2 {
        if weather < 5 {
            return intensity != 1;
        }
        if weather == 5 {
            return true;
        }
    }
    false
}

/// The callee `FUN_00492770` always invokes after its stores.
pub const COMPILED_92770_NEXT_CALL: u32 = 0x492050;

/// Effects of `FUN_00492770(param_1, param_2)` over a stable snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Compiled92770Effects {
    /// Three-dword writes at `param_1+0x10/+0x14/+0x18` (cases `1` and `2`).
    pub low_vector: Option<[i32; 3]>,
    /// Three-dword writes at `param_1+0xc4/+0xc8/+0xcc` (cases `0..=3`).
    pub high_vector: Option<[i32; 3]>,
    /// `param_1+0x188` is zeroed on every path.
    pub zero_188: bool,
}

/// Reconstructs the switch-driven vector stores of `FUN_00492770`.
///
/// The switch reads `param_1+0x180`. Cases `0` and `3` store the three source
/// dwords only at `+0xc4/+0xc8/+0xcc`; cases `1` and `2` additionally store
/// them at `+0x10/+0x14/+0x18`. Every path zeroes `+0x188` and then invokes
/// [`COMPILED_92770_NEXT_CALL`] (`FUN_00492050`), which is not yet
/// reconstructed.
#[must_use]
pub fn derive_compiled_92770(switch_180: i32, source: [i32; 3]) -> Compiled92770Effects {
    let (low_vector, high_vector) = match switch_180 {
        1 | 2 => (Some(source), Some(source)),
        0 | 3 => (None, Some(source)),
        _ => (None, None),
    };
    Compiled92770Effects {
        low_vector,
        high_vector,
        zero_188: true,
    }
}

/// Inputs for `FUN_004126b0`: the per-lane threshold scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledLaneThresholdInputs {
    /// Lane words in lane order: lane `i` is read from `+0x2e4 + 4*i`.
    pub lane_words: [i32; 8],
    /// Source Y words in lane order: lane `i` is read from `+0x100 + 0x0c*i`.
    pub source_y: [i32; 8],
}

/// Effects of the `FUN_004126b0` threshold scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledLaneThresholdEffects {
    /// Per-lane flags at `+0x73a+i`: `1` when `lane_word + 0x1999 < source_y`.
    pub flags_73a: [u8; 8],
    /// `+0x743`: initialized to `1` and cleared on the first failing lane.
    pub flag_743: u8,
    /// `+0x742`: initialized to `0` and incremented per failing lane.
    pub count_742: u8,
    /// `+0x744..`: failing lane indices in native descending processing order.
    pub indices_744: [u8; 8],
}

/// Reconstructs the `FUN_004126b0` threshold scan in native descending lane
/// order (lanes `7` down to `0`).
///
/// Each comparison is the signed test `lane_word + 0x1999 < source_y` with
/// wrapping addition. A passing lane sets its `+0x73a` flag; a failing lane
/// clears it, clears `+0x743`, appends the lane index at `+0x744 + count`, and
/// increments `+0x742`.
#[must_use]
pub fn derive_compiled_lane_thresholds(
    input: CompiledLaneThresholdInputs,
) -> CompiledLaneThresholdEffects {
    let mut effects = CompiledLaneThresholdEffects {
        flags_73a: [0; 8],
        flag_743: 1,
        count_742: 0,
        indices_744: [0; 8],
    };
    for lane in (0..8).rev() {
        if input.lane_words[lane].wrapping_add(0x1999) < input.source_y[lane] {
            effects.flags_73a[lane] = 1;
        } else {
            effects.flag_743 = 0;
            effects.indices_744[usize::from(effects.count_742)] = lane as u8;
            effects.count_742 += 1;
        }
    }
    effects
}

/// The `FUN_0046eda0` copy gate: the global copies run only when
/// `DAT_00550c08 == 1`.
#[must_use]
pub const fn compiled_special_copy_required(global_550c08: i32) -> bool {
    global_550c08 == 1
}

/// Global copy blocks performed by `FUN_0046eda0` when its gate passes:
/// `(source, destination, dword_count)`. Each destination begins immediately
/// after its source range, so the blocks never overlap.
pub const COMPILED_SPECIAL_COPY_BLOCKS: [(u32, u32, usize); 2] =
    [(0x5fffc8, 0x60042c, 0x119), (0x700300, 0x700a68, 0x1da)];

/// `FUN_00492050` follows the node link at `+0x8` and sets the flag at
/// `+0x170` to `1` on every node until the null link.
pub const COMPILED_92050_LINK_OFFSET: u16 = 0x8;
pub const COMPILED_92050_FLAG_OFFSET: u16 = 0x170;

/// Reconstructs `FUN_00492050`: every node in the chain gets its `+0x170`
/// flag set to `1`, stopping at the null link. The injected flag slice models
/// the chain (empty means a null initial pointer, so no writes), avoiding raw
/// pointer chasing in the safe contract.
pub fn apply_compiled_92050(flags: &mut [i32]) {
    flags.fill(1);
}

/// `state+0x6f0` is unconditionally set to `1` after the `+0x6f8` flag.
pub const COMPILED_INITIALIZER_6F0_VALUE: i32 = 1;

/// Inputs for the composed post-lane spine: the eight-lane executor followed
/// by AK11, AK13, AK14, the adjustment, and every later direct/callee effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledSpineInputs {
    pub execution: CompiledEightLaneExecutionInputs,
    /// AK11 selector words at `+0x25c/+0x268/+0x274`.
    pub normal_selector_words: [i32; 3],
    /// Prior vector at `+0x258/+0x25c/+0x260` before AK13.
    pub prior_vector_258: [i32; 3],
    /// Injected destination for the AK14 publication.
    pub publication_destination: [[i32; 4]; 3],
    /// State `+0xcc` before the adjustment accumulation.
    pub adjustment_prior_cc: i32,
    /// Injected destination for the adjustment publication.
    pub adjustment_destination: [i32; 4],
    /// State `+0xc8/+0xd0` before the adjustment.
    pub state_c8: i32,
    pub state_d0: i32,
    /// Switch word at `param_1+0x180` for `FUN_00492770`.
    pub switch_180: i32,
    /// Global detail level (`DAT_00592f00`).
    pub detail: i32,
    /// Global weather type (`DAT_00592ef8`).
    pub weather: i32,
    /// Global weather intensity (`DAT_00592efc`).
    pub intensity: i32,
    /// Config byte at `DAT_00d3ca80 + 0xb6 + state_738 * 0x9c`.
    pub config_b6: u8,
    /// Global `DAT_00550c08`.
    pub global_550c08: i32,
    /// Global `DAT_00d3ca88`.
    pub global_d3ca88: i32,
    /// The `FUN_004cfac0()` draw for the `+0x6ec` word.
    pub random: u32,
    /// Config byte at `DAT_00d3ca80 + 0x25`.
    pub config_25: u8,
}

/// The composed effects of the whole post-lane spine, in native order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledSpineEffects {
    pub lanes: CompiledEightLaneExecutionSuccess,
    pub normal: CompiledPostLaneNormalEffects,
    pub basis: CompiledPostLaneBasisEffects,
    pub destination: [[i32; 4]; 3],
    pub cc: i32,
    pub adjustment_destination: [i32; 4],
    pub callee_92770: Compiled92770Effects,
    pub lane_thresholds: CompiledLaneThresholdEffects,
    pub reset: CompiledAdjustmentResetEffects,
    pub flags: CompiledInitializerFlagEffects,
    pub control_flag: i32,
    pub flag_6f8: i32,
    pub flag_6f0: i32,
    pub words_6e8: CompiledInitializer6e8Effects,
    pub copies: CompiledInitializerCopyEffects,
    pub word_5fc: u16,
    pub final_call_required: bool,
}

/// Lane inputs whose source vectors are produced by the first basis rebuild.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledSetupPoseLaneInputs {
    pub gate_words: [i32; 8],
    pub drive_words: [i32; 8],
    pub first_four_subtractions: [i32; 4],
}

/// Stable setup-time pose snapshot entering the recovered compiled spine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledSetupPoseInputs {
    /// Destination words retained when `selector` is outside `0..14`.
    pub prior_case_words: CompiledCaseWords,
    pub selector: u32,
    pub center: [i32; 3],
    /// Actor matrix rows after the caller's rotation composition.
    pub matrix_rows: [[i32; 3]; 3],
    /// Fresh read of the common configuration byte at offset `+0x25`.
    pub common_config_25: u8,
    pub lane: CompiledSetupPoseLaneInputs,
}

/// The two corner-vector snapshots observed before adjusted-center publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledSetupPoseSourceMetadata {
    /// Initial actor basis, consumed by the eight-lane geometry pass.
    pub initial: [[i32; 3]; 8],
    /// Contact-normal basis before the road-plane center-Y adjustment.
    pub contact_basis: [[i32; 3]; 8],
}

/// Successful setup-time pose composition plus inspectable lane/source data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledSetupPoseEffects {
    pub selected_case: Option<u8>,
    pub effective_state_words: [i32; COMPILED_STATE_WORD_COUNT],
    pub initial_basis: CompiledStateBasisEffects,
    pub sources: CompiledSetupPoseSourceMetadata,
    pub adjusted_center: [i32; 3],
    pub lanes: CompiledEightLaneExecutionSuccess,
    pub normal: CompiledPostLaneNormalEffects,
    pub corrected_basis: CompiledPostLaneBasisEffects,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompiledSetupPoseError<ProviderError, ConfigError> {
    CommonPrefix(CompiledCommonPrefixReciprocalOverflow),
    Lanes(CompiledEightLaneExecutionFailure<ProviderError, ConfigError>),
}

/// Compose the setup-time actor pose with injected geometry providers.
///
/// The first corner rebuild uses actor matrix row zero and row two as the
/// `FUN_00418120` adjustment vectors. After the lane pass, the recovered
/// contact-normal basis drives a second rebuild used by the road-plane Y
/// correction. The result stops at the adjusted-center publication boundary;
/// later actor updates, corner rebuilds, thresholds, copies, and global flags
/// are deliberately outside this adapter.
///
/// State is accepted by value and no effect bundle is returned on failure.
/// Provider/config side effects remain owned by those injected interfaces.
pub fn execute_compiled_setup_pose<P, C>(
    direct_cases: &CompiledDirectCaseSet,
    input: CompiledSetupPoseInputs,
    state: CompiledEightLaneExecutionState,
    providers: &mut [P; 8],
    config: &mut C,
) -> Result<CompiledSetupPoseEffects, CompiledSetupPoseError<P::Error, C::Error>>
where
    P: CompiledLaneGeometryProvider,
    C: CompiledLaneConfigProvider,
{
    let direct = direct_cases.apply(input.prior_case_words, input.selector);
    let common = derive_compiled_common_prefix(direct.words, input.common_config_25)
        .map_err(CompiledSetupPoseError::CommonPrefix)?;
    let effective_state_words = common.state_words.unwrap_or(direct.words.state_words);
    let halves = derive_compiled_half_sign_state([
        effective_state_words[0],
        effective_state_words[1],
        effective_state_words[2],
    ]);
    let basis_input = CompiledStateBasisInputs {
        half_words: [halves.words[0], halves.words[1], halves.words[2]],
        matrix_rows: input.matrix_rows,
        center: input.center,
        adjustment_u: input.matrix_rows[0],
        adjustment_v: input.matrix_rows[2],
        scale_638: effective_state_words[3],
        scale_63c: effective_state_words[4],
        scale_640: effective_state_words[5],
    };
    let initial_basis = derive_compiled_state_basis_effects(basis_input);
    let execution = CompiledEightLaneExecutionInputs {
        source_vectors: initial_basis.vectors,
        gate_words: input.lane.gate_words,
        drive_words: input.lane.drive_words,
        first_four_subtractions: input.lane.first_four_subtractions,
    };
    let lanes = execute_compiled_eight_lanes(execution, state, providers, config)
        .map_err(CompiledSetupPoseError::Lanes)?;

    let normal_selector_words = input.matrix_rows.map(|row| row[1]);
    let normal = derive_compiled_post_lane_global_normal(CompiledPostLaneNormalInput {
        source_vectors: initial_basis.vectors,
        lane_words: lanes.state.lane_words,
        selector_words: normal_selector_words,
    });
    let basis = derive_compiled_post_lane_basis(CompiledPostLaneBasisInputs {
        vector_324: normal.final_published_normal,
        vector_258: input.matrix_rows[0],
    });
    let corrected_rows = [basis.vector_258, basis.vector_264, basis.vector_270];
    let corrected_basis_input = CompiledStateBasisInputs {
        matrix_rows: corrected_rows,
        adjustment_u: corrected_rows[0],
        adjustment_v: corrected_rows[2],
        ..basis_input
    };
    let contact_sources = derive_compiled_state_basis_effects(corrected_basis_input).vectors;
    let cc = derive_compiled_state_adjustment_cc(CompiledStateAdjustmentInputs {
        prior_cc: input.center[1],
        word_2e4: lanes.state.lane_words[0],
        word_100: contact_sources[0][1],
    });
    let adjusted_center = [input.center[0], cc, input.center[2]];

    Ok(CompiledSetupPoseEffects {
        selected_case: direct.selected_case,
        effective_state_words,
        initial_basis,
        sources: CompiledSetupPoseSourceMetadata {
            initial: initial_basis.vectors,
            contact_basis: contact_sources,
        },
        adjusted_center,
        lanes,
        normal,
        corrected_basis: basis,
    })
}

/// Ordinary authenticated-stage entry point for [`execute_compiled_setup_pose`].
///
/// All eight lane providers borrow the same validated `.hpc` snapshot. The
/// config reader remains injected because the native loop reloads that view on
/// every lane and may fail independently of geometry.
pub fn execute_stage_compiled_setup_pose<C>(
    direct_cases: &CompiledDirectCaseSet,
    input: CompiledSetupPoseInputs,
    state: CompiledEightLaneExecutionState,
    geometry: &crate::stage_compiled_geometry::StageCompiledGeometry,
    config: &mut C,
) -> Result<
    CompiledSetupPoseEffects,
    CompiledSetupPoseError<DecodedCompiledLaneGeometryError, C::Error>,
>
where
    C: CompiledLaneConfigProvider,
{
    let mut providers = geometry.eight_lane_providers();
    execute_compiled_setup_pose(direct_cases, input, state, &mut providers, config)
}

/// Composes the recovered post-lane spine in native order.
///
/// The eight-lane executor runs first; its failure aborts the spine before any
/// later effect. AK11 consumes the original source vectors and final lane
/// words, AK13 consumes its final publication, and the adjustment uses lane
/// zero's word (`+0x2e4`) and source Y (`+0x100`). The `+0x6f8` flag is
/// evaluated before `+0x6f0 = 1` and the `+0x6e8/+0x6ec` words; the copies use
/// the AK13 rows and the AK11 final vector. Callee calls are represented by
/// their pure contracts, not by executing native code; progressive faults,
/// pointer rebinding, and the early initializer stages remain separate.
pub fn execute_compiled_spine_post<P, C>(
    input: CompiledSpineInputs,
    state: CompiledEightLaneExecutionState,
    providers: &mut [P; 8],
    config: &mut C,
) -> Result<CompiledSpineEffects, CompiledEightLaneExecutionFailure<P::Error, C::Error>>
where
    P: CompiledLaneGeometryProvider,
    C: CompiledLaneConfigProvider,
{
    let lanes = execute_compiled_eight_lanes(input.execution, state, providers, config)?;
    let normal = derive_compiled_post_lane_global_normal(CompiledPostLaneNormalInput {
        source_vectors: input.execution.source_vectors,
        lane_words: lanes.state.lane_words,
        selector_words: input.normal_selector_words,
    });
    let basis = derive_compiled_post_lane_basis(CompiledPostLaneBasisInputs {
        vector_324: normal.final_published_normal,
        vector_258: input.prior_vector_258,
    });
    let mut destination = input.publication_destination;
    publish_compiled_post_lane_basis(&mut destination, basis);

    let cc = derive_compiled_state_adjustment_cc(CompiledStateAdjustmentInputs {
        prior_cc: input.adjustment_prior_cc,
        word_2e4: lanes.state.lane_words[0],
        word_100: input.execution.source_vectors[0][1],
    });
    let mut adjustment_destination = input.adjustment_destination;
    publish_compiled_state_adjustment(
        &mut adjustment_destination,
        input.state_c8,
        cc,
        input.state_d0,
    );
    let callee_92770 =
        derive_compiled_92770(input.switch_180, [input.state_c8, cc, input.state_d0]);
    let lane_thresholds = derive_compiled_lane_thresholds(CompiledLaneThresholdInputs {
        lane_words: lanes.state.lane_words,
        source_y: input.execution.source_vectors.map(|vector| vector[1]),
    });
    let reset = derive_compiled_adjustment_reset();
    let flags = derive_compiled_initializer_flag_effects();
    let control_flag = derive_compiled_initializer_control_flag(input.config_b6);
    let flag_6f8 = derive_compiled_initializer_6f8(input.detail, || {
        i32::from(derive_compiled_initializer_48d2d0(
            input.detail,
            input.weather,
            input.intensity,
        ))
    });
    let words_6e8 = derive_compiled_initializer_6e8(CompiledInitializer6e8Inputs {
        global_550c08: input.global_550c08,
        global_d3ca88: input.global_d3ca88,
        random: input.random,
    });
    let copies = derive_compiled_initializer_copies(CompiledInitializerCopyInputs {
        source_fc: std::array::from_fn(|index| {
            input.execution.source_vectors[index / 3][index % 3]
        }),
        source_258: std::array::from_fn(|index| {
            [basis.vector_258, basis.vector_264, basis.vector_270][index / 3][index % 3]
        }),
        source_c8: input.state_c8,
        source_cc: cc,
        source_d0: input.state_d0,
        source_324: normal.final_published_normal,
    });
    let word_5fc = derive_compiled_initializer_5fc(input.config_25);
    let final_call_required = compiled_initializer_final_call_required(input.global_d3ca88);

    Ok(CompiledSpineEffects {
        lanes,
        normal,
        basis,
        destination,
        cc,
        adjustment_destination,
        callee_92770,
        lane_thresholds,
        reset,
        flags,
        control_flag,
        flag_6f8,
        flag_6f0: COMPILED_INITIALIZER_6F0_VALUE,
        words_6e8,
        copies,
        word_5fc,
        final_call_required,
    })
}

/// Inputs for the complete recovered initializer composition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledInitializerInputs<'a> {
    /// Early post-dispatch stages (case words, config, and tail effects).
    pub early: RecoveredCompiledStageInputs<'a>,
    /// Inputs for the `FUN_00409c10`/`FUN_00418120` basis transform.
    pub basis: CompiledStateBasisInputs,
    /// Inputs for the eight-lane and post-lane spine.
    pub spine: CompiledSpineInputs,
}

/// The complete recovered initializer effects, in native stage order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledInitializerEffects {
    pub early: RecoveredCompiledStageEffects,
    pub basis: CompiledStateBasisEffects,
    pub locals: CompiledCallSpineLocalEffects,
    pub spine: CompiledSpineEffects,
}

/// Failures of the complete initializer composition, in stage order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompiledInitializerError<ProviderError, ConfigError> {
    /// An early post-dispatch stage failed before any lane ran.
    Early(RecoveredCompiledStageError),
    /// The eight-lane executor failed, aborting before the post-lane spine.
    Lanes(CompiledEightLaneExecutionFailure<ProviderError, ConfigError>),
}

/// Composes the whole recovered initializer: early post-dispatch stages, the
/// basis transform, the call-spine locals, then the eight-lane/post-lane
/// spine.
///
/// An early failure returns before any lane runs; a lane failure aborts before
/// the post-lane spine. All stages derive their effects through the recovered
/// pure contracts with injected inputs, so progressive faults, pointer
/// rebinding, and the exact native dataflow into those inputs remain separate
/// boundaries rather than invented behavior.
pub fn compose_compiled_initializer<P, C>(
    direct_cases: &CompiledDirectCaseSet,
    aux_constants: &AuxControlConstantSet,
    input: CompiledInitializerInputs<'_>,
    state: CompiledEightLaneExecutionState,
    providers: &mut [P; 8],
    config: &mut C,
) -> Result<CompiledInitializerEffects, CompiledInitializerError<P::Error, C::Error>>
where
    P: CompiledLaneGeometryProvider,
    C: CompiledLaneConfigProvider,
{
    let early = compose_recovered_compiled_stage_effects(direct_cases, aux_constants, input.early)
        .map_err(CompiledInitializerError::Early)?;
    let basis = derive_compiled_state_basis_effects(input.basis);
    let locals = derive_compiled_call_spine_local_effects();
    let spine = execute_compiled_spine_post(input.spine, state, providers, config)
        .map_err(CompiledInitializerError::Lanes)?;
    Ok(CompiledInitializerEffects {
        early,
        basis,
        locals,
        spine,
    })
}

fn solve_compiled_lane_plane(
    source_vector: [i32; 3],
    fallback: i32,
    v0: [i32; 3],
    type_word: u16,
    output_vector: [i32; 3],
) -> Result<i32, CompiledLaneBuildDivideOverflow> {
    let divisor = output_vector[1];
    if divisor == 0 {
        return Ok(fallback);
    }
    let plane_dot = (0..3).fold(0_i32, |sum, index| {
        sum.wrapping_add(common_prefix_q16_mul(output_vector[index], v0[index]))
    });
    let numerator = plane_dot
        .wrapping_sub(common_prefix_q16_mul(source_vector[2], output_vector[2]))
        .wrapping_sub(common_prefix_q16_mul(source_vector[0], output_vector[0]));
    let quotient = (i64::from(numerator) << 16) / i64::from(divisor);
    i32::try_from(quotient).map_err(|_| CompiledLaneBuildDivideOverflow {
        numerator,
        divisor,
        published_type_word: type_word,
        published_output_vector: output_vector,
    })
}

/// Stable/disjoint decoded-input contract for `FUN_00446fa0` arithmetic.
///
/// The caller's checked decoder-zero branch is retained as the provider's
/// defensive `Unavailable` policy, although the observed shipped decoder
/// returns one after its unchecked reads. A final signed
/// division overflow reports the type and normalized vector that native code
/// has already published. Component-level memory-fault prefixes and aliases
/// remain outside this pure aggregate.
pub fn derive_decoded_compiled_lane_build(
    source_vector: [i32; 3],
    fallback: i32,
    triangle: DecodedCompiledLaneTriangle,
) -> Result<CompiledLaneBuildEffects, CompiledLaneBuildDivideOverflow> {
    let [v0, v1, v2] = triangle.vertices;
    let a = normalize_compiled_lane_vector(std::array::from_fn(|index| {
        v1[index].wrapping_sub(v0[index])
    }));
    let b = normalize_compiled_lane_vector(std::array::from_fn(|index| {
        v2[index].wrapping_sub(v0[index])
    }));
    let raw_cross = [
        common_prefix_q16_mul(a[1], b[2]).wrapping_sub(common_prefix_q16_mul(a[2], b[1])),
        common_prefix_q16_mul(a[2], b[0]).wrapping_sub(common_prefix_q16_mul(a[0], b[2])),
        common_prefix_q16_mul(a[0], b[1]).wrapping_sub(common_prefix_q16_mul(a[1], b[0])),
    ];
    let output_vector = normalize_compiled_lane_vector(raw_cross);
    let lane_word = solve_compiled_lane_plane(
        source_vector,
        fallback,
        v0,
        triangle.type_word,
        output_vector,
    )?;
    Ok(CompiledLaneBuildEffects {
        lane_word,
        type_word: triangle.type_word,
        output_vector,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledLaneTypeReadError {
    pub handle: i16,
    pub available_records: usize,
}

/// Read the record type word used by `FUN_00446fa0` after vertex decoding.
///
/// This is deliberately separate from [`decode_compiled_lane_vertices`]. The
/// shipped builder calls the decoder first, then reloads the record-table base,
/// sign-extends the handle again, and reads the u16 at record offset `+6`.
pub fn read_compiled_lane_type_word(
    handle: i16,
    records: &[DecodedCompiledLaneRecord],
) -> Result<u16, CompiledLaneTypeReadError> {
    usize::try_from(handle)
        .ok()
        .and_then(|index| records.get(index))
        .map(|record| record.type_word)
        .ok_or(CompiledLaneTypeReadError {
            handle,
            available_records: records.len(),
        })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodedCompiledLaneGeometryError {
    Decode(CompiledLaneDecodeError),
    Search(CompiledLaneSpatialSearchFailure<CompiledLaneDecodeError>),
    TypeRead(CompiledLaneTypeReadError),
    BuildDivide(CompiledLaneBuildDivideOverflow),
}

/// Concrete stable-table provider for the recovered compiled-lane dispatcher.
///
/// Every validation, candidate metric decode, and final build resolves the
/// handle again. The final type word is read only after the build's vertex
/// decode succeeds. This preserves the native logical phase order while the
/// immutable slices intentionally collapse native global-pointer rebinding,
/// candidate-list mutation, component-level faults, and aliases into one
/// stable snapshot. SuperGhidra/REA observations that suggest a function start
/// remain discovery hints; this provider's behavior is grounded in the
/// separately recorded REA instruction evidence and the pure helpers above.
#[derive(Clone, Copy, Debug)]
pub struct DecodedCompiledLaneGeometryProvider<'a> {
    pub hierarchy: DecodedCompiledLaneSpatialHierarchy<'a>,
    pub candidate_pool: &'a [i16],
    pub tables: DecodedCompiledLaneTables<'a>,
}

/// Adapts one validated decoded stage `.hpc` payload to the stable geometry
/// provider used by the recovered lane dispatcher.
///
/// The shipped pointer array has five actual cell arrays followed by a sixth
/// slot that aliases the vertex pointer. `parse_hpc` rejects every reachable
/// level-four sentinel, so the safe adapter supplies an empty sixth level and
/// cannot reinterpret 12-byte vertices as an incompatible 8-byte cell array.
#[must_use]
pub fn decoded_compiled_lane_provider_from_hpc(
    hpc: &HpcFile,
) -> DecodedCompiledLaneGeometryProvider<'_> {
    let levels = hpc.levels();
    DecodedCompiledLaneGeometryProvider {
        hierarchy: DecodedCompiledLaneSpatialHierarchy {
            origin_xz: hpc.origin_xz(),
            width: hpc.width(),
            height: hpc.height(),
            levels: [
                &levels[0],
                &levels[1],
                &levels[2],
                &levels[3],
                &levels[4],
                &[],
            ],
        },
        candidate_pool: hpc.candidates(),
        tables: DecodedCompiledLaneTables {
            records: hpc.records(),
            vertices: hpc.vertices(),
        },
    }
}

impl CompiledLaneCandidateProvider for DecodedCompiledLaneGeometryProvider<'_> {
    type Error = CompiledLaneDecodeError;

    fn validate_candidate(
        &mut self,
        source_vector: [i32; 3],
        handle: i16,
    ) -> Result<bool, Self::Error> {
        let vertices = decode_compiled_lane_vertices(handle, self.tables)?;
        Ok(validate_decoded_compiled_lane_source(
            source_vector,
            vertices,
        ))
    }

    fn decode_metric_words(&mut self, handle: i16) -> Result<Option<[i32; 3]>, Self::Error> {
        let vertices = decode_compiled_lane_vertices(handle, self.tables)?;
        Ok(Some([vertices[0][1], vertices[1][1], vertices[2][1]]))
    }
}

impl CompiledLaneGeometryProvider for DecodedCompiledLaneGeometryProvider<'_> {
    type Error = DecodedCompiledLaneGeometryError;

    fn validate(&mut self, source_vector: [i32; 3], handle: i16) -> Result<bool, Self::Error> {
        let vertices = decode_compiled_lane_vertices(handle, self.tables)
            .map_err(DecodedCompiledLaneGeometryError::Decode)?;
        Ok(validate_decoded_compiled_lane_source(
            source_vector,
            vertices,
        ))
    }

    fn search(
        &mut self,
        source_vector: [i32; 3],
        fallback: i32,
    ) -> Result<Option<i16>, Self::Error> {
        let hierarchy = self.hierarchy;
        let candidate_pool = self.candidate_pool;
        let choice = search_decoded_compiled_lane_spatial_hierarchy(
            source_vector,
            fallback,
            hierarchy,
            candidate_pool,
            self,
        )
        .map_err(DecodedCompiledLaneGeometryError::Search)?;
        Ok(match choice {
            CompiledLaneCandidateChoice::Miss => None,
            CompiledLaneCandidateChoice::Found { handle, .. } => Some(handle),
        })
    }

    fn build(
        &mut self,
        source_vector: [i32; 3],
        handle: i16,
        fallback: i32,
    ) -> Result<CompiledLaneBuildCompletion, Self::Error> {
        let vertices = decode_compiled_lane_vertices(handle, self.tables)
            .map_err(DecodedCompiledLaneGeometryError::Decode)?;
        let type_word = read_compiled_lane_type_word(handle, self.tables.records)
            .map_err(DecodedCompiledLaneGeometryError::TypeRead)?;
        let effects = derive_decoded_compiled_lane_build(
            source_vector,
            fallback,
            DecodedCompiledLaneTriangle {
                vertices,
                type_word,
            },
        )
        .map_err(DecodedCompiledLaneGeometryError::BuildDivide)?;
        Ok(CompiledLaneBuildCompletion::Built {
            lane_word: effects.lane_word,
            type_word: effects.type_word,
            output_vector: effects.output_vector,
        })
    }
}

/// Two recovered control writes immediately after the reciprocal/copy families.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledControlProductPair {
    pub reciprocal_word_164: i32,
    pub product_word_168: i32,
}

/// A quarter-product whose signed `2^32 / value` quotient would fault `IDIV`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledControlProductReciprocalOverflow {
    pub divisor: i32,
}

impl fmt::Display for CompiledControlProductReciprocalOverflow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "compiled control +0x164 reciprocal overflows i32 for divisor {}",
            self.divisor
        )
    }
}

impl std::error::Error for CompiledControlProductReciprocalOverflow {}

/// Reconstruct the paired control writes at `+0x164` and `+0x168`.
///
/// The product keeps bits 16..47 of the signed full-width multiplication,
/// matching `IMUL` followed by `SHRD 16`. Its arithmetic quarter is the
/// reciprocal divisor. Zero maps to `-1`; a quotient outside signed 32-bit is
/// reported instead of reproducing the shipped x86 divide fault. The pure
/// result is pair-atomic, although the shipped initializer had already made
/// earlier writes before reaching this interval. Computing the product once is
/// equivalent to the shipped reload/recompute only for stable, non-volatile
/// object memory; concurrent mutation is outside this pure contract.
pub fn derive_compiled_control_product_pair(
    effective_control_word_15c: i32,
    reciprocal_word_208: i32,
) -> Result<CompiledControlProductPair, CompiledControlProductReciprocalOverflow> {
    let product_word_168 =
        ((i64::from(effective_control_word_15c) * i64::from(reciprocal_word_208)) >> 16) as i32;
    let divisor = product_word_168 >> 2;
    let reciprocal_word_164 = if divisor == 0 {
        -1
    } else {
        let quotient = (1_i64 << 32) / i64::from(divisor);
        i32::try_from(quotient).map_err(|_| CompiledControlProductReciprocalOverflow { divisor })?
    };

    Ok(CompiledControlProductPair {
        reciprocal_word_164,
        product_word_168,
    })
}

/// Sole state destination written by `FUN_00486090`.
pub const COMPILED_STATE_WORD_758_OFFSET: u16 = 0x758;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledStateWord758Effect {
    pub state_word_758: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompiledStateWord758SelectionError {
    TruncatedSelectionByte {
        required_len: usize,
        actual_len: usize,
    },
    TableTruncated {
        index: usize,
        table_len: usize,
    },
}

impl fmt::Display for CompiledStateWord758SelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedSelectionByte {
                required_len,
                actual_len,
            } => write!(
                formatter,
                "compiled state +0x758 selection needs {required_len} config bytes, got {actual_len}"
            ),
            Self::TableTruncated { index, table_len } => write!(
                formatter,
                "compiled state +0x758 selector needs table index {index}, but length is {table_len}"
            ),
        }
    }
}

impl std::error::Error for CompiledStateWord758SelectionError {}

/// Reconstruct the caller's requested-value selection before `FUN_00486090`.
///
/// The table remains caller-owned. Context-zero and the gated-default region
/// return the `-1` sentinel without inspecting it. A short table on the table
/// route is a typed safe-policy error; the shipped fixed-address read was
/// unchecked.
pub fn select_compiled_state_word_758_argument(
    context_nonzero: bool,
    selection_byte: u8,
    table: &[i32],
) -> Result<i32, CompiledStateWord758SelectionError> {
    if !context_nonzero || (selection_byte & 0x18) >= 0x10 && (selection_byte & 0xe0) >= 0x80 {
        return Ok(-1);
    }

    let row = usize::from((selection_byte >> 3) & 3);
    let column = usize::from(selection_byte >> 5);
    let index = row * 5 + column;
    table
        .get(index)
        .copied()
        .ok_or(CompiledStateWord758SelectionError::TableTruncated {
            index,
            table_len: table.len(),
        })
}

/// Apply the exact `FUN_00486090` sentinel normalization.
///
/// Only `-1` selects the record-relative config fallback. Every other 32-bit
/// value is written verbatim; the fallback is masked to its low three bits.
#[must_use]
pub const fn derive_compiled_state_word_758(
    requested_value: i32,
    config_fallback: u8,
) -> CompiledStateWord758Effect {
    CompiledStateWord758Effect {
        state_word_758: if requested_value == -1 {
            (config_fallback & 7) as i32
        } else {
            requested_value
        },
    }
}

/// Exact writes performed after the recovered helpers and before the family
/// reciprocal/copy tail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledPostHelperEffects {
    /// Values corresponding to [`COMPILED_POST_HELPER_CONTROL_WORD_OFFSETS`].
    pub control_words: [i32; COMPILED_POST_HELPER_CONTROL_WORD_OFFSETS.len()],
    pub state_words: [i32; COMPILED_POST_HELPER_STATE_WORD_OFFSETS.len()],
    pub control_bytes: [u8; COMPILED_POST_HELPER_CONTROL_BYTE_OFFSETS.len()],
    pub control_u16: [u16; COMPILED_POST_HELPER_CONTROL_U16_OFFSETS.len()],
}

/// A control `+0x15c` value whose signed `2^32 / value` quotient would make
/// the shipped x86 `IDIV` fault before the family tail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledPostHelperReciprocalOverflow {
    pub control_word_15c: i32,
}

impl fmt::Display for CompiledPostHelperReciprocalOverflow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "compiled post-helper reciprocal overflows i32 for control +0x15c value {}",
            self.control_word_15c
        )
    }
}

impl std::error::Error for CompiledPostHelperReciprocalOverflow {}

/// Derive the exact writes in `[0x406324,0x40645f)`.
///
/// The two inputs are the effective values after the recovered common prefix.
/// Zero at `+0x15c` maps to `-1`; another value uses signed `2^32 / value`
/// with truncation toward zero. A quotient overflow returns no patch, an
/// explicit safe policy; the shipped in-place routine had already performed
/// earlier stores before its unguarded `IDIV`.
pub fn derive_compiled_post_helper_effects(
    control_word_148: i32,
    control_word_15c: i32,
) -> Result<CompiledPostHelperEffects, CompiledPostHelperReciprocalOverflow> {
    let reciprocal_160 = if control_word_15c == 0 {
        -1
    } else {
        let quotient = (1_i64 << 32) / i64::from(control_word_15c);
        i32::try_from(quotient)
            .map_err(|_| CompiledPostHelperReciprocalOverflow { control_word_15c })?
    };

    Ok(CompiledPostHelperEffects {
        control_words: [
            0x83,
            control_word_148,
            0x0ccc,
            0x0ccc,
            reciprocal_160,
            0x0ccc,
            0x0ccc,
            0x0ccc,
            0x0ccc,
            0x8000,
            0x3333,
            0x3333,
            0x0_fae1,
            0x8000,
            0,
        ],
        state_words: [0],
        control_bytes: [3],
        control_u16: [0x02aa],
    })
}

/// Select the Q16 center-drive-split word from the low three option bits.
///
/// This is the total value policy recovered from `FUN_00404100`. Resolving the
/// option byte through the shipped signed config-record index is deliberately
/// outside this pure boundary.
#[must_use]
pub const fn compiled_center_split_q16(option_byte: u8) -> i32 {
    match option_byte & 7 {
        0 => 0x4000,
        1 => 0x8000,
        _ => 0xc000,
    }
}

/// Raw option/flag bytes resolved for the recovered common path.
///
/// The existing arithmetic helpers own low-three-bit masking. Keeping the raw
/// bytes here makes this resolver only the checked replacement for the
/// observed configuration reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledConfigOptions {
    pub common_prefix_flags: u8,
    pub unindexed_flags_26: u8,
    pub state_word_758_fallback: u8,
    pub center_split: u8,
    pub control_run: u8,
    pub aux_control: AuxControlOption,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompiledConfigResolveError {
    NegativeIndex {
        index: i8,
    },
    IndexOutOfRange {
        index: i8,
        slot_count: i8,
    },
    TruncatedConfig {
        index: i8,
        required_len: usize,
        actual_len: usize,
    },
}

impl fmt::Display for CompiledConfigResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NegativeIndex { index } => {
                write!(
                    formatter,
                    "compiled configuration index is negative: {index}"
                )
            }
            Self::IndexOutOfRange { index, slot_count } => write!(
                formatter,
                "compiled configuration index {index} is outside {slot_count} constructed slots"
            ),
            Self::TruncatedConfig {
                index,
                required_len,
                actual_len,
            } => write!(
                formatter,
                "compiled configuration index {index} requires {required_len} bytes, got {actual_len}"
            ),
        }
    }
}

impl std::error::Error for CompiledConfigResolveError {}

/// Resolve the option and flag bytes read by the recovered common path.
///
/// The shipped code sign-extends a state byte, multiplies it by `0x9c`, and
/// performs unchecked reads. This safe reconstruction rejects a negative
/// index, applies a fail-closed rejection outside the eight-slot domain of the
/// resolved explicit constructors, and proves the caller-owned slice contains
/// the highest addressed byte before reading anything. The eight slots are not
/// claimed to be homogeneous authored records, and the enclosing image need
/// not end on a stride boundary.
pub fn resolve_compiled_config_options(
    config: &[u8],
    signed_index: i8,
) -> Result<CompiledConfigOptions, CompiledConfigResolveError> {
    let index =
        usize::try_from(signed_index).map_err(|_| CompiledConfigResolveError::NegativeIndex {
            index: signed_index,
        })?;
    if signed_index >= COMPILED_CONFIG_SLOT_COUNT {
        return Err(CompiledConfigResolveError::IndexOutOfRange {
            index: signed_index,
            slot_count: COMPILED_CONFIG_SLOT_COUNT,
        });
    }
    let record_base = index * COMPILED_CONFIG_RECORD_STRIDE;
    let required_len = record_base + COMPILED_CONFIG_CONTROL_RUN_OPTION_OFFSET + 1;
    if config.len() < required_len {
        return Err(CompiledConfigResolveError::TruncatedConfig {
            index: signed_index,
            required_len,
            actual_len: config.len(),
        });
    }

    let aux_control = if config[COMPILED_CONFIG_UNINDEXED_FLAGS_26_OFFSET]
        & COMPILED_CONFIG_AUX_FORCE_DEFAULT_MASK
        != 0
    {
        AuxControlOption::ForcedDefault
    } else {
        AuxControlOption::Resolved(config[record_base + COMPILED_CONFIG_AUX_OPTION_OFFSET])
    };

    Ok(CompiledConfigOptions {
        common_prefix_flags: config[COMPILED_CONFIG_COMMON_FLAGS_OFFSET],
        unindexed_flags_26: config[COMPILED_CONFIG_UNINDEXED_FLAGS_26_OFFSET],
        state_word_758_fallback: config[record_base + COMPILED_CONFIG_STATE_758_OPTION_OFFSET],
        center_split: config[record_base + COMPILED_CONFIG_CENTER_SPLIT_OPTION_OFFSET],
        control_run: config[record_base + COMPILED_CONFIG_CONTROL_RUN_OPTION_OFFSET],
        aux_control,
    })
}

/// Caller-owned literals for one auxiliary-control policy.
///
/// `words` follows [`AUX_CONTROL_POLICY_WORD_OFFSETS`]. The first divisor feeds
/// control `+0x11c`; the reciprocal of the second is Q16-multiplied into
/// control `+0x12c`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuxControlPolicyConstants {
    pub words: [i32; 7],
    pub reciprocal_divisor: i32,
    pub scaled_reciprocal_divisor: i32,
}

/// Caller-owned literal set for the auxiliary 22-word control patch.
///
/// No shipped tuning payload is embedded here. `common_words` follows
/// [`AUX_CONTROL_COMMON_WORD_OFFSETS`]. The Q16 multipliers feed `+0xf4` and
/// `+0x100`, respectively.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuxControlConstants {
    pub common_words: [i32; 9],
    pub q16_multipliers: [i32; 2],
    pub policy_one: AuxControlPolicyConstants,
    pub policy_two: AuxControlPolicyConstants,
    pub default_policy: AuxControlPolicyConstants,
}

/// Owned auxiliary constants copied from a verified user-data store.
///
/// Construction checks exact format/core offset order. The policy order is
/// copied explicitly as one, two, then default; fixed array types prove each
/// record's word count. Codec shape is authenticated by the input store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuxControlConstantSet {
    constants: AuxControlConstants,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuxControlConstantSetError {
    OffsetOrder {
        group: &'static str,
        index: usize,
        core_offset: u16,
        format_offset: u16,
    },
}

impl fmt::Display for AuxControlConstantSetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "auxiliary-control store schema does not match the core contract: {self:?}"
        )
    }
}

impl std::error::Error for AuxControlConstantSetError {}

impl AuxControlConstantSet {
    pub fn from_store(
        store: &AuxControlConstantsStore,
    ) -> Result<Self, AuxControlConstantSetError> {
        Self::from_format(store.constants())
    }

    fn from_format(
        constants: &FormatAuxControlConstantWords,
    ) -> Result<Self, AuxControlConstantSetError> {
        verify_aux_control_format_schema()?;
        Ok(Self {
            constants: AuxControlConstants {
                common_words: constants.common_words,
                q16_multipliers: constants.q16_multipliers,
                policy_one: copy_aux_policy(constants.policy_one),
                policy_two: copy_aux_policy(constants.policy_two),
                default_policy: copy_aux_policy(constants.default_policy),
            },
        })
    }

    #[must_use]
    pub const fn constants(&self) -> &AuxControlConstants {
        &self.constants
    }

    pub fn derive(
        &self,
        option: AuxControlOption,
        state_word_48: i32,
        global_word: i32,
    ) -> Result<AuxControlPatch, AuxControlReciprocalOverflow> {
        derive_aux_control_patch(option, state_word_48, global_word, &self.constants)
    }
}

impl TryFrom<&AuxControlConstantsStore> for AuxControlConstantSet {
    type Error = AuxControlConstantSetError;

    fn try_from(store: &AuxControlConstantsStore) -> Result<Self, Self::Error> {
        Self::from_store(store)
    }
}

const fn copy_aux_policy(policy: FormatAuxControlPolicyWords) -> AuxControlPolicyConstants {
    AuxControlPolicyConstants {
        words: policy.words,
        reciprocal_divisor: policy.reciprocal_divisor,
        scaled_reciprocal_divisor: policy.scaled_reciprocal_divisor,
    }
}

fn verify_aux_control_format_schema() -> Result<(), AuxControlConstantSetError> {
    for (group, core_offsets, format_offsets) in [
        (
            "all outputs",
            AUX_CONTROL_WORD_OFFSETS.as_slice(),
            FORMAT_AUX_WORD_OFFSETS.as_slice(),
        ),
        (
            "common words",
            AUX_CONTROL_COMMON_WORD_OFFSETS.as_slice(),
            FORMAT_AUX_COMMON_WORD_OFFSETS.as_slice(),
        ),
        (
            "policy words",
            AUX_CONTROL_POLICY_WORD_OFFSETS.as_slice(),
            FORMAT_AUX_POLICY_WORD_OFFSETS.as_slice(),
        ),
    ] {
        for (index, (&core_offset, &format_offset)) in
            core_offsets.iter().zip(format_offsets).enumerate()
        {
            if core_offset != format_offset {
                return Err(AuxControlConstantSetError::OffsetOrder {
                    group,
                    index,
                    core_offset,
                    format_offset,
                });
            }
        }
    }

    Ok(())
}

/// Option source for the auxiliary policy selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuxControlOption {
    /// The controlling flag bypasses the option lookup and selects the default.
    ForcedDefault,
    /// A safely resolved option byte; only its low three bits are observed.
    Resolved(u8),
}

/// The complete auxiliary control patch in [`AUX_CONTROL_WORD_OFFSETS`] order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuxControlPatch {
    pub words: [i32; AUX_CONTROL_WORD_COUNT],
}

/// Reciprocal input whose signed quotient does not fit the x86 destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuxControlReciprocalInput {
    CommonWord,
    PolicyDivisor,
    ScaledPolicyDivisor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuxControlReciprocalOverflow {
    pub input: AuxControlReciprocalInput,
    pub divisor: i32,
}

impl fmt::Display for AuxControlReciprocalOverflow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "auxiliary control reciprocal {:?} overflows i32 for divisor {}",
            self.input, self.divisor
        )
    }
}

impl std::error::Error for AuxControlReciprocalOverflow {}

/// Derive the standalone auxiliary 22-word control patch.
///
/// Policy one is selected by masked option `1`, policy two by `2`, and options
/// `0,3..=7` select the default. [`AuxControlOption::ForcedDefault`] represents
/// the observed bypass without requiring an option lookup. The result is
/// assembled only after every signed reciprocal has been validated.
pub fn derive_aux_control_patch(
    option: AuxControlOption,
    state_word_48: i32,
    global_word: i32,
    constants: &AuxControlConstants,
) -> Result<AuxControlPatch, AuxControlReciprocalOverflow> {
    let policy = match option {
        AuxControlOption::Resolved(option_byte) => match option_byte & 7 {
            1 => &constants.policy_one,
            2 => &constants.policy_two,
            _ => &constants.default_policy,
        },
        AuxControlOption::ForcedDefault => &constants.default_policy,
    };

    let common_reciprocal = aux_control_reciprocal(
        constants.common_words[8],
        AuxControlReciprocalInput::CommonWord,
    )?;
    let policy_reciprocal = aux_control_reciprocal(
        policy.reciprocal_divisor,
        AuxControlReciprocalInput::PolicyDivisor,
    )?;
    let scaled_policy_reciprocal = aux_control_reciprocal(
        policy.scaled_reciprocal_divisor,
        AuxControlReciprocalInput::ScaledPolicyDivisor,
    )?;

    Ok(AuxControlPatch {
        words: [
            constants.common_words[0],
            constants.common_words[1],
            constants.common_words[2],
            constants.common_words[3],
            constants.common_words[4],
            constants.common_words[5],
            constants.common_words[6],
            aux_control_q16_mul(global_word, constants.q16_multipliers[0]),
            policy.words[0],
            constants.common_words[7],
            aux_control_q16_mul(global_word, constants.q16_multipliers[1]),
            constants.common_words[8],
            common_reciprocal,
            policy.words[1],
            policy.words[2],
            state_word_48 >> 3,
            policy_reciprocal,
            policy.words[3],
            policy.words[4],
            policy.words[5],
            aux_control_q16_mul(scaled_policy_reciprocal, global_word),
            policy.words[6],
        ],
    })
}

fn aux_control_q16_mul(left: i32, right: i32) -> i32 {
    ((i64::from(left) * i64::from(right)) >> 16) as i32
}

fn aux_control_reciprocal(
    divisor: i32,
    input: AuxControlReciprocalInput,
) -> Result<i32, AuxControlReciprocalOverflow> {
    if divisor == 0 {
        return Ok(-1);
    }
    let quotient = (1_i64 << 32) / i64::from(divisor);
    i32::try_from(quotient).map_err(|_| AuxControlReciprocalOverflow { input, divisor })
}

/// Transform the six-word middle run selected by the low three option bits.
///
/// All five differences are taken from `original` before rebuilding the
/// output. Products use signed Q16 arithmetic and retain the low 32 bits after
/// the arithmetic right shift. Subtraction, the first-word bias, and the
/// cumulative rebuild use 32-bit wrapping arithmetic.
#[must_use]
pub fn transform_compiled_control_run(
    original: [i32; COMPILED_CONTROL_RUN_WORD_COUNT],
    option_byte: u8,
) -> [i32; COMPILED_CONTROL_RUN_WORD_COUNT] {
    let (scale, bias) = match option_byte & 7 {
        0 => (0x1_1999_i32, -0x3333_i32),
        1 => (0x1_0ccc, -0x1999),
        2 => (0x1_0000, 0),
        3 => (0x0_f334, 0x1999),
        _ => (0x0_e667, 0x3333),
    };
    let deltas: [i32; COMPILED_CONTROL_RUN_WORD_COUNT - 1] =
        std::array::from_fn(|index| original[index + 1].wrapping_sub(original[index]));
    let scaled = deltas.map(|delta| ((i64::from(delta) * i64::from(scale)) >> 16) as i32);

    let mut output = [0; COMPILED_CONTROL_RUN_WORD_COUNT];
    output[0] = original[0].wrapping_add(bias);
    for index in 0..scaled.len() {
        output[index + 1] = output[index].wrapping_add(scaled[index]);
    }
    output
}

/// Apply [`transform_compiled_control_run`] to indices `1..=6` of the
/// eight-word family, preserving endpoints zero and seven exactly.
///
/// This adapter performs neither case selection nor reciprocal/copy-tail
/// derivation.
#[must_use]
pub fn transform_compiled_control_family(
    original: [i32; COMPILED_CONTROL_FAMILY_WORD_COUNT],
    option_byte: u8,
) -> [i32; COMPILED_CONTROL_FAMILY_WORD_COUNT] {
    let run = std::array::from_fn(|index| original[index + 1]);
    let transformed = transform_compiled_control_run(run, option_byte);
    let mut output = original;
    output[1..=COMPILED_CONTROL_RUN_WORD_COUNT].copy_from_slice(&transformed);
    output
}

/// Caller-owned inputs for composing only the recovered initializer stages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveredCompiledStageInputs<'a> {
    pub prior_case_words: CompiledCaseWords,
    pub selector: u32,
    pub config: &'a [u8],
    pub state_slot: i8,
    pub global_word: i32,
    pub state_758_context_nonzero: bool,
    pub state_758_table: &'a [i32],
}

/// Separate effects of the recovered post-dispatch initializer stages.
///
/// This is deliberately not a complete state/control snapshot. In particular,
/// `direct_case_stores` remains the raw case-body outcome before omitted
/// common-prefix mutations. Only `control_families.post_processed_raw`
/// supersedes its eight family words; the other three arrays are the later
/// reciprocal/copy-tail effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveredCompiledStageEffects {
    pub direct_case_stores: CompiledCaseOutcome,
    pub common_prefix: CompiledCommonPrefixEffects,
    pub center_split_q16: i32,
    pub aux_control_patch: AuxControlPatch,
    pub state_word_758: CompiledStateWord758Effect,
    pub post_helper: CompiledPostHelperEffects,
    pub control_families: CompiledControlFamilies,
    pub control_product_pair: CompiledControlProductPair,
    pub suffix_defaults: CompiledSuffixDefaultEffects,
    pub state_reciprocal_trio: CompiledStateReciprocalTrio,
    pub control_clear_loop: CompiledControlClearLoop,
    pub half_sign_state: CompiledHalfSignState,
    pub mixed_reset: CompiledMixedResetEffects,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveredCompiledStageError {
    Config(CompiledConfigResolveError),
    Prefix(CompiledCommonPrefixReciprocalOverflow),
    Aux(AuxControlReciprocalOverflow),
    State758Selection(CompiledStateWord758SelectionError),
    PostHelper(CompiledPostHelperReciprocalOverflow),
    Family(CompiledReciprocalOverflow),
    ProductPair(CompiledControlProductReciprocalOverflow),
    StateReciprocalTrio(CompiledStateReciprocalTrioOverflow),
}

impl fmt::Display for RecoveredCompiledStageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(source) => write!(formatter, "cannot resolve compiled options: {source}"),
            Self::Prefix(source) => write!(formatter, "cannot derive common prefix: {source}"),
            Self::Aux(source) => write!(formatter, "cannot derive auxiliary patch: {source}"),
            Self::State758Selection(source) => {
                write!(formatter, "cannot select state +0x758 value: {source}")
            }
            Self::PostHelper(source) => {
                write!(formatter, "cannot derive post-helper patch: {source}")
            }
            Self::Family(source) => {
                write!(
                    formatter,
                    "cannot derive compiled control families: {source}"
                )
            }
            Self::ProductPair(source) => {
                write!(
                    formatter,
                    "cannot derive compiled control product pair: {source}"
                )
            }
            Self::StateReciprocalTrio(source) => {
                write!(
                    formatter,
                    "cannot derive compiled state reciprocal trio: {source}"
                )
            }
        }
    }
}

impl std::error::Error for RecoveredCompiledStageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(source) => Some(source),
            Self::Prefix(source) => Some(source),
            Self::Aux(source) => Some(source),
            Self::State758Selection(source) => Some(source),
            Self::PostHelper(source) => Some(source),
            Self::Family(source) => Some(source),
            Self::ProductPair(source) => Some(source),
            Self::StateReciprocalTrio(source) => Some(source),
        }
    }
}

/// Compose only the recovered post-dispatch stages in their observed order.
///
/// The returned bundle keeps raw direct stores separate from the exact prefix,
/// center-split, auxiliary, and family/tail effects. The family/tail is derived
/// from and supersedes only the raw direct-case family, which the prefix and
/// post-helper patch do not touch. Config, prefix, auxiliary, state-`+0x758`
/// selection, post-helper, family, then product-pair validation order applies
/// only to the represented fallible stages. The infallible suffix-default
/// effect follows the product pair, followed by the state reciprocal trio, the
/// control-clear loop, and the half/sign plus mixed-reset effects. Success does
/// not prove a complete shipped initializer snapshot. Any represented failure
/// returns no partial bundle.
/// That atomic error boundary is a safe
/// reconstruction policy; the shipped in-place code could have performed
/// earlier writes before an `IDIV` fault.
pub fn compose_recovered_compiled_stage_effects(
    direct_cases: &CompiledDirectCaseSet,
    aux_constants: &AuxControlConstantSet,
    input: RecoveredCompiledStageInputs<'_>,
) -> Result<RecoveredCompiledStageEffects, RecoveredCompiledStageError> {
    let options = resolve_compiled_config_options(input.config, input.state_slot)
        .map_err(RecoveredCompiledStageError::Config)?;
    let direct_case_stores = direct_cases.apply(input.prior_case_words, input.selector);
    let common_prefix =
        derive_compiled_common_prefix(direct_case_stores.words, options.common_prefix_flags)
            .map_err(RecoveredCompiledStageError::Prefix)?;
    let center_split_q16 = compiled_center_split_q16(options.center_split);
    let transformed_control_family = transform_compiled_control_family(
        direct_case_stores.words.control_family(),
        options.control_run,
    );
    let aux_control_patch = aux_constants
        .derive(
            options.aux_control,
            common_prefix.state_word_48,
            input.global_word,
        )
        .map_err(RecoveredCompiledStageError::Aux)?;
    let selection_byte = if input.state_758_context_nonzero {
        *input
            .config
            .get(COMPILED_CONFIG_STATE_758_SELECTION_OFFSET)
            .ok_or(RecoveredCompiledStageError::State758Selection(
                CompiledStateWord758SelectionError::TruncatedSelectionByte {
                    required_len: COMPILED_CONFIG_STATE_758_SELECTION_OFFSET + 1,
                    actual_len: input.config.len(),
                },
            ))?
    } else {
        0
    };
    let state_word_758_argument = select_compiled_state_word_758_argument(
        input.state_758_context_nonzero,
        selection_byte,
        input.state_758_table,
    )
    .map_err(RecoveredCompiledStageError::State758Selection)?;
    let state_word_758 =
        derive_compiled_state_word_758(state_word_758_argument, options.state_word_758_fallback);
    let effective_control_word_15c = common_prefix
        .control_word_15c
        .unwrap_or(direct_case_stores.words.control_words[3]);
    let post_helper = derive_compiled_post_helper_effects(
        common_prefix.control_word_148,
        effective_control_word_15c,
    )
    .map_err(RecoveredCompiledStageError::PostHelper)?;
    let control_families = derive_compiled_control_families(transformed_control_family)
        .map_err(RecoveredCompiledStageError::Family)?;
    let control_product_pair = derive_compiled_control_product_pair(
        effective_control_word_15c,
        control_families.reciprocal[6],
    )
    .map_err(RecoveredCompiledStageError::ProductPair)?;
    let effective_state_words = common_prefix
        .state_words
        .unwrap_or(direct_case_stores.words.state_words);
    let suffix_defaults = derive_compiled_suffix_defaults(
        [
            effective_state_words[0],
            effective_state_words[1],
            effective_state_words[2],
        ],
        options.common_prefix_flags,
    );
    let state_reciprocal_trio = derive_compiled_state_reciprocal_trio([
        effective_state_words[0],
        effective_state_words[1],
        effective_state_words[2],
    ])
    .map_err(RecoveredCompiledStageError::StateReciprocalTrio)?;
    let control_clear_loop = derive_compiled_control_clear_loop();
    let half_sign_state = derive_compiled_half_sign_state([
        effective_state_words[0],
        effective_state_words[1],
        effective_state_words[2],
    ]);
    let mixed_reset = derive_compiled_mixed_reset_effects(options.unindexed_flags_26);

    Ok(RecoveredCompiledStageEffects {
        direct_case_stores,
        common_prefix,
        center_split_q16,
        aux_control_patch,
        state_word_758,
        post_helper,
        control_families,
        control_product_pair,
        suffix_defaults,
        state_reciprocal_trio,
        control_clear_loop,
        half_sign_state,
        mixed_reset,
    })
}

/// The four eight-word families after the observed reciprocal/copy tail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledControlFamilies {
    pub post_processed_raw: [i32; COMPILED_CONTROL_FAMILY_WORD_COUNT],
    pub reciprocal: [i32; COMPILED_CONTROL_FAMILY_WORD_COUNT],
    pub raw_copy: [i32; COMPILED_CONTROL_FAMILY_WORD_COUNT],
    pub reciprocal_copy: [i32; COMPILED_CONTROL_FAMILY_WORD_COUNT],
}

/// A nonzero primary word whose signed `2^32 / word` quotient does not fit in
/// the 32-bit destination used by x86 `IDIV`.
///
/// The shipped instruction would raise a divide exception. The pure contract
/// returns an error before publishing any derived families; this atomic error
/// is a reconstruction safety decision, not observed recovery behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledReciprocalOverflow {
    pub slot: usize,
    pub raw: i32,
}

impl fmt::Display for CompiledReciprocalOverflow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "compiled reciprocal at slot {} overflows i32 for raw value {}",
            self.slot, self.raw
        )
    }
}

impl std::error::Error for CompiledReciprocalOverflow {}

/// Derive the later reciprocal and copy families from the already
/// post-processed eight words present at control offsets `+0x1d0..+0x1ec`
/// after `FUN_00404160` and when the common tail reaches the reciprocal code.
///
/// Reciprocal slot zero is always zero. For slots `1..=7`, zero maps to `-1`;
/// otherwise the value is signed `2^32 / raw`, truncated toward zero.
/// Copies preserve every raw word exactly. Intervening initializer calls that
/// occur between case selection and this tail remain outside this contract;
/// callers must not pass [`CompiledCaseWords::control_family`] directly unless
/// they have separately applied that omitted processing.
pub fn derive_compiled_control_families(
    post_processed_raw: [i32; COMPILED_CONTROL_FAMILY_WORD_COUNT],
) -> Result<CompiledControlFamilies, CompiledReciprocalOverflow> {
    let mut reciprocal = [0; COMPILED_CONTROL_FAMILY_WORD_COUNT];
    for slot in 1..COMPILED_CONTROL_FAMILY_WORD_COUNT {
        let raw = post_processed_raw[slot];
        reciprocal[slot] = if raw == 0 {
            -1
        } else {
            let quotient = (1_i64 << 32) / i64::from(raw);
            i32::try_from(quotient).map_err(|_| CompiledReciprocalOverflow { slot, raw })?
        };
    }

    Ok(CompiledControlFamilies {
        post_processed_raw,
        reciprocal,
        raw_copy: post_processed_raw,
        reciprocal_copy: reciprocal,
    })
}

/// The car constants the four-wheel integrator and control processor read,
/// taken from the recovered initializer stages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledFourWheelConstants {
    /// State `+0x48`, the angular pass's moment scale (`0x40a7af`).
    pub state_word_48: i32,
    /// State `+0x4c`, the translation's inverse mass (`0x40a9f0`).
    pub state_word_4c: i32,
    /// State `+0x50/+0x54/+0x58`, the angular inertia (`0x40aa75..0x40aab4`).
    pub inertia_50_54_58: [i32; 3],
    /// Control `+0x144`: engine-braking drag response from the post-helper patch.
    pub control_word_144: i32,
    /// Control `+0x148`: full throttle response.
    pub control_word_148: i32,
    /// Control `+0x160`: driven-wheel smoothing response from the post-helper patch.
    pub control_word_160: i32,
    /// Control `+0x16c`: full brake response.
    pub control_word_16c: i32,
    /// Control `+0x170`: front/rear brake and coasting drag split (`FUN_00404120`, `0x8000` default).
    pub control_word_170: i32,
    /// Control `+0x158`: the drive split the wheel-force producer and the
    /// drivetrain read (front wheels compare it with `0`, rear with `1.0`).
    pub control_word_158: i32,
    /// Control `+0x150/+0x174/+0x17c/+0x184/+0x408` from the post-helper patch.
    pub control_word_150: i32,
    pub control_word_174: i32,
    pub control_word_17c: i32,
    pub control_word_184: i32,
    pub control_word_408: i32,
    /// Steering: `+0x188` centering rate, `+0x3e4` maximum angle and the
    /// `+0x440` option byte, all from the post-helper patch.
    pub control_word_188: i32,
    pub control_u16_3e4: u16,
    pub control_byte_440: u8,
    /// The effective `+0x15c` engine-speed scale (redline): the common
    /// prefix's scaled value, else the direct case word.
    pub control_word_15c: i32,
    /// `DAT_0050c5fc`, the gravity constant the car initializer sets from
    /// config `+0x25` (`0x407202..0x40721b`): `0x9999` normally, `0x1999`
    /// when bit 0 is set.
    pub gravity_c5fc: i32,
    /// Control `+0x420`, which the same initializer stage sets to 1
    /// (`0x4071ed`): the second loop's correction gate.
    pub control_word_420: i32,
}

/// The gravity constant `DAT_0050c5fc` for a config `+0x25` byte.
#[must_use]
pub const fn compiled_gravity_constant(config_25: u8) -> i32 {
    ((((!config_25) & 1) as i32) << 15) | 0x1999
}

fn post_helper_word(effects: &CompiledPostHelperEffects, offset: u16) -> i32 {
    let index = COMPILED_POST_HELPER_CONTROL_WORD_OFFSETS
        .iter()
        .position(|candidate| *candidate == offset)
        .expect("the offset is in the recovered post-helper schema");
    effects.control_words[index]
}

/// Derive [`CompiledFourWheelConstants`] from one car's direct case words and
/// its common-prefix config flags (config `+0x25`).
///
/// This runs only the stages that write these words, in the order
/// [`compose_recovered_compiled_stage_effects`] does: the common prefix
/// (`+0x48/+0x4c`, `+0x148`, optional `+0x15c` and state scaling), the
/// post-helper patch (`+0x150..+0x184`, `+0x408`), and the suffix defaults
/// (`+0x50..+0x58`). `+0x16c` is a direct case word that none of them rewrites.
/// Later initializer stages are not known to rewrite any of these words, but
/// that is not established for every caller.
///
/// # Errors
///
/// Returns the shared stage error for a common-prefix or post-helper
/// reciprocal that would fault the shipped `IDIV`.
pub fn derive_compiled_four_wheel_constants(
    words: CompiledCaseWords,
    config_flags: u8,
) -> Result<CompiledFourWheelConstants, RecoveredCompiledStageError> {
    let prefix = derive_compiled_common_prefix(words, config_flags)
        .map_err(RecoveredCompiledStageError::Prefix)?;
    let control_word_15c = prefix.control_word_15c.unwrap_or(words.control_words[3]);
    let post_helper =
        derive_compiled_post_helper_effects(prefix.control_word_148, control_word_15c)
            .map_err(RecoveredCompiledStageError::PostHelper)?;
    let state_words = prefix.state_words.unwrap_or(words.state_words);
    let suffix = derive_compiled_suffix_defaults(
        [state_words[0], state_words[1], state_words[2]],
        config_flags,
    );
    Ok(CompiledFourWheelConstants {
        state_word_48: prefix.state_word_48,
        state_word_4c: prefix.state_word_4c,
        inertia_50_54_58: suffix.state_words_50_54_58,
        control_word_144: post_helper_word(&post_helper, 0x144),
        control_word_148: prefix.control_word_148,
        control_word_160: post_helper_word(&post_helper, 0x160),
        control_word_16c: words.control_words[2],
        control_word_170: 0x8000,
        control_word_158: case_control_word(&words, 0x158),
        control_word_150: post_helper_word(&post_helper, 0x150),
        control_word_174: post_helper_word(&post_helper, 0x174),
        control_word_17c: post_helper_word(&post_helper, 0x17c),
        control_word_184: post_helper_word(&post_helper, 0x184),
        control_word_408: post_helper_word(&post_helper, 0x408),
        control_word_188: post_helper_word(&post_helper, 0x188),
        control_u16_3e4: post_helper.control_u16[0],
        control_byte_440: post_helper.control_bytes[0],
        control_word_15c,
        gravity_c5fc: compiled_gravity_constant(config_flags),
        control_word_420: 1,
    })
}

fn case_control_word(words: &CompiledCaseWords, offset: u16) -> i32 {
    let index = COMPILED_CONTROL_WORD_OFFSETS
        .iter()
        .position(|candidate| *candidate == offset)
        .expect("the offset is in the recovered case schema");
    words.control_words[index]
}

/// Derive the control processor's gearbox tables from one car's case words,
/// config flags and compiled control families.
///
/// Thresholds come from the direct case words (`+0x194..+0x1a4` upper,
/// `+0x1b8..+0x1c8` lower) and the post-helper patch (`+0x1a8` upper for gear
/// 6, `+0x1b4` lower for gear 1); ratios and inverses are the families at
/// `+0x1d0` and `+0x1f0`; the scale is the effective `+0x15c`. Entries for
/// gears 0 and 7 are left zero: no recovered stage writes them, and no walk
/// decision depends on them.
///
/// # Errors
///
/// Returns the shared stage error for a common-prefix or post-helper
/// reciprocal that would fault the shipped `IDIV`.
pub fn derive_compiled_gearbox_tables(
    words: CompiledCaseWords,
    config_flags: u8,
    families: &CompiledControlFamilies,
) -> Result<crate::gearbox::GearboxTables, RecoveredCompiledStageError> {
    let prefix = derive_compiled_common_prefix(words, config_flags)
        .map_err(RecoveredCompiledStageError::Prefix)?;
    let control_word_15c = prefix.control_word_15c.unwrap_or(words.control_words[3]);
    let post_helper =
        derive_compiled_post_helper_effects(prefix.control_word_148, control_word_15c)
            .map_err(RecoveredCompiledStageError::PostHelper)?;
    let mut upper_190 = [0; 8];
    let mut lower_1b0 = [0; 8];
    for gear in 1..=6_u16 {
        upper_190[usize::from(gear)] = if gear == 6 {
            post_helper_word(&post_helper, 0x1a8)
        } else {
            case_control_word(&words, 0x190 + gear * 4)
        };
        lower_1b0[usize::from(gear)] = if gear == 1 {
            post_helper_word(&post_helper, 0x1b4)
        } else {
            case_control_word(&words, 0x1b0 + gear * 4)
        };
    }
    Ok(crate::gearbox::GearboxTables {
        upper_190,
        lower_1b0,
        ratio_1d0: families.post_processed_raw,
        inverse_1f0: families.reciprocal,
        redline_15c: control_word_15c,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cmr_assets::{
        AUX_CONTROL_DATA_FILE, AUX_CONTROL_IMPORT_SCHEMA_VERSION,
        AUX_CONTROL_PROFILE_SCHEMA_VERSION, AUX_CONTROL_STATE_FILE, AuxControlEncoding,
        AuxControlExecutable, AuxControlImportFile, AuxControlImportShape, AuxControlImportSummary,
        AuxControlProfile, AuxControlSpec, COMPILED_INITIALIZER_DATA_FILE,
        COMPILED_INITIALIZER_IMPORT_SCHEMA_VERSION, COMPILED_INITIALIZER_STATE_FILE,
        CompiledInitializerEncoding, CompiledInitializerExecutable, CompiledInitializerImportFile,
        CompiledInitializerImportShape, CompiledInitializerImportSummary,
        CompiledInitializerProfile, CompiledInitializerSpec, ReviewedAuxControlProfile,
        ReviewedCompiledInitializerProfile,
    };
    use cmr_formats::pe_aux_control_initializer::{
        AUX_CONTROL_COMMON_WORD_COUNT as FORMAT_AUX_COMMON_WORD_COUNT,
        AUX_CONTROL_ENCODED_LEN as FORMAT_AUX_ENCODED_LEN,
        AUX_CONTROL_FORMAT_VERSION as FORMAT_AUX_FORMAT_VERSION,
        AUX_CONTROL_MULTIPLIER_COUNT as FORMAT_AUX_MULTIPLIER_COUNT,
        AUX_CONTROL_POLICY_COUNT as FORMAT_AUX_POLICY_COUNT,
        AUX_CONTROL_POLICY_DIVISOR_COUNT as FORMAT_AUX_POLICY_DIVISOR_COUNT,
        AUX_CONTROL_POLICY_WORD_COUNT as FORMAT_AUX_POLICY_WORD_COUNT,
        AUX_CONTROL_TOTAL_WORD_COUNT as FORMAT_AUX_TOTAL_WORD_COUNT,
        AuxControlConstantWords as FormatAuxControlConstantWords,
        AuxControlPolicyWords as FormatAuxControlPolicyWords,
        OFFICIAL_EU_AUX_CONTROL_INITIALIZER_LAYOUT, encode_aux_control_constants,
    };
    use cmr_formats::pe_compiled_initializer::{
        COMPILED_CASE_ENCODED_LEN, COMPILED_CASE_FORMAT_VERSION,
        COMPILED_CONTROL_WORD_COUNT as FORMAT_CONTROL_WORD_COUNT,
        COMPILED_STATE_WORD_COUNT as FORMAT_STATE_WORD_COUNT,
        CompiledCaseWords as FormatCompiledCaseWords, OFFICIAL_EU_COMPILED_INITIALIZER_LAYOUT,
        encode_compiled_cases,
    };
    use sha2::{Digest, Sha256};
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "cmr-core-compiled-direct-cases-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn digest(bytes: &[u8]) -> String {
        use std::fmt::Write as _;
        let mut output = String::with_capacity(64);
        for byte in Sha256::digest(bytes) {
            write!(output, "{byte:02x}").unwrap();
        }
        output
    }

    fn format_cases() -> [FormatCompiledCaseWords; FORMAT_CASE_COUNT] {
        let mut cases = std::array::from_fn(|selector| FormatCompiledCaseWords {
            state_words: std::array::from_fn(|index| selector as i32 * 100 + index as i32 - 50),
            control_words: std::array::from_fn(|index| selector as i32 * -1_000 - index as i32),
        });
        cases[0].state_words[..4].copy_from_slice(&[i32::MIN, i32::MAX, 0, -1]);
        cases[13].control_words[..4].copy_from_slice(&[i32::MIN, i32::MAX, 0, -1]);
        cases
    }

    fn open_synthetic_store(
        cases: &[FormatCompiledCaseWords; FORMAT_CASE_COUNT],
    ) -> (TempDirectory, CompiledInitializerStore) {
        let temp = TempDirectory::new();
        let root = temp.0.join("store");
        fs::create_dir(&root).unwrap();
        let source = b"synthetic source identity";
        let layout = OFFICIAL_EU_COMPILED_INITIALIZER_LAYOUT;
        let profile = CompiledInitializerProfile {
            schema_version: 1,
            id: "synthetic-core-adapter".into(),
            executable: CompiledInitializerExecutable {
                path: "Rally.exe".into(),
                size: source.len() as u64,
                sha256: digest(source),
            },
            initializer: CompiledInitializerSpec {
                image_base: layout.image_base,
                zero_seed_va: layout.zero_seed_va,
                esi_seed_va: layout.esi_seed_va,
                edi_seed_va: layout.edi_seed_va,
                seed_call_targets: layout.seed_call_targets,
                dispatch_va: layout.dispatch_va,
                jump_table_va: layout.jump_table_va,
                first_case_va: layout.first_case_va,
                common_join_va: layout.common_join_va,
                state_pointer_va: layout.state_pointer_va,
                control_pointer_va: layout.control_pointer_va,
                selectors: FORMAT_CASE_COUNT as u32,
                state_offsets: layout.state_offsets,
                control_offsets: layout.control_offsets,
                output_encoding: CompiledInitializerEncoding::CmrCcaseV1,
            },
        };
        let reviewed =
            ReviewedCompiledInitializerProfile::from_json(&serde_json::to_vec(&profile).unwrap())
                .unwrap();
        let data = encode_compiled_cases(cases);
        fs::write(root.join(COMPILED_INITIALIZER_DATA_FILE), data).unwrap();
        let summary = CompiledInitializerImportSummary {
            schema_version: COMPILED_INITIALIZER_IMPORT_SCHEMA_VERSION,
            profile: reviewed.profile().id.clone(),
            profile_sha256: reviewed.sha256().into(),
            source: CompiledInitializerImportFile {
                path: reviewed.profile().executable.path.clone(),
                size: reviewed.profile().executable.size,
                sha256: reviewed.profile().executable.sha256.clone(),
            },
            output: CompiledInitializerImportFile {
                path: COMPILED_INITIALIZER_DATA_FILE.into(),
                size: COMPILED_CASE_ENCODED_LEN as u64,
                sha256: digest(&data),
            },
            shape: CompiledInitializerImportShape {
                format_version: COMPILED_CASE_FORMAT_VERSION,
                selectors: FORMAT_CASE_COUNT as u32,
                state_words_per_selector: FORMAT_STATE_WORD_COUNT as u32,
                control_words_per_selector: FORMAT_CONTROL_WORD_COUNT as u32,
                encoding: CompiledInitializerEncoding::CmrCcaseV1,
            },
        };
        fs::write(
            root.join(COMPILED_INITIALIZER_STATE_FILE),
            serde_json::to_vec(&summary).unwrap(),
        )
        .unwrap();
        let store = CompiledInitializerStore::open(&root, &reviewed).unwrap();
        (temp, store)
    }

    fn synthetic_case(tag: i32) -> CompiledCaseWords {
        CompiledCaseWords {
            state_words: std::array::from_fn(|index| tag * 100 + index as i32),
            control_words: std::array::from_fn(|index| tag * 1_000 + index as i32 + 100),
        }
    }

    fn synthetic_cases() -> [CompiledCaseWords; COMPILED_CASE_COUNT] {
        std::array::from_fn(|index| synthetic_case(index as i32 + 1))
    }

    fn aux_policy(tag: i32) -> AuxControlPolicyConstants {
        AuxControlPolicyConstants {
            words: std::array::from_fn(|index| tag + index as i32 + 1),
            reciprocal_divisor: 0x1_0000,
            scaled_reciprocal_divisor: 0x2_0000,
        }
    }

    fn aux_constants() -> AuxControlConstants {
        AuxControlConstants {
            common_words: [10, 11, 12, 13, 14, 15, 16, 17, 0x1_0000],
            q16_multipliers: [0x1_0000, 0x2_0000],
            policy_one: aux_policy(100),
            policy_two: aux_policy(200),
            default_policy: aux_policy(300),
        }
    }

    fn format_aux_constants() -> FormatAuxControlConstantWords {
        FormatAuxControlConstantWords {
            common_words: [i32::MIN, i32::MAX, 0, -1, -2, 3, -4, 5, 0x1_0000],
            q16_multipliers: [i32::MIN, i32::MAX],
            policy_one: FormatAuxControlPolicyWords {
                words: [i32::MAX, i32::MIN, 0, -1, 11, -12, 13],
                reciprocal_divisor: 0x1_0000,
                scaled_reciprocal_divisor: -0x3_0000,
            },
            policy_two: FormatAuxControlPolicyWords {
                words: [-20, 21, -22, 23, -24, 25, -26],
                reciprocal_divisor: -0x2_0000,
                scaled_reciprocal_divisor: 0,
            },
            default_policy: FormatAuxControlPolicyWords {
                words: [30, -31, 32, -33, 34, -35, 36],
                reciprocal_divisor: i32::MIN,
                scaled_reciprocal_divisor: i32::MAX,
            },
        }
    }

    fn core_aux_constants(format: &FormatAuxControlConstantWords) -> AuxControlConstants {
        AuxControlConstants {
            common_words: format.common_words,
            q16_multipliers: format.q16_multipliers,
            policy_one: copy_aux_policy(format.policy_one),
            policy_two: copy_aux_policy(format.policy_two),
            default_policy: copy_aux_policy(format.default_policy),
        }
    }

    fn open_synthetic_aux_store(
        constants: &FormatAuxControlConstantWords,
    ) -> (TempDirectory, AuxControlConstantsStore) {
        let temp = TempDirectory::new();
        let root = temp.0.join("aux-store");
        fs::create_dir(&root).unwrap();
        let source = b"synthetic aux-control source identity";
        let layout = OFFICIAL_EU_AUX_CONTROL_INITIALIZER_LAYOUT;
        let profile = AuxControlProfile {
            schema_version: AUX_CONTROL_PROFILE_SCHEMA_VERSION,
            id: "synthetic-core-aux-adapter".into(),
            executable: AuxControlExecutable {
                path: "Rally.exe".into(),
                size: source.len() as u64,
                sha256: digest(source),
            },
            initializer: AuxControlSpec {
                image_base: layout.image_base,
                function_start_va: layout.function_start_va,
                function_end_va: layout.function_end_va,
                lookup_va: layout.lookup_va,
                select_va: layout.select_va,
                default_policy_va: layout.default_policy_va,
                policy_two_va: layout.policy_two_va,
                policy_one_va: layout.policy_one_va,
                common_va: layout.common_va,
                first_divide_va: layout.first_divide_va,
                first_divide_join_va: layout.first_divide_join_va,
                second_divide_va: layout.second_divide_va,
                second_divide_join_va: layout.second_divide_join_va,
                third_divide_va: layout.third_divide_va,
                third_divide_join_va: layout.third_divide_join_va,
                config_pointer_va: layout.config_pointer_va,
                state_pointer_va: layout.state_pointer_va,
                control_pointer_va: layout.control_pointer_va,
                global_word_va: layout.global_word_va,
                config_flag_offset: layout.config_flag_offset,
                state_index_offset: layout.state_index_offset,
                config_option_offset: layout.config_option_offset,
                config_record_stride: layout.config_record_stride,
                state_word_offset: layout.state_word_offset,
                output_offsets: layout.output_offsets,
                output_encoding: AuxControlEncoding::CmrAuxcV1,
            },
        };
        let reviewed =
            ReviewedAuxControlProfile::from_json(&serde_json::to_vec(&profile).unwrap()).unwrap();
        let data = encode_aux_control_constants(constants);
        fs::write(root.join(AUX_CONTROL_DATA_FILE), data).unwrap();
        let summary = AuxControlImportSummary {
            schema_version: AUX_CONTROL_IMPORT_SCHEMA_VERSION,
            profile: reviewed.profile().id.clone(),
            profile_sha256: reviewed.sha256().into(),
            source: AuxControlImportFile {
                path: reviewed.profile().executable.path.clone(),
                size: reviewed.profile().executable.size,
                sha256: reviewed.profile().executable.sha256.clone(),
            },
            output: AuxControlImportFile {
                path: AUX_CONTROL_DATA_FILE.into(),
                size: FORMAT_AUX_ENCODED_LEN as u64,
                sha256: digest(&data),
            },
            shape: AuxControlImportShape {
                format_version: FORMAT_AUX_FORMAT_VERSION,
                common_word_count: FORMAT_AUX_COMMON_WORD_COUNT as u32,
                multiplier_count: FORMAT_AUX_MULTIPLIER_COUNT as u32,
                policy_count: FORMAT_AUX_POLICY_COUNT as u32,
                policy_word_count: FORMAT_AUX_POLICY_WORD_COUNT as u32,
                policy_divisor_count: FORMAT_AUX_POLICY_DIVISOR_COUNT as u32,
                encoding: AuxControlEncoding::CmrAuxcV1,
            },
        };
        fs::write(
            root.join(AUX_CONTROL_STATE_FILE),
            serde_json::to_vec(&summary).unwrap(),
        )
        .unwrap();
        let store = AuxControlConstantsStore::open(&root, &reviewed).unwrap();
        (temp, store)
    }

    fn compiled_stage_config(
        slot: i8,
        common_prefix_flags: u8,
        aux_option: u8,
        center_option: u8,
        run_option: u8,
        force_default: bool,
    ) -> Vec<u8> {
        let record_base = slot as usize * COMPILED_CONFIG_RECORD_STRIDE;
        let mut config = vec![0; record_base + COMPILED_CONFIG_CONTROL_RUN_OPTION_OFFSET + 1];
        config[COMPILED_CONFIG_COMMON_FLAGS_OFFSET] = common_prefix_flags;
        config[record_base + COMPILED_CONFIG_AUX_OPTION_OFFSET] = aux_option;
        config[record_base + COMPILED_CONFIG_CENTER_SPLIT_OPTION_OFFSET] = center_option;
        config[record_base + COMPILED_CONFIG_CONTROL_RUN_OPTION_OFFSET] = run_option;
        if force_default {
            config[COMPILED_CONFIG_AUX_FORCE_FLAG_OFFSET] = COMPILED_CONFIG_AUX_FORCE_DEFAULT_MASK;
        }
        config
    }

    fn aux_value(patch: AuxControlPatch, offset: u16) -> i32 {
        let index = AUX_CONTROL_WORD_OFFSETS
            .iter()
            .position(|candidate| *candidate == offset)
            .unwrap();
        patch.words[index]
    }

    fn post_helper_control_value(effects: CompiledPostHelperEffects, offset: u16) -> i32 {
        let index = COMPILED_POST_HELPER_CONTROL_WORD_OFFSETS
            .iter()
            .position(|candidate| *candidate == offset)
            .unwrap();
        effects.control_words[index]
    }

    fn aux_policy_words(patch: AuxControlPatch) -> [i32; 7] {
        AUX_CONTROL_POLICY_WORD_OFFSETS.map(|offset| aux_value(patch, offset))
    }

    #[test]
    fn format_adapter_pins_counts_offsets_and_selector_order() {
        let format = format_cases();
        let owned = CompiledDirectCaseSet::from_format_cases(&format).unwrap();
        assert_eq!(owned.cases().len(), COMPILED_CASE_COUNT);
        for (selector, format_case) in format.iter().enumerate() {
            assert_eq!(owned.cases()[selector].state_words, format_case.state_words);
            assert_eq!(
                owned.cases()[selector].control_words,
                format_case.control_words
            );
        }
    }

    #[test]
    fn verified_store_adapts_to_owned_direct_cases_and_unsigned_selection() {
        let format = format_cases();
        let (temp, owned) = {
            let (temp, store) = open_synthetic_store(&format);
            let owned = CompiledDirectCaseSet::try_from(&store).unwrap();
            (temp, owned)
        };
        drop(temp);

        for (selector, format_case) in format.iter().enumerate() {
            assert_eq!(owned.cases()[selector].state_words, format_case.state_words);
            assert_eq!(
                owned.cases()[selector].control_words,
                format_case.control_words
            );
        }
        assert_eq!(
            owned.cases()[0].state_words[..4],
            [i32::MIN, i32::MAX, 0, -1]
        );
        assert_eq!(
            owned.cases()[13].control_words[..4],
            [i32::MIN, i32::MAX, 0, -1]
        );

        let prior = synthetic_case(-77);

        for selector in [0_u32, 7, 13] {
            let outcome = owned.apply(prior, selector);
            assert_eq!(outcome.selected_case, Some(selector as u8));
            assert_eq!(
                outcome.words.state_words,
                format[selector as usize].state_words
            );
            assert_eq!(
                outcome.words.control_words,
                format[selector as usize].control_words
            );
        }
        for selector in [14, u32::MAX] {
            assert_eq!(
                owned.apply(prior, selector),
                CompiledCaseOutcome {
                    words: prior,
                    selected_case: None,
                }
            );
        }
    }

    #[test]
    fn schema_offsets_pin_every_destination_and_family_slice() {
        assert_eq!(
            COMPILED_STATE_WORD_OFFSETS,
            [0x020, 0x024, 0x028, 0x638, 0x63c, 0x640, 0x648, 0x644]
        );
        assert_eq!(
            COMPILED_CONTROL_WORD_OFFSETS,
            [
                0x148, 0x158, 0x16c, 0x15c, 0x194, 0x198, 0x19c, 0x1a0, 0x1a4, 0x1b8, 0x1bc, 0x1c0,
                0x1c4, 0x1c8, 0x1d0, 0x1d4, 0x1d8, 0x1dc, 0x1e0, 0x1e4, 0x1e8, 0x1ec, 0x140,
            ]
        );

        let words = synthetic_case(7);
        assert_eq!(words.control_family(), words.control_words[14..22]);
        for offset in COMPILED_CONTROL_PRODUCT_OFFSETS {
            assert!(!COMPILED_CONTROL_WORD_OFFSETS.contains(&offset));
            assert!(!COMPILED_COMMON_PREFIX_CONTROL_OFFSETS.contains(&offset));
            assert!(!AUX_CONTROL_WORD_OFFSETS.contains(&offset));
            assert!(!COMPILED_POST_HELPER_CONTROL_WORD_OFFSETS.contains(&offset));
            assert_ne!(offset, 0x170);
            for family_base in [0x1f0_u16, 0x210, 0x230] {
                for family_index in 0..COMPILED_CONTROL_FAMILY_WORD_COUNT as u16 {
                    assert_ne!(offset, family_base + family_index * 4);
                }
            }
        }
        for offset in COMPILED_SUFFIX_DEFAULT_STATE_OFFSETS {
            assert!(!COMPILED_STATE_WORD_OFFSETS.contains(&offset));
            assert!(!COMPILED_COMMON_PREFIX_STATE_OFFSETS.contains(&offset));
            assert!(!COMPILED_POST_HELPER_STATE_WORD_OFFSETS.contains(&offset));
            assert_ne!(offset, COMPILED_STATE_WORD_758_OFFSET);
        }
        for offset in COMPILED_STATE_RECIPROCAL_TRIO_OFFSETS {
            assert!(!COMPILED_STATE_WORD_OFFSETS.contains(&offset));
            assert!(!COMPILED_COMMON_PREFIX_STATE_OFFSETS.contains(&offset));
            assert!(!COMPILED_POST_HELPER_STATE_WORD_OFFSETS.contains(&offset));
            assert!(!COMPILED_SUFFIX_DEFAULT_STATE_OFFSETS.contains(&offset));
            assert_ne!(offset, COMPILED_STATE_WORD_758_OFFSET);
        }
        for offset in COMPILED_CONTROL_CLEAR_LOOP_OFFSETS {
            assert!(!COMPILED_CONTROL_WORD_OFFSETS.contains(&offset));
            assert!(!COMPILED_COMMON_PREFIX_CONTROL_OFFSETS.contains(&offset));
            assert!(!AUX_CONTROL_WORD_OFFSETS.contains(&offset));
            assert!(!COMPILED_POST_HELPER_CONTROL_WORD_OFFSETS.contains(&offset));
            assert!(!COMPILED_CONTROL_PRODUCT_OFFSETS.contains(&offset));
            assert!(!COMPILED_POST_HELPER_CONTROL_U16_OFFSETS.contains(&offset));
            assert!(!COMPILED_POST_HELPER_CONTROL_BYTE_OFFSETS.contains(&offset));
            assert_ne!(offset, 0x170);
            for family_base in [0x1d0_u16, 0x1f0, 0x210, 0x230] {
                for family_index in 0..COMPILED_CONTROL_FAMILY_WORD_COUNT as u16 {
                    assert_ne!(offset, family_base + family_index * 4);
                }
            }
        }
        for offset in COMPILED_HALF_SIGN_STATE_OFFSETS {
            assert!(!COMPILED_STATE_WORD_OFFSETS.contains(&offset));
            assert!(!COMPILED_COMMON_PREFIX_STATE_OFFSETS.contains(&offset));
            assert!(!COMPILED_POST_HELPER_STATE_WORD_OFFSETS.contains(&offset));
            assert!(!COMPILED_SUFFIX_DEFAULT_STATE_OFFSETS.contains(&offset));
            assert!(!COMPILED_STATE_RECIPROCAL_TRIO_OFFSETS.contains(&offset));
            assert_ne!(offset, COMPILED_STATE_WORD_758_OFFSET);
        }
        for offset in COMPILED_MIXED_RESET_CONTROL_OFFSETS {
            assert!(!COMPILED_CONTROL_WORD_OFFSETS.contains(&offset));
            assert!(!COMPILED_COMMON_PREFIX_CONTROL_OFFSETS.contains(&offset));
            assert!(!AUX_CONTROL_WORD_OFFSETS.contains(&offset));
            assert!(!COMPILED_POST_HELPER_CONTROL_WORD_OFFSETS.contains(&offset));
            assert!(!COMPILED_CONTROL_PRODUCT_OFFSETS.contains(&offset));
            assert!(!COMPILED_CONTROL_CLEAR_LOOP_OFFSETS.contains(&offset));
            assert!(!COMPILED_POST_HELPER_CONTROL_U16_OFFSETS.contains(&offset));
            assert!(!COMPILED_POST_HELPER_CONTROL_BYTE_OFFSETS.contains(&offset));
            assert_ne!(offset, 0x170);
            for family_base in [0x1d0_u16, 0x1f0, 0x210, 0x230] {
                for family_index in 0..COMPILED_CONTROL_FAMILY_WORD_COUNT as u16 {
                    assert_ne!(offset, family_base + family_index * 4);
                }
            }
        }
        for offset in COMPILED_MIXED_RESET_STATE_DWORD_OFFSETS {
            assert!(!COMPILED_STATE_WORD_OFFSETS.contains(&offset));
            assert!(!COMPILED_COMMON_PREFIX_STATE_OFFSETS.contains(&offset));
            assert!(!COMPILED_POST_HELPER_STATE_WORD_OFFSETS.contains(&offset));
            assert!(!COMPILED_SUFFIX_DEFAULT_STATE_OFFSETS.contains(&offset));
            assert!(!COMPILED_STATE_RECIPROCAL_TRIO_OFFSETS.contains(&offset));
            assert!(!COMPILED_HALF_SIGN_STATE_OFFSETS.contains(&offset));
            assert_ne!(offset, COMPILED_STATE_WORD_758_OFFSET);
        }
        let represented_state_dwords: Vec<u16> = COMPILED_STATE_WORD_OFFSETS
            .into_iter()
            .chain(COMPILED_COMMON_PREFIX_STATE_OFFSETS)
            .chain(COMPILED_POST_HELPER_STATE_WORD_OFFSETS)
            .chain(COMPILED_SUFFIX_DEFAULT_STATE_OFFSETS)
            .chain(COMPILED_STATE_RECIPROCAL_TRIO_OFFSETS)
            .chain(COMPILED_HALF_SIGN_STATE_OFFSETS)
            .chain(COMPILED_MIXED_RESET_STATE_DWORD_OFFSETS)
            .chain([COMPILED_STATE_WORD_758_OFFSET])
            .collect();
        for (index, offset) in COMPILED_MIXED_RESET_STATE_U16_OFFSETS
            .into_iter()
            .enumerate()
        {
            assert_eq!(offset % 2, 0);
            let start = u32::from(offset);
            let end = start + 2;
            for (other_index, other) in COMPILED_MIXED_RESET_STATE_U16_OFFSETS
                .into_iter()
                .enumerate()
            {
                if index != other_index {
                    let other_start = u32::from(other);
                    assert!(end <= other_start || other_start + 2 <= start);
                }
            }
            for dword in &represented_state_dwords {
                let dword_start = u32::from(*dword);
                assert!(end <= dword_start || dword_start + 4 <= start);
            }
        }
        assert_eq!(
            u32::from(COMPILED_MIXED_RESET_STATE_U16_OFFSETS[3]) + 2,
            u32::from(COMPILED_MIXED_RESET_STATE_DWORD_OFFSETS[0])
        );
    }

    #[test]
    fn every_valid_selector_replaces_all_thirty_one_words() {
        let cases = synthetic_cases();
        let prior = synthetic_case(-1);
        for selector in 0..COMPILED_CASE_COUNT as u32 {
            let outcome = apply_compiled_case(prior, &cases, selector);
            assert_eq!(outcome.selected_case, Some(selector as u8));
            assert_eq!(outcome.words, cases[selector as usize]);
        }
    }

    #[test]
    fn first_and_last_selectors_keep_offset_order_intact() {
        let cases = synthetic_cases();
        for selector in [0, 13] {
            let outcome = apply_compiled_case(CompiledCaseWords::default(), &cases, selector);
            assert_eq!(
                outcome.words.state_words,
                cases[selector as usize].state_words
            );
            assert_eq!(
                outcome.words.control_words,
                cases[selector as usize].control_words
            );
        }
    }

    #[test]
    fn unsigned_invalid_selectors_preserve_every_case_destination() {
        let cases = synthetic_cases();
        let prior = synthetic_case(-9);
        for selector in [14, 15, u32::MAX, i32::MAX as u32, i32::MIN as u32] {
            assert_eq!(
                apply_compiled_case(prior, &cases, selector),
                CompiledCaseOutcome {
                    words: prior,
                    selected_case: None,
                }
            );
        }
    }

    #[test]
    fn common_prefix_always_adjusts_control_and_derives_zero_reciprocal() {
        let mut words = synthetic_case(3);
        words.control_words[0] = 0x051e;
        words.control_words[22] = -0x78_0000;
        let original_state = words.state_words;
        let original_control_15c = words.control_words[3];

        let effects = derive_compiled_common_prefix(words, 0).unwrap();

        assert_eq!(effects.control_word_148, 0);
        assert_eq!(effects.control_word_15c, None);
        assert_eq!(effects.state_words, None);
        assert_eq!(words.control_words[3], original_control_15c);
        assert_eq!(words.state_words, original_state);
        assert_eq!(effects.state_word_48, 0);
        assert_eq!(effects.state_word_4c, -1);
    }

    #[test]
    fn common_prefix_control_flag_scales_adjusted_signed_words() {
        let mut words = synthetic_case(4);
        words.control_words[0] = i32::MAX;
        words.control_words[3] = -0x1_0001;
        words.control_words[22] = -0x77_fffc;
        let adjusted = i32::MAX.wrapping_sub(0x051e);

        let effects =
            derive_compiled_common_prefix(words, COMPILED_COMMON_CONTROL_SCALE_MASK).unwrap();

        assert_eq!(
            effects.control_word_148,
            ((i64::from(adjusted) * 0x2_0000_i64) >> 16) as i32
        );
        assert_eq!(effects.control_word_15c, Some(-0x1_3335));
        assert_eq!(effects.state_words, None);
        assert_eq!(effects.state_word_48, 4);
        assert_eq!(effects.state_word_4c, 0x4000_0000);
    }

    #[test]
    fn common_prefix_state_flag_halves_all_eight_words_with_arithmetic_shift() {
        let mut words = synthetic_case(5);
        words.state_words = [-3, -2, -1, 0, 1, 2, 3, i32::MAX];
        words.control_words[22] = -0x77_fffd;

        let effects =
            derive_compiled_common_prefix(words, COMPILED_COMMON_STATE_SCALE_MASK).unwrap();

        assert_eq!(
            effects.state_words,
            Some([-2, -1, -1, 0, 0, 1, 1, 0x3fff_ffff])
        );
        assert_eq!(effects.control_word_15c, None);
        assert_eq!(effects.state_word_48, 3);
        assert_eq!(effects.state_word_4c, 1_431_655_765);
    }

    #[test]
    fn common_prefix_combines_both_masks_and_ignores_other_bits() {
        let mut words = synthetic_case(9);
        words.state_words = [2, 4, 6, 8, 10, 12, 14, 16];
        words.control_words[0] = 0x151e;
        words.control_words[3] = 0x2_0000;
        words.control_words[22] = -0x77_fffc;
        let flags = COMPILED_COMMON_CONTROL_SCALE_MASK | COMPILED_COMMON_STATE_SCALE_MASK;

        let combined = derive_compiled_common_prefix(words, flags).unwrap();
        let with_other_bits = derive_compiled_common_prefix(words, flags | 0xf9).unwrap();

        assert_eq!(combined, with_other_bits);
        assert_eq!(combined.control_word_148, 0x2000);
        assert_eq!(combined.control_word_15c, Some(0x2_6666));
        assert_eq!(combined.state_words, Some([1, 2, 3, 4, 5, 6, 7, 8]));
    }

    #[test]
    fn common_prefix_ignores_unobserved_flag_bits_and_wraps_state_derivation() {
        let mut words = synthetic_case(6);
        words.control_words[22] = i32::MAX;

        let baseline = derive_compiled_common_prefix(words, 0).unwrap();
        let unknown_bits = derive_compiled_common_prefix(words, 0xf9).unwrap();

        assert_eq!(unknown_bits, baseline);
        assert_eq!(baseline.state_word_48, i32::MIN + 0x77_ffff);
    }

    #[test]
    fn common_prefix_reports_every_signed_reciprocal_overflow_boundary() {
        for state_word_48 in [-1_i32, 1, 2] {
            let mut words = synthetic_case(7);
            words.control_words[22] = state_word_48.wrapping_sub(0x78_0000);
            assert_eq!(
                derive_compiled_common_prefix(words, 0),
                Err(CompiledCommonPrefixReciprocalOverflow { state_word_48 })
            );
        }

        let mut words = synthetic_case(8);
        words.control_words[22] = (-2_i32).wrapping_sub(0x78_0000);
        let effects = derive_compiled_common_prefix(words, 0).unwrap();
        assert_eq!(effects.state_word_4c, i32::MIN);
    }

    #[test]
    fn post_helper_patch_pins_exact_mixed_width_schema_and_values() {
        assert_eq!(
            COMPILED_POST_HELPER_CONTROL_WORD_OFFSETS,
            [
                0x144, 0x14c, 0x150, 0x154, 0x160, 0x174, 0x178, 0x17c, 0x180, 0x184, 0x188, 0x18c,
                0x1a8, 0x1b4, 0x408
            ]
        );
        assert_eq!(COMPILED_POST_HELPER_STATE_WORD_OFFSETS, [0x0ec]);
        assert_eq!(COMPILED_POST_HELPER_CONTROL_BYTE_OFFSETS, [0x440]);
        assert_eq!(COMPILED_POST_HELPER_CONTROL_U16_OFFSETS, [0x3e4]);

        let effects = derive_compiled_post_helper_effects(-77, 0x2_0000).unwrap();
        assert_eq!(post_helper_control_value(effects, 0x144), 0x83);
        assert_eq!(post_helper_control_value(effects, 0x14c), -77);
        assert_eq!(post_helper_control_value(effects, 0x150), 0x0ccc);
        assert_eq!(post_helper_control_value(effects, 0x154), 0x0ccc);
        assert_eq!(post_helper_control_value(effects, 0x160), 0x8000);
        for offset in [0x174, 0x178, 0x17c, 0x180] {
            assert_eq!(post_helper_control_value(effects, offset), 0x0ccc);
        }
        assert_eq!(post_helper_control_value(effects, 0x184), 0x8000);
        assert_eq!(post_helper_control_value(effects, 0x188), 0x3333);
        assert_eq!(post_helper_control_value(effects, 0x18c), 0x3333);
        assert_eq!(post_helper_control_value(effects, 0x1a8), 0x0fae1);
        assert_eq!(post_helper_control_value(effects, 0x1b4), 0x8000);
        assert_eq!(post_helper_control_value(effects, 0x408), 0);
        assert_eq!(effects.state_words, [0]);
        assert_eq!(effects.control_bytes, [3]);
        assert_eq!(effects.control_u16, [0x02aa]);
    }

    #[test]
    fn post_helper_reciprocal_pins_zero_signed_truncation_and_boundaries() {
        for (divisor, expected) in [
            (0, -1),
            (0x3_0000, 0x5555),
            (-0x3_0000, -0x5555),
            (-2, i32::MIN),
            (3, 1_431_655_765),
            (i32::MIN, -2),
            (i32::MAX, 2),
        ] {
            let effects = derive_compiled_post_helper_effects(0, divisor).unwrap();
            assert_eq!(post_helper_control_value(effects, 0x160), expected);
        }
    }

    #[test]
    fn post_helper_reciprocal_overflow_is_typed_and_atomic() {
        for control_word_15c in [-1, 1, 2] {
            assert_eq!(
                derive_compiled_post_helper_effects(123, control_word_15c),
                Err(CompiledPostHelperReciprocalOverflow { control_word_15c })
            );
        }
    }

    #[test]
    fn state_758_helper_preserves_every_non_sentinel_and_masks_fallback() {
        for value in [i32::MIN, -2, 0, 1, i32::MAX] {
            assert_eq!(
                derive_compiled_state_word_758(value, 0xff),
                CompiledStateWord758Effect {
                    state_word_758: value
                }
            );
        }
        for fallback in u8::MIN..=u8::MAX {
            assert_eq!(
                derive_compiled_state_word_758(-1, fallback).state_word_758,
                i32::from(fallback & 7)
            );
        }
    }

    #[test]
    fn control_product_pair_pins_offsets_zero_quarter_and_nominal_values() {
        assert_eq!(COMPILED_CONTROL_PRODUCT_OFFSETS, [0x164, 0x168]);
        assert_eq!(
            derive_compiled_control_product_pair(0x1_0000, 0x8000),
            Ok(CompiledControlProductPair {
                reciprocal_word_164: 0x8_0000,
                product_word_168: 0x8000,
            })
        );
        assert_eq!(
            derive_compiled_control_product_pair(1, 0x1_0000),
            Ok(CompiledControlProductPair {
                reciprocal_word_164: -1,
                product_word_168: 1,
            })
        );
        assert_eq!(
            derive_compiled_control_product_pair(3, 0x1_0000),
            Ok(CompiledControlProductPair {
                reciprocal_word_164: -1,
                product_word_168: 3,
            })
        );
        assert_eq!(
            derive_compiled_control_product_pair(-5, 0x1_0000),
            Ok(CompiledControlProductPair {
                reciprocal_word_164: i32::MIN,
                product_word_168: -5,
            })
        );
        assert_eq!(
            derive_compiled_control_product_pair(0x2_0000, 0x4000_0000),
            Ok(CompiledControlProductPair {
                reciprocal_word_164: -8,
                product_word_168: i32::MIN,
            })
        );
    }

    #[test]
    fn control_product_pair_pins_exact_idiv_overflow_and_safe_boundaries() {
        for (product, divisor) in [(-4, -1), (4, 1), (8, 2)] {
            assert_eq!(
                derive_compiled_control_product_pair(product, 0x1_0000),
                Err(CompiledControlProductReciprocalOverflow { divisor })
            );
        }
        assert_eq!(
            derive_compiled_control_product_pair(-8, 0x1_0000),
            Ok(CompiledControlProductPair {
                reciprocal_word_164: i32::MIN,
                product_word_168: -8,
            })
        );
        assert_eq!(
            derive_compiled_control_product_pair(12, 0x1_0000),
            Ok(CompiledControlProductPair {
                reciprocal_word_164: 0x5555_5555,
                product_word_168: 12,
            })
        );
        assert_eq!(
            derive_compiled_control_product_pair(-12, 0x1_0000),
            Ok(CompiledControlProductPair {
                reciprocal_word_164: -0x5555_5555,
                product_word_168: -12,
            })
        );
    }

    #[test]
    fn control_product_pair_keeps_wrapped_bits_16_through_47() {
        for (left, right) in [
            (i32::MIN, i32::MIN),
            (i32::MAX, i32::MAX),
            (i32::MIN, i32::MAX),
            (-0x0345_6789, 0x0765_4321),
        ] {
            let expected_product = ((i64::from(left) * i64::from(right)) >> 16) as i32;
            let expected_divisor = expected_product >> 2;
            let result = derive_compiled_control_product_pair(left, right);
            if [-1, 1, 2].contains(&expected_divisor) {
                assert_eq!(
                    result,
                    Err(CompiledControlProductReciprocalOverflow {
                        divisor: expected_divisor,
                    })
                );
            } else {
                assert_eq!(result.unwrap().product_word_168, expected_product);
            }
        }
    }

    #[test]
    fn suffix_defaults_pin_offsets_norm_anchor_and_raw_seed_values() {
        assert_eq!(
            COMPILED_SUFFIX_DEFAULT_STATE_OFFSETS,
            [0x044, 0x050, 0x054, 0x058]
        );
        assert_eq!(
            derive_compiled_suffix_defaults([0, 0, 0], 0),
            CompiledSuffixDefaultEffects {
                state_word_44: 0,
                state_words_50_54_58: [1, 2, 1],
            }
        );
        assert_eq!(
            derive_compiled_suffix_defaults([0x4_0000, 0, 0], 0).state_word_44,
            131_100
        );
    }

    #[test]
    fn suffix_defaults_exhaustively_use_only_config_mask_04() {
        for config_flags in u8::MIN..=u8::MAX {
            assert_eq!(
                derive_compiled_suffix_defaults([0; 3], config_flags).state_words_50_54_58,
                if config_flags & COMPILED_COMMON_STATE_SCALE_MASK == 0 {
                    [1, 2, 1]
                } else {
                    [2, 4, 2]
                }
            );
        }
    }

    #[test]
    fn suffix_defaults_use_arithmetic_halves_and_wrapping_q16_square_sum() {
        assert_eq!(
            derive_compiled_suffix_defaults([-0x2_0001, 0, 0], 0).state_word_44,
            sqrt_q16(0x1_0002) as i32
        );
        assert_eq!(
            derive_compiled_suffix_defaults([0x0100_0000; 3], 0).state_word_44,
            sqrt_q16(0xc000_0000) as i32
        );
        for words in [
            [i32::MIN, i32::MAX, -3],
            [-1, -5, 7],
            [0x4000_0000, 0x4000_0000, 0x4000_0000],
        ] {
            let halves = words.map(|word| word >> 1);
            let expected_sum = halves.into_iter().fold(0_i32, |sum, word| {
                sum.wrapping_add(((i64::from(word) * i64::from(word)) >> 16) as i32)
            });
            assert_eq!(
                derive_compiled_suffix_defaults(words, 0).state_word_44,
                sqrt_q16(expected_sum as u32) as i32
            );
        }
    }

    #[test]
    fn state_reciprocal_trio_pins_zero_truncation_and_signed_boundaries() {
        assert_eq!(COMPILED_STATE_RECIPROCAL_TRIO_OFFSETS, [0x38, 0x3c, 0x40]);
        assert_eq!(
            derive_compiled_state_reciprocal_trio([0, 0, 0]),
            Ok(CompiledStateReciprocalTrio { words: [-1; 3] })
        );
        assert_eq!(
            derive_compiled_state_reciprocal_trio([-2, 3, -3]),
            Ok(CompiledStateReciprocalTrio {
                words: [i32::MIN, 0x5555_5555, -0x5555_5555],
            })
        );
    }

    #[test]
    fn state_reciprocal_trio_reports_exact_fault_and_completed_store_order() {
        for (input, index, divisor, completed) in [
            ([1, 4, 4], 0, 1, [None, None, None]),
            ([0, 2, 4], 1, 2, [Some(-1), None, None]),
            (
                [-2, 3, -1],
                2,
                -1,
                [Some(i32::MIN), Some(0x5555_5555), None],
            ),
        ] {
            assert_eq!(
                derive_compiled_state_reciprocal_trio(input),
                Err(CompiledStateReciprocalTrioOverflow {
                    index,
                    divisor,
                    completed,
                })
            );
        }
    }

    #[test]
    fn control_clear_loop_pins_interleaved_store_order_and_zero_effect() {
        assert_eq!(
            COMPILED_CONTROL_CLEAR_LOOP_OFFSETS,
            [
                0x31c, 0x33c, 0x320, 0x340, 0x324, 0x344, 0x328, 0x348, 0x32c, 0x34c, 0x330, 0x350,
                0x334, 0x354, 0x338, 0x358,
            ]
        );
        let (pairs, remainder) = COMPILED_CONTROL_CLEAR_LOOP_OFFSETS.as_chunks::<2>();
        assert!(remainder.is_empty());
        for (iteration, pair) in pairs.iter().enumerate() {
            let step = u16::try_from(iteration).unwrap() * 4;
            assert_eq!(*pair, [0x31c + step, 0x33c + step]);
        }
        let mut canonical = COMPILED_CONTROL_CLEAR_LOOP_OFFSETS;
        canonical.sort_unstable();
        assert_eq!(
            canonical,
            [
                0x31c, 0x320, 0x324, 0x328, 0x32c, 0x330, 0x334, 0x338, 0x33c, 0x340, 0x344, 0x348,
                0x34c, 0x350, 0x354, 0x358,
            ]
        );
        assert_eq!(
            derive_compiled_control_clear_loop(),
            CompiledControlClearLoop { words: [0; 16] }
        );
    }

    #[test]
    fn half_sign_state_pins_exact_store_schema_and_sign_combinations() {
        assert_eq!(
            COMPILED_HALF_SIGN_STATE_OFFSETS,
            [
                0x02c, 0x030, 0x034, 0x068, 0x06c, 0x070, 0x05c, 0x060, 0x064, 0x074, 0x078, 0x07c,
                0x080, 0x084, 0x088, 0x098, 0x09c, 0x0a0, 0x08c, 0x090, 0x094, 0x0a4, 0x0a8, 0x0ac,
                0x0b0, 0x0b4, 0x0b8,
            ]
        );
        assert_eq!(
            derive_compiled_half_sign_state([3, -5, 7]).words,
            [
                1, -3, 3, 1, 3, -3, 1, 3, 3, -1, 3, 3, -1, 3, -3, 1, -3, -3, 1, -3, 3, -1, -3, 3,
                -1, -3, -3,
            ]
        );

        let mut canonical = COMPILED_HALF_SIGN_STATE_OFFSETS;
        canonical.sort_unstable();
        assert_eq!(
            canonical,
            [
                0x02c, 0x030, 0x034, 0x05c, 0x060, 0x064, 0x068, 0x06c, 0x070, 0x074, 0x078, 0x07c,
                0x080, 0x084, 0x088, 0x08c, 0x090, 0x094, 0x098, 0x09c, 0x0a0, 0x0a4, 0x0a8, 0x0ac,
                0x0b0, 0x0b4, 0x0b8,
            ]
        );
    }

    #[test]
    fn half_sign_state_uses_arithmetic_halves_at_signed_boundaries() {
        let effects = derive_compiled_half_sign_state([i32::MIN, i32::MAX, -1]);
        let x = -0x4000_0000;
        let y = 0x3fff_ffff;
        let z = -1;
        assert_eq!(&effects.words[..3], &[x, y, z]);
        assert_eq!(
            &effects.words[12..15],
            &[x.wrapping_neg(), y.wrapping_neg(), 1]
        );
        assert_eq!(&effects.words[24..], &[x.wrapping_neg(), y, 1]);
    }

    #[test]
    fn mixed_reset_pins_widths_offsets_and_mask_for_every_flag_byte() {
        assert_eq!(
            COMPILED_MIXED_RESET_CONTROL_OFFSETS,
            [
                0x2a8, 0x2b8, 0x2ac, 0x2bc, 0x2b0, 0x2c0, 0x2b4, 0x2c4, 0x250, 0x254
            ]
        );
        assert_eq!(
            COMPILED_MIXED_RESET_STATE_DWORD_OFFSETS,
            [0x584, 0x588, 0x58c, 0x590]
        );
        assert_eq!(
            COMPILED_MIXED_RESET_STATE_U16_OFFSETS,
            [0x57c, 0x57e, 0x580, 0x582]
        );
        for flags in u8::MIN..=u8::MAX {
            assert_eq!(
                derive_compiled_mixed_reset_effects(flags),
                CompiledMixedResetEffects {
                    control_words: [0; 10],
                    state_words: [if flags & COMPILED_MIXED_RESET_NEGATIVE_VALUE_MASK == 0 {
                        0
                    } else {
                        -0x8000
                    }; 4],
                    state_u16_words: [0; 4],
                }
            );
        }
    }

    #[test]
    fn call_spine_plan_pins_barriers_and_caller_local_mixed_width_effects() {
        assert_eq!(
            COMPILED_CALL_SPINE_STEPS,
            [
                CompiledCallSpineStep::Call {
                    target_va: 0x409c10,
                    stack_arg_count: 1,
                    pointer_source: CompiledCallPointerSource::CapturedState,
                },
                CompiledCallSpineStep::ControlU16Clear,
                CompiledCallSpineStep::GlobalSeed,
                CompiledCallSpineStep::Call {
                    target_va: 0x419a40,
                    stack_arg_count: 2,
                    pointer_source: CompiledCallPointerSource::CapturedStateAndControl,
                },
                CompiledCallSpineStep::Call {
                    target_va: 0x408fe0,
                    stack_arg_count: 0,
                    pointer_source: CompiledCallPointerSource::ReloadedGlobalState,
                },
            ]
        );
        assert_eq!(
            COMPILED_CALL_SPINE_CONTROL_U16_OFFSETS,
            [0x3d0, 0x3d2, 0x3d4, 0x3d6, 0x3d8, 0x3da, 0x3dc, 0x3de]
        );
        assert_eq!(
            COMPILED_CALL_SPINE_GLOBAL_ADDRESSES,
            [0x50c5f0, 0x50c5f4, 0x50c5f8]
        );
        assert_eq!(
            derive_compiled_call_spine_local_effects(),
            CompiledCallSpineLocalEffects {
                control_u16_words: [0; 8],
                global_words: [0, -0x8000, 0],
            }
        );

        let represented_control_dwords: Vec<u16> = COMPILED_CONTROL_WORD_OFFSETS
            .into_iter()
            .chain(COMPILED_COMMON_PREFIX_CONTROL_OFFSETS)
            .chain(AUX_CONTROL_WORD_OFFSETS)
            .chain(COMPILED_POST_HELPER_CONTROL_WORD_OFFSETS)
            .chain(COMPILED_CONTROL_PRODUCT_OFFSETS)
            .chain(COMPILED_CONTROL_CLEAR_LOOP_OFFSETS)
            .chain(COMPILED_MIXED_RESET_CONTROL_OFFSETS)
            .chain(
                (0..COMPILED_CONTROL_FAMILY_WORD_COUNT as u16)
                    .flat_map(|index| [0x1f0 + index * 4, 0x210 + index * 4, 0x230 + index * 4]),
            )
            .chain([0x170])
            .collect();
        for (index, offset) in COMPILED_CALL_SPINE_CONTROL_U16_OFFSETS
            .into_iter()
            .enumerate()
        {
            let start = u32::from(offset);
            let end = start + 2;
            for (other_index, other) in COMPILED_CALL_SPINE_CONTROL_U16_OFFSETS
                .into_iter()
                .enumerate()
            {
                if index != other_index {
                    let other_start = u32::from(other);
                    assert!(end <= other_start || other_start + 2 <= start);
                }
            }
            for dword in &represented_control_dwords {
                let dword_start = u32::from(*dword);
                assert!(end <= dword_start || dword_start + 4 <= start);
            }
            for existing_u16 in COMPILED_POST_HELPER_CONTROL_U16_OFFSETS {
                let existing_start = u32::from(existing_u16);
                assert!(end <= existing_start || existing_start + 2 <= start);
            }
            for existing_byte in COMPILED_POST_HELPER_CONTROL_BYTE_OFFSETS {
                let existing_start = u32::from(existing_byte);
                assert!(end <= existing_start || existing_start < start);
            }
        }
    }

    fn state_basis_input() -> CompiledStateBasisInputs {
        CompiledStateBasisInputs {
            half_words: [2, 3, 5],
            matrix_rows: [[0x1_0000, 0, 0], [0, 0x1_0000, 0], [0, 0, 0x1_0000]],
            center: [10, 20, 30],
            adjustment_u: [0; 3],
            adjustment_v: [0; 3],
            scale_638: 0,
            scale_63c: 0,
            scale_640: 0,
        }
    }

    #[test]
    fn state_basis_pins_exact_schemas_and_all_eight_sign_vectors() {
        assert_eq!(
            COMPILED_STATE_BASIS_MATRIX_OFFSETS,
            [[0x00, 0x04, 0x08], [0x10, 0x14, 0x18], [0x20, 0x24, 0x28]]
        );
        assert_eq!(
            COMPILED_STATE_BASIS_VECTOR_BASES,
            [0x0fc, 0x108, 0x114, 0x120, 0x12c, 0x138, 0x144, 0x150]
        );
        assert_eq!(
            COMPILED_STATE_BASIS_OFFSETS,
            std::array::from_fn(|index| 0x0fc + u16::try_from(index).unwrap() * 4)
        );
        assert_eq!(
            COMPILED_STATE_BASIS_ADJUSTMENT_STORE_OFFSETS,
            [
                0x12c, 0x130, 0x134, 0x138, 0x13c, 0x140, 0x144, 0x148, 0x14c, 0x154, 0x150, 0x158
            ]
        );
        assert_eq!(
            derive_compiled_state_basis_effects(state_basis_input()).vectors,
            [
                [12, 17, 35],
                [12, 17, 25],
                [8, 17, 35],
                [8, 17, 25],
                [12, 23, 35],
                [12, 23, 25],
                [8, 23, 35],
                [8, 23, 25],
            ]
        );
    }

    #[test]
    fn state_basis_routes_asymmetric_matrix_rows_to_components() {
        let mut input = state_basis_input();
        input.matrix_rows = [
            [0x1_0000, 0x2_0000, 0x3_0000],
            [0x4_0000, 0x5_0000, 0x6_0000],
            [0x7_0000, 0x8_0000, 0x9_0000],
        ];
        input.center = [0; 3];
        assert_eq!(
            derive_compiled_state_basis_effects(input).vectors,
            [
                [25, 29, 33],
                [-45, -51, -57],
                [21, 21, 21],
                [-49, -59, -69],
                [49, 59, 69],
                [-21, -21, -21],
                [45, 51, 57],
                [-25, -29, -33],
            ]
        );
    }

    #[test]
    fn state_basis_adjustment_pins_each_helper_sign_and_pre_adjustment_reflections() {
        let baseline = derive_compiled_state_basis_effects(state_basis_input());
        let mut input = state_basis_input();
        input.adjustment_u = [1, 2, 3];
        input.adjustment_v = [4, 5, 6];
        input.scale_638 = 0x1_0000;
        input.scale_63c = 0x1_0000;
        input.scale_640 = 0x1_0000;
        let effects = derive_compiled_state_basis_effects(input);
        assert_eq!(&effects.vectors[..4], &baseline.vectors[..4]);
        assert_eq!(
            &effects.vectors[4..],
            &[[7, 16, 26], [15, 26, 28], [5, 20, 32], [13, 30, 34]]
        );
    }

    #[test]
    fn state_basis_center_translates_all_eight_world_points() {
        let baseline = derive_compiled_state_basis_effects(state_basis_input());
        let mut input = state_basis_input();
        input.center = [-100, 200, -300];
        let changed = derive_compiled_state_basis_effects(input);
        assert_ne!(&changed.vectors[..4], &baseline.vectors[..4]);
        assert_ne!(&changed.vectors[4..], &baseline.vectors[4..]);
        for index in 0..8 {
            assert_eq!(
                changed.vectors[index],
                std::array::from_fn(|component| {
                    baseline.vectors[index][component].wrapping_add(
                        input.center[component] - state_basis_input().center[component],
                    )
                })
            );
        }
    }

    #[test]
    fn state_basis_q16_projection_and_outer_combination_wrap_at_signed_boundaries() {
        assert_eq!(common_prefix_q16_mul(-1, 0x8000), -1);
        assert_eq!(common_prefix_q16_mul(-3, 0x8000), -2);
        assert_eq!(common_prefix_q16_mul(i32::MIN, i32::MAX), 0x8000);

        let input = CompiledStateBasisInputs {
            half_words: [0x1_0000; 3],
            matrix_rows: [[i32::MAX, i32::MIN, -1], [1, -1, i32::MIN], [1, 1, 1]],
            center: [i32::MIN, i32::MAX, 0],
            adjustment_u: [0; 3],
            adjustment_v: [0; 3],
            scale_638: 0,
            scale_63c: 0,
            scale_640: 0,
        };
        assert_eq!(
            derive_compiled_state_basis_effects(input).vectors,
            [
                [-1, 1, i32::MIN],
                [-3, -1, 2_147_483_646],
                [1, 1, -2_147_483_646],
                [-1, -1, i32::MIN],
                [1, -1, i32::MIN],
                [-1, -3, 2_147_483_646],
                [3, -1, -2_147_483_646],
                [1, -3, i32::MIN],
            ]
        );
    }

    #[test]
    fn state_basis_adjustment_is_reusable_and_wraps_every_operation() {
        let primary = [[i32::MAX; 3], [i32::MIN; 3], [-1; 3], [1; 3]];
        assert_eq!(
            apply_compiled_state_basis_adjustment(primary, [0; 3], [0; 3], 0, 0, 0),
            primary
        );
        let adjusted = apply_compiled_state_basis_adjustment(
            primary,
            [i32::MIN, i32::MAX, -3],
            [i32::MAX, i32::MIN, 5],
            i32::MAX,
            i32::MIN,
            -0x1_0000,
        );
        let p = [i32::MIN, i32::MAX, -3].map(|word| common_prefix_q16_mul(word, i32::MAX));
        let q = [i32::MAX, i32::MIN, 5].map(|word| common_prefix_q16_mul(word, -0x1_0000));
        let r = [i32::MIN, i32::MAX, -3].map(|word| common_prefix_q16_mul(word, i32::MIN));
        for component in 0..3 {
            assert_eq!(
                adjusted[0][component],
                i32::MAX.wrapping_sub(p[component].wrapping_add(q[component]))
            );
            assert_eq!(
                adjusted[1][component],
                i32::MIN.wrapping_add(q[component].wrapping_sub(p[component]))
            );
            assert_eq!(
                adjusted[2][component],
                (-1_i32).wrapping_add(r[component].wrapping_sub(q[component]))
            );
            assert_eq!(
                adjusted[3][component],
                1_i32.wrapping_add(r[component].wrapping_add(q[component]))
            );
        }
    }

    fn lane_postprocess_input() -> CompiledEightLanePostprocessInputs {
        CompiledEightLanePostprocessInputs {
            post_dispatch: std::array::from_fn(|index| CompiledLanePostDispatchSnapshot {
                lane_word: i32::try_from(index).unwrap() + 100,
                type_word: 0,
            }),
            gate_words: [0; 8],
            drive_words: [0; 8],
            accumulator_words: [0; 8],
            first_four_subtractions: [0; 4],
            config_flag_26: [0; 8],
        }
    }

    #[test]
    fn eight_lane_postprocess_pins_every_strided_schema_and_caller_write() {
        assert_eq!(
            COMPILED_LANE_SOURCE_VECTOR_BASES,
            COMPILED_STATE_BASIS_VECTOR_BASES
        );
        assert_eq!(
            COMPILED_LANE_OUTPUT_VECTOR_BASES,
            std::array::from_fn(|index| 0x33c + u16::try_from(index).unwrap() * 12)
        );
        assert_eq!(
            COMPILED_LANE_WORD_OFFSETS,
            std::array::from_fn(|index| 0x2e4 + u16::try_from(index).unwrap() * 4)
        );
        assert_eq!(
            COMPILED_LANE_TYPE_U16_OFFSETS,
            std::array::from_fn(|index| 0x42a + u16::try_from(index).unwrap() * 2)
        );
        assert_eq!(
            COMPILED_LANE_HANDLE_I16_OFFSETS,
            COMPILED_CALL_SPINE_CONTROL_U16_OFFSETS
        );
        assert_eq!(
            COMPILED_LANE_GATE_OFFSETS,
            std::array::from_fn(|index| 0x69c + u16::try_from(index).unwrap() * 4)
        );
        assert_eq!(
            COMPILED_LANE_DRIVE_OFFSETS,
            std::array::from_fn(|index| 0x304 + u16::try_from(index).unwrap() * 4)
        );
        assert_eq!(
            COMPILED_LANE_ACCUMULATOR_OFFSETS,
            std::array::from_fn(|index| 0x618 + u16::try_from(index).unwrap() * 4)
        );
        assert_eq!(
            COMPILED_LANE_FIRST_FOUR_SUBTRACTION_OFFSETS,
            [0x5e8, 0x5f0, 0x5f8, 0x600]
        );
        assert_eq!(COMPILED_LANE_LATCH_OFFSET, 0x67c);
        assert_eq!(COMPILED_LANE_CONFIG_FLAG_OFFSET, 0x26);
        assert_eq!(COMPILED_LANE_ACCUMULATOR_LATCH_THRESHOLD, 0x00c8_9cbc);

        let mut state_destination_ranges = Vec::new();
        state_destination_ranges
            .extend(COMPILED_LANE_WORD_OFFSETS.map(|offset| (offset, offset + 4)));
        state_destination_ranges
            .extend(COMPILED_LANE_OUTPUT_VECTOR_BASES.map(|offset| (offset, offset + 12)));
        state_destination_ranges
            .extend(COMPILED_LANE_ACCUMULATOR_OFFSETS.map(|offset| (offset, offset + 4)));
        state_destination_ranges.push((COMPILED_LANE_LATCH_OFFSET, COMPILED_LANE_LATCH_OFFSET + 4));
        for left in 0..state_destination_ranges.len() {
            for right in left + 1..state_destination_ranges.len() {
                let (left_start, left_end) = state_destination_ranges[left];
                let (right_start, right_end) = state_destination_ranges[right];
                assert!(left_end <= right_start || right_end <= left_start);
            }
        }
        for type_offset in COMPILED_LANE_TYPE_U16_OFFSETS {
            for handle_offset in COMPILED_LANE_HANDLE_I16_OFFSETS {
                assert!(type_offset + 2 <= handle_offset || handle_offset + 2 <= type_offset);
            }
        }

        let input = lane_postprocess_input();
        let effects = derive_compiled_eight_lane_postprocess(input);
        assert_eq!(
            effects.lane_words,
            input.post_dispatch.map(|lane| lane.lane_word)
        );
        assert_eq!(effects.output_vector_writes, [None; 8]);
        assert_eq!(effects.type_word_writes, [None; 8]);
        assert_eq!(effects.accumulator_writes, [None; 8]);
        assert_eq!(effects.latch_67c, None);
    }

    #[test]
    fn lane_type_effect_exhaustively_partitions_all_u16_words() {
        let mut reset = Vec::new();
        let mut reduce = Vec::new();
        let mut accumulate = Vec::new();
        for word in 0..=u16::MAX {
            match compiled_lane_type_effect(word) {
                CompiledLaneTypeEffect::None => {}
                CompiledLaneTypeEffect::ReduceAndResetVector => reset.push(word),
                CompiledLaneTypeEffect::Reduce => reduce.push(word),
                CompiledLaneTypeEffect::ReduceAndAccumulate => accumulate.push(word),
            }
        }
        assert_eq!(reset, [0x3b, 0x3f, 0x40]);
        assert_eq!(reduce, [0x51, 0x53]);
        assert_eq!(accumulate, [0x52]);
    }

    #[test]
    fn eight_lane_postprocess_preserves_native_stage_order_and_late_overrides() {
        let mut input = lane_postprocess_input();
        input.post_dispatch[0].type_word = 0x3b;
        input.gate_words[0] = 1;
        input.drive_words[0] = 0x1_0000;
        input.first_four_subtractions[0] = 7;
        input.config_flag_26[0] = 0xfe;

        input.post_dispatch[1].type_word = 0x51;
        input.first_four_subtractions[1] = -9;

        input.post_dispatch[4].lane_word = 1000;
        input.post_dispatch[4].type_word = 0x52;
        input.accumulator_words[4] = 0x00c7_9cbc;

        let effects = derive_compiled_eight_lane_postprocess(input);
        assert_eq!(effects.lane_words[0], 85_290);
        assert_eq!(effects.output_vector_writes[0], Some([0, 0x1_0000, 0]));
        assert_eq!(effects.type_word_writes[0], Some(0x000a));
        assert_eq!(effects.lane_words[1], 110_i32.wrapping_sub(0x4ccc));
        assert_eq!(effects.output_vector_writes[1], None);
        assert_eq!(effects.lane_words[4], -57_783);
        assert_eq!(effects.accumulator_writes[4], Some(0x00c8_9cbc));
        assert_eq!(effects.latch_67c, Some(1));
    }

    #[test]
    fn eight_lane_postprocess_uses_signed_upper_only_clamp_and_lane_local_config() {
        let mut input = lane_postprocess_input();
        input.gate_words[4] = 1;
        input.drive_words[4] = -1;
        input.gate_words[5] = 1;
        input.drive_words[5] = 0x1_0000;
        input.post_dispatch[6].lane_word = i32::MAX;
        input.config_flag_26[6] = 0x02;
        input.config_flag_26[7] = 0x01;

        let effects = derive_compiled_eight_lane_postprocess(input);
        assert_eq!(effects.lane_words[4], 102);
        assert_eq!(effects.lane_words[5], 105_i32.wrapping_add(0x9999));
        assert_eq!(effects.lane_words[6], i32::MIN + 0xffff);
        assert_eq!(effects.type_word_writes[6], Some(0x000a));
        assert_eq!(effects.output_vector_writes[6], Some([0, 0x1_0000, 0]));
        assert_eq!(effects.lane_words[7], 107);
        assert_eq!(effects.type_word_writes[7], None);
    }

    #[test]
    fn eight_lane_accumulator_latch_uses_updated_signed_word_and_wraps() {
        let mut below = lane_postprocess_input();
        below.post_dispatch[0].type_word = 0x52;
        below.accumulator_words[0] = 0x00c7_9cbb;
        assert_eq!(
            derive_compiled_eight_lane_postprocess(below).latch_67c,
            None
        );

        let mut wrapping = lane_postprocess_input();
        wrapping.post_dispatch[6].lane_word = i32::MIN.wrapping_add(0x4ccc);
        wrapping.post_dispatch[6].type_word = 0x52;
        wrapping.accumulator_words[6] = 0x1_0000;
        wrapping.post_dispatch[7].type_word = 0x52;
        wrapping.accumulator_words[7] = i32::MAX;
        let effects = derive_compiled_eight_lane_postprocess(wrapping);
        assert_eq!(effects.lane_words[6], i32::MIN.wrapping_sub(0xc4));
        assert_eq!(effects.accumulator_writes[6], Some(0x2_0000));
        assert_eq!(effects.accumulator_writes[7], Some(i32::MIN + 0xffff));
        assert_eq!(effects.latch_67c, None);
    }

    #[test]
    fn eight_lane_word_arithmetic_wraps_and_first_four_cutoff_is_exact() {
        let mut input = lane_postprocess_input();
        input.post_dispatch[0].lane_word = i32::MAX;
        input.gate_words[0] = 1;
        input.drive_words[0] = 0x1_0000;
        input.post_dispatch[1].lane_word = i32::MIN;
        input.post_dispatch[1].type_word = 0x51;
        input.post_dispatch[2].lane_word = i32::MIN;
        input.first_four_subtractions = [0, 0, 1, 4];
        input.post_dispatch[4].lane_word = 104;

        let effects = derive_compiled_eight_lane_postprocess(input);
        assert_eq!(effects.lane_words[0], i32::MAX.wrapping_add(0x9999));
        assert_eq!(effects.lane_words[1], i32::MIN.wrapping_sub(0x4ccc));
        assert_eq!(effects.lane_words[2], i32::MAX);
        assert_eq!(effects.lane_words[3], 99);
        assert_eq!(effects.lane_words[4], 104);
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum DispatcherCall {
        Validate(i16),
        Search(i32),
        Build(i16, i32),
    }

    struct ScriptedGeometryProvider {
        validate: Option<Result<bool, &'static str>>,
        search: Option<Result<Option<i16>, &'static str>>,
        build: Option<Result<CompiledLaneBuildCompletion, &'static str>>,
        calls: Vec<DispatcherCall>,
    }

    impl ScriptedGeometryProvider {
        fn new(
            validate: Option<Result<bool, &'static str>>,
            search: Option<Result<Option<i16>, &'static str>>,
            build: Option<Result<CompiledLaneBuildCompletion, &'static str>>,
        ) -> Self {
            Self {
                validate,
                search,
                build,
                calls: Vec::new(),
            }
        }
    }

    impl CompiledLaneGeometryProvider for ScriptedGeometryProvider {
        type Error = &'static str;

        fn validate(&mut self, _source: [i32; 3], handle: i16) -> Result<bool, Self::Error> {
            self.calls.push(DispatcherCall::Validate(handle));
            self.validate.take().expect("unexpected validate call")
        }

        fn search(&mut self, _source: [i32; 3], fallback: i32) -> Result<Option<i16>, Self::Error> {
            self.calls.push(DispatcherCall::Search(fallback));
            self.search.take().expect("unexpected search call")
        }

        fn build(
            &mut self,
            _source: [i32; 3],
            handle: i16,
            fallback: i32,
        ) -> Result<CompiledLaneBuildCompletion, Self::Error> {
            self.calls.push(DispatcherCall::Build(handle, fallback));
            self.build.take().expect("unexpected build call")
        }
    }

    fn dispatch_input(handle: i16, fallback: i32) -> CompiledLaneDispatchInput {
        CompiledLaneDispatchInput {
            source_vector: [11, 22, 33],
            initial_handle: handle,
            fallback,
        }
    }

    #[test]
    fn lane_dispatcher_exhausts_signed_handle_partition_and_redundant_normalization() {
        for raw in 0..=u16::MAX {
            let handle = raw as i16;
            let mut provider = if handle < 0 {
                ScriptedGeometryProvider::new(None, Some(Ok(None)), None)
            } else {
                ScriptedGeometryProvider::new(Some(Ok(false)), Some(Ok(None)), None)
            };
            let completion = dispatch_compiled_lane(dispatch_input(handle, 77), &mut provider)
                .expect("scripted no-fault path");
            assert_eq!(completion.lane_word, 77);
            assert_eq!(completion.handle_writes.normalized_to_minus_one, handle < 0);
            assert_eq!(
                completion.handle_writes.invalidated_after_search_miss,
                handle >= 0
            );
            if handle < 0 {
                assert_eq!(provider.calls, [DispatcherCall::Search(77)]);
            } else {
                assert_eq!(
                    provider.calls,
                    [DispatcherCall::Validate(handle), DispatcherCall::Search(77)]
                );
            }
        }
    }

    #[test]
    fn lane_dispatcher_pins_all_successful_branch_orders_and_provider_ownership() {
        let built = CompiledLaneBuildCompletion::Built {
            lane_word: i32::MIN,
            type_word: 0xffff,
            output_vector: [i32::MIN, 0, i32::MAX],
        };
        let mut negative_hit =
            ScriptedGeometryProvider::new(None, Some(Ok(Some(7))), Some(Ok(built)));
        let completion = dispatch_compiled_lane(dispatch_input(-2, 9), &mut negative_hit).unwrap();
        assert_eq!(
            negative_hit.calls,
            [DispatcherCall::Search(9), DispatcherCall::Build(7, 9)]
        );
        assert_eq!(completion.lane_word, i32::MIN);
        assert_eq!(completion.type_word_write, Some(0xffff));
        assert_eq!(
            completion.output_vector_write,
            Some([i32::MIN, 0, i32::MAX])
        );
        assert_eq!(
            completion.handle_writes,
            CompiledLaneHandleWriteTrace {
                normalized_to_minus_one: true,
                searched_handle: Some(7),
                invalidated_after_search_miss: false,
            }
        );

        let mut valid = ScriptedGeometryProvider::new(
            Some(Ok(true)),
            None,
            Some(Ok(CompiledLaneBuildCompletion::Unavailable)),
        );
        let completion = dispatch_compiled_lane(dispatch_input(0, -77), &mut valid).unwrap();
        assert_eq!(
            valid.calls,
            [DispatcherCall::Validate(0), DispatcherCall::Build(0, -77)]
        );
        assert_eq!(completion.lane_word, 0);
        assert_eq!(completion.type_word_write, None);
        assert_eq!(completion.output_vector_write, None);

        let mut invalid_hit =
            ScriptedGeometryProvider::new(Some(Ok(false)), Some(Ok(Some(-9))), Some(Ok(built)));
        dispatch_compiled_lane(dispatch_input(42, 5), &mut invalid_hit).unwrap();
        assert_eq!(
            invalid_hit.calls,
            [
                DispatcherCall::Validate(42),
                DispatcherCall::Search(5),
                DispatcherCall::Build(-9, 5)
            ]
        );
    }

    #[test]
    fn lane_dispatcher_preserves_orchestration_write_prefixes_on_provider_errors() {
        let mut validate_error = ScriptedGeometryProvider::new(Some(Err("v")), None, None);
        let error = dispatch_compiled_lane(dispatch_input(3, 4), &mut validate_error).unwrap_err();
        assert_eq!(error.phase, CompiledLaneDispatchFailurePhase::Validate);
        assert_eq!(error.handle_writes, CompiledLaneHandleWriteTrace::default());

        let mut search_error = ScriptedGeometryProvider::new(None, Some(Err("s")), None);
        let error = dispatch_compiled_lane(dispatch_input(-1, 4), &mut search_error).unwrap_err();
        assert_eq!(error.phase, CompiledLaneDispatchFailurePhase::Search);
        assert!(error.handle_writes.normalized_to_minus_one);

        let mut invalid_search_error =
            ScriptedGeometryProvider::new(Some(Ok(false)), Some(Err("is")), None);
        let error =
            dispatch_compiled_lane(dispatch_input(3, 4), &mut invalid_search_error).unwrap_err();
        assert_eq!(error.phase, CompiledLaneDispatchFailurePhase::Search);
        assert_eq!(error.handle_writes, CompiledLaneHandleWriteTrace::default());
        assert_eq!(
            invalid_search_error.calls,
            [DispatcherCall::Validate(3), DispatcherCall::Search(4)]
        );

        let mut direct_build_error =
            ScriptedGeometryProvider::new(Some(Ok(true)), None, Some(Err("db")));
        let error =
            dispatch_compiled_lane(dispatch_input(3, 4), &mut direct_build_error).unwrap_err();
        assert_eq!(error.phase, CompiledLaneDispatchFailurePhase::Build);
        assert_eq!(error.handle_writes, CompiledLaneHandleWriteTrace::default());

        let mut build_error =
            ScriptedGeometryProvider::new(Some(Ok(false)), Some(Ok(Some(9))), Some(Err("b")));
        let error = dispatch_compiled_lane(dispatch_input(3, 4), &mut build_error).unwrap_err();
        assert_eq!(error.phase, CompiledLaneDispatchFailurePhase::Build);
        assert_eq!(error.handle_writes.searched_handle, Some(9));
        assert!(!error.handle_writes.invalidated_after_search_miss);

        let mut normalized_build_error =
            ScriptedGeometryProvider::new(None, Some(Ok(Some(7))), Some(Err("nb")));
        let error =
            dispatch_compiled_lane(dispatch_input(i16::MIN, 4), &mut normalized_build_error)
                .unwrap_err();
        assert_eq!(error.phase, CompiledLaneDispatchFailurePhase::Build);
        assert!(error.handle_writes.normalized_to_minus_one);
        assert_eq!(error.handle_writes.searched_handle, Some(7));
    }

    #[derive(Clone, Copy)]
    enum IntegratedLaneMode {
        Built(u16),
        SearchMiss,
        SearchThenBuildFailure,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum IntegratedLaneEvent {
        Validate(usize),
        Search(usize),
        Build(usize),
        Config(usize),
    }

    struct IntegratedLaneProvider {
        lane: usize,
        mode: IntegratedLaneMode,
        events: Rc<RefCell<Vec<IntegratedLaneEvent>>>,
    }

    impl CompiledLaneGeometryProvider for IntegratedLaneProvider {
        type Error = &'static str;

        fn validate(&mut self, _source: [i32; 3], _handle: i16) -> Result<bool, Self::Error> {
            self.events
                .borrow_mut()
                .push(IntegratedLaneEvent::Validate(self.lane));
            Ok(!matches!(self.mode, IntegratedLaneMode::SearchMiss))
        }

        fn search(
            &mut self,
            _source: [i32; 3],
            _fallback: i32,
        ) -> Result<Option<i16>, Self::Error> {
            self.events
                .borrow_mut()
                .push(IntegratedLaneEvent::Search(self.lane));
            Ok(match self.mode {
                IntegratedLaneMode::SearchMiss => None,
                IntegratedLaneMode::SearchThenBuildFailure => Some(7),
                IntegratedLaneMode::Built(_) => panic!("unexpected search"),
            })
        }

        fn build(
            &mut self,
            _source: [i32; 3],
            _handle: i16,
            _fallback: i32,
        ) -> Result<CompiledLaneBuildCompletion, Self::Error> {
            self.events
                .borrow_mut()
                .push(IntegratedLaneEvent::Build(self.lane));
            match self.mode {
                IntegratedLaneMode::Built(type_word) => Ok(CompiledLaneBuildCompletion::Built {
                    lane_word: 100 + i32::try_from(self.lane).unwrap(),
                    type_word,
                    output_vector: [i32::try_from(self.lane).unwrap(); 3],
                }),
                IntegratedLaneMode::SearchThenBuildFailure => Err("build"),
                IntegratedLaneMode::SearchMiss => panic!("unexpected build"),
            }
        }
    }

    fn integrated_execution_input() -> CompiledEightLaneExecutionInputs {
        CompiledEightLaneExecutionInputs {
            source_vectors: std::array::from_fn(|lane| [i32::try_from(lane).unwrap(); 3]),
            gate_words: [0; 8],
            drive_words: [0; 8],
            first_four_subtractions: [1, 2, 3, 4],
        }
    }

    fn integrated_execution_state() -> CompiledEightLaneExecutionState {
        CompiledEightLaneExecutionState {
            handles: [0; 8],
            type_words: [0; 8],
            output_vectors: [[-1; 3]; 8],
            lane_words: std::array::from_fn(|lane| 200 + i32::try_from(lane).unwrap()),
            accumulator_words: [0; 8],
            latch_67c: 73,
        }
    }

    fn integrated_providers(
        events: &Rc<RefCell<Vec<IntegratedLaneEvent>>>,
        modes: [IntegratedLaneMode; 8],
    ) -> [IntegratedLaneProvider; 8] {
        std::array::from_fn(|lane| IntegratedLaneProvider {
            lane,
            mode: modes[lane],
            events: Rc::clone(events),
        })
    }

    #[test]
    fn eight_lane_executor_interleaves_dispatch_and_late_config_and_preserves_write_layers() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(
            &events,
            [
                IntegratedLaneMode::Built(0x52),
                IntegratedLaneMode::SearchMiss,
                IntegratedLaneMode::Built(0x51),
                IntegratedLaneMode::Built(0),
                IntegratedLaneMode::SearchMiss,
                IntegratedLaneMode::Built(0),
                IntegratedLaneMode::Built(0),
                IntegratedLaneMode::Built(0),
            ],
        );
        let mut state = integrated_execution_state();
        state.handles[1] = 5;
        state.type_words[1] = 0x3b;
        state.accumulator_words[0] = COMPILED_LANE_ACCUMULATOR_LATCH_THRESHOLD - 0x1_0000;
        let config_events = Rc::clone(&events);
        let mut config = move |lane| -> Result<u8, &'static str> {
            config_events
                .borrow_mut()
                .push(IntegratedLaneEvent::Config(lane));
            Ok(if lane == 2 { 0x02 } else { 0 })
        };

        let success = execute_compiled_eight_lanes(
            integrated_execution_input(),
            state,
            &mut providers,
            &mut config,
        )
        .unwrap();

        let expected_events: Vec<_> = (0..8)
            .flat_map(|lane| {
                if lane == 1 || lane == 4 {
                    vec![
                        IntegratedLaneEvent::Validate(lane),
                        IntegratedLaneEvent::Search(lane),
                        IntegratedLaneEvent::Config(lane),
                    ]
                } else {
                    vec![
                        IntegratedLaneEvent::Validate(lane),
                        IntegratedLaneEvent::Build(lane),
                        IntegratedLaneEvent::Config(lane),
                    ]
                }
            })
            .collect();
        assert_eq!(*events.borrow(), expected_events);
        assert_eq!(success.state.handles[1], -1);
        assert_eq!(success.state.type_words[0], 0x52);
        assert_eq!(
            success.state.accumulator_words[0],
            COMPILED_LANE_ACCUMULATOR_LATCH_THRESHOLD
        );
        assert_eq!(success.state.latch_67c, 1);
        assert_eq!(success.state.lane_words[1], 201 - 0x4ccc - 2);
        assert_eq!(success.state.output_vectors[1], [0, 0x1_0000, 0]);
        assert_eq!(success.state.output_vectors[4], [-1; 3]);
        assert_eq!(success.state.type_words[4], 0);
        assert_eq!(success.lanes[2].dispatch.type_word_write, Some(0x51));
        assert_eq!(success.lanes[2].dispatch.output_vector_write, Some([2; 3]));
        assert_eq!(success.lanes[2].postprocess.type_word_write, Some(0x000a));
        assert_eq!(success.state.type_words[2], 0x000a);
        assert_eq!(success.state.output_vectors[2], [0, 0x1_0000, 0]);
        assert_eq!(success.state.lane_words[2], 102 - 0x4ccc - 3 + 0x1_0000);
    }

    #[test]
    fn runtime_four_lane_query_runs_lanes_zero_to_three_then_derives_the_hull_lanes() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let modes = [
            IntegratedLaneMode::Built(0x51),
            IntegratedLaneMode::SearchMiss,
            IntegratedLaneMode::Built(0),
            IntegratedLaneMode::Built(0x3b),
            IntegratedLaneMode::Built(0),
            IntegratedLaneMode::Built(0),
            IntegratedLaneMode::Built(0),
            IntegratedLaneMode::Built(0),
        ];
        let mut providers = integrated_providers(&events, modes);
        let mut config = |_lane| -> Result<u8, &'static str> { Ok(0) };
        let initial = integrated_execution_state();
        let runtime = execute_runtime_four_lanes(
            integrated_execution_input(),
            initial,
            &mut providers,
            &mut config,
        )
        .unwrap();
        // Only lanes 0..3 query the geometry.
        assert!(events.borrow().iter().all(|event| match event {
            IntegratedLaneEvent::Validate(lane)
            | IntegratedLaneEvent::Search(lane)
            | IntegratedLaneEvent::Build(lane)
            | IntegratedLaneEvent::Config(lane) => *lane < 4,
        }));
        // Lanes 0..3 match the eight-lane executor exactly.
        let mut eight_providers = integrated_providers(&Rc::new(RefCell::new(Vec::new())), modes);
        let eight = execute_compiled_eight_lanes(
            integrated_execution_input(),
            initial,
            &mut eight_providers,
            &mut |_lane| -> Result<u8, &'static str> { Ok(0) },
        )
        .unwrap();
        assert_eq!(runtime.lane_words[..4], eight.state.lane_words[..4]);
        assert_eq!(runtime.type_words[..4], eight.state.type_words[..4]);
        assert_eq!(runtime.output_vectors[..4], eight.state.output_vectors[..4]);
        // Lanes 4..7 take only `lane_word[i] - 5.0`.
        for lane in 0..4 {
            assert_eq!(
                runtime.lane_words[lane + 4],
                runtime.lane_words[lane].wrapping_sub(RUNTIME_HULL_LANE_OFFSET)
            );
            assert_eq!(runtime.type_words[lane + 4], initial.type_words[lane + 4]);
            assert_eq!(
                runtime.output_vectors[lane + 4],
                initial.output_vectors[lane + 4]
            );
            assert_eq!(runtime.handles[lane + 4], initial.handles[lane + 4]);
        }
    }

    #[test]
    fn eight_lane_executor_config_failure_preserves_same_lane_pre_config_prefix() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(
            &events,
            std::array::from_fn(|lane| IntegratedLaneMode::Built(if lane == 3 { 0x52 } else { 0 })),
        );
        let mut state = integrated_execution_state();
        state.accumulator_words[3] = 0x2_0000;
        let initial_lane_four = state.lane_words[4];
        let config_events = Rc::clone(&events);
        let mut config = move |lane| -> Result<u8, &'static str> {
            config_events
                .borrow_mut()
                .push(IntegratedLaneEvent::Config(lane));
            if lane == 3 { Err("config") } else { Ok(0) }
        };

        let failure = execute_compiled_eight_lanes(
            integrated_execution_input(),
            state,
            &mut providers,
            &mut config,
        )
        .unwrap_err();
        let CompiledEightLaneExecutionFailure::Config {
            failed_lane,
            progress,
            dispatch,
            pre_config_postprocess,
            config_error,
        } = failure
        else {
            panic!("expected config failure")
        };
        assert_eq!(failed_lane, 3);
        assert_eq!(progress.completed_lanes, 3);
        assert_eq!(config_error, "config");
        assert!(progress.lanes[..3].iter().all(Option::is_some));
        assert!(progress.lanes[3..].iter().all(Option::is_none));
        assert_eq!(dispatch.type_word_write, Some(0x52));
        let expected_lane_three = 103_i32
            .wrapping_sub(0x4ccc)
            .wrapping_sub(common_prefix_q16_mul(0x2_0000, 0xc4))
            .wrapping_sub(4);
        assert_eq!(expected_lane_three, -19_953);
        assert_eq!(pre_config_postprocess.lane_word, expected_lane_three);
        assert_eq!(progress.state.type_words[3], 0x52);
        assert_eq!(progress.state.output_vectors[3], [3; 3]);
        assert_eq!(progress.state.accumulator_words[3], 0x3_0000);
        assert_eq!(progress.state.lane_words[3], expected_lane_three);
        assert_eq!(progress.state.lane_words[4], initial_lane_four);
        assert!(!events.borrow().contains(&IntegratedLaneEvent::Validate(4)));
    }

    #[test]
    fn eight_lane_executor_dispatch_failure_keeps_handle_prefix_and_stops_before_config() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(
            &events,
            std::array::from_fn(|lane| {
                if lane == 3 {
                    IntegratedLaneMode::SearchThenBuildFailure
                } else {
                    IntegratedLaneMode::Built(0)
                }
            }),
        );
        let mut state = integrated_execution_state();
        state.handles[3] = -9;
        let initial_lane_three = state.lane_words[3];
        let config_events = Rc::clone(&events);
        let mut config = move |lane| -> Result<u8, &'static str> {
            config_events
                .borrow_mut()
                .push(IntegratedLaneEvent::Config(lane));
            Ok(0)
        };

        let failure = execute_compiled_eight_lanes(
            integrated_execution_input(),
            state,
            &mut providers,
            &mut config,
        )
        .unwrap_err();
        let CompiledEightLaneExecutionFailure::Dispatch {
            failed_lane,
            progress,
            failure,
        } = failure
        else {
            panic!("expected dispatch failure")
        };
        assert_eq!(failed_lane, 3);
        assert_eq!(progress.completed_lanes, 3);
        assert_eq!(failure.phase, CompiledLaneDispatchFailurePhase::Build);
        assert!(failure.handle_writes.normalized_to_minus_one);
        assert_eq!(failure.handle_writes.searched_handle, Some(7));
        assert_eq!(progress.state.handles[3], 7);
        assert_eq!(progress.state.lane_words[3], initial_lane_three);
        assert!(progress.lanes[..3].iter().all(Option::is_some));
        assert!(progress.lanes[3..].iter().all(Option::is_none));
        assert!(!events.borrow().contains(&IntegratedLaneEvent::Config(3)));
        assert!(!events.borrow().contains(&IntegratedLaneEvent::Validate(4)));
    }

    struct RecordingPostLane {
        calls: Rc<Cell<usize>>,
        fail: bool,
    }

    impl CompiledPostLaneTransform for RecordingPostLane {
        type Completion = [i32; 8];
        type Error = &'static str;

        fn run_after_lanes(
            &mut self,
            _input: &CompiledEightLaneExecutionInputs,
            lanes: &CompiledEightLaneExecutionSuccess,
        ) -> Result<Self::Completion, Self::Error> {
            self.calls.set(self.calls.get() + 1);
            if self.fail {
                Err("post")
            } else {
                Ok(lanes.state.lane_words)
            }
        }
    }

    #[test]
    fn lane_and_post_composition_invokes_post_once_after_complete_lane_success() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(&events, [IntegratedLaneMode::Built(0); 8]);
        let mut config = |_lane| -> Result<u8, &'static str> { Ok(0) };
        let calls = Rc::new(Cell::new(0));
        let mut post = RecordingPostLane {
            calls: Rc::clone(&calls),
            fail: false,
        };
        let success = execute_compiled_eight_lanes_then(
            integrated_execution_input(),
            integrated_execution_state(),
            &mut providers,
            &mut config,
            &mut post,
        )
        .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(success.post_lane, success.lanes.state.lane_words);
        assert_eq!(success.lanes.state.lane_words[7], 107);
    }

    #[test]
    fn lane_failure_skips_the_post_lane_stage() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(
            &events,
            std::array::from_fn(|lane| {
                if lane == 7 {
                    IntegratedLaneMode::SearchThenBuildFailure
                } else {
                    IntegratedLaneMode::Built(0)
                }
            }),
        );
        let mut state = integrated_execution_state();
        state.handles[7] = -1;
        let mut config = |_lane| -> Result<u8, &'static str> { Ok(0) };
        let calls = Rc::new(Cell::new(0));
        let mut post = RecordingPostLane {
            calls: Rc::clone(&calls),
            fail: false,
        };
        let failure = execute_compiled_eight_lanes_then(
            integrated_execution_input(),
            state,
            &mut providers,
            &mut config,
            &mut post,
        )
        .unwrap_err();
        assert_eq!(calls.get(), 0);
        assert!(matches!(
            failure,
            CompiledLaneAndPostFailure::Lanes(CompiledEightLaneExecutionFailure::Dispatch {
                failed_lane: 7,
                ..
            })
        ));
    }

    #[test]
    fn late_config_failure_skips_post_and_preserves_the_lane_prefix() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(&events, [IntegratedLaneMode::Built(0x52); 8]);
        let mut config =
            |lane| -> Result<u8, &'static str> { if lane == 7 { Err("config") } else { Ok(0) } };
        let calls = Rc::new(Cell::new(0));
        let mut post = RecordingPostLane {
            calls: Rc::clone(&calls),
            fail: false,
        };
        let failure = execute_compiled_eight_lanes_then(
            integrated_execution_input(),
            integrated_execution_state(),
            &mut providers,
            &mut config,
            &mut post,
        )
        .unwrap_err();
        assert_eq!(calls.get(), 0);
        let CompiledLaneAndPostFailure::Lanes(CompiledEightLaneExecutionFailure::Config {
            failed_lane,
            progress,
            dispatch,
            pre_config_postprocess,
            config_error,
        }) = failure
        else {
            panic!("expected lane config failure")
        };
        assert_eq!(failed_lane, 7);
        assert_eq!(progress.completed_lanes, 7);
        assert_eq!(config_error, "config");
        assert_eq!(dispatch.type_word_write, Some(0x52));
        assert_eq!(progress.state.type_words[7], 0x52);
        assert_eq!(
            progress.state.lane_words[7],
            pre_config_postprocess.lane_word
        );
        assert!(progress.lanes[..7].iter().all(Option::is_some));
        assert!(progress.lanes[7].is_none());
    }

    #[test]
    fn post_failure_retains_the_complete_eight_lane_success() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(&events, [IntegratedLaneMode::Built(0); 8]);
        let mut config = |_lane| -> Result<u8, &'static str> { Ok(0) };
        let calls = Rc::new(Cell::new(0));
        let mut post = RecordingPostLane {
            calls: Rc::clone(&calls),
            fail: true,
        };
        let failure = execute_compiled_eight_lanes_then(
            integrated_execution_input(),
            integrated_execution_state(),
            &mut providers,
            &mut config,
            &mut post,
        )
        .unwrap_err();
        let CompiledLaneAndPostFailure::PostLane { lanes, post_error } = failure else {
            panic!("expected post-lane failure")
        };
        assert_eq!(calls.get(), 1);
        assert_eq!(post_error, "post");
        assert_eq!(lanes.state.lane_words[7], 107);
        assert_eq!(lanes.lanes.len(), 8);
    }

    #[test]
    fn concrete_post_normal_uses_sources_final_lane_words_and_its_selector() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(
            &events,
            std::array::from_fn(|lane| IntegratedLaneMode::Built(if lane == 2 { 0x51 } else { 0 })),
        );
        let mut input = integrated_execution_input();
        input.source_vectors = post_lane_square_input([0, 0, 0]).source_vectors;
        let mut config =
            |lane| -> Result<u8, &'static str> { Ok(if lane == 2 { 0x02 } else { 0 }) };
        let mut post = CompiledPostLaneGlobalNormal {
            selector_words: [0, 0, 9],
        };
        let success = execute_compiled_eight_lanes_then(
            input,
            integrated_execution_state(),
            &mut providers,
            &mut config,
            &mut post,
        )
        .unwrap();
        assert_ne!(success.lanes.state.lane_words[2], 102);
        let expected = derive_compiled_post_lane_global_normal(CompiledPostLaneNormalInput {
            source_vectors: input.source_vectors,
            lane_words: success.lanes.state.lane_words,
            selector_words: [0, 0, 9],
        });
        assert_eq!(success.post_lane, expected);
        assert_eq!(success.post_lane.selected_quad, CompiledPostLaneQuad::C);
    }

    #[test]
    fn concrete_post_basis_consumes_the_ak11_final_publication_after_lane_success() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(
            &events,
            std::array::from_fn(|lane| IntegratedLaneMode::Built(if lane == 2 { 0x51 } else { 0 })),
        );
        let mut input = integrated_execution_input();
        input.source_vectors = post_lane_square_input([0, 0, 0]).source_vectors;
        let mut config =
            |lane| -> Result<u8, &'static str> { Ok(if lane == 2 { 0x02 } else { 0 }) };
        let mut post = CompiledPostLaneBasis {
            normal: CompiledPostLaneGlobalNormal {
                selector_words: [0, 0, 9],
            },
            prior_vector_258: [0, 0x10000, 0],
        };
        let success = execute_compiled_eight_lanes_then(
            input,
            integrated_execution_state(),
            &mut providers,
            &mut config,
            &mut post,
        )
        .unwrap();

        let normal = derive_compiled_post_lane_global_normal(CompiledPostLaneNormalInput {
            source_vectors: input.source_vectors,
            lane_words: success.lanes.state.lane_words,
            selector_words: [0, 0, 9],
        });
        let basis = derive_compiled_post_lane_basis(CompiledPostLaneBasisInputs {
            vector_324: normal.final_published_normal,
            vector_258: [0, 0x10000, 0],
        });
        assert_eq!(
            success.post_lane,
            CompiledPostLaneBasisCompletion { normal, basis }
        );
        assert_eq!(
            success.post_lane.normal.selected_quad,
            CompiledPostLaneQuad::C
        );
        assert_eq!(
            success.post_lane.basis.vector_264,
            normal.final_published_normal
        );
    }

    #[test]
    fn post_basis_is_not_produced_when_a_lane_fails() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(
            &events,
            std::array::from_fn(|lane| {
                if lane == 7 {
                    IntegratedLaneMode::SearchThenBuildFailure
                } else {
                    IntegratedLaneMode::Built(0)
                }
            }),
        );
        let mut state = integrated_execution_state();
        state.handles[7] = -1;
        let mut config = |_lane| -> Result<u8, &'static str> { Ok(0) };
        let mut post = CompiledPostLaneBasis {
            normal: CompiledPostLaneGlobalNormal {
                selector_words: [0, 0, 9],
            },
            prior_vector_258: [0, 0x10000, 0],
        };
        let failure = execute_compiled_eight_lanes_then(
            integrated_execution_input(),
            state,
            &mut providers,
            &mut config,
            &mut post,
        )
        .unwrap_err();
        assert!(matches!(
            failure,
            CompiledLaneAndPostFailure::Lanes(CompiledEightLaneExecutionFailure::Dispatch {
                failed_lane: 7,
                ..
            })
        ));
    }

    #[test]
    fn concrete_post_publication_publishes_the_rebuilt_rows_after_lane_success() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(
            &events,
            std::array::from_fn(|lane| IntegratedLaneMode::Built(if lane == 2 { 0x51 } else { 0 })),
        );
        let mut input = integrated_execution_input();
        input.source_vectors = post_lane_square_input([0, 0, 0]).source_vectors;
        let mut config =
            |lane| -> Result<u8, &'static str> { Ok(if lane == 2 { 0x02 } else { 0 }) };
        let mut post = CompiledPostLanePublishedBasis {
            basis: CompiledPostLaneBasis {
                normal: CompiledPostLaneGlobalNormal {
                    selector_words: [0, 0, 9],
                },
                prior_vector_258: [0, 0x10000, 0],
            },
            destination: [[9, 9, 9, 0x1111], [8, 8, 8, 0x2222], [7, 7, 7, 0x3333]],
        };
        let success = execute_compiled_eight_lanes_then(
            input,
            integrated_execution_state(),
            &mut providers,
            &mut config,
            &mut post,
        )
        .unwrap();

        let normal = derive_compiled_post_lane_global_normal(CompiledPostLaneNormalInput {
            source_vectors: input.source_vectors,
            lane_words: success.lanes.state.lane_words,
            selector_words: [0, 0, 9],
        });
        let basis = derive_compiled_post_lane_basis(CompiledPostLaneBasisInputs {
            vector_324: normal.final_published_normal,
            vector_258: [0, 0x10000, 0],
        });
        let mut destination = [[9, 9, 9, 0x1111], [8, 8, 8, 0x2222], [7, 7, 7, 0x3333]];
        publish_compiled_post_lane_basis(&mut destination, basis);
        assert_eq!(
            success.post_lane,
            CompiledPostLanePublicationCompletion {
                normal,
                basis,
                destination,
            }
        );
        assert_eq!(success.post_lane.destination[0][3], 0x1111);
        assert_eq!(success.post_lane.destination[1][3], 0x2222);
        assert_eq!(success.post_lane.destination[2][3], 0x3333);
    }

    #[test]
    fn post_publication_is_not_produced_when_a_lane_fails() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(
            &events,
            std::array::from_fn(|lane| {
                if lane == 7 {
                    IntegratedLaneMode::SearchThenBuildFailure
                } else {
                    IntegratedLaneMode::Built(0)
                }
            }),
        );
        let mut state = integrated_execution_state();
        state.handles[7] = -1;
        let mut config = |_lane| -> Result<u8, &'static str> { Ok(0) };
        let mut post = CompiledPostLanePublishedBasis {
            basis: CompiledPostLaneBasis {
                normal: CompiledPostLaneGlobalNormal {
                    selector_words: [0, 0, 9],
                },
                prior_vector_258: [0, 0x10000, 0],
            },
            destination: [[0; 4]; 3],
        };
        let failure = execute_compiled_eight_lanes_then(
            integrated_execution_input(),
            state,
            &mut providers,
            &mut config,
            &mut post,
        )
        .unwrap_err();
        assert!(matches!(
            failure,
            CompiledLaneAndPostFailure::Lanes(CompiledEightLaneExecutionFailure::Dispatch {
                failed_lane: 7,
                ..
            })
        ));
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum CandidateCall {
        Validate([i32; 3], i16),
        Decode(i16),
    }

    struct ScriptedCandidateProvider {
        validations: Vec<Result<bool, &'static str>>,
        decodes: Vec<Result<Option<[i32; 3]>, &'static str>>,
        calls: Vec<CandidateCall>,
    }

    impl ScriptedCandidateProvider {
        fn new(
            validations: impl IntoIterator<Item = Result<bool, &'static str>>,
            decodes: impl IntoIterator<Item = Result<Option<[i32; 3]>, &'static str>>,
        ) -> Self {
            Self {
                validations: validations.into_iter().collect(),
                decodes: decodes.into_iter().collect(),
                calls: Vec::new(),
            }
        }
    }

    impl CompiledLaneCandidateProvider for ScriptedCandidateProvider {
        type Error = &'static str;

        fn validate_candidate(
            &mut self,
            source_vector: [i32; 3],
            handle: i16,
        ) -> Result<bool, Self::Error> {
            self.calls
                .push(CandidateCall::Validate(source_vector, handle));
            self.validations.remove(0)
        }

        fn decode_metric_words(&mut self, handle: i16) -> Result<Option<[i32; 3]>, Self::Error> {
            self.calls.push(CandidateCall::Decode(handle));
            self.decodes.remove(0)
        }
    }

    #[test]
    fn decoded_candidate_search_pins_signed_count_and_lazy_list_bounds() {
        for signed_count in [0, -1, i16::MIN] {
            let mut provider = ScriptedCandidateProvider::new([], []);
            assert_eq!(
                search_decoded_compiled_lane_candidates(
                    [1, 2, 3],
                    9,
                    signed_count,
                    &[],
                    &mut provider
                ),
                Ok(CompiledLaneCandidateChoice::Miss)
            );
            assert!(provider.calls.is_empty());
        }

        let mut provider = ScriptedCandidateProvider::new([Ok(false), Ok(true)], [Ok(None)]);
        assert_eq!(
            search_decoded_compiled_lane_candidates([1, 2, 3], 9, 2, &[10, 20, 30], &mut provider),
            Ok(CompiledLaneCandidateChoice::Miss)
        );
        assert_eq!(
            provider.calls,
            [
                CandidateCall::Validate([1, 2, 3], 10),
                CandidateCall::Validate([1, 2, 3], 20),
                CandidateCall::Decode(20),
            ]
        );
    }

    #[test]
    fn decoded_candidate_search_uses_strict_signed_score_order_and_keeps_ties() {
        let mut provider = ScriptedCandidateProvider::new(
            [Ok(true), Ok(true), Ok(true)],
            [
                Ok(Some([0, 0, 9])),
                Ok(Some([3, 3, 3])),
                Ok(Some([6, 6, 6])),
            ],
        );
        assert_eq!(
            search_decoded_compiled_lane_candidates([4, 5, 6], 10, 3, &[7, 8, 9], &mut provider),
            Ok(CompiledLaneCandidateChoice::Found {
                ordinal: 2,
                handle: 9,
                score: 4,
            })
        );
        assert_eq!(
            provider.calls,
            [
                CandidateCall::Validate([4, 5, 6], 7),
                CandidateCall::Decode(7),
                CandidateCall::Validate([4, 5, 6], 8),
                CandidateCall::Decode(8),
                CandidateCall::Validate([4, 5, 6], 9),
                CandidateCall::Decode(9),
            ]
        );

        let mut tied = ScriptedCandidateProvider::new(
            [Ok(true), Ok(true)],
            [Ok(Some([0, 0, 9])), Ok(Some([3, 3, 3]))],
        );
        assert_eq!(
            search_decoded_compiled_lane_candidates([0; 3], 10, 2, &[70, 80], &mut tied),
            Ok(CompiledLaneCandidateChoice::Found {
                ordinal: 0,
                handle: 70,
                score: 7,
            })
        );
    }

    #[test]
    fn decoded_candidate_metric_pins_wrapping_division_and_minimum_pseudo_abs() {
        assert_eq!(compiled_lane_candidate_metric([-1, -2, -4], 0), 2);
        assert_eq!(compiled_lane_candidate_metric([1, 2, 4], 0), 2);
        assert_eq!(
            compiled_lane_candidate_metric([i32::MAX, 1, 1], 0),
            715_827_882
        );
        assert_eq!(
            compiled_lane_candidate_metric([-1, -1, -1], i32::MAX),
            i32::MIN
        );

        let mut provider = ScriptedCandidateProvider::new(
            [Ok(true), Ok(true)],
            [Ok(Some([-1, -1, -1])), Ok(Some([0, 0, 0]))],
        );
        assert_eq!(
            search_decoded_compiled_lane_candidates(
                [0; 3],
                i32::MAX,
                2,
                &[i16::MIN, i16::MAX],
                &mut provider
            ),
            Ok(CompiledLaneCandidateChoice::Found {
                ordinal: 0,
                handle: i16::MIN,
                score: i32::MIN,
            })
        );
    }

    #[test]
    fn decoded_candidate_search_errors_preserve_lazy_precedence_and_publish_nothing() {
        let mut validate_error = ScriptedCandidateProvider::new([Err("validate")], []);
        assert_eq!(
            search_decoded_compiled_lane_candidates([0; 3], 0, 2, &[11], &mut validate_error),
            Err(CompiledLaneCandidateFailure::Provider {
                phase: CompiledLaneCandidateFailurePhase::Validate,
                ordinal: 0,
                handle: 11,
                provider_error: "validate",
            })
        );

        let mut decode_error = ScriptedCandidateProvider::new([Ok(true)], [Err("decode")]);
        assert_eq!(
            search_decoded_compiled_lane_candidates([0; 3], 0, 2, &[12], &mut decode_error),
            Err(CompiledLaneCandidateFailure::Provider {
                phase: CompiledLaneCandidateFailurePhase::Decode,
                ordinal: 0,
                handle: 12,
                provider_error: "decode",
            })
        );

        let mut bounds = ScriptedCandidateProvider::new([Ok(false)], []);
        assert_eq!(
            search_decoded_compiled_lane_candidates([0; 3], 0, 2, &[13], &mut bounds),
            Err(CompiledLaneCandidateFailure::CandidateOutOfBounds {
                ordinal: 1,
                requested_count: 2,
                available: 1,
            })
        );

        let mut winner_then_bounds =
            ScriptedCandidateProvider::new([Ok(true)], [Ok(Some([0, 0, 0]))]);
        assert_eq!(
            search_decoded_compiled_lane_candidates([0; 3], 0, 2, &[13], &mut winner_then_bounds),
            Err(CompiledLaneCandidateFailure::CandidateOutOfBounds {
                ordinal: 1,
                requested_count: 2,
                available: 1,
            })
        );

        let mut late_error =
            ScriptedCandidateProvider::new([Ok(true), Err("late")], [Ok(Some([0, 0, 0]))]);
        assert!(matches!(
            search_decoded_compiled_lane_candidates([0; 3], 0, 2, &[14, 15], &mut late_error),
            Err(CompiledLaneCandidateFailure::Provider { ordinal: 1, .. })
        ));
    }

    #[test]
    fn decoded_candidate_search_forwards_full_handles_and_skips_rejected_decodes() {
        let mut provider = ScriptedCandidateProvider::new(
            [Ok(false), Ok(true), Ok(true)],
            [Ok(None), Ok(Some([30, 30, 30]))],
        );
        assert_eq!(
            search_decoded_compiled_lane_candidates(
                [i32::MIN, 0, i32::MAX],
                29,
                3,
                &[i16::MAX, -7, i16::MIN],
                &mut provider
            ),
            Ok(CompiledLaneCandidateChoice::Found {
                ordinal: 2,
                handle: i16::MIN,
                score: 1,
            })
        );
        assert_eq!(
            provider.calls,
            [
                CandidateCall::Validate([i32::MIN, 0, i32::MAX], i16::MAX),
                CandidateCall::Validate([i32::MIN, 0, i32::MAX], -7),
                CandidateCall::Decode(-7),
                CandidateCall::Validate([i32::MIN, 0, i32::MAX], i16::MIN),
                CandidateCall::Decode(i16::MIN),
            ]
        );
    }

    const fn spatial_cell(marker: i16, link: i32) -> DecodedCompiledLaneSpatialCell {
        DecodedCompiledLaneSpatialCell {
            marker,
            reserved: 0,
            link_or_candidate_offset: link,
        }
    }

    #[test]
    fn decoded_spatial_lookup_pins_wrapping_origin_signed_grid_and_source_y_omission() {
        let occupied = [spatial_cell(4, 7)];
        let hierarchy = DecodedCompiledLaneSpatialHierarchy {
            origin_xz: [i32::MAX, i32::MAX],
            width: 1,
            height: 1,
            levels: [&occupied, &[], &[], &[], &[], &[]],
        };
        let first = locate_decoded_compiled_lane_candidate_region(
            [i32::MIN, i32::MIN, i32::MIN],
            hierarchy,
        );
        let second = locate_decoded_compiled_lane_candidate_region(
            [i32::MIN, i32::MAX, i32::MIN],
            hierarchy,
        );
        assert_eq!(first, second);
        assert_eq!(
            first,
            Ok(CompiledLaneSpatialLookup::CandidateRegion {
                level: 0,
                cell_index: 0,
                signed_count: 4,
                candidate_offset_words: 7,
            })
        );

        let mut row_major = [spatial_cell(-1, 0); 15];
        row_major[14] = spatial_cell(3, 9);
        let row_major_hierarchy = DecodedCompiledLaneSpatialHierarchy {
            origin_xz: [0, 0],
            width: 4,
            height: 4,
            levels: [&row_major, &[], &[], &[], &[], &[]],
        };
        assert_eq!(
            locate_decoded_compiled_lane_candidate_region(
                [2 << 24, 77, 3 << 24],
                row_major_hierarchy
            ),
            Ok(CompiledLaneSpatialLookup::CandidateRegion {
                level: 0,
                cell_index: 14,
                signed_count: 3,
                candidate_offset_words: 9,
            })
        );

        let mut wrapped_subtraction = [spatial_cell(-1, 0); 128];
        wrapped_subtraction[127] = spatial_cell(1, 5);
        let wrapped_hierarchy = DecodedCompiledLaneSpatialHierarchy {
            origin_xz: [1, 0],
            width: 128,
            height: 1,
            levels: [&wrapped_subtraction, &[], &[], &[], &[], &[]],
        };
        assert_eq!(
            locate_decoded_compiled_lane_candidate_region([i32::MIN, 0, 0], wrapped_hierarchy),
            Ok(CompiledLaneSpatialLookup::CandidateRegion {
                level: 0,
                cell_index: 127,
                signed_count: 1,
                candidate_offset_words: 5,
            })
        );

        let no_cells = DecodedCompiledLaneSpatialHierarchy {
            origin_xz: [0, 0],
            width: 2,
            height: 2,
            levels: [&[]; COMPILED_LANE_SPATIAL_LEVEL_COUNT],
        };
        for source in [[-1, 0, 0], [2 << 24, 0, 0], [0, 0, -1], [0, 0, 2 << 24]] {
            assert_eq!(
                locate_decoded_compiled_lane_candidate_region(source, no_cells),
                Ok(CompiledLaneSpatialLookup::Miss)
            );
        }
        for (width, height) in [(0, 1), (-1, 1), (1, 0), (1, -1)] {
            let nonpositive_dimension = DecodedCompiledLaneSpatialHierarchy {
                width,
                height,
                ..no_cells
            };
            assert_eq!(
                locate_decoded_compiled_lane_candidate_region([0; 3], nonpositive_dimension),
                Ok(CompiledLaneSpatialLookup::Miss)
            );
        }
    }

    #[test]
    fn decoded_spatial_lookup_consumes_exact_two_bit_subcells_across_six_levels() {
        let empty = spatial_cell(-1, 0);
        let occupied = spatial_cell(2, 4);
        let level0 = [spatial_cell(-1, 10)];
        let mut level1 = [occupied; 24];
        let mut level2 = [occupied; 31];
        let mut level3 = [occupied; 38];
        let mut level4 = [occupied; 41];
        let mut level5 = [occupied; 60];
        level1[23] = spatial_cell(-1, 20);
        level2[30] = spatial_cell(-1, 30);
        level3[37] = spatial_cell(-1, 40);
        level4[40] = spatial_cell(-1, 50);
        level5[59] = occupied;
        let hierarchy = DecodedCompiledLaneSpatialHierarchy {
            origin_xz: [0, 0],
            width: 1,
            height: 1,
            levels: [&level0, &level1, &level2, &level3, &level4, &level5],
        };
        let delta_x = 0x006c_4000;
        let delta_z = 0x00e4_8000;
        assert_eq!(
            locate_decoded_compiled_lane_candidate_region([delta_x, i32::MIN, delta_z], hierarchy),
            Ok(CompiledLaneSpatialLookup::CandidateRegion {
                level: 5,
                cell_index: 59,
                signed_count: 2,
                candidate_offset_words: 4,
            })
        );

        let mut level5_empty = level5;
        level5_empty[59] = empty;
        let empty_leaf = DecodedCompiledLaneSpatialHierarchy {
            levels: [&level0, &level1, &level2, &level3, &level4, &level5_empty],
            ..hierarchy
        };
        assert_eq!(
            locate_decoded_compiled_lane_candidate_region([delta_x, i32::MAX, delta_z], empty_leaf),
            Ok(CompiledLaneSpatialLookup::Miss)
        );
    }

    #[test]
    fn decoded_spatial_lookup_treats_only_minus_one_as_empty_and_wraps_child_links() {
        let terminal = [spatial_cell(-2, i32::MIN)];
        let hierarchy = DecodedCompiledLaneSpatialHierarchy {
            origin_xz: [0, 0],
            width: 1,
            height: 1,
            levels: [&terminal, &[], &[], &[], &[], &[]],
        };
        assert_eq!(
            locate_decoded_compiled_lane_candidate_region([0; 3], hierarchy),
            Ok(CompiledLaneSpatialLookup::CandidateRegion {
                level: 0,
                cell_index: 0,
                signed_count: -2,
                candidate_offset_words: i32::MIN,
            })
        );

        let zero = [spatial_cell(0, i32::MAX)];
        let zero_count = DecodedCompiledLaneSpatialHierarchy {
            levels: [&zero, &[], &[], &[], &[], &[]],
            ..hierarchy
        };
        assert_eq!(
            locate_decoded_compiled_lane_candidate_region([0; 3], zero_count),
            Ok(CompiledLaneSpatialLookup::CandidateRegion {
                level: 0,
                cell_index: 0,
                signed_count: 0,
                candidate_offset_words: i32::MAX,
            })
        );

        let root = [spatial_cell(-1, i32::MAX)];
        let wrapping = DecodedCompiledLaneSpatialHierarchy {
            levels: [&root, &[], &[], &[], &[], &[]],
            ..hierarchy
        };
        assert_eq!(
            locate_decoded_compiled_lane_candidate_region([3 << 22, 0, 3 << 22], wrapping),
            Err(CompiledLaneSpatialBoundsError {
                level: 1,
                cell_index: i32::MIN + 14,
                available_cells: 0,
            })
        );
    }

    #[test]
    fn decoded_spatial_lookup_reports_the_first_missing_cell_exactly() {
        let level0 = [spatial_cell(1, 2); 3];
        let hierarchy = DecodedCompiledLaneSpatialHierarchy {
            origin_xz: [0, 0],
            width: 2,
            height: 2,
            levels: [&level0, &[], &[], &[], &[], &[]],
        };
        assert_eq!(
            locate_decoded_compiled_lane_candidate_region([1 << 24, 0, 1 << 24], hierarchy),
            Err(CompiledLaneSpatialBoundsError {
                level: 0,
                cell_index: 3,
                available_cells: 3,
            })
        );
    }

    #[test]
    fn decoded_spatial_search_bypasses_candidate_offsets_for_nonpositive_counts() {
        for (signed_count, offset) in [(0, i32::MIN), (-2, i32::MAX), (i16::MIN, 0)] {
            let leaf = [spatial_cell(signed_count, offset)];
            let hierarchy = DecodedCompiledLaneSpatialHierarchy {
                origin_xz: [0, 0],
                width: 1,
                height: 1,
                levels: [&leaf, &[], &[], &[], &[], &[]],
            };
            let mut provider = ScriptedCandidateProvider::new([], []);
            assert_eq!(
                search_decoded_compiled_lane_spatial_hierarchy(
                    [0; 3],
                    77,
                    hierarchy,
                    &[],
                    &mut provider
                ),
                Ok(CompiledLaneCandidateChoice::Miss)
            );
            assert!(provider.calls.is_empty());
        }

        let outside = DecodedCompiledLaneSpatialHierarchy {
            origin_xz: [0, 0],
            width: 1,
            height: 1,
            levels: [&[]; COMPILED_LANE_SPATIAL_LEVEL_COUNT],
        };
        let mut provider = ScriptedCandidateProvider::new([], []);
        assert_eq!(
            search_decoded_compiled_lane_spatial_hierarchy(
                [-1, 0, 0],
                0,
                outside,
                &[],
                &mut provider
            ),
            Ok(CompiledLaneCandidateChoice::Miss)
        );
        assert!(provider.calls.is_empty());
    }

    #[test]
    fn decoded_spatial_search_passes_the_exact_word_tail_to_the_lazy_chooser() {
        let leaf = [spatial_cell(2, 2)];
        let hierarchy = DecodedCompiledLaneSpatialHierarchy {
            origin_xz: [0, 0],
            width: 1,
            height: 1,
            levels: [&leaf, &[], &[], &[], &[], &[]],
        };
        let source = [4, 5, 6];
        let mut provider = ScriptedCandidateProvider::new(
            [Ok(true), Ok(true)],
            [Ok(Some([0, 0, 9])), Ok(Some([6, 6, 6]))],
        );
        assert_eq!(
            search_decoded_compiled_lane_spatial_hierarchy(
                source,
                10,
                hierarchy,
                &[99, 98, 7, 8, 9],
                &mut provider
            ),
            Ok(CompiledLaneCandidateChoice::Found {
                ordinal: 1,
                handle: 8,
                score: 4,
            })
        );
        assert_eq!(
            provider.calls,
            [
                CandidateCall::Validate(source, 7),
                CandidateCall::Decode(7),
                CandidateCall::Validate(source, 8),
                CandidateCall::Decode(8),
            ]
        );
    }

    #[test]
    fn decoded_spatial_search_preserves_start_and_per_ordinal_error_precedence() {
        let leaf = [spatial_cell(2, 4)];
        let hierarchy = DecodedCompiledLaneSpatialHierarchy {
            origin_xz: [0, 0],
            width: 1,
            height: 1,
            levels: [&leaf, &[], &[], &[], &[], &[]],
        };
        let pool = [0, 0, 0, 0, 11];

        let mut provider_error = ScriptedCandidateProvider::new([Err("first")], []);
        assert_eq!(
            search_decoded_compiled_lane_spatial_hierarchy(
                [0; 3],
                0,
                hierarchy,
                &pool,
                &mut provider_error
            ),
            Err(CompiledLaneSpatialSearchFailure::Candidate(
                CompiledLaneCandidateFailure::Provider {
                    phase: CompiledLaneCandidateFailurePhase::Validate,
                    ordinal: 0,
                    handle: 11,
                    provider_error: "first",
                }
            ))
        );

        let mut later_bounds = ScriptedCandidateProvider::new([Ok(false)], []);
        assert_eq!(
            search_decoded_compiled_lane_spatial_hierarchy(
                [0; 3],
                0,
                hierarchy,
                &pool,
                &mut later_bounds
            ),
            Err(CompiledLaneSpatialSearchFailure::Candidate(
                CompiledLaneCandidateFailure::CandidateOutOfBounds {
                    ordinal: 1,
                    requested_count: 2,
                    available: 1,
                }
            ))
        );

        let mut winner_then_bounds =
            ScriptedCandidateProvider::new([Ok(true)], [Ok(Some([0, 0, 0]))]);
        assert_eq!(
            search_decoded_compiled_lane_spatial_hierarchy(
                [0; 3],
                0,
                hierarchy,
                &pool,
                &mut winner_then_bounds
            ),
            Err(CompiledLaneSpatialSearchFailure::Candidate(
                CompiledLaneCandidateFailure::CandidateOutOfBounds {
                    ordinal: 1,
                    requested_count: 2,
                    available: 1,
                }
            ))
        );

        for offset in [i32::MIN, -1, 6, i32::MAX] {
            let invalid_leaf = [spatial_cell(1, offset)];
            let invalid = DecodedCompiledLaneSpatialHierarchy {
                levels: [&invalid_leaf, &[], &[], &[], &[], &[]],
                ..hierarchy
            };
            let mut untouched = ScriptedCandidateProvider::new([], []);
            assert_eq!(
                search_decoded_compiled_lane_spatial_hierarchy(
                    [0; 3],
                    0,
                    invalid,
                    &pool,
                    &mut untouched
                ),
                Err(
                    CompiledLaneSpatialSearchFailure::CandidateStartOutOfBounds {
                        candidate_offset_words: offset,
                        available_words: 5,
                    }
                )
            );
            assert!(untouched.calls.is_empty());
        }

        let one_past_leaf = [spatial_cell(1, 5)];
        let one_past = DecodedCompiledLaneSpatialHierarchy {
            levels: [&one_past_leaf, &[], &[], &[], &[], &[]],
            ..hierarchy
        };
        let mut untouched = ScriptedCandidateProvider::new([], []);
        assert_eq!(
            search_decoded_compiled_lane_spatial_hierarchy(
                [0; 3],
                0,
                one_past,
                &pool,
                &mut untouched
            ),
            Err(CompiledLaneSpatialSearchFailure::Candidate(
                CompiledLaneCandidateFailure::CandidateOutOfBounds {
                    ordinal: 0,
                    requested_count: 1,
                    available: 0,
                }
            ))
        );

        let missing_root = DecodedCompiledLaneSpatialHierarchy {
            origin_xz: [0, 0],
            width: 1,
            height: 1,
            levels: [&[]; COMPILED_LANE_SPATIAL_LEVEL_COUNT],
        };
        let mut untouched = ScriptedCandidateProvider::new([], []);
        assert_eq!(
            search_decoded_compiled_lane_spatial_hierarchy(
                [0; 3],
                0,
                missing_root,
                &[],
                &mut untouched
            ),
            Err(CompiledLaneSpatialSearchFailure::Hierarchy(
                CompiledLaneSpatialBoundsError {
                    level: 0,
                    cell_index: 0,
                    available_cells: 0,
                }
            ))
        );
    }

    fn validator_triangle() -> [[i32; 3]; 3] {
        [[0, 11, 0], [0, 22, 0x1_0000], [0x1_0000, 33, 0]]
    }

    #[test]
    fn decoded_lane_validator_is_winding_sensitive_and_ignores_every_y_word() {
        let source = [0x4000, i32::MIN, 0x4000];
        let vertices = validator_triangle();
        assert_eq!(
            compiled_lane_xz_edge_scores(source, vertices),
            [0x4000, 0x8000, 0x4000]
        );
        assert!(validate_decoded_compiled_lane_source(source, vertices));
        let mut changed_y = vertices;
        changed_y[0][1] = i32::MIN;
        changed_y[1][1] = 0;
        changed_y[2][1] = i32::MAX;
        assert!(validate_decoded_compiled_lane_source(
            [source[0], i32::MAX, source[2]],
            changed_y
        ));
        let mut reversed = vertices;
        reversed.swap(1, 2);
        assert!(!validate_decoded_compiled_lane_source(source, reversed));
    }

    #[test]
    fn decoded_lane_validator_accepts_vertices_and_edges_but_rejects_bbox_outside() {
        let vertices = validator_triangle();
        for source in [
            vertices[0],
            vertices[1],
            vertices[2],
            [0, 9, 0x8000],
            [0x8000, 9, 0],
            [0x8000, 9, 0x8000],
        ] {
            assert!(validate_decoded_compiled_lane_source(source, vertices));
        }
        let source = [0xc000, 0, 0xc000];
        let mut rotated = vertices;
        for expected_negative_slot in [1, 0, 2] {
            let scores = compiled_lane_xz_edge_scores(source, rotated);
            assert!(scores[expected_negative_slot] < 0);
            assert!(!validate_decoded_compiled_lane_source(source, rotated));
            rotated.rotate_left(1);
        }
    }

    #[test]
    fn decoded_lane_validator_pins_signed_bbox_and_degenerate_inclusion() {
        let vertices = validator_triangle();
        for source in [[-1, 0, 0], [0x1_0001, 0, 0], [0, 0, -1], [0, 0, 0x1_0001]] {
            assert!(!validate_decoded_compiled_lane_source(source, vertices));
        }
        let collapsed = [[7, i32::MIN, 9], [7, 0, 9], [7, i32::MAX, 9]];
        assert!(validate_decoded_compiled_lane_source(
            [7, 123, 9],
            collapsed
        ));
        assert!(!validate_decoded_compiled_lane_source(
            [8, 123, 9],
            collapsed
        ));
        assert!(!validate_decoded_compiled_lane_source(
            [7, 123, 8],
            collapsed
        ));
        let collinear = [[-0x1_0000, 0, 0], [0, 0, 0], [0x1_0000, 0, 0]];
        assert!(validate_decoded_compiled_lane_source(
            [0x8000, 0, 0],
            collinear
        ));
        assert!(!validate_decoded_compiled_lane_source(
            [0x8000, 0, 1],
            collinear
        ));
    }

    #[test]
    fn decoded_lane_validator_keeps_bbox_even_when_wrapped_edges_would_accept() {
        let source = [1_073_741_824, 0, 1];
        let vertices = [
            [1, 0, 2_147_483_646],
            [-1_073_741_824, 0, i32::MAX],
            [-65_536, 0, i32::MIN],
        ];
        assert_eq!(
            compiled_lane_xz_edge_scores(source, vertices),
            [32_767, 2_147_418_114, 2_147_483_644]
        );
        assert!(!validate_decoded_compiled_lane_source(source, vertices));
    }

    #[test]
    fn decoded_lane_validator_pins_independent_q16_projection_and_wrapping_sum() {
        let rounding_source = [3, 0, 1];
        let rounding_vertices = [[1, 0, 3], [2, 0, 5], [4, 0, 0]];
        assert!(compiled_lane_xz_edge_scores(rounding_source, rounding_vertices).contains(&-1));
        assert!(!validate_decoded_compiled_lane_source(
            rounding_source,
            rounding_vertices
        ));
        let wrapping_source = [-65_536, 0, 65_536];
        let wrapping_vertices = [
            [100_000_000, 0, 500_000_000],
            [0, 0, 0],
            [-1_000_000_000, 0, -750_000_000],
        ];
        assert_eq!(
            compiled_lane_xz_edge_scores(wrapping_source, wrapping_vertices),
            [600_000_000, 1_750_000_000, 1_529_701_898]
        );
        assert!(validate_decoded_compiled_lane_source(
            wrapping_source,
            wrapping_vertices
        ));
    }

    #[test]
    fn decoded_lane_validator_pins_odd_rounding_and_wrapping_subtraction() {
        let odd_source = [0x3457, 0, -0x4569];
        let odd_vertices = [
            [0x1235, 0, -0x2347],
            [0x1_89ab, 0, 0x4567],
            [-0x789d, 0, -0x1_5679],
        ];
        assert_eq!(
            compiled_lane_xz_edge_scores(odd_source, odd_vertices),
            [16_388, 69_195, 15_223]
        );
        assert!(validate_decoded_compiled_lane_source(
            odd_source,
            odd_vertices
        ));
        let extreme_source = [-65_536, 0, -1];
        let extreme_vertices = [
            [-1, 0, i32::MIN],
            [i32::MAX, 0, 65_535],
            [-65_536, 0, 65_535],
        ];
        assert_eq!(
            compiled_lane_xz_edge_scores(extreme_source, extreme_vertices),
            [2_147_418_113, 2_147_418_113, 65_535]
        );
        assert!(validate_decoded_compiled_lane_source(
            extreme_source,
            extreme_vertices
        ));
    }

    #[test]
    fn compiled_lane_decoder_preserves_record_slot_order_and_signed_endpoints() {
        let vertices = [[10, 11, 12], [20, 21, 22], [30, 31, 32], [40, 41, 42]];
        let records = [
            DecodedCompiledLaneRecord {
                vertex_indices: [3, 1, 2],
                type_word: 0xabcd,
            },
            DecodedCompiledLaneRecord {
                vertex_indices: [0, 2, 1],
                type_word: 0x1234,
            },
        ];
        let tables = DecodedCompiledLaneTables {
            records: &records,
            vertices: &vertices,
        };
        assert_eq!(
            decode_compiled_lane_vertices(0, tables),
            Ok([vertices[3], vertices[1], vertices[2]])
        );
        assert_eq!(
            decode_compiled_lane_vertices(1, tables),
            Ok([vertices[0], vertices[2], vertices[1]])
        );
        for handle in [-1, i16::MIN, 2, i16::MAX] {
            assert_eq!(
                decode_compiled_lane_vertices(handle, tables),
                Err(CompiledLaneDecodeError::HandleOutOfBounds {
                    handle,
                    available_records: 2,
                })
            );
        }
        let mut high_records = vec![records[0]; 32_769];
        high_records[usize::from(i16::MAX as u16)].type_word = 0x7fff;
        assert_eq!(
            read_compiled_lane_type_word(i16::MAX, &high_records),
            Ok(0x7fff)
        );
        assert_eq!(
            read_compiled_lane_type_word(i16::MIN, &high_records),
            Err(CompiledLaneTypeReadError {
                handle: i16::MIN,
                available_records: 32_769,
            })
        );

        let mut high_vertices = vec![[0; 3]; 32_769];
        high_vertices[0x8000] = [101, 102, 103];
        let mut high_records = vec![
            DecodedCompiledLaneRecord {
                vertex_indices: [0, 0, 0],
                type_word: 0,
            };
            32_769
        ];
        high_records[0].vertex_indices = [0x8000, 0, 0];
        let high_tables = DecodedCompiledLaneTables {
            records: &high_records,
            vertices: &high_vertices,
        };
        assert_eq!(
            decode_compiled_lane_vertices(0, high_tables).unwrap()[0],
            [101, 102, 103]
        );
        assert_eq!(
            decode_compiled_lane_vertices(i16::MIN, high_tables),
            Err(CompiledLaneDecodeError::HandleOutOfBounds {
                handle: i16::MIN,
                available_records: 32_769,
            })
        );
    }

    #[test]
    fn compiled_lane_decoder_reports_the_first_invalid_vertex_slot_lazily() {
        let vertices = [[1, 2, 3]];
        for (indices, expected_slot, expected_index) in [
            ([2, 3, u16::MAX], 0, 2),
            ([0, 3, u16::MAX], 1, 3),
            ([0, 0, u16::MAX], 2, u16::MAX),
        ] {
            let records = [DecodedCompiledLaneRecord {
                vertex_indices: indices,
                type_word: 0xffff,
            }];
            assert_eq!(
                decode_compiled_lane_vertices(
                    0,
                    DecodedCompiledLaneTables {
                        records: &records,
                        vertices: &vertices,
                    }
                ),
                Err(CompiledLaneDecodeError::VertexOutOfBounds {
                    handle: 0,
                    slot: expected_slot,
                    vertex_index: expected_index,
                    available_vertices: 1,
                })
            );
        }
    }

    #[test]
    fn compiled_lane_decoder_type_and_y_words_do_not_change_validation() {
        let footprint = validator_triangle();
        let mut changed = footprint;
        changed[0][1] = i32::MAX;
        changed[1][1] = i32::MIN;
        changed[2][1] = 0;
        let vertices = [
            footprint[0],
            footprint[1],
            footprint[2],
            changed[0],
            changed[1],
            changed[2],
        ];
        let records = [
            DecodedCompiledLaneRecord {
                vertex_indices: [0, 1, 2],
                type_word: 0,
            },
            DecodedCompiledLaneRecord {
                vertex_indices: [3, 4, 5],
                type_word: u16::MAX,
            },
        ];
        let tables = DecodedCompiledLaneTables {
            records: &records,
            vertices: &vertices,
        };
        let first = decode_compiled_lane_vertices(0, tables).unwrap();
        let second = decode_compiled_lane_vertices(1, tables).unwrap();
        assert!(validate_decoded_compiled_lane_source(
            [0x4000, 7, 0x4000],
            first
        ));
        assert!(validate_decoded_compiled_lane_source(
            [0x4000, -9, 0x4000],
            second
        ));
    }

    fn horizontal_lane_triangle() -> DecodedCompiledLaneTriangle {
        DecodedCompiledLaneTriangle {
            vertices: [
                [0, 0x2_0000, 0],
                [0x1_0000, 0x2_0000, 0],
                [0, 0x2_0000, 0x1_0000],
            ],
            type_word: 0xabcd,
        }
    }

    #[test]
    fn decoded_lane_build_pins_store_order_orientation_and_ignores_source_y() {
        assert_eq!(
            COMPILED_LANE_BUILD_STORE_ORDER,
            [
                CompiledLaneBuildStore::TypeWord,
                CompiledLaneBuildStore::RawOutputX,
                CompiledLaneBuildStore::RawOutputY,
                CompiledLaneBuildStore::RawOutputZ,
                CompiledLaneBuildStore::FinalOutputX,
                CompiledLaneBuildStore::FinalOutputY,
                CompiledLaneBuildStore::FinalOutputZ,
            ]
        );
        let first = derive_decoded_compiled_lane_build(
            [0x7_0000, i32::MIN, -0x4_0000],
            -99,
            horizontal_lane_triangle(),
        )
        .unwrap();
        let second = derive_decoded_compiled_lane_build(
            [0x7_0000, i32::MAX, -0x4_0000],
            -99,
            horizontal_lane_triangle(),
        )
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.output_vector, [0, -0x1_0001, 0]);
        assert_eq!(first.lane_word, 0x2_0000);
        assert_eq!(first.type_word, 0xabcd);

        let mut reversed = horizontal_lane_triangle();
        reversed.vertices.swap(1, 2);
        let reversed = derive_decoded_compiled_lane_build([0; 3], 5, reversed).unwrap();
        assert_eq!(reversed.output_vector, [0, 0x1_0000, 0]);
        assert_eq!(reversed.lane_word, 0x2_0000);
    }

    #[test]
    fn decoded_lane_build_returns_fallback_for_collinear_and_vertical_planes() {
        let collinear = DecodedCompiledLaneTriangle {
            vertices: [[0; 3], [0x1_0000, 0, 0], [0x2_0000, 0, 0]],
            type_word: 7,
        };
        let effect = derive_decoded_compiled_lane_build([1, 2, 3], i32::MIN, collinear).unwrap();
        assert_eq!(effect.lane_word, i32::MIN);
        assert_eq!(effect.output_vector, [0; 3]);
        assert_eq!(effect.type_word, 7);

        let vertical = DecodedCompiledLaneTriangle {
            vertices: [[0; 3], [0x1_0000, 0, 0], [0, 0x1_0000, 0]],
            type_word: 8,
        };
        let effect = derive_decoded_compiled_lane_build([9, 8, 7], i32::MAX, vertical).unwrap();
        assert_eq!(effect.lane_word, i32::MAX);
        assert_eq!(effect.output_vector[1], 0);
    }

    #[test]
    fn decoded_lane_plane_divide_reports_published_effects_and_exact_boundaries() {
        let overflow =
            solve_compiled_lane_plane([0; 3], 99, [0x1_0000, 0, 0], 0x1234, [0x1_0000, 1, 0])
                .unwrap_err();
        assert_eq!(overflow.numerator, 0x1_0000);
        assert_eq!(overflow.divisor, 1);
        assert_eq!(overflow.published_type_word, 0x1234);
        assert_eq!(overflow.published_output_vector, [0x1_0000, 1, 0]);
        assert_eq!(
            solve_compiled_lane_plane([0; 3], 99, [0; 3], 1, [0, 0, 0]).unwrap(),
            99
        );
        assert_eq!(
            solve_compiled_lane_plane([0; 3], 0, [i32::MAX, 0, 0], 1, [0x1_0000, 0x1_0000, 0])
                .unwrap(),
            i32::MAX
        );
        assert_eq!(
            solve_compiled_lane_plane([0; 3], 0, [1, 0, 0], 1, [0x1_0000, -3, 0]).unwrap(),
            -21_845
        );
        assert_eq!(
            solve_compiled_lane_plane([0; 3], 0, [-1, 0, 0], 1, [0x1_0000, 3, 0]).unwrap(),
            -21_845
        );
        assert!(
            solve_compiled_lane_plane([0; 3], 0, [i32::MIN, 0, 0], 1, [0x1_0000, 1, 0]).is_err()
        );
    }

    #[test]
    fn decoded_lane_normalization_pins_safe_reciprocal_domain_and_wrapped_zero() {
        assert_eq!(sqrt_q16(1), 0x100);
        assert!(sqrt_q16(u32::MAX) >= 0x100);
        assert_eq!(normalize_compiled_lane_vector([i32::MIN, 0, 0]), [0; 3]);
        assert_ne!(normalize_compiled_lane_vector([0x100, 0, 0]), [0; 3]);
    }

    #[test]
    fn decoded_lane_build_reports_its_computed_vector_on_final_divide_overflow() {
        let triangle = DecodedCompiledLaneTriangle {
            vertices: [
                [0, 0, i32::MAX - 255],
                [0x1_0000, 0, i32::MAX - 255],
                [0, 0x1_0000, i32::MIN],
            ],
            type_word: 0x55aa,
        };
        let error = derive_decoded_compiled_lane_build([0; 3], 123, triangle).unwrap_err();
        assert_eq!(
            error,
            CompiledLaneBuildDivideOverflow {
                numerator: 2_147_483_392,
                divisor: -255,
                published_type_word: 0x55aa,
                published_output_vector: [0, -255, 0x1_0000],
            }
        );
    }

    #[test]
    fn decoded_lane_type_read_is_separate_signed_and_exact() {
        let records = [
            DecodedCompiledLaneRecord {
                vertex_indices: [9, 8, 7],
                type_word: 0,
            },
            DecodedCompiledLaneRecord {
                vertex_indices: [6, 5, 4],
                type_word: u16::MAX,
            },
        ];
        assert_eq!(read_compiled_lane_type_word(0, &records), Ok(0));
        assert_eq!(read_compiled_lane_type_word(1, &records), Ok(u16::MAX));
        for handle in [-1, i16::MIN, 2, i16::MAX] {
            assert_eq!(
                read_compiled_lane_type_word(handle, &records),
                Err(CompiledLaneTypeReadError {
                    handle,
                    available_records: 2,
                })
            );
        }
    }

    #[test]
    fn decoded_geometry_provider_dispatches_a_direct_valid_handle() {
        let triangle = DecodedCompiledLaneTriangle {
            vertices: [
                [0, 0x2_0000, 0],
                [0, 0x2_0000, 0x1_0000],
                [0x1_0000, 0x2_0000, 0],
            ],
            type_word: 0xabcd,
        };
        let records = [DecodedCompiledLaneRecord {
            vertex_indices: [0, 1, 2],
            type_word: triangle.type_word,
        }];
        let leaf = [spatial_cell(0, i32::MIN)];
        let hierarchy = DecodedCompiledLaneSpatialHierarchy {
            origin_xz: [0, 0],
            width: 1,
            height: 1,
            levels: [&leaf, &[], &[], &[], &[], &[]],
        };
        let tables = DecodedCompiledLaneTables {
            records: &records,
            vertices: &triangle.vertices,
        };
        let source = [0x4000, i32::MIN, 0x4000];
        let expected = derive_decoded_compiled_lane_build(source, -99, triangle).unwrap();
        let mut provider = DecodedCompiledLaneGeometryProvider {
            hierarchy,
            candidate_pool: &[],
            tables,
        };
        assert_eq!(
            dispatch_compiled_lane(
                CompiledLaneDispatchInput {
                    source_vector: source,
                    initial_handle: 0,
                    fallback: -99,
                },
                &mut provider,
            ),
            Ok(CompiledLaneDispatchCompletion {
                lane_word: expected.lane_word,
                handle_writes: CompiledLaneHandleWriteTrace::default(),
                type_word_write: Some(expected.type_word),
                output_vector_write: Some(expected.output_vector),
            })
        );
    }

    #[test]
    fn eight_lane_executor_accepts_a_parsed_hpc_geometry_provider() {
        let triangle = DecodedCompiledLaneTriangle {
            vertices: [
                [0, 0x2_0000, 0],
                [0, 0x2_0000, 0x1_0000],
                [0x1_0000, 0x2_0000, 0],
            ],
            type_word: 0,
        };
        let mut payload = Vec::new();
        for word in [0_i32, 0] {
            payload.extend_from_slice(&word.to_le_bytes());
        }
        for count in [1_i16, 0, 0, 0, 0] {
            payload.extend_from_slice(&count.to_le_bytes());
        }
        payload.extend_from_slice(&0_u16.to_le_bytes());
        payload.extend_from_slice(&1_i16.to_le_bytes());
        payload.extend_from_slice(&0_u16.to_le_bytes());
        payload.extend_from_slice(&0_i32.to_le_bytes());
        payload.extend_from_slice(&3_u16.to_le_bytes());
        payload.extend_from_slice(&0_u16.to_le_bytes());
        for vertex in triangle.vertices {
            for component in vertex {
                payload.extend_from_slice(&component.to_le_bytes());
            }
        }
        payload.extend_from_slice(&1_u16.to_le_bytes());
        payload.extend_from_slice(&0_u16.to_le_bytes());
        for word in [0_u16, 1, 2, triangle.type_word] {
            payload.extend_from_slice(&word.to_le_bytes());
        }
        payload.extend_from_slice(&1_u32.to_le_bytes());
        payload.extend_from_slice(&0_i16.to_le_bytes());
        payload.extend_from_slice(&1_i16.to_le_bytes());
        payload.extend_from_slice(&1_i16.to_le_bytes());
        let hpc = cmr_formats::hpc::parse_hpc(&payload).unwrap();
        let provider = decoded_compiled_lane_provider_from_hpc(&hpc);
        let mut providers = [provider; 8];
        let source = [0x4000, i32::MIN, 0x4000];
        let input = CompiledEightLaneExecutionInputs {
            source_vectors: [source; 8],
            gate_words: [0; 8],
            drive_words: [0; 8],
            first_four_subtractions: [0; 4],
        };
        let state = CompiledEightLaneExecutionState {
            handles: [0; 8],
            type_words: [u16::MAX; 8],
            output_vectors: [[i32::MIN; 3]; 8],
            lane_words: [-99; 8],
            accumulator_words: [0; 8],
            latch_67c: 0,
        };
        let mut config = |_lane| -> Result<u8, std::convert::Infallible> { Ok(0) };

        let success = execute_compiled_eight_lanes(input, state, &mut providers, &mut config)
            .expect("decoded stable-table providers should complete");
        let expected = derive_decoded_compiled_lane_build(source, -99, triangle).unwrap();
        assert_eq!(success.state.lane_words, [expected.lane_word; 8]);
        assert_eq!(success.state.type_words, [expected.type_word; 8]);
        assert_eq!(success.state.output_vectors, [expected.output_vector; 8]);
        assert!(
            success
                .lanes
                .iter()
                .all(|lane| lane.dispatch.type_word_write == Some(triangle.type_word))
        );
    }

    #[test]
    fn decoded_geometry_provider_builds_the_winner_not_the_last_candidate() {
        let first_vertices = [
            [0, 0x2_0000, 0],
            [0, 0x2_0000, 0x1_0000],
            [0x1_0000, 0x2_0000, 0],
        ];
        let last_vertices = [
            [0, 0x8_0000, 0],
            [0, 0x8_0000, 0x1_0000],
            [0x1_0000, 0x8_0000, 0],
        ];
        let vertices = [
            first_vertices[0],
            first_vertices[1],
            first_vertices[2],
            last_vertices[0],
            last_vertices[1],
            last_vertices[2],
        ];
        let mut records = [DecodedCompiledLaneRecord {
            vertex_indices: [3, 4, 5],
            type_word: 0x9999,
        }; 10];
        records[5] = DecodedCompiledLaneRecord {
            vertex_indices: [0, 1, 2],
            type_word: 0x5555,
        };
        records[9] = DecodedCompiledLaneRecord {
            vertex_indices: [3, 4, 5],
            type_word: 0x9999,
        };
        let leaf = [spatial_cell(2, 1)];
        let hierarchy = DecodedCompiledLaneSpatialHierarchy {
            origin_xz: [0, 0],
            width: 1,
            height: 1,
            levels: [&leaf, &[], &[], &[], &[], &[]],
        };
        let source = [0x4000, 123, 0x4000];
        let fallback = 0x2_0000;
        let expected = derive_decoded_compiled_lane_build(
            source,
            fallback,
            DecodedCompiledLaneTriangle {
                vertices: first_vertices,
                type_word: 0x5555,
            },
        )
        .unwrap();
        let mut provider = DecodedCompiledLaneGeometryProvider {
            hierarchy,
            candidate_pool: &[77, 5, 9],
            tables: DecodedCompiledLaneTables {
                records: &records,
                vertices: &vertices,
            },
        };
        assert_eq!(
            dispatch_compiled_lane(
                CompiledLaneDispatchInput {
                    source_vector: source,
                    initial_handle: -7,
                    fallback,
                },
                &mut provider,
            ),
            Ok(CompiledLaneDispatchCompletion {
                lane_word: expected.lane_word,
                handle_writes: CompiledLaneHandleWriteTrace {
                    normalized_to_minus_one: true,
                    searched_handle: Some(5),
                    invalidated_after_search_miss: false,
                },
                type_word_write: Some(0x5555),
                output_vector_write: Some(expected.output_vector),
            })
        );
    }

    fn post_lane_square_input(selector_words: [i32; 3]) -> CompiledPostLaneNormalInput {
        let mut source_vectors = [[0; 3]; 8];
        for (lane, x, z) in [
            (0, 0, 0),
            (1, 0x1_0000, 0),
            (2, 0, 0x1_0000),
            (3, 0x1_0000, 0x1_0000),
            (4, 0, 0x2_0000),
            (5, 0x1_0000, 0x2_0000),
            (6, 0x2_0000, 0x1_0000),
            (7, 0x3_0000, 0x3_0000),
        ] {
            source_vectors[lane] = [x, lane as i32 * 0x1111, z];
        }
        CompiledPostLaneNormalInput {
            source_vectors,
            lane_words: [0; 8],
            selector_words,
        }
    }

    #[test]
    fn post_lane_normal_selects_all_quads_with_b_then_a_then_c_ties() {
        for (selectors, expected) in [
            ([9, 2, 1], CompiledPostLaneQuad::A),
            ([1, 9, 2], CompiledPostLaneQuad::B),
            ([1, 2, 9], CompiledPostLaneQuad::C),
            ([9, 9, 9], CompiledPostLaneQuad::B),
            ([9, 1, 9], CompiledPostLaneQuad::A),
            ([1, 9, 9], CompiledPostLaneQuad::B),
        ] {
            assert_eq!(
                derive_compiled_post_lane_global_normal(post_lane_square_input(selectors))
                    .selected_quad,
                expected
            );
        }
    }

    #[test]
    fn post_lane_normal_builds_two_upward_faces_and_their_final_sum() {
        let effects = derive_compiled_post_lane_global_normal(post_lane_square_input([0, 1, 0]));
        assert_eq!(effects.selected_quad, CompiledPostLaneQuad::B);
        assert_eq!(effects.first_published_normal, [0, 65_522, 0]);
        assert_eq!(effects.second_normal, [0, 65_522, 0]);
        assert_eq!(effects.final_square_sum, 262_032);
        assert_eq!(effects.final_published_normal, [0, 65_533, 0]);

        let mut reversed = post_lane_square_input([0, 1, 0]);
        reversed.source_vectors.swap(1, 2);
        let reversed = derive_compiled_post_lane_global_normal(reversed);
        assert!(reversed.first_published_normal[1] >= 0);
        assert!(reversed.second_normal[1] >= 0);

        let mut degenerate = post_lane_square_input([0, 1, 0]);
        degenerate.source_vectors = [[7, i32::MIN, -3]; 8];
        degenerate.lane_words = [11; 8];
        let degenerate = derive_compiled_post_lane_global_normal(degenerate);
        assert_eq!(degenerate.first_published_normal, [0; 3]);
        assert_eq!(degenerate.second_normal, [0; 3]);
        assert_eq!(degenerate.final_published_normal, [0; 3]);
        assert_eq!(degenerate.final_square_sum, 0);
    }

    #[test]
    fn post_lane_normal_ignores_source_y_and_unselected_lanes() {
        let base = post_lane_square_input([0, 7, 0]);
        let expected = derive_compiled_post_lane_global_normal(base);
        let mut poisoned = base;
        for (lane, source) in poisoned.source_vectors.iter_mut().enumerate() {
            source[1] = if lane & 1 == 0 { i32::MIN } else { i32::MAX };
        }
        for lane in 4..8 {
            poisoned.source_vectors[lane][0] = i32::MIN;
            poisoned.source_vectors[lane][2] = i32::MAX;
            poisoned.lane_words[lane] = i32::MIN;
        }
        assert_eq!(derive_compiled_post_lane_global_normal(poisoned), expected);
    }

    #[test]
    fn post_lane_selector_preserves_wrapping_int_min_absolute_value() {
        assert_eq!(compiled_post_lane_pseudo_abs(i32::MIN), i32::MIN);
        assert_eq!(
            derive_compiled_post_lane_global_normal(post_lane_square_input([i32::MIN, -1, -2,]))
                .selected_quad,
            CompiledPostLaneQuad::C
        );
        assert_eq!(
            derive_compiled_post_lane_global_normal(post_lane_square_input([i32::MIN, 0, 0,]))
                .selected_quad,
            CompiledPostLaneQuad::B
        );
    }

    #[test]
    fn post_lane_layout_and_quad_routing_match_the_static_contract() {
        assert_eq!(COMPILED_POST_LANE_SELECTOR_Y_OFFSETS, [0x25c, 0x268, 0x274]);
        assert_eq!(COMPILED_POST_LANE_OUTPUT_OFFSETS, [0x324, 0x328, 0x32c]);
        assert_eq!(CompiledPostLaneQuad::A.lane_indices(), [0, 1, 4, 5]);
        assert_eq!(CompiledPostLaneQuad::B.lane_indices(), [0, 1, 2, 3]);
        assert_eq!(CompiledPostLaneQuad::C.lane_indices(), [0, 2, 4, 6]);
    }

    fn post_lane_basis_input(n: [i32; 3], s: [i32; 3]) -> CompiledPostLaneBasisInputs {
        CompiledPostLaneBasisInputs {
            vector_324: n,
            vector_258: s,
        }
    }

    #[test]
    fn post_lane_basis_layout_store_order_and_disjoint_rows() {
        assert_eq!(
            COMPILED_POST_LANE_VECTOR_BASES,
            [0x258, 0x264, 0x270, 0x324]
        );
        assert_eq!(COMPILED_POST_LANE_STORE_BASE_ORDER, [0x264, 0x258, 0x270]);
        // Each destination row is three dwords and the rows are disjoint. The
        // old +0x264/+0x270 rows are write-only destinations: the pure contract
        // has no inputs for their prior contents, so they cannot affect it.
        for pair in COMPILED_POST_LANE_VECTOR_BASES[..3].windows(2) {
            assert!(pair[0] + 0x0c <= pair[1]);
        }
        for selector in COMPILED_POST_LANE_SELECTOR_Y_OFFSETS {
            assert!(
                COMPILED_POST_LANE_VECTOR_BASES[..3]
                    .iter()
                    .any(|base| (*base..*base + 0x0c).contains(&selector)),
                "old selector {selector:#x} is superseded by a destination row"
            );
        }
        assert!(
            COMPILED_POST_LANE_VECTOR_BASES[..3]
                .iter()
                .all(|base| !(*base..*base + 0x0c).contains(&0x324))
        );
    }

    #[test]
    fn post_lane_basis_orthogonal_vectors_match_the_generated_table_results() {
        let effects = derive_compiled_post_lane_basis(post_lane_basis_input(
            [0x10000, 0, 0],
            [0, 0x10000, 0],
        ));
        assert_eq!(effects.vector_258, [0, 65_522, 0]);
        assert_eq!(effects.vector_264, [0x10000, 0, 0]);
        assert_eq!(effects.vector_270, [0, 0, -65_535]);

        let effects = derive_compiled_post_lane_basis(post_lane_basis_input(
            [0x10000, 0, 0],
            [0x8000, 0, 0x10000],
        ));
        assert_eq!(effects.vector_258, [0, 0, 65_522]);
        assert_eq!(effects.vector_270, [0, 65_534, 0]);
    }

    #[test]
    fn post_lane_basis_removes_the_projection_onto_the_ak11_vector() {
        // dot = qmul(0x10000, 0x8000) = 0x8000, so rejected = s - n*dot leaves
        // exactly [0, 0x10000, 0] before normalization.
        let effects = derive_compiled_post_lane_basis(post_lane_basis_input(
            [0x10000, 0, 0],
            [0x8000, 0x10000, 0],
        ));
        assert_eq!(effects.vector_258, [0, 65_522, 0]);
        assert_eq!(effects.vector_270, [0, 0, -65_535]);
    }

    #[test]
    fn post_lane_basis_parallel_and_zero_vectors_degenerate_to_zero() {
        let parallel = derive_compiled_post_lane_basis(post_lane_basis_input(
            [0x10000, 0, 0],
            [0x20000, 0, 0],
        ));
        assert_eq!(parallel.vector_258, [0; 3]);
        assert_eq!(parallel.vector_264, [0x10000, 0, 0]);
        assert_eq!(parallel.vector_270, [0; 3]);

        let zero_normal =
            derive_compiled_post_lane_basis(post_lane_basis_input([0; 3], [0, 0x10000, 0]));
        assert_eq!(zero_normal.vector_258, [0, 65_522, 0]);
        assert_eq!(zero_normal.vector_264, [0; 3]);
        assert_eq!(zero_normal.vector_270, [0; 3]);

        let zero_prior =
            derive_compiled_post_lane_basis(post_lane_basis_input([0x10000, 0, 0], [0; 3]));
        assert_eq!(zero_prior.vector_258, [0; 3]);
        assert_eq!(zero_prior.vector_270, [0; 3]);
    }

    #[test]
    fn post_lane_basis_cross_is_u_cross_n_not_n_cross_u() {
        let effects = derive_compiled_post_lane_basis(post_lane_basis_input(
            [0x10000, 0, 0],
            [0, 0x10000, 0],
        ));
        assert_eq!(effects.vector_270, [0, 0, -65_535]);
        assert_ne!(effects.vector_270, [0, 0, 65_535]);
    }

    #[test]
    fn post_lane_basis_negative_odd_products_round_down_in_q16() {
        // qmul(3, -5) = -1 (the arithmetic >>16 floors the negative product),
        // so the rejected x component is -5 - qmul(3, -1) = -4, not the -5 a
        // truncate-toward-zero implementation would leave.
        let effects =
            derive_compiled_post_lane_basis(post_lane_basis_input([3, 0, 0], [-5, 0x10000, 0]));
        assert_eq!(effects.vector_258, [-4, 65_522, 0]);
        assert_eq!(effects.vector_264, [3, 0, 0]);
        // The cross z is -3; its Q16 square sum underflows to zero.
        assert_eq!(effects.vector_270, [0; 3]);
    }

    #[test]
    fn post_lane_basis_wraps_dot_projection_cross_and_i32_extremes() {
        fn reference(input: CompiledPostLaneBasisInputs) -> CompiledPostLaneBasisEffects {
            fn qmul(a: i32, b: i32) -> i32 {
                ((i64::from(a) * i64::from(b)) >> 16) as i32
            }
            fn normalize(vector: [i32; 3]) -> [i32; 3] {
                let square_sum = vector.into_iter().fold(0_i32, |sum, component| {
                    sum.wrapping_add(qmul(component, component))
                });
                let root = sqrt_q16(square_sum as u32);
                if root == 0 {
                    return [0; 3];
                }
                let reciprocal = ((1_u64 << 32) / u64::from(root)) as i32;
                vector.map(|component| qmul(component, reciprocal))
            }
            let n = input.vector_324;
            let s = input.vector_258;
            let dot = (0..3).fold(0_i32, |sum, index| {
                sum.wrapping_add(qmul(n[index], s[index]))
            });
            let rejected = std::array::from_fn(|index| s[index].wrapping_sub(qmul(n[index], dot)));
            let u = normalize(rejected);
            let cross = [
                qmul(u[1], n[2]).wrapping_sub(qmul(u[2], n[1])),
                qmul(u[2], n[0]).wrapping_sub(qmul(u[0], n[2])),
                qmul(u[0], n[1]).wrapping_sub(qmul(u[1], n[0])),
            ];
            CompiledPostLaneBasisEffects {
                vector_258: u,
                vector_264: n,
                vector_270: normalize(cross),
            }
        }

        for (n, s) in [
            ([i32::MAX, 0, 0], [i32::MAX, 0, 0]),
            ([i32::MIN, i32::MAX, -1], [i32::MAX, i32::MIN, 1]),
            (
                [i32::MIN, i32::MIN, i32::MIN],
                [i32::MIN, i32::MIN, i32::MIN],
            ),
            ([1, -1, i32::MAX], [i32::MIN, 1, -1]),
        ] {
            let input = post_lane_basis_input(n, s);
            assert_eq!(derive_compiled_post_lane_basis(input), reference(input));
        }
    }

    #[test]
    fn post_lane_basis_stores_the_ak11_vector_verbatim_not_renormalized() {
        let effects =
            derive_compiled_post_lane_basis(post_lane_basis_input([3, 5, 7], [11, 13, 17]));
        assert_eq!(effects.vector_264, [3, 5, 7]);
    }

    #[test]
    fn post_lane_publication_writes_rows_in_order_and_preserves_padding() {
        assert_eq!(COMPILED_POST_LANE_PUBLICATION_ROW_STRIDE, 0x10);
        assert_eq!(
            COMPILED_POST_LANE_PUBLICATION_SOURCE_BASES,
            [0x258, 0x264, 0x270]
        );
        assert_eq!(
            COMPILED_POST_LANE_PUBLICATION_DESTINATION_OFFSETS,
            [0x00, 0x10, 0x20]
        );
        for offsets in COMPILED_POST_LANE_PUBLICATION_DESTINATION_OFFSETS.windows(2) {
            assert_eq!(
                offsets[1] - offsets[0],
                COMPILED_POST_LANE_PUBLICATION_ROW_STRIDE
            );
        }

        let effects = CompiledPostLaneBasisEffects {
            vector_258: [1, 2, 3],
            vector_264: [4, 5, 6],
            vector_270: [7, 8, 9],
        };
        let mut destination = [
            [-1, -2, -3, 0x5555],
            [-4, -5, -6, 0x6666],
            [-7, -8, -9, 0x7777],
        ];
        publish_compiled_post_lane_basis(&mut destination, effects);
        // Publication order is the rebuilt prior row, the verbatim AK11 row,
        // then the cross row; each padding dword (+0x0c/+0x1c/+0x2c) survives.
        assert_eq!(destination[0], [1, 2, 3, 0x5555]);
        assert_eq!(destination[1], [4, 5, 6, 0x6666]);
        assert_eq!(destination[2], [7, 8, 9, 0x7777]);
    }

    #[test]
    fn state_adjustment_accumulates_with_wrapping_difference_and_sum() {
        assert_eq!(
            derive_compiled_state_adjustment_cc(CompiledStateAdjustmentInputs {
                prior_cc: 5,
                word_2e4: 10,
                word_100: 3,
            }),
            12
        );
        assert_eq!(
            derive_compiled_state_adjustment_cc(CompiledStateAdjustmentInputs {
                prior_cc: 5,
                word_2e4: 3,
                word_100: 10,
            }),
            -2
        );
        // The difference wraps before it is added.
        assert_eq!(
            derive_compiled_state_adjustment_cc(CompiledStateAdjustmentInputs {
                prior_cc: 0,
                word_2e4: i32::MIN,
                word_100: 1,
            }),
            i32::MAX
        );
        // The accumulated sum wraps too.
        assert_eq!(
            derive_compiled_state_adjustment_cc(CompiledStateAdjustmentInputs {
                prior_cc: i32::MAX,
                word_2e4: 1,
                word_100: 0,
            }),
            i32::MIN
        );
        assert_eq!(
            derive_compiled_state_adjustment_cc(CompiledStateAdjustmentInputs {
                prior_cc: i32::MIN,
                word_2e4: 0,
                word_100: 1,
            }),
            i32::MAX
        );
    }

    #[test]
    fn state_adjustment_publication_uses_updated_values_and_preserves_padding() {
        assert_eq!(
            COMPILED_STATE_ADJUSTMENT_PUBLICATION_OFFSETS,
            [0x30, 0x34, 0x38]
        );
        assert_eq!(COMPILED_STATE_ADJUSTMENT_SOURCE_BASES, [0xc8, 0xcc, 0xd0]);
        assert_eq!(
            COMPILED_STATE_ADJUSTMENT_CALL_TARGETS,
            [0x492770, 0x409c10, 0x4126b0]
        );

        let adjusted = derive_compiled_state_adjustment_cc(CompiledStateAdjustmentInputs {
            prior_cc: 5,
            word_2e4: 10,
            word_100: 3,
        });
        let mut destination = [-1, -2, -3, 0x5555];
        publish_compiled_state_adjustment(&mut destination, 100, adjusted, 300);
        assert_eq!(destination, [100, 12, 300, 0x5555]);
    }

    #[test]
    fn adjustment_reset_pins_state_zero_offsets_and_words() {
        let effects = derive_compiled_adjustment_reset();
        assert_eq!(
            effects.state_zero_offsets,
            [0x3cc, 0x3d0, 0x3d4, 0x3d8, 0x3dc, 0x3e0]
        );
        for pair in effects.state_zero_offsets.windows(2) {
            assert_eq!(pair[1] - pair[0], 4);
        }
        assert_eq!(effects.state_words, [(0xbc, 0), (0xc0, 0x10000), (0xc4, 0)]);
        for (offset, _) in effects.state_words {
            assert!(!effects.state_zero_offsets.contains(&offset));
        }
    }

    #[test]
    fn adjustment_control_clear_matches_the_descending_native_loop() {
        // Independent simulation of the native do-while: start at 0x3cc, write
        // offset-0x10 then offset, subtract four, continue while above 0x3bf.
        let mut simulated = Vec::new();
        let mut offset = 0x3cc_u16;
        loop {
            simulated.push(offset - 0x10);
            simulated.push(offset);
            let next = offset - 4;
            if next <= 0x3bf {
                break;
            }
            offset = next;
        }
        let effects = derive_compiled_adjustment_reset();
        assert_eq!(simulated.as_slice(), &effects.control_zero_offsets);

        // The eight cleared dwords are exactly the contiguous 0x3b0..0x3cc range.
        let mut sorted = effects.control_zero_offsets;
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            [0x3b0, 0x3b4, 0x3b8, 0x3bc, 0x3c0, 0x3c4, 0x3c8, 0x3cc]
        );
    }

    #[test]
    fn initializer_control_flag_depends_only_on_config_bit_0x10() {
        assert_eq!(COMPILED_INITIALIZER_CONTROL_FLAG_OFFSET, 0x3f0);
        for (config, expected) in [(0x00, 2), (0x0f, 2), (0x10, 1), (0x1f, 1), (0xff, 1)] {
            assert_eq!(derive_compiled_initializer_control_flag(config), expected);
        }
    }

    #[test]
    fn initializer_flag_effects_pin_constant_writes_and_the_vector_copy() {
        let effects = derive_compiled_initializer_flag_effects();
        assert_eq!(effects.control_zero_dwords, [0x3f4, 0x3f8, 0x3fc, 0x3ec]);
        assert_eq!(effects.control_zero_bytes, [0x43c, 0x43d, 0x43e]);
        assert_eq!(
            effects.control_constant_dwords,
            [(0x37c, -0x4ccc), (0x41c, 0)]
        );
        assert_eq!(effects.state_zero_dwords, [0x684, 0x610, 0x678, 0x6e4]);
        assert_eq!(effects.state_zero_u16_offsets, [0x60c, 0x614]);
        assert_eq!(effects.state_vector_copy, (0x258, 0x5d4));

        // The flag and the constant writes target distinct control dwords.
        for offset in effects.control_zero_dwords {
            assert_ne!(offset, COMPILED_INITIALIZER_CONTROL_FLAG_OFFSET);
        }
        for (offset, _) in effects.control_constant_dwords {
            assert!(!effects.control_zero_dwords.contains(&offset));
            assert_ne!(offset, COMPILED_INITIALIZER_CONTROL_FLAG_OFFSET);
        }
        // The copy moves three dwords from +0x258 to +0x5d4 without overlap.
        let (source, destination) = effects.state_vector_copy;
        assert!(destination >= source + 0x0c || source >= destination + 0x0c);
    }

    #[test]
    fn initializer_6f8_short_circuits_the_second_call() {
        let calls = Cell::new(0);
        let second = || {
            calls.set(calls.get() + 1);
            0
        };
        assert_eq!(derive_compiled_initializer_6f8(8, second), 1);
        assert_eq!(calls.get(), 0, "the second callee is skipped after an 8");

        let calls = Cell::new(0);
        let second = || {
            calls.set(calls.get() + 1);
            0
        };
        assert_eq!(derive_compiled_initializer_6f8(7, second), 0);
        assert_eq!(calls.get(), 1);
        assert_eq!(derive_compiled_initializer_6f8(7, || 3), 1);
        assert_eq!(derive_compiled_initializer_6f8(0, || -1), 1);
    }

    #[test]
    fn initializer_6e8_zero_branch_and_signed_remainder_word() {
        // The zero branch ignores the RNG value entirely.
        for input in [
            CompiledInitializer6e8Inputs {
                global_550c08: 2,
                global_d3ca88: 0,
                random: 1,
            },
            CompiledInitializer6e8Inputs {
                global_550c08: 0,
                global_d3ca88: 1,
                random: u32::MAX,
            },
        ] {
            assert_eq!(
                derive_compiled_initializer_6e8(input),
                CompiledInitializer6e8Effects {
                    word_6e8: 0,
                    word_6ec: 0,
                }
            );
        }

        let word = |random| {
            derive_compiled_initializer_6e8(CompiledInitializer6e8Inputs {
                global_550c08: 0,
                global_d3ca88: 0,
                random,
            })
        };
        // Signed remainder by two, then *10+13: 0 -> 13, +1 -> 23, -1 -> 3.
        for (random, expected) in [
            (0, 13),
            (1, 23),
            (2, 13),
            (3, 23),
            (0x8000_0000, 13),
            (0x8000_0001, 3),
            (u32::MAX, 3),
        ] {
            let effects = word(random);
            assert_eq!(effects.word_6e8, 1);
            assert_eq!(effects.word_6ec, expected, "random {random:#x}");
        }
    }

    #[test]
    fn initializer_copies_are_verbatim_and_pinned_to_their_offsets() {
        assert_eq!(COMPILED_INITIALIZER_COPY_FC, (0xfc, 0x1bc, 24));
        assert_eq!(COMPILED_INITIALIZER_COPY_258, (0x258, 0x21c, 9));
        assert_eq!(COMPILED_INITIALIZER_COPY_240_SOURCES, [0xc8, 0xcc, 0xd0]);
        assert_eq!(COMPILED_INITIALIZER_COPY_24C_SOURCE, 0x324);

        let input = CompiledInitializerCopyInputs {
            source_fc: std::array::from_fn(|index| index as i32 - 12),
            source_258: std::array::from_fn(|index| 100 + index as i32),
            source_c8: 1,
            source_cc: 2,
            source_d0: 3,
            source_324: [7, 8, 9],
        };
        let effects = derive_compiled_initializer_copies(input);
        assert_eq!(effects.block_1bc, input.source_fc);
        assert_eq!(effects.block_21c, input.source_258);
        assert_eq!(effects.block_240, [1, 2, 3]);
        assert_eq!(effects.block_24c, [7, 8, 9]);

        // Source and destination ranges of each loop are disjoint.
        let (fc_source, fc_destination, fc_len) = COMPILED_INITIALIZER_COPY_FC;
        assert!(fc_destination >= fc_source + 4 * fc_len as u16);
        let (source, destination, len) = COMPILED_INITIALIZER_COPY_258;
        // The second loop copies downward: +0x21c..+0x240 precedes +0x258.
        assert!(destination + 4 * len as u16 <= source);
        // The +0x240 write lands immediately after the second loop's range.
        assert_eq!(destination + 4 * len as u16, 0x240);
    }

    #[test]
    fn initializer_5fc_inverts_config_bit_zero_and_final_flags_are_pinned() {
        for (config, expected) in [
            (0x00, 0x9999),
            (0x01, 0x1999),
            (0x02, 0x9999),
            (0xff, 0x1999),
        ] {
            assert_eq!(derive_compiled_initializer_5fc(config), expected);
        }
        assert_eq!(COMPILED_INITIALIZER_FINAL_ZERO_STATE_BYTES, [0x74d, 0x74e]);
        assert_eq!(COMPILED_INITIALIZER_FINAL_ZERO_CONTROL_BYTE, 0x428);
        assert_eq!(COMPILED_INITIALIZER_FINAL_CONTROL_ONE_DWORD, 0x420);
        assert!(!compiled_initializer_final_call_required(0));
        assert!(compiled_initializer_final_call_required(-1));
    }

    #[test]
    fn initializer_48d2d0_reproduces_the_detail_weather_intensity_branches() {
        // Detail 8 is always false, even with otherwise-true inputs.
        assert!(!derive_compiled_initializer_48d2d0(8, 5, 0));
        assert!(!derive_compiled_initializer_48d2d0(8, 3, 0));
        // Weather outside 3..=5 is false.
        for weather in [0, 1, 2, 6, -1] {
            assert!(!derive_compiled_initializer_48d2d0(0, weather, 0));
        }
        // Weather 3/4 defers to intensity != 1.
        for weather in [3, 4] {
            assert!(!derive_compiled_initializer_48d2d0(0, weather, 1));
            assert!(derive_compiled_initializer_48d2d0(0, weather, 0));
            assert!(derive_compiled_initializer_48d2d0(0, weather, 2));
        }
        // Weather 5 is true regardless of intensity.
        assert!(derive_compiled_initializer_48d2d0(0, 5, 1));
        assert!(derive_compiled_initializer_48d2d0(0, 5, 0));
    }

    #[test]
    fn callee_92770_stores_vectors_by_switch_and_always_zeroes_188() {
        assert_eq!(COMPILED_92770_NEXT_CALL, 0x492050);
        let source = [1, 2, 3];
        for switch in [0, 3] {
            let effects = derive_compiled_92770(switch, source);
            assert_eq!(effects.low_vector, None);
            assert_eq!(effects.high_vector, Some(source));
            assert!(effects.zero_188);
        }
        for switch in [1, 2] {
            let effects = derive_compiled_92770(switch, source);
            assert_eq!(effects.low_vector, Some(source));
            assert_eq!(effects.high_vector, Some(source));
            assert!(effects.zero_188);
        }
        for switch in [4, -1, i32::MAX] {
            let effects = derive_compiled_92770(switch, source);
            assert_eq!(effects.low_vector, None);
            assert_eq!(effects.high_vector, None);
            assert!(effects.zero_188);
        }
    }

    #[test]
    fn lane_thresholds_pass_fail_and_record_failing_lanes_in_descending_order() {
        // All pass.
        let effects = derive_compiled_lane_thresholds(CompiledLaneThresholdInputs {
            lane_words: [0; 8],
            source_y: [0x1999 + 1; 8],
        });
        assert_eq!(effects.flags_73a, [1; 8]);
        assert_eq!(effects.flag_743, 1);
        assert_eq!(effects.count_742, 0);
        assert_eq!(effects.indices_744, [0; 8]);

        // Lane 2 and lane 5 fail; equality fails because the test is strict <.
        let mut source_y = [0x1999 + 1; 8];
        source_y[2] = 0x1999;
        source_y[5] = 0;
        let effects = derive_compiled_lane_thresholds(CompiledLaneThresholdInputs {
            lane_words: [0; 8],
            source_y,
        });
        let mut expected_flags = [1; 8];
        expected_flags[2] = 0;
        expected_flags[5] = 0;
        assert_eq!(effects.flags_73a, expected_flags);
        assert_eq!(effects.flag_743, 0);
        assert_eq!(effects.count_742, 2);
        assert_eq!(effects.indices_744, [5, 2, 0, 0, 0, 0, 0, 0]);

        // All fail: indices are recorded in native descending lane order.
        let effects = derive_compiled_lane_thresholds(CompiledLaneThresholdInputs {
            lane_words: [0; 8],
            source_y: [0; 8],
        });
        assert_eq!(effects.flags_73a, [0; 8]);
        assert_eq!(effects.flag_743, 0);
        assert_eq!(effects.count_742, 8);
        assert_eq!(effects.indices_744, [7, 6, 5, 4, 3, 2, 1, 0]);
    }

    #[test]
    fn lane_threshold_comparison_wraps_its_lane_word_addition() {
        // lane_word + 0x1999 wraps negative, so the comparison passes.
        let effects = derive_compiled_lane_thresholds(CompiledLaneThresholdInputs {
            lane_words: [i32::MAX; 8],
            source_y: [0; 8],
        });
        assert_eq!(effects.flags_73a, [1; 8]);
        assert_eq!(effects.flag_743, 1);
        assert_eq!(effects.count_742, 0);
    }

    #[test]
    fn special_copy_gate_and_blocks_match_the_global_copy_contract() {
        assert!(!compiled_special_copy_required(0));
        assert!(!compiled_special_copy_required(2));
        assert!(compiled_special_copy_required(1));
        assert_eq!(
            COMPILED_SPECIAL_COPY_BLOCKS,
            [(0x5fffc8, 0x60042c, 0x119), (0x700300, 0x700a68, 0x1da)]
        );
        for (source, destination, count) in COMPILED_SPECIAL_COPY_BLOCKS {
            // Each destination begins immediately after its source range.
            assert_eq!(destination, source + 4 * count as u32);
        }
    }

    #[test]
    fn callee_92050_sets_every_chain_flag_and_tolerates_an_empty_chain() {
        assert_eq!(COMPILED_92050_LINK_OFFSET, 0x8);
        assert_eq!(COMPILED_92050_FLAG_OFFSET, 0x170);
        let mut empty: [i32; 0] = [];
        apply_compiled_92050(&mut empty);
        let mut flags = [0, 7, -3, 0x1234];
        apply_compiled_92050(&mut flags);
        assert_eq!(flags, [1, 1, 1, 1]);
    }

    fn spine_input() -> CompiledSpineInputs {
        let mut execution = integrated_execution_input();
        execution.source_vectors = post_lane_square_input([0, 0, 0]).source_vectors;
        CompiledSpineInputs {
            execution,
            normal_selector_words: [0, 0, 9],
            prior_vector_258: [0, 0x10000, 0],
            publication_destination: [[9, 9, 9, 0x1111], [8, 8, 8, 0x2222], [7, 7, 7, 0x3333]],
            adjustment_prior_cc: 5,
            adjustment_destination: [-1, -2, -3, 0x5555],
            state_c8: 100,
            state_d0: 300,
            switch_180: 1,
            detail: 0,
            weather: 5,
            intensity: 0,
            config_b6: 0,
            global_550c08: 0,
            global_d3ca88: 0,
            random: 1,
            config_25: 0,
        }
    }

    fn setup_pose_fixture() -> (CompiledDirectCaseSet, CompiledSetupPoseInputs) {
        let mut cases = synthetic_cases();
        cases[0] = CompiledCaseWords {
            state_words: [0x2_0000, 0x2_0000, 0x2_0000, 0x2000, 0x3000, 0x4000, 0, 0],
            control_words: [0; COMPILED_CONTROL_WORD_COUNT],
        };
        (
            CompiledDirectCaseSet { cases },
            CompiledSetupPoseInputs {
                prior_case_words: synthetic_case(-1),
                selector: 0,
                center: [0x10_0000, 0x20_0000, 0x30_0000],
                matrix_rows: [[0x1_0000, 0, 0], [0, 0x1_0000, 0], [0, 0, 0x1_0000]],
                common_config_25: 0,
                lane: CompiledSetupPoseLaneInputs {
                    gate_words: [0; 8],
                    drive_words: [0; 8],
                    first_four_subtractions: [0; 4],
                },
            },
        )
    }

    #[test]
    fn setup_pose_derives_all_source_snapshots_and_uses_contact_source_for_lane0_y() {
        let (cases, mut input) = setup_pose_fixture();
        input.common_config_25 = COMPILED_COMMON_STATE_SCALE_MASK;
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(&events, [IntegratedLaneMode::Built(0); 8]);
        let mut config = |_lane| -> Result<u8, &'static str> { Ok(0) };
        let effects = execute_compiled_setup_pose(
            &cases,
            input,
            integrated_execution_state(),
            &mut providers,
            &mut config,
        )
        .unwrap();

        assert_eq!(effects.selected_case, Some(0));
        assert_eq!(
            effects.effective_state_words,
            [0x1_0000, 0x1_0000, 0x1_0000, 0x1000, 0x1800, 0x2000, 0, 0]
        );
        assert_eq!(effects.sources.initial, effects.initial_basis.vectors);
        assert_eq!(
            effects.adjusted_center[1],
            input.center[1]
                .wrapping_add(effects.lanes.state.lane_words[0])
                .wrapping_sub(effects.sources.contact_basis[0][1])
        );
        assert_eq!(
            effects.corrected_basis,
            derive_compiled_post_lane_basis(CompiledPostLaneBasisInputs {
                vector_324: effects.normal.final_published_normal,
                vector_258: input.matrix_rows[0],
            })
        );
    }

    #[test]
    fn setup_pose_config_failure_returns_no_pose_and_preserves_caller_state() {
        let (cases, input) = setup_pose_fixture();
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(&events, [IntegratedLaneMode::Built(0); 8]);
        let state = integrated_execution_state();
        let retained_state = state;
        let mut config = |lane| -> Result<u8, &'static str> {
            if lane == 0 {
                Err("fresh config")
            } else {
                Ok(0)
            }
        };

        let failure =
            execute_compiled_setup_pose(&cases, input, state, &mut providers, &mut config)
                .unwrap_err();

        assert_eq!(state, retained_state);
        assert!(matches!(
            failure,
            CompiledSetupPoseError::Lanes(CompiledEightLaneExecutionFailure::Config {
                failed_lane: 0,
                ..
            })
        ));
    }

    #[test]
    fn composed_spine_runs_every_stage_in_native_order() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(
            &events,
            std::array::from_fn(|lane| IntegratedLaneMode::Built(if lane == 2 { 0x51 } else { 0 })),
        );
        let mut config =
            |lane| -> Result<u8, &'static str> { Ok(if lane == 2 { 0x02 } else { 0 }) };
        let input = spine_input();
        let success = execute_compiled_spine_post(
            input.clone(),
            integrated_execution_state(),
            &mut providers,
            &mut config,
        )
        .unwrap();

        let normal = derive_compiled_post_lane_global_normal(CompiledPostLaneNormalInput {
            source_vectors: input.execution.source_vectors,
            lane_words: success.lanes.state.lane_words,
            selector_words: [0, 0, 9],
        });
        let basis = derive_compiled_post_lane_basis(CompiledPostLaneBasisInputs {
            vector_324: normal.final_published_normal,
            vector_258: [0, 0x10000, 0],
        });
        let mut destination = input.publication_destination;
        publish_compiled_post_lane_basis(&mut destination, basis);
        let cc = derive_compiled_state_adjustment_cc(CompiledStateAdjustmentInputs {
            prior_cc: 5,
            word_2e4: success.lanes.state.lane_words[0],
            word_100: input.execution.source_vectors[0][1],
        });
        let mut adjustment_destination = input.adjustment_destination;
        publish_compiled_state_adjustment(&mut adjustment_destination, 100, cc, 300);

        assert_eq!(success.normal, normal);
        assert_eq!(success.basis, basis);
        assert_eq!(success.destination, destination);
        assert_eq!(success.cc, cc);
        assert_eq!(success.adjustment_destination, adjustment_destination);
        assert_eq!(
            success.callee_92770,
            derive_compiled_92770(1, [100, cc, 300])
        );
        assert_eq!(
            success.lane_thresholds,
            derive_compiled_lane_thresholds(CompiledLaneThresholdInputs {
                lane_words: success.lanes.state.lane_words,
                source_y: input.execution.source_vectors.map(|vector| vector[1]),
            })
        );
        assert_eq!(success.reset, derive_compiled_adjustment_reset());
        assert_eq!(success.flags, derive_compiled_initializer_flag_effects());
        assert_eq!(success.control_flag, 2);
        assert_eq!(success.flag_6f8, 1);
        assert_eq!(success.flag_6f0, 1);
        assert_eq!(
            success.words_6e8,
            CompiledInitializer6e8Effects {
                word_6e8: 1,
                word_6ec: 23,
            }
        );
        assert_eq!(success.copies.block_24c, normal.final_published_normal);
        assert_eq!(success.copies.block_240, [100, cc, 300]);
        assert_eq!(
            success.copies.block_1bc,
            std::array::from_fn(|index| input.execution.source_vectors[index / 3][index % 3])
        );
        assert_eq!(
            success.copies.block_21c,
            std::array::from_fn(|index| {
                [basis.vector_258, basis.vector_264, basis.vector_270][index / 3][index % 3]
            })
        );
        assert_eq!(success.word_5fc, 0x9999);
        assert!(!success.final_call_required);
    }

    #[test]
    fn composed_spine_aborts_before_any_post_effect_when_a_lane_fails() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(
            &events,
            std::array::from_fn(|lane| {
                if lane == 7 {
                    IntegratedLaneMode::SearchThenBuildFailure
                } else {
                    IntegratedLaneMode::Built(0)
                }
            }),
        );
        let mut state = integrated_execution_state();
        state.handles[7] = -1;
        let mut config = |_lane| -> Result<u8, &'static str> { Ok(0) };
        let failure =
            execute_compiled_spine_post(spine_input(), state, &mut providers, &mut config)
                .unwrap_err();
        assert!(matches!(
            failure,
            CompiledEightLaneExecutionFailure::Dispatch { failed_lane: 7, .. }
        ));
    }

    fn full_initializer_input(config: &[u8]) -> CompiledInitializerInputs<'_> {
        CompiledInitializerInputs {
            early: RecoveredCompiledStageInputs {
                prior_case_words: synthetic_case(-7),
                selector: 3,
                config,
                state_slot: 1,
                global_word: 0x1_0000,
                state_758_context_nonzero: false,
                state_758_table: &[],
            },
            basis: state_basis_input(),
            spine: spine_input(),
        }
    }

    #[test]
    fn full_initializer_composes_early_basis_locals_and_spine_in_order() {
        let direct_cases = CompiledDirectCaseSet {
            cases: synthetic_cases(),
        };
        let aux_constants = AuxControlConstantSet {
            constants: aux_constants(),
        };
        let config = compiled_stage_config(1, 0, 1, 1, 0, false);
        let input = full_initializer_input(&config);
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(
            &events,
            std::array::from_fn(|lane| IntegratedLaneMode::Built(if lane == 2 { 0x51 } else { 0 })),
        );
        let mut lane_config =
            |lane| -> Result<u8, &'static str> { Ok(if lane == 2 { 0x02 } else { 0 }) };
        let success = compose_compiled_initializer(
            &direct_cases,
            &aux_constants,
            input.clone(),
            integrated_execution_state(),
            &mut providers,
            &mut lane_config,
        )
        .unwrap();

        assert_eq!(
            success.early,
            compose_recovered_compiled_stage_effects(&direct_cases, &aux_constants, input.early)
                .unwrap()
        );
        assert_eq!(
            success.basis,
            derive_compiled_state_basis_effects(input.basis)
        );
        assert_eq!(success.locals, derive_compiled_call_spine_local_effects());
        let mut replay_providers = integrated_providers(
            &events,
            std::array::from_fn(|lane| IntegratedLaneMode::Built(if lane == 2 { 0x51 } else { 0 })),
        );
        assert_eq!(
            success.spine,
            execute_compiled_spine_post(
                input.spine,
                integrated_execution_state(),
                &mut replay_providers,
                &mut lane_config,
            )
            .unwrap()
        );
    }

    #[test]
    fn full_initializer_early_failure_returns_before_any_lane() {
        let direct_cases = CompiledDirectCaseSet {
            cases: synthetic_cases(),
        };
        let aux_constants = AuxControlConstantSet {
            constants: aux_constants(),
        };
        let config = compiled_stage_config(1, 0, 1, 1, 0, false);
        let mut input = full_initializer_input(&config);
        input.early.state_758_context_nonzero = true;
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(&events, [IntegratedLaneMode::Built(0); 8]);
        let mut lane_config = |_lane| -> Result<u8, &'static str> { Ok(0) };
        let failure = compose_compiled_initializer(
            &direct_cases,
            &aux_constants,
            input,
            integrated_execution_state(),
            &mut providers,
            &mut lane_config,
        )
        .unwrap_err();
        assert!(matches!(
            failure,
            CompiledInitializerError::Early(RecoveredCompiledStageError::State758Selection(_))
        ));
        assert!(
            events.borrow().is_empty(),
            "no lane provider ran after the early failure"
        );
    }

    #[test]
    fn full_initializer_lane_failure_aborts_before_the_post_spine() {
        let direct_cases = CompiledDirectCaseSet {
            cases: synthetic_cases(),
        };
        let aux_constants = AuxControlConstantSet {
            constants: aux_constants(),
        };
        let config = compiled_stage_config(1, 0, 1, 1, 0, false);
        let input = full_initializer_input(&config);
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(
            &events,
            std::array::from_fn(|lane| {
                if lane == 7 {
                    IntegratedLaneMode::SearchThenBuildFailure
                } else {
                    IntegratedLaneMode::Built(0)
                }
            }),
        );
        let mut state = integrated_execution_state();
        state.handles[7] = -1;
        let mut lane_config = |_lane| -> Result<u8, &'static str> { Ok(0) };
        let failure = compose_compiled_initializer(
            &direct_cases,
            &aux_constants,
            input,
            state,
            &mut providers,
            &mut lane_config,
        )
        .unwrap_err();
        assert!(matches!(
            failure,
            CompiledInitializerError::Lanes(CompiledEightLaneExecutionFailure::Dispatch {
                failed_lane: 7,
                ..
            })
        ));
    }

    #[test]
    fn composed_initializer_runs_over_an_authenticated_stage_snapshot() {
        use std::fs;
        use std::io::Write;
        use std::path::PathBuf;
        use std::sync::atomic::{AtomicU64, Ordering};

        use cmr_assets::{AssetFile, AssetProfile, AssetStore, ReviewedProfile, StorageEncoding};
        use cmr_formats::stored_asset::DecodeLimits;
        use flate2::Compression;
        use flate2::write::GzEncoder;

        use crate::stage_compiled_geometry::StageCompiledGeometry;

        static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
        struct TestDirectory(PathBuf);
        impl TestDirectory {
            fn new() -> Self {
                let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "cmr-initializer-authenticated-{}-{id}",
                    std::process::id()
                ));
                fs::create_dir(&path).unwrap();
                Self(path)
            }
        }
        impl Drop for TestDirectory {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        fn sha256(bytes: &[u8]) -> String {
            use std::fmt::Write as _;
            let mut output = String::with_capacity(64);
            for byte in Sha256::digest(bytes) {
                write!(output, "{byte:02x}").unwrap();
            }
            output
        }
        fn cmpr(payload: &[u8]) -> Vec<u8> {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(b"CMPR").unwrap();
            encoder
                .write_all(&(payload.len() as u32).to_le_bytes())
                .unwrap();
            encoder.write_all(payload).unwrap();
            encoder.finish().unwrap()
        }
        fn triangle_hpc() -> Vec<u8> {
            let vertices: [[i32; 3]; 3] = [
                [0, 0x2_0000, 0],
                [0, 0x2_0000, 0x1_0000],
                [0x1_0000, 0x2_0000, 0],
            ];
            let mut payload = Vec::new();
            for word in [0_i32, 0] {
                payload.extend_from_slice(&word.to_le_bytes());
            }
            for count in [1_i16, 0, 0, 0, 0] {
                payload.extend_from_slice(&count.to_le_bytes());
            }
            payload.extend_from_slice(&0_u16.to_le_bytes());
            payload.extend_from_slice(&1_i16.to_le_bytes());
            payload.extend_from_slice(&0_u16.to_le_bytes());
            payload.extend_from_slice(&0_i32.to_le_bytes());
            payload.extend_from_slice(&3_u16.to_le_bytes());
            payload.extend_from_slice(&0_u16.to_le_bytes());
            for vertex in vertices {
                for component in vertex {
                    payload.extend_from_slice(&component.to_le_bytes());
                }
            }
            payload.extend_from_slice(&1_u16.to_le_bytes());
            payload.extend_from_slice(&0_u16.to_le_bytes());
            for word in [0_u16, 1, 2, 0x1234] {
                payload.extend_from_slice(&word.to_le_bytes());
            }
            payload.extend_from_slice(&1_u32.to_le_bytes());
            payload.extend_from_slice(&0_i16.to_le_bytes());
            payload.extend_from_slice(&1_i16.to_le_bytes());
            payload.extend_from_slice(&1_i16.to_le_bytes());
            payload
        }

        let temp = TestDirectory::new();
        let source_dir = temp.0.join("source");
        let stage_dir = source_dir.join("Game/Test");
        fs::create_dir_all(&stage_dir).unwrap();
        let stored = cmpr(&triangle_hpc());
        fs::write(stage_dir.join("TEST.hpc"), &stored).unwrap();
        let profile = AssetProfile {
            schema_version: 1,
            id: "synthetic-initializer-hpc".to_owned(),
            roots: vec!["Game".to_owned()],
            empty_directories: Vec::new(),
            sentinels: Vec::new(),
            files: vec![AssetFile {
                path: "Game/Test/TEST.hpc".to_owned(),
                size: stored.len() as u64,
                sha256: sha256(&stored),
                encoding: StorageEncoding::GzipCmpr,
            }],
        };
        let reviewed = ReviewedProfile::from_json(&serde_json::to_vec(&profile).unwrap()).unwrap();
        let imported = temp.0.join("runtime-data");
        cmr_assets::import_source(&source_dir, &imported, &reviewed).unwrap();
        let store = AssetStore::open(
            &imported,
            &reviewed,
            DecodeLimits {
                max_stored_bytes: 1024 * 1024,
                max_payload_bytes: 1024 * 1024,
            },
        )
        .unwrap();
        let geometry = StageCompiledGeometry::load(&store, "Game/Test/TEST").unwrap();
        let mut providers = geometry.eight_lane_providers();

        let direct_cases = CompiledDirectCaseSet {
            cases: synthetic_cases(),
        };
        let aux_constants = AuxControlConstantSet {
            constants: aux_constants(),
        };
        let config = compiled_stage_config(1, 0, 1, 1, 0, false);
        let mut input = full_initializer_input(&config);
        let source = [0x4000, i32::MIN, 0x4000];
        input.spine.execution.source_vectors = [source; 8];
        let state = CompiledEightLaneExecutionState {
            handles: [0; 8],
            type_words: [u16::MAX; 8],
            output_vectors: [[i32::MIN; 3]; 8],
            lane_words: [-99; 8],
            accumulator_words: [0; 8],
            latch_67c: 0,
        };
        let mut lane_config = |_lane| -> Result<u8, &'static str> { Ok(0) };
        let success = compose_compiled_initializer(
            &direct_cases,
            &aux_constants,
            input.clone(),
            state,
            &mut providers,
            &mut lane_config,
        )
        .unwrap();

        assert_eq!(success.spine.lanes.state.handles, [0; 8]);
        assert_eq!(success.spine.lanes.state.type_words, [0x1234; 8]);
        assert_eq!(
            success.spine.lanes.state.output_vectors,
            [[0, 0x1_0000, 0]; 8]
        );
        let normal = derive_compiled_post_lane_global_normal(CompiledPostLaneNormalInput {
            source_vectors: [source; 8],
            lane_words: success.spine.lanes.state.lane_words,
            selector_words: [0, 0, 9],
        });
        let basis = derive_compiled_post_lane_basis(CompiledPostLaneBasisInputs {
            vector_324: normal.final_published_normal,
            vector_258: [0, 0x10000, 0],
        });
        assert_eq!(success.spine.normal, normal);
        assert_eq!(success.spine.basis, basis);
        assert_eq!(
            success.spine.copies.block_24c,
            normal.final_published_normal
        );
        assert_eq!(success.spine.copies.block_240, [100, success.spine.cc, 300]);
        assert_eq!(
            success.basis,
            derive_compiled_state_basis_effects(input.basis)
        );
        assert_eq!(success.locals, derive_compiled_call_spine_local_effects());
    }

    #[test]
    fn decoded_geometry_provider_preserves_lazy_search_failure_and_miss_prefixes() {
        let vertices = [
            [0, 0x2_0000, 0],
            [0, 0x2_0000, 0x1_0000],
            [0x1_0000, 0x2_0000, 0],
        ];
        let records = [DecodedCompiledLaneRecord {
            vertex_indices: [0, 1, 2],
            type_word: 5,
        }; 6];
        let leaf = [spatial_cell(2, 0)];
        let hierarchy = DecodedCompiledLaneSpatialHierarchy {
            origin_xz: [0, 0],
            width: 1,
            height: 1,
            levels: [&leaf, &[], &[], &[], &[], &[]],
        };
        let mut provider = DecodedCompiledLaneGeometryProvider {
            hierarchy,
            candidate_pool: &[5],
            tables: DecodedCompiledLaneTables {
                records: &records,
                vertices: &vertices,
            },
        };
        let failure = dispatch_compiled_lane(
            CompiledLaneDispatchInput {
                source_vector: [0x4000, 0, 0x4000],
                initial_handle: -1,
                fallback: 0,
            },
            &mut provider,
        )
        .unwrap_err();
        assert_eq!(failure.phase, CompiledLaneDispatchFailurePhase::Search);
        assert!(failure.handle_writes.normalized_to_minus_one);
        assert!(failure.handle_writes.searched_handle.is_none());
        assert_eq!(
            failure.provider_error,
            DecodedCompiledLaneGeometryError::Search(CompiledLaneSpatialSearchFailure::Candidate(
                CompiledLaneCandidateFailure::CandidateOutOfBounds {
                    ordinal: 1,
                    requested_count: 2,
                    available: 1,
                }
            ))
        );

        let decode_leaf = [spatial_cell(1, 0)];
        let mut decode_provider = DecodedCompiledLaneGeometryProvider {
            hierarchy: DecodedCompiledLaneSpatialHierarchy {
                levels: [&decode_leaf, &[], &[], &[], &[], &[]],
                ..hierarchy
            },
            candidate_pool: &[i16::MIN],
            tables: DecodedCompiledLaneTables {
                records: &records,
                vertices: &vertices,
            },
        };
        let decode_failure = dispatch_compiled_lane(
            CompiledLaneDispatchInput {
                source_vector: [0x4000, 0, 0x4000],
                initial_handle: -1,
                fallback: 0,
            },
            &mut decode_provider,
        )
        .unwrap_err();
        assert_eq!(
            decode_failure.phase,
            CompiledLaneDispatchFailurePhase::Search
        );
        assert!(decode_failure.handle_writes.normalized_to_minus_one);
        assert!(decode_failure.handle_writes.searched_handle.is_none());
        assert_eq!(
            decode_failure.provider_error,
            DecodedCompiledLaneGeometryError::Search(CompiledLaneSpatialSearchFailure::Candidate(
                CompiledLaneCandidateFailure::Provider {
                    phase: CompiledLaneCandidateFailurePhase::Validate,
                    ordinal: 0,
                    handle: i16::MIN,
                    provider_error: CompiledLaneDecodeError::HandleOutOfBounds {
                        handle: i16::MIN,
                        available_records: records.len(),
                    },
                }
            ))
        );

        let miss_leaf = [spatial_cell(0, i32::MIN)];
        let mut miss_provider = DecodedCompiledLaneGeometryProvider {
            hierarchy: DecodedCompiledLaneSpatialHierarchy {
                origin_xz: [0, 0],
                width: 1,
                height: 1,
                levels: [&miss_leaf, &[], &[], &[], &[], &[]],
            },
            candidate_pool: &[],
            tables: DecodedCompiledLaneTables {
                records: &records,
                vertices: &vertices,
            },
        };
        assert_eq!(
            dispatch_compiled_lane(
                CompiledLaneDispatchInput {
                    source_vector: [-1, 0, 0],
                    initial_handle: 0,
                    fallback: 1234,
                },
                &mut miss_provider,
            ),
            Ok(CompiledLaneDispatchCompletion {
                lane_word: 1234,
                handle_writes: CompiledLaneHandleWriteTrace {
                    normalized_to_minus_one: false,
                    searched_handle: None,
                    invalidated_after_search_miss: true,
                },
                type_word_write: None,
                output_vector_write: None,
            })
        );
    }

    #[test]
    fn decoded_geometry_provider_maps_direct_decode_and_build_overflow_phases() {
        let leaf = [spatial_cell(0, 0)];
        let hierarchy = DecodedCompiledLaneSpatialHierarchy {
            origin_xz: [0, 0],
            width: 1,
            height: 1,
            levels: [&leaf, &[], &[], &[], &[], &[]],
        };
        let empty_tables = DecodedCompiledLaneTables {
            records: &[],
            vertices: &[],
        };
        let mut empty_provider = DecodedCompiledLaneGeometryProvider {
            hierarchy,
            candidate_pool: &[],
            tables: empty_tables,
        };
        assert_eq!(
            dispatch_compiled_lane(
                CompiledLaneDispatchInput {
                    source_vector: [0; 3],
                    initial_handle: 0,
                    fallback: 9,
                },
                &mut empty_provider,
            ),
            Err(CompiledLaneDispatchFailure {
                phase: CompiledLaneDispatchFailurePhase::Validate,
                handle_writes: CompiledLaneHandleWriteTrace::default(),
                provider_error: DecodedCompiledLaneGeometryError::Decode(
                    CompiledLaneDecodeError::HandleOutOfBounds {
                        handle: 0,
                        available_records: 0,
                    }
                ),
            })
        );

        let overflow_triangle = DecodedCompiledLaneTriangle {
            vertices: [
                [0, 0, i32::MAX - 255],
                [0x1_0000, 0, i32::MAX - 255],
                [0, 0x1_0000, i32::MIN],
            ],
            type_word: 0x55aa,
        };
        let overflow_source = [0, 0, -1];
        assert!(validate_decoded_compiled_lane_source(
            overflow_source,
            overflow_triangle.vertices
        ));
        let records = [DecodedCompiledLaneRecord {
            vertex_indices: [0, 1, 2],
            type_word: overflow_triangle.type_word,
        }];
        let mut overflow_provider = DecodedCompiledLaneGeometryProvider {
            hierarchy,
            candidate_pool: &[],
            tables: DecodedCompiledLaneTables {
                records: &records,
                vertices: &overflow_triangle.vertices,
            },
        };
        let expected = derive_decoded_compiled_lane_build(overflow_source, 123, overflow_triangle)
            .expect_err("fixture must reach the final signed divide overflow");
        assert_eq!(
            dispatch_compiled_lane(
                CompiledLaneDispatchInput {
                    source_vector: overflow_source,
                    initial_handle: 0,
                    fallback: 123,
                },
                &mut overflow_provider,
            ),
            Err(CompiledLaneDispatchFailure {
                phase: CompiledLaneDispatchFailurePhase::Build,
                handle_writes: CompiledLaneHandleWriteTrace::default(),
                provider_error: DecodedCompiledLaneGeometryError::BuildDivide(expected),
            })
        );
    }

    #[test]
    fn state_758_caller_selection_exhaustively_pins_gate_and_indices() {
        let table: Vec<i32> = (0..19).map(|index| index + 100).collect();
        assert_eq!(
            select_compiled_state_word_758_argument(false, 0xff, &[]),
            Ok(-1)
        );

        let mut reached = [false; 19];
        for selection_byte in u8::MIN..=u8::MAX {
            let row = usize::from((selection_byte >> 3) & 3);
            let column = usize::from(selection_byte >> 5);
            let gated = row >= 2 && column >= 4;
            let selected =
                select_compiled_state_word_758_argument(true, selection_byte, &table).unwrap();
            if gated {
                assert_eq!(selected, -1);
            } else {
                let index = row * 5 + column;
                reached[index] = true;
                assert_eq!(selected, table[index]);
            }
        }
        assert_eq!(
            reached
                .iter()
                .enumerate()
                .filter_map(|(index, reached)| reached.then_some(index))
                .collect::<Vec<_>>(),
            (0..=13).chain(15..=18).collect::<Vec<_>>()
        );
    }

    #[test]
    fn state_758_selection_checks_only_the_live_table_route() {
        assert_eq!(
            select_compiled_state_word_758_argument(true, 0x00, &[]),
            Err(CompiledStateWord758SelectionError::TableTruncated {
                index: 0,
                table_len: 0,
            })
        );
        assert_eq!(
            select_compiled_state_word_758_argument(true, 0x90, &[]),
            Ok(-1)
        );
        assert_eq!(
            select_compiled_state_word_758_argument(true, 0x78, &[0; 18]),
            Err(CompiledStateWord758SelectionError::TableTruncated {
                index: 18,
                table_len: 18,
            })
        );
        assert_eq!(
            select_compiled_state_word_758_argument(true, 0x78, &[0; 19]),
            Ok(0)
        );
        assert_eq!(COMPILED_STATE_WORD_758_OFFSET, 0x758);
        assert!(!COMPILED_STATE_WORD_OFFSETS.contains(&COMPILED_STATE_WORD_758_OFFSET));
        assert!(!COMPILED_COMMON_PREFIX_STATE_OFFSETS.contains(&COMPILED_STATE_WORD_758_OFFSET));
        assert!(!COMPILED_POST_HELPER_STATE_WORD_OFFSETS.contains(&COMPILED_STATE_WORD_758_OFFSET));
    }

    #[test]
    fn center_split_pins_all_low_three_bit_policies() {
        assert_eq!(compiled_center_split_q16(0), 0x4000);
        assert_eq!(compiled_center_split_q16(1), 0x8000);
        for mode in 2..=7 {
            assert_eq!(compiled_center_split_q16(mode), 0xc000);
        }
    }

    #[test]
    fn center_split_exhaustively_ignores_upper_option_bits() {
        for option_byte in u8::MIN..=u8::MAX {
            assert_eq!(
                compiled_center_split_q16(option_byte),
                compiled_center_split_q16(option_byte & 7)
            );
        }
    }

    #[test]
    fn config_option_resolver_rejects_the_signed_negative_domain_first() {
        for index in [-1, i8::MIN] {
            assert_eq!(
                resolve_compiled_config_options(&[], index),
                Err(CompiledConfigResolveError::NegativeIndex { index })
            );
        }
    }

    #[test]
    fn config_option_resolver_requires_only_the_highest_observed_read() {
        let required_len = COMPILED_CONFIG_CONTROL_RUN_OPTION_OFFSET + 1;
        for actual_len in [0, required_len - 1] {
            assert_eq!(
                resolve_compiled_config_options(&vec![0; actual_len], 0),
                Err(CompiledConfigResolveError::TruncatedConfig {
                    index: 0,
                    required_len,
                    actual_len,
                })
            );
        }

        assert_eq!(
            resolve_compiled_config_options(&vec![0; required_len], 0),
            Ok(CompiledConfigOptions {
                common_prefix_flags: 0,
                unindexed_flags_26: 0,
                state_word_758_fallback: 0,
                center_split: 0,
                control_run: 0,
                aux_control: AuxControlOption::Resolved(0),
            })
        );
        assert!(resolve_compiled_config_options(&vec![0; required_len + 7], 0).is_ok());
    }

    #[test]
    fn config_option_resolver_selects_one_record_and_preserves_raw_bytes() {
        let record_base = COMPILED_CONFIG_RECORD_STRIDE;
        let required_len = record_base + COMPILED_CONFIG_CONTROL_RUN_OPTION_OFFSET + 1;
        let mut config = vec![0; required_len];
        config[COMPILED_CONFIG_AUX_OPTION_OFFSET] = 0x11;
        config[COMPILED_CONFIG_CENTER_SPLIT_OPTION_OFFSET] = 0x22;
        config[COMPILED_CONFIG_CONTROL_RUN_OPTION_OFFSET] = 0x33;
        config[COMPILED_CONFIG_STATE_758_OPTION_OFFSET] = 0x44;
        config[record_base + COMPILED_CONFIG_STATE_758_OPTION_OFFSET] = 0x7b;
        config[record_base + COMPILED_CONFIG_AUX_OPTION_OFFSET] = 0xe5;
        config[record_base + COMPILED_CONFIG_CENTER_SPLIT_OPTION_OFFSET] = 0xfa;
        config[record_base + COMPILED_CONFIG_CONTROL_RUN_OPTION_OFFSET] = 0xf9;
        let original = config.clone();

        assert_eq!(
            resolve_compiled_config_options(&config, 1),
            Ok(CompiledConfigOptions {
                common_prefix_flags: 0,
                unindexed_flags_26: 0,
                state_word_758_fallback: 0x7b,
                center_split: 0xfa,
                control_run: 0xf9,
                aux_control: AuxControlOption::Resolved(0xe5),
            })
        );
        assert_eq!(config, original);

        assert_eq!(
            resolve_compiled_config_options(&config[..required_len - 1], 1),
            Err(CompiledConfigResolveError::TruncatedConfig {
                index: 1,
                required_len,
                actual_len: required_len - 1,
            })
        );
    }

    #[test]
    fn config_option_resolver_pins_the_constructed_slot_boundary() {
        for index in [COMPILED_CONFIG_SLOT_COUNT, i8::MAX] {
            assert_eq!(
                resolve_compiled_config_options(&[], index),
                Err(CompiledConfigResolveError::IndexOutOfRange {
                    index,
                    slot_count: COMPILED_CONFIG_SLOT_COUNT,
                })
            );
        }

        let index = COMPILED_CONFIG_SLOT_COUNT - 1;
        let record_base = index as usize * COMPILED_CONFIG_RECORD_STRIDE;
        let required_len = record_base + COMPILED_CONFIG_CONTROL_RUN_OPTION_OFFSET + 1;
        assert_eq!(
            resolve_compiled_config_options(&vec![0; required_len - 1], index),
            Err(CompiledConfigResolveError::TruncatedConfig {
                index,
                required_len,
                actual_len: required_len - 1,
            })
        );

        let mut config = vec![0; required_len];
        config[record_base + COMPILED_CONFIG_AUX_OPTION_OFFSET] = 0xa3;
        config[record_base + COMPILED_CONFIG_CENTER_SPLIT_OPTION_OFFSET] = 0xa5;
        config[record_base + COMPILED_CONFIG_CONTROL_RUN_OPTION_OFFSET] = 0xa9;
        assert_eq!(
            resolve_compiled_config_options(&config, index),
            Ok(CompiledConfigOptions {
                common_prefix_flags: 0,
                unindexed_flags_26: 0,
                state_word_758_fallback: 0,
                center_split: 0xa5,
                control_run: 0xa9,
                aux_control: AuxControlOption::Resolved(0xa3),
            })
        );
    }

    #[test]
    fn config_option_resolver_uses_only_the_unindexed_force_default_flag() {
        let record_base = COMPILED_CONFIG_RECORD_STRIDE;
        let required_len = record_base + COMPILED_CONFIG_CONTROL_RUN_OPTION_OFFSET + 1;
        let mut config = vec![0; required_len];
        config[record_base + COMPILED_CONFIG_AUX_OPTION_OFFSET] = 0xa7;
        config[record_base + COMPILED_CONFIG_AUX_FORCE_FLAG_OFFSET] =
            COMPILED_CONFIG_AUX_FORCE_DEFAULT_MASK;

        assert_eq!(
            resolve_compiled_config_options(&config, 1)
                .unwrap()
                .aux_control,
            AuxControlOption::Resolved(0xa7)
        );

        config[COMPILED_CONFIG_AUX_FORCE_FLAG_OFFSET] = 0xfd;
        let options = resolve_compiled_config_options(&config, 1).unwrap();
        assert_eq!(options.aux_control, AuxControlOption::Resolved(0xa7));
        assert_eq!(options.unindexed_flags_26, 0xfd);

        config[COMPILED_CONFIG_AUX_FORCE_FLAG_OFFSET] = COMPILED_CONFIG_AUX_FORCE_DEFAULT_MASK;
        let options = resolve_compiled_config_options(&config, 1).unwrap();
        assert_eq!(options.aux_control, AuxControlOption::ForcedDefault);
        assert_eq!(
            options.unindexed_flags_26,
            COMPILED_CONFIG_AUX_FORCE_DEFAULT_MASK
        );
    }

    #[test]
    fn config_option_resolver_preserves_unindexed_common_prefix_flags() {
        let mut config = compiled_stage_config(1, 0xa6, 1, 2, 3, false);
        config[COMPILED_CONFIG_COMMON_FLAGS_OFFSET + COMPILED_CONFIG_RECORD_STRIDE] = 0x19;

        assert_eq!(
            resolve_compiled_config_options(&config, 1)
                .unwrap()
                .common_prefix_flags,
            0xa6
        );
    }

    #[test]
    fn recovered_stage_bundle_routes_selected_case_options_and_tail_in_order() {
        let cases = synthetic_cases();
        let direct_cases = CompiledDirectCaseSet { cases };
        let constants = aux_constants();
        let aux_constants = AuxControlConstantSet { constants };
        let config = compiled_stage_config(1, 0, 1, 1, 0, false);
        let prior = synthetic_case(-7);
        let effects = compose_recovered_compiled_stage_effects(
            &direct_cases,
            &aux_constants,
            RecoveredCompiledStageInputs {
                prior_case_words: prior,
                selector: 3,
                config: &config,
                state_slot: 1,
                global_word: 0x1_0000,
                state_758_context_nonzero: false,
                state_758_table: &[],
            },
        )
        .unwrap();

        let selected = CompiledCaseOutcome {
            words: cases[3],
            selected_case: Some(3),
        };
        let transformed = transform_compiled_control_family(cases[3].control_family(), 0);
        let prefix = derive_compiled_common_prefix(cases[3], 0).unwrap();
        assert_eq!(effects.direct_case_stores, selected);
        assert_eq!(effects.common_prefix, prefix);
        assert_eq!(
            effects.post_helper,
            derive_compiled_post_helper_effects(
                prefix.control_word_148,
                cases[3].control_words[3],
            )
            .unwrap()
        );
        assert_ne!(
            effects.direct_case_stores.words.control_family(),
            transformed
        );
        assert_eq!(effects.center_split_q16, 0x8000);
        assert_eq!(
            effects.suffix_defaults,
            derive_compiled_suffix_defaults(
                [
                    cases[3].state_words[0],
                    cases[3].state_words[1],
                    cases[3].state_words[2],
                ],
                0,
            )
        );
        assert_eq!(
            effects.state_reciprocal_trio,
            derive_compiled_state_reciprocal_trio([
                cases[3].state_words[0],
                cases[3].state_words[1],
                cases[3].state_words[2],
            ])
            .unwrap()
        );
        assert_eq!(
            effects.aux_control_patch,
            derive_aux_control_patch(
                AuxControlOption::Resolved(1),
                prefix.state_word_48,
                0x1_0000,
                &constants,
            )
            .unwrap()
        );
        assert_eq!(
            effects.control_families,
            derive_compiled_control_families(transformed).unwrap()
        );
    }

    #[test]
    fn gear_ratio_wires_control_families_to_final_ratio_and_aux() {
        let direct_cases = CompiledDirectCaseSet {
            cases: synthetic_cases(),
        };
        let aux_constants = AuxControlConstantSet {
            constants: aux_constants(),
        };
        let config = compiled_stage_config(1, 0, 1, 1, 0, false);
        let effects = compose_recovered_compiled_stage_effects(
            &direct_cases,
            &aux_constants,
            RecoveredCompiledStageInputs {
                prior_case_words: synthetic_case(-7),
                selector: 3,
                config: &config,
                state_slot: 1,
                global_word: 0x1_0000,
                state_758_context_nonzero: false,
                state_758_table: &[],
            },
        )
        .unwrap();

        let (ratio, aux) = crate::drivetrain::derive_gear_ratio_and_aux_from_families(
            &effects.control_families,
            crate::fixed_point::Fixed16::from_raw(0x8000),
            crate::fixed_point::Fixed16::from_raw(0x8000),
        );
        assert_eq!(
            ratio,
            crate::drivetrain::derive_gear_ratio(
                crate::fixed_point::Fixed16::from_raw(effects.control_families.raw_copy[0]),
                crate::fixed_point::Fixed16::from_raw(0x8000)
            )
        );
        assert_eq!(
            aux,
            crate::drivetrain::derive_gear_aux_value(
                crate::fixed_point::Fixed16::from_raw(effects.control_families.reciprocal_copy[0]),
                crate::fixed_point::Fixed16::from_raw(0x8000)
            )
        );
        // raw_copy and reciprocal_copy are copies of the direct and reciprocal families.
        assert_eq!(
            effects.control_families.raw_copy,
            effects.control_families.post_processed_raw
        );
        assert_eq!(
            effects.control_families.reciprocal_copy,
            effects.control_families.reciprocal
        );
    }

    #[test]
    fn composed_initializer_runs_from_a_reviewed_case_store() {
        use std::fs;
        use std::path::PathBuf;
        use std::sync::atomic::{AtomicU64, Ordering};

        use cmr_formats::pe_compiled_initializer::{
            OFFICIAL_EU_COMPILED_INITIALIZER_LAYOUT, encode_compiled_cases,
        };
        use sha2::{Digest, Sha256};

        static NEXT: AtomicU64 = AtomicU64::new(0);
        struct Temp(PathBuf);
        impl Temp {
            fn new() -> Self {
                let path = std::env::temp_dir().join(format!(
                    "cmr-initializer-store-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                fs::create_dir(&path).unwrap();
                Self(path)
            }
        }
        impl Drop for Temp {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        fn sha256(bytes: &[u8]) -> String {
            let digest = Sha256::digest(bytes);
            let mut encoded = String::with_capacity(digest.len() * 2);
            for byte in digest {
                encoded.push_str(&format!("{byte:02x}"));
            }
            encoded
        }
        fn cases() -> [FormatCompiledCaseWords; COMPILED_CASE_COUNT] {
            let mut cases = std::array::from_fn(|selector| FormatCompiledCaseWords {
                state_words: std::array::from_fn(|index| selector as i32 * 100 + index as i32),
                control_words: std::array::from_fn(|index| -(selector as i32 * 100 + index as i32)),
            });
            cases[0].state_words[0] = i32::MIN;
            cases[13].control_words[22] = i32::MAX;
            cases
        }

        let temp = Temp::new();
        let root = temp.0.join("store");
        fs::create_dir(&root).unwrap();
        let executable = b"synthetic";
        let reviewed = ReviewedCompiledInitializerProfile::from_json(
            &serde_json::to_vec(&serde_json::json!({
                "schema_version": 1,
                "id": "synthetic-compiled-cases",
                "executable": {
                    "path": "Rally.exe",
                    "size": executable.len() as u64,
                    "sha256": sha256(executable),
                },
                "initializer": {
                    "image_base": OFFICIAL_EU_COMPILED_INITIALIZER_LAYOUT.image_base,
                    "zero_seed_va": OFFICIAL_EU_COMPILED_INITIALIZER_LAYOUT.zero_seed_va,
                    "esi_seed_va": OFFICIAL_EU_COMPILED_INITIALIZER_LAYOUT.esi_seed_va,
                    "edi_seed_va": OFFICIAL_EU_COMPILED_INITIALIZER_LAYOUT.edi_seed_va,
                    "seed_call_targets": OFFICIAL_EU_COMPILED_INITIALIZER_LAYOUT.seed_call_targets,
                    "dispatch_va": OFFICIAL_EU_COMPILED_INITIALIZER_LAYOUT.dispatch_va,
                    "jump_table_va": OFFICIAL_EU_COMPILED_INITIALIZER_LAYOUT.jump_table_va,
                    "first_case_va": OFFICIAL_EU_COMPILED_INITIALIZER_LAYOUT.first_case_va,
                    "common_join_va": OFFICIAL_EU_COMPILED_INITIALIZER_LAYOUT.common_join_va,
                    "state_pointer_va": OFFICIAL_EU_COMPILED_INITIALIZER_LAYOUT.state_pointer_va,
                    "control_pointer_va": OFFICIAL_EU_COMPILED_INITIALIZER_LAYOUT.control_pointer_va,
                    "selectors": 14,
                    "state_offsets": OFFICIAL_EU_COMPILED_INITIALIZER_LAYOUT.state_offsets,
                    "control_offsets": OFFICIAL_EU_COMPILED_INITIALIZER_LAYOUT.control_offsets,
                    "output_encoding": "cmr-ccase-v1",
                },
            }))
            .unwrap(),
        )
        .unwrap();
        let cases = cases();
        let data = encode_compiled_cases(&cases);
        fs::write(root.join(COMPILED_INITIALIZER_DATA_FILE), data).unwrap();
        let summary = serde_json::json!({
            "schema_version": 1,
            "profile": reviewed.profile().id.clone(),
            "profile_sha256": reviewed.sha256(),
            "source": {
                "path": "Rally.exe",
                "size": executable.len() as u64,
                "sha256": sha256(executable),
            },
            "output": {
                "path": "compiled-car-cases-v1.bin",
                "size": data.len() as u64,
                "sha256": sha256(&data),
            },
            "shape": {
                "format_version": 1,
                "selectors": 14,
                "state_words_per_selector": 8,
                "control_words_per_selector": 23,
                "encoding": "cmr-ccase-v1",
            },
        });
        fs::write(
            root.join(COMPILED_INITIALIZER_STATE_FILE),
            serde_json::to_vec(&summary).unwrap(),
        )
        .unwrap();

        let store = CompiledInitializerStore::open(&root, &reviewed).unwrap();
        let direct_cases = CompiledDirectCaseSet::from_store(&store).unwrap();
        let aux_constants = AuxControlConstantSet {
            constants: aux_constants(),
        };
        let config = compiled_stage_config(1, 0, 1, 1, 0, false);
        let input = full_initializer_input(&config);
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut providers = integrated_providers(&events, [IntegratedLaneMode::Built(0); 8]);
        let mut lane_config = |_lane| -> Result<u8, &'static str> { Ok(0) };
        let success = compose_compiled_initializer(
            &direct_cases,
            &aux_constants,
            input,
            integrated_execution_state(),
            &mut providers,
            &mut lane_config,
        )
        .unwrap();

        assert_eq!(
            success.early.direct_case_stores.words.state_words,
            store.case(3).unwrap().state_words
        );
        assert_eq!(
            success.early.direct_case_stores.words.control_words,
            store.case(3).unwrap().control_words
        );
        let (ratio, aux) = crate::drivetrain::derive_gear_ratio_and_aux_from_families(
            &success.early.control_families,
            crate::fixed_point::Fixed16::from_raw(0x8000),
            crate::fixed_point::Fixed16::from_raw(0x8000),
        );
        assert_eq!(
            ratio,
            crate::drivetrain::derive_gear_ratio(
                crate::fixed_point::Fixed16::from_raw(success.early.control_families.raw_copy[0]),
                crate::fixed_point::Fixed16::from_raw(0x8000)
            )
        );
        assert_eq!(
            aux,
            crate::drivetrain::derive_gear_aux_value(
                crate::fixed_point::Fixed16::from_raw(
                    success.early.control_families.reciprocal_copy[0]
                ),
                crate::fixed_point::Fixed16::from_raw(0x8000)
            )
        );
    }

    #[test]
    fn recovered_stage_bundle_pins_separate_destination_relationships() {
        let family_offsets = &COMPILED_CONTROL_WORD_OFFSETS
            [CONTROL_FAMILY_START..CONTROL_FAMILY_START + COMPILED_CONTROL_FAMILY_WORD_COUNT];
        assert_eq!(
            family_offsets,
            &[0x1d0, 0x1d4, 0x1d8, 0x1dc, 0x1e0, 0x1e4, 0x1e8, 0x1ec]
        );
        assert!(!COMPILED_CONTROL_WORD_OFFSETS.contains(&0x170));
        assert!(!AUX_CONTROL_WORD_OFFSETS.contains(&0x170));
        assert_eq!(COMPILED_COMMON_PREFIX_CONTROL_OFFSETS, [0x148, 0x15c]);
        assert_eq!(
            COMPILED_COMMON_PREFIX_STATE_OFFSETS,
            [
                0x020, 0x024, 0x028, 0x638, 0x63c, 0x640, 0x648, 0x644, 0x048, 0x04c
            ]
        );
        for offset in COMPILED_COMMON_PREFIX_CONTROL_OFFSETS {
            assert_ne!(offset, 0x170);
            assert!(!family_offsets.contains(&offset));
            assert!(!AUX_CONTROL_WORD_OFFSETS.contains(&offset));
        }
        for offset in COMPILED_POST_HELPER_CONTROL_WORD_OFFSETS {
            assert_ne!(offset, 0x170);
            assert!(!COMPILED_CONTROL_WORD_OFFSETS.contains(&offset));
            assert!(!AUX_CONTROL_WORD_OFFSETS.contains(&offset));
            assert!(!family_offsets.contains(&offset));
            assert!(!COMPILED_COMMON_PREFIX_CONTROL_OFFSETS.contains(&offset));
        }
        for offset in family_offsets {
            assert!(COMPILED_CONTROL_WORD_OFFSETS.contains(offset));
            assert!(!AUX_CONTROL_WORD_OFFSETS.contains(offset));
        }
    }

    #[test]
    fn recovered_stage_bundle_runs_common_effects_for_invalid_selectors() {
        let direct_cases = CompiledDirectCaseSet {
            cases: synthetic_cases(),
        };
        let constants = aux_constants();
        let aux_constants = AuxControlConstantSet { constants };
        let config = compiled_stage_config(0, COMPILED_COMMON_STATE_SCALE_MASK, 2, 0, 0, true);
        let prior = synthetic_case(40);
        let transformed = transform_compiled_control_family(prior.control_family(), 0);
        assert_ne!(transformed, prior.control_family());

        for selector in [14, u32::MAX] {
            let effects = compose_recovered_compiled_stage_effects(
                &direct_cases,
                &aux_constants,
                RecoveredCompiledStageInputs {
                    prior_case_words: prior,
                    selector,
                    config: &config,
                    state_slot: 0,
                    global_word: 0x8000,
                    state_758_context_nonzero: false,
                    state_758_table: &[],
                },
            )
            .unwrap();

            assert_eq!(
                effects.direct_case_stores,
                CompiledCaseOutcome {
                    words: prior,
                    selected_case: None,
                }
            );
            assert_eq!(effects.center_split_q16, 0x4000);
            assert_eq!(
                effects.common_prefix,
                derive_compiled_common_prefix(prior, COMPILED_COMMON_STATE_SCALE_MASK).unwrap()
            );
            assert_eq!(effects.control_families.post_processed_raw, transformed);
            assert_eq!(
                effects.post_helper,
                derive_compiled_post_helper_effects(
                    effects.common_prefix.control_word_148,
                    prior.control_words[3],
                )
                .unwrap()
            );
            assert_eq!(
                effects.aux_control_patch,
                derive_aux_control_patch(
                    AuxControlOption::ForcedDefault,
                    effects.common_prefix.state_word_48,
                    0x8000,
                    &constants,
                )
                .unwrap()
            );
            assert_eq!(
                effects.control_product_pair,
                derive_compiled_control_product_pair(
                    prior.control_words[3],
                    effects.control_families.reciprocal[6],
                )
                .unwrap()
            );
            assert_eq!(
                effects.suffix_defaults,
                derive_compiled_suffix_defaults(
                    [
                        effects.common_prefix.state_words.unwrap()[0],
                        effects.common_prefix.state_words.unwrap()[1],
                        effects.common_prefix.state_words.unwrap()[2],
                    ],
                    COMPILED_COMMON_STATE_SCALE_MASK,
                )
            );
            assert_eq!(
                effects.control_clear_loop,
                derive_compiled_control_clear_loop()
            );
            assert_eq!(
                effects.half_sign_state,
                derive_compiled_half_sign_state([
                    effects.common_prefix.state_words.unwrap()[0],
                    effects.common_prefix.state_words.unwrap()[1],
                    effects.common_prefix.state_words.unwrap()[2],
                ])
            );
            assert_eq!(
                effects.mixed_reset,
                derive_compiled_mixed_reset_effects(
                    config[COMPILED_CONFIG_UNINDEXED_FLAGS_26_OFFSET]
                )
            );
        }
    }

    #[test]
    fn recovered_stage_bundle_feeds_scaled_prefix_words_to_post_helper() {
        let mut cases = synthetic_cases();
        cases[0].control_words[0] = 0x151e;
        cases[0].control_words[3] = 0x2_0000;
        cases[0].control_words[CONTROL_FAMILY_START + 6] = 0x2_0000;
        let direct_cases = CompiledDirectCaseSet { cases };
        let aux_constants = AuxControlConstantSet {
            constants: aux_constants(),
        };
        let config = compiled_stage_config(
            0,
            COMPILED_COMMON_CONTROL_SCALE_MASK | COMPILED_COMMON_STATE_SCALE_MASK,
            0,
            0,
            2,
            false,
        );

        let effects = compose_recovered_compiled_stage_effects(
            &direct_cases,
            &aux_constants,
            RecoveredCompiledStageInputs {
                prior_case_words: synthetic_case(-1),
                selector: 0,
                config: &config,
                state_slot: 0,
                global_word: 0,
                state_758_context_nonzero: false,
                state_758_table: &[],
            },
        )
        .unwrap();

        assert_eq!(effects.common_prefix.control_word_148, 0x2000);
        assert_eq!(effects.common_prefix.control_word_15c, Some(0x2_6666));
        assert_eq!(
            post_helper_control_value(effects.post_helper, 0x14c),
            0x2000
        );
        assert_eq!(
            effects.post_helper,
            derive_compiled_post_helper_effects(0x2000, 0x2_6666).unwrap()
        );
        assert_eq!(effects.control_families.reciprocal[6], 0x8000);
        assert_eq!(
            effects.control_product_pair,
            derive_compiled_control_product_pair(0x2_6666, 0x8000).unwrap()
        );
        assert_eq!(
            effects.suffix_defaults,
            derive_compiled_suffix_defaults(
                [
                    effects.common_prefix.state_words.unwrap()[0],
                    effects.common_prefix.state_words.unwrap()[1],
                    effects.common_prefix.state_words.unwrap()[2],
                ],
                COMPILED_COMMON_CONTROL_SCALE_MASK | COMPILED_COMMON_STATE_SCALE_MASK,
            )
        );
        let scaled_state_words = effects.common_prefix.state_words.unwrap();
        assert_eq!(
            effects.state_reciprocal_trio,
            derive_compiled_state_reciprocal_trio([
                scaled_state_words[0],
                scaled_state_words[1],
                scaled_state_words[2],
            ])
            .unwrap()
        );
        assert_eq!(
            effects.control_clear_loop,
            derive_compiled_control_clear_loop()
        );
        assert_eq!(
            effects.half_sign_state,
            derive_compiled_half_sign_state([
                scaled_state_words[0],
                scaled_state_words[1],
                scaled_state_words[2],
            ])
        );
        assert_eq!(
            effects.mixed_reset,
            derive_compiled_mixed_reset_effects(config[COMPILED_CONFIG_UNINDEXED_FLAGS_26_OFFSET])
        );
    }

    #[test]
    fn recovered_stage_bundle_reports_product_pair_after_family() {
        let mut cases = synthetic_cases();
        cases[0].state_words[0] = 1;
        cases[0].control_words[3] = 0x1_0000;
        cases[0].control_words[CONTROL_FAMILY_START + 6] = 0;
        let direct_cases = CompiledDirectCaseSet { cases };
        let aux_constants = AuxControlConstantSet {
            constants: aux_constants(),
        };
        let config = compiled_stage_config(0, 0, 0, 0, 2, false);

        assert_eq!(
            compose_recovered_compiled_stage_effects(
                &direct_cases,
                &aux_constants,
                RecoveredCompiledStageInputs {
                    prior_case_words: synthetic_case(-1),
                    selector: 0,
                    config: &config,
                    state_slot: 0,
                    global_word: 0,
                    state_758_context_nonzero: false,
                    state_758_table: &[],
                },
            ),
            Err(RecoveredCompiledStageError::ProductPair(
                CompiledControlProductReciprocalOverflow { divisor: -1 }
            ))
        );

        cases[0].control_words[CONTROL_FAMILY_START + 1] = 1;
        let family_fails_first = CompiledDirectCaseSet { cases };
        assert!(matches!(
            compose_recovered_compiled_stage_effects(
                &family_fails_first,
                &aux_constants,
                RecoveredCompiledStageInputs {
                    prior_case_words: synthetic_case(-1),
                    selector: 0,
                    config: &config,
                    state_slot: 0,
                    global_word: 0,
                    state_758_context_nonzero: false,
                    state_758_table: &[],
                },
            ),
            Err(RecoveredCompiledStageError::Family(_))
        ));
    }

    #[test]
    fn recovered_stage_bundle_reports_state_reciprocal_after_product_pair() {
        let mut cases = synthetic_cases();
        cases[0].state_words[0] = 1;
        let direct_cases = CompiledDirectCaseSet { cases };
        let aux_constants = AuxControlConstantSet {
            constants: aux_constants(),
        };
        let config = compiled_stage_config(0, 0, 0, 0, 2, false);

        assert_eq!(
            compose_recovered_compiled_stage_effects(
                &direct_cases,
                &aux_constants,
                RecoveredCompiledStageInputs {
                    prior_case_words: synthetic_case(-1),
                    selector: 0,
                    config: &config,
                    state_slot: 0,
                    global_word: 0,
                    state_758_context_nonzero: false,
                    state_758_table: &[],
                },
            ),
            Err(RecoveredCompiledStageError::StateReciprocalTrio(
                CompiledStateReciprocalTrioOverflow {
                    index: 0,
                    divisor: 1,
                    completed: [None; 3],
                }
            ))
        );
    }

    #[test]
    fn recovered_stage_bundle_routes_state_758_table_and_fallback_values() {
        let direct_cases = CompiledDirectCaseSet {
            cases: synthetic_cases(),
        };
        let aux_constants = AuxControlConstantSet {
            constants: aux_constants(),
        };
        let mut config = compiled_stage_config(1, 0, 0, 0, 2, false);
        config[COMPILED_CONFIG_RECORD_STRIDE + COMPILED_CONFIG_STATE_758_OPTION_OFFSET] = 0xfe;
        config.resize(
            COMPILED_CONFIG_STATE_758_SELECTION_OFFSET + COMPILED_CONFIG_RECORD_STRIDE + 1,
            0,
        );
        config[COMPILED_CONFIG_STATE_758_SELECTION_OFFSET] = 0;
        config[COMPILED_CONFIG_STATE_758_SELECTION_OFFSET + COMPILED_CONFIG_RECORD_STRIDE] = 0x90;
        let prior = synthetic_case(-3);

        for (selector, table_value, expected) in [(0, 77, 77), (u32::MAX, -1, 6)] {
            let table = [table_value];
            let effects = compose_recovered_compiled_stage_effects(
                &direct_cases,
                &aux_constants,
                RecoveredCompiledStageInputs {
                    prior_case_words: prior,
                    selector,
                    config: &config,
                    state_slot: 1,
                    global_word: 0,
                    state_758_context_nonzero: true,
                    state_758_table: &table,
                },
            )
            .unwrap();
            assert_eq!(effects.state_word_758.state_word_758, expected);
            let selected_words = if selector == 0 {
                direct_cases.cases()[0]
            } else {
                prior
            };
            assert_eq!(
                effects.suffix_defaults,
                derive_compiled_suffix_defaults(
                    [
                        selected_words.state_words[0],
                        selected_words.state_words[1],
                        selected_words.state_words[2],
                    ],
                    0,
                )
            );
            assert_eq!(
                effects.half_sign_state,
                derive_compiled_half_sign_state([
                    selected_words.state_words[0],
                    selected_words.state_words[1],
                    selected_words.state_words[2],
                ])
            );
        }
    }

    #[test]
    fn recovered_stage_bundle_reports_config_before_auxiliary_failure() {
        let mut cases = synthetic_cases();
        cases[0].control_words[22] = 1_i32.wrapping_sub(0x78_0000);
        let direct_cases = CompiledDirectCaseSet { cases };
        let mut constants = aux_constants();
        constants.policy_one.reciprocal_divisor = 1;
        let aux_constants = AuxControlConstantSet { constants };
        let prior = synthetic_case(9);

        for (config, state_slot, expected) in [
            (
                vec![],
                -1,
                CompiledConfigResolveError::NegativeIndex { index: -1 },
            ),
            (
                vec![],
                COMPILED_CONFIG_SLOT_COUNT,
                CompiledConfigResolveError::IndexOutOfRange {
                    index: COMPILED_CONFIG_SLOT_COUNT,
                    slot_count: COMPILED_CONFIG_SLOT_COUNT,
                },
            ),
            (
                vec![0; COMPILED_CONFIG_CONTROL_RUN_OPTION_OFFSET],
                0,
                CompiledConfigResolveError::TruncatedConfig {
                    index: 0,
                    required_len: COMPILED_CONFIG_CONTROL_RUN_OPTION_OFFSET + 1,
                    actual_len: COMPILED_CONFIG_CONTROL_RUN_OPTION_OFFSET,
                },
            ),
        ] {
            assert_eq!(
                compose_recovered_compiled_stage_effects(
                    &direct_cases,
                    &aux_constants,
                    RecoveredCompiledStageInputs {
                        prior_case_words: prior,
                        selector: 0,
                        config: &config,
                        state_slot,
                        global_word: 0,
                        state_758_context_nonzero: false,
                        state_758_table: &[],
                    },
                ),
                Err(RecoveredCompiledStageError::Config(expected))
            );
        }

        let config = compiled_stage_config(0, 0, 1, 0, 2, false);
        assert_eq!(
            compose_recovered_compiled_stage_effects(
                &direct_cases,
                &aux_constants,
                RecoveredCompiledStageInputs {
                    prior_case_words: prior,
                    selector: 1,
                    config: &config,
                    state_slot: 0,
                    global_word: 0,
                    state_758_context_nonzero: false,
                    state_758_table: &[],
                },
            ),
            Err(RecoveredCompiledStageError::Aux(
                AuxControlReciprocalOverflow {
                    input: AuxControlReciprocalInput::PolicyDivisor,
                    divisor: 1,
                }
            ))
        );
    }

    #[test]
    fn recovered_stage_bundle_reports_prefix_before_auxiliary_and_family_failure() {
        let mut cases = synthetic_cases();
        cases[0].control_words[22] = 1_i32.wrapping_sub(0x78_0000);
        cases[0].control_words[CONTROL_FAMILY_START + 1] = 1;
        let direct_cases = CompiledDirectCaseSet { cases };
        let mut constants = aux_constants();
        constants.policy_one.reciprocal_divisor = 1;
        let aux_constants = AuxControlConstantSet { constants };
        let config = compiled_stage_config(0, 0, 1, 0, 2, false);

        assert_eq!(
            compose_recovered_compiled_stage_effects(
                &direct_cases,
                &aux_constants,
                RecoveredCompiledStageInputs {
                    prior_case_words: synthetic_case(-1),
                    selector: 0,
                    config: &config,
                    state_slot: 0,
                    global_word: 0,
                    state_758_context_nonzero: false,
                    state_758_table: &[],
                },
            ),
            Err(RecoveredCompiledStageError::Prefix(
                CompiledCommonPrefixReciprocalOverflow { state_word_48: 1 }
            ))
        );
    }

    #[test]
    fn recovered_stage_bundle_orders_aux_post_helper_and_family_failures() {
        let mut cases = synthetic_cases();
        cases[0].control_words[3] = 1;
        cases[0].control_words[CONTROL_FAMILY_START + 1] = 1;
        let direct_cases = CompiledDirectCaseSet { cases };
        let config = compiled_stage_config(0, 0, 1, 0, 2, false);
        let input = RecoveredCompiledStageInputs {
            prior_case_words: synthetic_case(-1),
            selector: 0,
            config: &config,
            state_slot: 0,
            global_word: 0,
            state_758_context_nonzero: false,
            state_758_table: &[],
        };

        let mut failing_aux_constants = aux_constants();
        failing_aux_constants.policy_one.reciprocal_divisor = 1;
        let selection_input = RecoveredCompiledStageInputs {
            state_758_context_nonzero: true,
            ..input
        };
        assert_eq!(
            compose_recovered_compiled_stage_effects(
                &direct_cases,
                &AuxControlConstantSet {
                    constants: failing_aux_constants,
                },
                selection_input,
            ),
            Err(RecoveredCompiledStageError::Aux(
                AuxControlReciprocalOverflow {
                    input: AuxControlReciprocalInput::PolicyDivisor,
                    divisor: 1,
                }
            ))
        );

        assert_eq!(
            compose_recovered_compiled_stage_effects(
                &direct_cases,
                &AuxControlConstantSet {
                    constants: aux_constants(),
                },
                selection_input,
            ),
            Err(RecoveredCompiledStageError::State758Selection(
                CompiledStateWord758SelectionError::TruncatedSelectionByte {
                    required_len: COMPILED_CONFIG_STATE_758_SELECTION_OFFSET + 1,
                    actual_len: config.len(),
                }
            ))
        );

        assert_eq!(
            compose_recovered_compiled_stage_effects(
                &direct_cases,
                &AuxControlConstantSet {
                    constants: aux_constants(),
                },
                input,
            ),
            Err(RecoveredCompiledStageError::PostHelper(
                CompiledPostHelperReciprocalOverflow {
                    control_word_15c: 1,
                }
            ))
        );
    }

    #[test]
    fn recovered_stage_bundle_reports_auxiliary_before_family_overflow() {
        let mut cases = synthetic_cases();
        cases[0].control_words[CONTROL_FAMILY_START + 1] = 1;
        let direct_cases = CompiledDirectCaseSet { cases };
        let config = compiled_stage_config(0, 0, 1, 0, 2, false);
        let input = RecoveredCompiledStageInputs {
            prior_case_words: synthetic_case(-2),
            selector: 0,
            config: &config,
            state_slot: 0,
            global_word: 0,
            state_758_context_nonzero: false,
            state_758_table: &[],
        };

        let safe_aux = AuxControlConstantSet {
            constants: aux_constants(),
        };
        assert_eq!(
            compose_recovered_compiled_stage_effects(&direct_cases, &safe_aux, input),
            Err(RecoveredCompiledStageError::Family(
                CompiledReciprocalOverflow { slot: 1, raw: 1 }
            ))
        );

        let mut overflowing_constants = aux_constants();
        overflowing_constants.policy_one.reciprocal_divisor = 1;
        let overflowing_aux = AuxControlConstantSet {
            constants: overflowing_constants,
        };
        assert_eq!(
            compose_recovered_compiled_stage_effects(&direct_cases, &overflowing_aux, input),
            Err(RecoveredCompiledStageError::Aux(
                AuxControlReciprocalOverflow {
                    input: AuxControlReciprocalInput::PolicyDivisor,
                    divisor: 1,
                }
            ))
        );
    }

    #[test]
    fn aux_cross_layer_regression_pins_offset_and_codec_invariants() {
        assert_eq!(AUX_CONTROL_WORD_OFFSETS, FORMAT_AUX_WORD_OFFSETS);
        assert_eq!(
            AUX_CONTROL_COMMON_WORD_OFFSETS,
            FORMAT_AUX_COMMON_WORD_OFFSETS
        );
        assert_eq!(
            AUX_CONTROL_POLICY_WORD_OFFSETS,
            FORMAT_AUX_POLICY_WORD_OFFSETS
        );
        assert_eq!(FORMAT_AUX_FORMAT_VERSION, 1);
        assert_eq!(FORMAT_AUX_COMMON_WORD_COUNT, 9);
        assert_eq!(FORMAT_AUX_MULTIPLIER_COUNT, 2);
        assert_eq!(FORMAT_AUX_POLICY_COUNT, 3);
        assert_eq!(FORMAT_AUX_POLICY_WORD_COUNT, 7);
        assert_eq!(FORMAT_AUX_POLICY_DIVISOR_COUNT, 2);
        assert_eq!(FORMAT_AUX_TOTAL_WORD_COUNT, 38);
        assert_eq!(FORMAT_AUX_ENCODED_LEN, 184);
        assert_eq!(verify_aux_control_format_schema(), Ok(()));
    }

    #[test]
    fn verified_aux_store_becomes_owned_and_derives_each_policy_after_drop() {
        let format = format_aux_constants();
        let expected = core_aux_constants(&format);
        let (temp, owned) = {
            let (temp, store) = open_synthetic_aux_store(&format);
            let owned = AuxControlConstantSet::try_from(&store).unwrap();
            (temp, owned)
        };
        drop(temp);

        assert_eq!(owned.constants(), &expected);
        assert_eq!(owned.constants().common_words[0], i32::MIN);
        assert_eq!(owned.constants().common_words[1], i32::MAX);
        assert_eq!(owned.constants().common_words[2], 0);
        assert_eq!(owned.constants().common_words[3], -1);
        assert_eq!(
            owned.constants().policy_one.words[..4],
            [i32::MAX, i32::MIN, 0, -1]
        );

        for option in [
            AuxControlOption::Resolved(1),
            AuxControlOption::Resolved(2),
            AuxControlOption::Resolved(0),
            AuxControlOption::Resolved(7),
            AuxControlOption::ForcedDefault,
        ] {
            assert_eq!(
                owned.derive(option, -9, 0x1_0000),
                derive_aux_control_patch(option, -9, 0x1_0000, &expected)
            );
        }
    }

    #[test]
    fn verified_aux_store_preserves_typed_overflow_through_owned_adapter() {
        let mut format = format_aux_constants();
        format.policy_one.reciprocal_divisor = 1;
        let (temp, owned) = {
            let (temp, store) = open_synthetic_aux_store(&format);
            let owned = AuxControlConstantSet::from_store(&store).unwrap();
            (temp, owned)
        };
        drop(temp);

        assert_eq!(
            owned.derive(AuxControlOption::Resolved(1), 0, 0),
            Err(AuxControlReciprocalOverflow {
                input: AuxControlReciprocalInput::PolicyDivisor,
                divisor: 1,
            })
        );
    }

    #[test]
    fn auxiliary_patch_pins_policy_partition_masking_and_forced_default() {
        let constants = aux_constants();
        for option in [1_u8, 9, 0xf9] {
            let patch =
                derive_aux_control_patch(AuxControlOption::Resolved(option), 0, 0, &constants)
                    .unwrap();
            assert_eq!(aux_policy_words(patch), constants.policy_one.words);
        }
        for option in [2_u8, 10, 0xfa] {
            let patch =
                derive_aux_control_patch(AuxControlOption::Resolved(option), 0, 0, &constants)
                    .unwrap();
            assert_eq!(aux_policy_words(patch), constants.policy_two.words);
        }
        for option in [0_u8, 3, 4, 5, 6, 7, 8, 0xfb, 0xff] {
            let patch =
                derive_aux_control_patch(AuxControlOption::Resolved(option), 0, 0, &constants)
                    .unwrap();
            assert_eq!(aux_policy_words(patch), constants.default_policy.words);
        }

        let forced =
            derive_aux_control_patch(AuxControlOption::ForcedDefault, 0, 0, &constants).unwrap();
        assert_eq!(aux_policy_words(forced), constants.default_policy.words);
    }

    #[test]
    fn auxiliary_patch_maps_every_literal_and_derived_word_to_its_exact_offset() {
        let patch = derive_aux_control_patch(
            AuxControlOption::Resolved(1),
            80,
            0x1_0000,
            &aux_constants(),
        )
        .unwrap();
        assert_eq!(
            patch.words,
            [
                10, 11, 12, 13, 14, 15, 16, 0x1_0000, 101, 17, 0x2_0000, 0x1_0000, 0x1_0000, 102,
                103, 10, 0x1_0000, 104, 105, 106, 0x8000, 107,
            ]
        );
    }

    #[test]
    fn auxiliary_patch_schema_is_exact_unique_and_disjoint_from_direct_cases() {
        assert_eq!(
            AUX_CONTROL_WORD_OFFSETS,
            [
                0x0d8, 0x0dc, 0x0e0, 0x0e4, 0x0e8, 0x0ec, 0x0f0, 0x0f4, 0x0f8, 0x0fc, 0x100, 0x104,
                0x108, 0x10c, 0x110, 0x118, 0x11c, 0x120, 0x124, 0x128, 0x12c, 0x13c,
            ]
        );
        assert!(
            AUX_CONTROL_WORD_OFFSETS
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
        for omitted in [0x114, 0x130, 0x134, 0x138] {
            assert!(!AUX_CONTROL_WORD_OFFSETS.contains(&omitted));
        }
        for direct_offset in COMPILED_CONTROL_WORD_OFFSETS {
            assert!(!AUX_CONTROL_WORD_OFFSETS.contains(&direct_offset));
        }
    }

    #[test]
    fn auxiliary_patch_uses_signed_q16_bits_and_arithmetic_shift() {
        let mut constants = aux_constants();
        constants.q16_multipliers = [0x1_8000, -1];
        let positive =
            derive_aux_control_patch(AuxControlOption::Resolved(1), -9, 0x2_0000, &constants)
                .unwrap();
        assert_eq!(aux_value(positive, 0x0f4), 0x3_0000);
        assert_eq!(aux_value(positive, 0x100), -2);
        assert_eq!(aux_value(positive, 0x118), -2);

        let negative_odd =
            derive_aux_control_patch(AuxControlOption::Resolved(1), 0, 1, &constants).unwrap();
        assert_eq!(aux_value(negative_odd, 0x100), -1);

        constants.q16_multipliers[0] = i32::MAX;
        let low_32 =
            derive_aux_control_patch(AuxControlOption::Resolved(1), 0, i32::MIN, &constants)
                .unwrap();
        assert_eq!(aux_value(low_32, 0x0f4), 32_768);
    }

    #[test]
    fn auxiliary_patch_reciprocals_pin_zero_and_signed_truncation() {
        let mut constants = aux_constants();
        constants.common_words[8] = 0;
        constants.policy_one.reciprocal_divisor = 0x3_0000;
        constants.policy_one.scaled_reciprocal_divisor = -0x3_0000;
        let patch =
            derive_aux_control_patch(AuxControlOption::Resolved(1), 0, 0x1_0000, &constants)
                .unwrap();
        assert_eq!(aux_value(patch, 0x108), -1);
        assert_eq!(aux_value(patch, 0x11c), 0x5555);
        assert_eq!(aux_value(patch, 0x12c), -0x5555);
    }

    #[test]
    fn auxiliary_patch_reciprocal_overflow_is_typed_and_atomic() {
        let mut constants = aux_constants();
        for (input, divisor) in [
            (AuxControlReciprocalInput::CommonWord, 1),
            (AuxControlReciprocalInput::PolicyDivisor, 2),
            (AuxControlReciprocalInput::ScaledPolicyDivisor, -1),
        ] {
            constants.common_words[8] = 0x1_0000;
            constants.policy_one.reciprocal_divisor = 0x1_0000;
            constants.policy_one.scaled_reciprocal_divisor = 0x1_0000;
            match input {
                AuxControlReciprocalInput::CommonWord => constants.common_words[8] = divisor,
                AuxControlReciprocalInput::PolicyDivisor => {
                    constants.policy_one.reciprocal_divisor = divisor;
                }
                AuxControlReciprocalInput::ScaledPolicyDivisor => {
                    constants.policy_one.scaled_reciprocal_divisor = divisor;
                }
            }
            assert_eq!(
                derive_aux_control_patch(AuxControlOption::Resolved(1), 0, 0, &constants,),
                Err(AuxControlReciprocalOverflow { input, divisor })
            );
        }
    }

    #[test]
    fn control_run_pins_all_five_mode_policies() {
        let original = [0, 0x1_0000, 0x2_0000, 0x3_0000, 0x4_0000, 0x5_0000];
        assert_eq!(
            transform_compiled_control_run(original, 0),
            [-13_107, 58_982, 131_071, 203_160, 275_249, 347_338]
        );
        assert_eq!(
            transform_compiled_control_run(original, 1),
            [-6_553, 62_259, 131_071, 199_883, 268_695, 337_507]
        );
        assert_eq!(transform_compiled_control_run(original, 2), original);
        assert_eq!(
            transform_compiled_control_run(original, 3),
            [6_553, 68_813, 131_073, 193_333, 255_593, 317_853]
        );

        let default = [13_107, 72_090, 131_073, 190_056, 249_039, 308_022];
        for mode in 4..=7 {
            assert_eq!(transform_compiled_control_run(original, mode), default);
        }
    }

    #[test]
    fn control_run_masks_option_to_its_low_three_bits() {
        let original = [-70_001, -2, 0, 9, 80_000, i32::MAX];
        for mode in 0..=7 {
            assert_eq!(
                transform_compiled_control_run(original, mode),
                transform_compiled_control_run(original, mode | 0xf8)
            );
        }
    }

    #[test]
    fn control_run_mode_two_is_identity_at_signed_boundaries() {
        for original in [
            [0, 1, 2, 3, 4, 5],
            [i32::MIN, i32::MAX, 0, -1, 1, i32::MIN],
            [i32::MAX, i32::MIN, i32::MAX, i32::MIN, -1, 0],
        ] {
            assert_eq!(transform_compiled_control_run(original, 2), original);
        }
    }

    #[test]
    fn control_run_uses_original_deltas_and_arithmetic_right_shift() {
        assert_eq!(
            transform_compiled_control_run([100, 200, 400, 800, 1_600, 3_200], 0),
            [-13_007, -12_898, -12_679, -12_240, -11_361, -9_602]
        );

        // A signed Q16 product of -1 by 0x11999 shifts to -2, not the -1
        // produced by truncating signed division toward zero.
        assert_eq!(
            transform_compiled_control_run([1, 0, 0, 0, 0, 0], 0),
            [-13_106, -13_108, -13_108, -13_108, -13_108, -13_108]
        );
    }

    #[test]
    fn control_run_wraps_delta_shift_retention_bias_and_rebuild() {
        // 0 - MIN wraps during delta formation. Its scaled signed product
        // shifts below i32::MIN, so the cast retains the low 32 bits. The
        // first bias and the first cumulative rebuild both wrap as well.
        assert_eq!(
            transform_compiled_control_run([i32::MIN, 0, 0, 0, 0, 0], 0),
            [
                2_147_470_541,
                -214_741_811,
                -214_741_811,
                -214_741_811,
                -214_741_811,
                -214_741_811,
            ]
        );
    }

    #[test]
    fn eight_word_adapter_preserves_endpoints_and_only_transforms_middle_run() {
        let original = [
            i32::MIN,
            0,
            0x1_0000,
            0x2_0000,
            0x3_0000,
            0x4_0000,
            0x5_0000,
            i32::MAX,
        ];
        let transformed = transform_compiled_control_family(original, 0);
        assert_eq!(transformed[0], i32::MIN);
        assert_eq!(transformed[7], i32::MAX);
        assert_eq!(
            transformed[1..=6],
            [-13_107, 58_982, 131_071, 203_160, 275_249, 347_338]
        );
    }

    #[test]
    fn reciprocal_tail_pins_slot_zero_zero_divisors_and_exact_copies() {
        let post_processed_raw = [
            0x1234, 0, 0x2_0000, -0x2_0000, 0x8_000, -0x8_000, 0, 0x1_0000,
        ];
        let families = derive_compiled_control_families(post_processed_raw).unwrap();
        assert_eq!(
            families.reciprocal,
            [0, -1, 0x8000, -0x8000, 0x20000, -0x20000, -1, 0x10000]
        );
        assert_eq!(families.post_processed_raw, post_processed_raw);
        assert_eq!(families.raw_copy, post_processed_raw);
        assert_eq!(families.reciprocal_copy, families.reciprocal);
    }

    #[test]
    fn reciprocal_tail_truncates_signed_quotients_toward_zero() {
        let families = derive_compiled_control_families([
            0, 0x3_0000, -0x3_0000, 0x5_0000, -0x5_0000, 0x7_0000, -0x7_0000, 0x1_0000,
        ])
        .unwrap();
        assert_eq!(families.reciprocal[1], 0x5555);
        assert_eq!(families.reciprocal[2], -0x5555);
        assert_eq!(families.reciprocal[3], 0x3333);
        assert_eq!(families.reciprocal[4], -0x3333);
        assert_eq!(families.reciprocal[5], 0x2492);
        assert_eq!(families.reciprocal[6], -0x2492);

        let near_overflow = derive_compiled_control_families([
            0, 3, -3, 0x1_0000, 0x1_0000, 0x1_0000, 0x1_0000, 0x1_0000,
        ])
        .unwrap();
        assert_eq!(near_overflow.reciprocal[1], 1_431_655_765);
        assert_eq!(near_overflow.reciprocal[2], -1_431_655_765);
    }

    #[test]
    fn reciprocal_tail_accepts_signed_integer_extremes() {
        let post_processed_raw = [
            9,
            i32::MAX,
            i32::MIN,
            0x1_0000,
            -0x1_0000,
            0,
            0x2_0000,
            -0x2_0000,
        ];
        let families = derive_compiled_control_families(post_processed_raw).unwrap();
        assert_eq!(families.reciprocal[1], 2);
        assert_eq!(families.reciprocal[2], -2);
        assert_eq!(families.raw_copy, post_processed_raw);
    }

    #[test]
    fn reciprocal_overflow_is_typed_and_reports_first_slot() {
        for raw in [1, 2, -1] {
            let mut post_processed_raw = [0x1_0000; COMPILED_CONTROL_FAMILY_WORD_COUNT];
            post_processed_raw[3] = raw;
            assert_eq!(
                derive_compiled_control_families(post_processed_raw),
                Err(CompiledReciprocalOverflow { slot: 3, raw })
            );
        }

        let post_processed_raw = [0, 0x1_0000, 2, 1, 0x1_0000, 0x1_0000, 0x1_0000, 0x1_0000];
        assert_eq!(
            derive_compiled_control_families(post_processed_raw),
            Err(CompiledReciprocalOverflow { slot: 2, raw: 2 })
        );
    }

    #[test]
    fn slot_zero_is_never_reciprocated_even_when_it_would_overflow() {
        let post_processed_raw = [
            1, 0x1_0000, 0x1_0000, 0x1_0000, 0x1_0000, 0x1_0000, 0x1_0000, 0x1_0000,
        ];
        let families = derive_compiled_control_families(post_processed_raw).unwrap();
        assert_eq!(families.reciprocal[0], 0);
        assert_eq!(families.raw_copy[0], 1);
    }

    #[test]
    fn four_wheel_constants_follow_the_prefix_post_helper_and_suffix_stages() {
        assert_eq!(COMPILED_CONTROL_WORD_OFFSETS[2], 0x16c);
        let words = synthetic_case(3);
        for flags in [
            0,
            COMPILED_COMMON_CONTROL_SCALE_MASK | COMPILED_COMMON_STATE_SCALE_MASK,
        ] {
            let constants = derive_compiled_four_wheel_constants(words, flags).unwrap();
            let prefix = derive_compiled_common_prefix(words, flags).unwrap();
            assert_eq!(constants.state_word_48, prefix.state_word_48);
            assert_eq!(constants.state_word_4c, prefix.state_word_4c);
            assert_eq!(constants.control_word_148, prefix.control_word_148);
            assert_eq!(constants.control_word_16c, words.control_words[2]);
            // The drive split is the direct case word, never rescaled.
            assert_eq!(COMPILED_CONTROL_WORD_OFFSETS[1], 0x158);
            assert_eq!(constants.control_word_158, words.control_words[1]);
            assert_eq!(
                constants.control_word_15c,
                prefix.control_word_15c.unwrap_or(words.control_words[3])
            );
            assert_eq!(constants.gravity_c5fc, compiled_gravity_constant(flags));
            assert_eq!(constants.control_word_420, 1);
            assert_eq!(
                (
                    constants.control_word_150,
                    constants.control_word_174,
                    constants.control_word_17c,
                    constants.control_word_184,
                    constants.control_word_408,
                ),
                (0x0ccc, 0x0ccc, 0x0ccc, 0x8000, 0)
            );
            assert_eq!(COMPILED_POST_HELPER_CONTROL_U16_OFFSETS, [0x3e4]);
            assert_eq!(COMPILED_POST_HELPER_CONTROL_BYTE_OFFSETS, [0x440]);
            assert_eq!(
                (
                    constants.control_word_188,
                    constants.control_u16_3e4,
                    constants.control_byte_440,
                ),
                (0x3333, 0x02aa, 3)
            );
            let expected_inertia = if flags & COMPILED_COMMON_STATE_SCALE_MASK == 0 {
                [1, 2, 1]
            } else {
                [2, 4, 2]
            };
            assert_eq!(constants.inertia_50_54_58, expected_inertia);
        }
        // The control-scale flag doubles the adjusted `+0x148`.
        let plain = derive_compiled_four_wheel_constants(words, 0).unwrap();
        let scaled =
            derive_compiled_four_wheel_constants(words, COMPILED_COMMON_CONTROL_SCALE_MASK)
                .unwrap();
        assert_eq!(
            scaled.control_word_148,
            common_prefix_q16_mul(plain.control_word_148, 0x2_0000)
        );
    }

    #[test]
    fn four_wheel_constants_report_a_faulting_prefix_reciprocal() {
        let mut words = synthetic_case(1);
        words.control_words[22] = 1_i32.wrapping_sub(0x78_0000);
        assert_eq!(
            derive_compiled_four_wheel_constants(words, 0),
            Err(RecoveredCompiledStageError::Prefix(
                CompiledCommonPrefixReciprocalOverflow { state_word_48: 1 }
            ))
        );
    }

    #[test]
    fn gearbox_tables_take_case_words_post_helper_words_and_families() {
        let words = synthetic_case(4);
        let families = CompiledControlFamilies {
            post_processed_raw: std::array::from_fn(|index| 0x1_0000 * (8 - index as i32)),
            reciprocal: std::array::from_fn(|index| 0x100 + index as i32),
            raw_copy: [0; COMPILED_CONTROL_FAMILY_WORD_COUNT],
            reciprocal_copy: [0; COMPILED_CONTROL_FAMILY_WORD_COUNT],
        };
        let tables = derive_compiled_gearbox_tables(words, 0, &families).unwrap();
        let word = |offset: u16| case_control_word(&words, offset);
        assert_eq!(
            tables.upper_190,
            [
                0,
                word(0x194),
                word(0x198),
                word(0x19c),
                word(0x1a0),
                word(0x1a4),
                0x0fae1,
                0
            ]
        );
        assert_eq!(
            tables.lower_1b0,
            [
                0,
                0x8000,
                word(0x1b8),
                word(0x1bc),
                word(0x1c0),
                word(0x1c4),
                word(0x1c8),
                0
            ]
        );
        assert_eq!(tables.ratio_1d0, families.post_processed_raw);
        assert_eq!(tables.inverse_1f0, families.reciprocal);
        assert_eq!(tables.redline_15c, words.control_words[3]);
        let scaled =
            derive_compiled_gearbox_tables(words, COMPILED_COMMON_CONTROL_SCALE_MASK, &families)
                .unwrap();
        assert_eq!(
            scaled.redline_15c,
            common_prefix_q16_mul(words.control_words[3], 0x1_3333)
        );
    }

    #[test]
    fn gravity_constant_follows_config_bit_zero() {
        assert_eq!(compiled_gravity_constant(0), 0x9999);
        assert_eq!(compiled_gravity_constant(1), 0x1999);
        assert_eq!(compiled_gravity_constant(0xfe), 0x9999);
        assert_eq!(compiled_gravity_constant(0xff), 0x1999);
    }
}
