//! Q16.16 signed fixed-point and 16-bit BAM angles: the numeric representation
//! of the driving simulation.
//!
//! The recovered in-race update (`FUN_0043afd0` "Mechanics" and its callees)
//! stores world scalars as signed 32-bit **Q16.16** fixed-point: `0x10000` is
//! `1.0`, and the canonical multiply widens to 64-bit and shifts right 16
//! (`(longlong)a * (longlong)b >> 0x10`, keeping the low 32 bits). Floating
//! point appears only at input boundaries (`__ftol` converts analog input to
//! fixed-point); integration, trig, and stored state are all integer.
//!
//! Angles are 16-bit **BAM** (binary angular measure) into a 4096-entry trig
//! table (`DAT_0082d380`): `sin = table[angle & 0xfff]`,
//! `cos = table[(angle + 0x400) & 0xfff]`, so `0x1000` is a full turn and
//! `0x400` is 90 degrees.
//!
//! Observations: the Q16.16 multiply/unit, the `>>0x10` truncation on signed
//! ints, and the 4096-entry table indexing with the `+0x400` cosine offset.
//! Inferences/design: the table holds a standard sine scaled to Q16.16. The
//! source-owned reconstruction in [`crate::legacy_math`] reproduces the startup
//! values without embedding game bytes. See evidence
//! `rea.official-eu.sim-fixed-step`.

/// `1.0` in Q16.16.
pub const FIXED_ONE: i32 = 0x1_0000;

/// Fractional bits in the fixed-point representation.
pub const FIXED_SHIFT: u32 = 16;

/// A signed Q16.16 fixed-point scalar, as stored in the recovered world state.
///
/// Arithmetic uses 32-bit modular (wrapping) semantics to match the shipped
/// integer math; the multiply reproduces the recovered 64-bit-widen/`>>16`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Fixed16(i32);

impl Fixed16 {
    /// `0.0`.
    pub const ZERO: Fixed16 = Fixed16(0);
    /// `1.0` (`0x10000`).
    pub const ONE: Fixed16 = Fixed16(FIXED_ONE);

    /// Wrap a raw Q16.16 bit pattern.
    pub const fn from_raw(raw: i32) -> Self {
        Fixed16(raw)
    }

    /// The raw Q16.16 bit pattern.
    pub const fn raw(self) -> i32 {
        self.0
    }

    /// Build from a whole number (`n * 1.0`), wrapping like the shipped shift.
    pub const fn from_int(n: i32) -> Self {
        Fixed16(n.wrapping_shl(FIXED_SHIFT))
    }

    /// Truncate toward negative infinity to a whole number, reproducing the
    /// recovered arithmetic `value >> 0x10` on a signed int.
    pub const fn to_int_floor(self) -> i32 {
        self.0 >> FIXED_SHIFT
    }

    /// The recovered fixed-point multiply: widen to 64-bit, shift right 16, keep
    /// the low 32 bits.
    pub const fn mul(self, rhs: Fixed16) -> Fixed16 {
        Fixed16(((self.0 as i64 * rhs.0 as i64) >> FIXED_SHIFT) as i32)
    }

    /// Modular (wrapping) addition.
    pub const fn add(self, rhs: Fixed16) -> Fixed16 {
        Fixed16(self.0.wrapping_add(rhs.0))
    }

    /// Modular (wrapping) subtraction.
    pub const fn sub(self, rhs: Fixed16) -> Fixed16 {
        Fixed16(self.0.wrapping_sub(rhs.0))
    }

    /// Modular (wrapping) negation.
    pub const fn neg(self) -> Fixed16 {
        Fixed16(self.0.wrapping_neg())
    }

    /// Non-negative square root in Q16.16 (negative inputs yield `0`).
    ///
    /// This is the mathematical integer-square-root helper, distinct from the
    /// compatibility lookup behavior in [`crate::legacy_math::sqrt_q16`]:
    /// `sqrt(r / 2^16) * 2^16 == isqrt(r << 16)`.
    pub const fn sqrt(self) -> Fixed16 {
        if self.0 <= 0 {
            return Fixed16::ZERO;
        }
        Fixed16(((self.0 as u64) << FIXED_SHIFT).isqrt() as i32)
    }
}

impl std::ops::Add for Fixed16 {
    type Output = Fixed16;
    fn add(self, rhs: Fixed16) -> Fixed16 {
        Fixed16::add(self, rhs)
    }
}

impl std::ops::Sub for Fixed16 {
    type Output = Fixed16;
    fn sub(self, rhs: Fixed16) -> Fixed16 {
        Fixed16::sub(self, rhs)
    }
}

impl std::ops::Mul for Fixed16 {
    type Output = Fixed16;
    fn mul(self, rhs: Fixed16) -> Fixed16 {
        Fixed16::mul(self, rhs)
    }
}

impl std::ops::Neg for Fixed16 {
    type Output = Fixed16;
    fn neg(self) -> Fixed16 {
        Fixed16::neg(self)
    }
}

/// Entries in the BAM trig table (a full turn).
pub const TRIG_TABLE_LEN: usize = 0x1000;

/// BAM value of a full turn (`0x1000` = 4096 steps).
pub const BAM_FULL_TURN: u16 = 0x1000;

/// BAM value of a quarter turn (90 degrees, the cosine phase offset `0x400`).
pub const BAM_QUARTER_TURN: u16 = 0x400;

/// Build the reconstructed Q16.16 sine table indexed by `angle & 0xfff`.
///
/// The indexing and startup values are recovered. The source-owned quarter-wave
/// reconstruction in [`crate::legacy_math::sin_q16`] supplies the exact
/// truncating values without copied game bytes.
/// Builds the 4096-entry Q16.16 sine table (`0x82d380`, used by `FUN_0044f870` and physics/camera math).
pub fn build_sine_table() -> [i32; TRIG_TABLE_LEN] {
    std::array::from_fn(|index| crate::legacy_math::sin_q16(index as u16).raw())
}

/// Sine of a BAM angle: `table[angle & 0xfff]`.
pub fn sin_bam(table: &[i32; TRIG_TABLE_LEN], angle: u16) -> Fixed16 {
    Fixed16(table[(angle & 0xfff) as usize])
}

/// Cosine of a BAM angle: `table[(angle + 0x400) & 0xfff]`.
pub fn cos_bam(table: &[i32; TRIG_TABLE_LEN], angle: u16) -> Fixed16 {
    sin_bam(table, angle.wrapping_add(BAM_QUARTER_TURN))
}

/// A 3-component vector of Q16.16 scalars, the shape of the recovered world
/// vectors (velocity, orientation axis, gravity, contact forces).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Vec3 {
    pub x: Fixed16,
    pub y: Fixed16,
    pub z: Fixed16,
}

impl Vec3 {
    /// The zero vector.
    pub const ZERO: Vec3 = Vec3 {
        x: Fixed16::ZERO,
        y: Fixed16::ZERO,
        z: Fixed16::ZERO,
    };

    /// Construct from components.
    pub const fn new(x: Fixed16, y: Fixed16, z: Fixed16) -> Self {
        Vec3 { x, y, z }
    }

    /// Component-wise addition.
    pub const fn add(self, rhs: Vec3) -> Vec3 {
        Vec3::new(self.x.add(rhs.x), self.y.add(rhs.y), self.z.add(rhs.z))
    }

    /// Component-wise subtraction.
    pub const fn sub(self, rhs: Vec3) -> Vec3 {
        Vec3::new(self.x.sub(rhs.x), self.y.sub(rhs.y), self.z.sub(rhs.z))
    }

    /// Scale every component by a scalar (the recovered per-axis `v_i * s >> 16`).
    pub const fn scale(self, s: Fixed16) -> Vec3 {
        Vec3::new(self.x.mul(s), self.y.mul(s), self.z.mul(s))
    }

    /// Dot product `x*x' + y*y' + z*z'`.
    pub const fn dot(self, rhs: Vec3) -> Fixed16 {
        self.x
            .mul(rhs.x)
            .add(self.y.mul(rhs.y))
            .add(self.z.mul(rhs.z))
    }

    /// Mathematical Euclidean length `sqrt(x^2 + y^2 + z^2)`. Compatibility
    /// paths that use the recovered lookup approximation call
    /// [`crate::legacy_math::sqrt_q16`] explicitly.
    pub const fn length(self) -> Fixed16 {
        self.dot(self).sqrt()
    }
}

impl std::ops::Add for Vec3 {
    type Output = Vec3;
    fn add(self, rhs: Vec3) -> Vec3 {
        Vec3::add(self, rhs)
    }
}

impl std::ops::Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, rhs: Vec3) -> Vec3 {
        Vec3::sub(self, rhs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_and_whole_number_conversions() {
        assert_eq!(Fixed16::ONE.raw(), 0x1_0000);
        assert_eq!(Fixed16::from_int(3).raw(), 0x3_0000);
        assert_eq!(Fixed16::from_int(1).to_int_floor(), 1);
        // 1.5 truncates toward -inf to 1; -0.5 to -1 (arithmetic shift).
        assert_eq!(Fixed16::from_raw(0x1_8000).to_int_floor(), 1);
        assert_eq!(Fixed16::from_raw(-0x8000).to_int_floor(), -1);
    }

    #[test]
    fn multiply_reproduces_the_recovered_widen_and_shift() {
        // 1 * 1 = 1.
        assert_eq!(Fixed16::ONE.mul(Fixed16::ONE), Fixed16::ONE);
        // 0.5 * 0.5 = 0.25.
        let half = Fixed16::from_raw(0x8000);
        assert_eq!(half.mul(half), Fixed16::from_raw(0x4000));
        // Recovered coefficient 0xa3d is ~0.039993, so * 100 is ~3.9993; the
        // floor truncation yields 3 (confirming the <<16/>>16 round-trip and
        // that the constant sits just under 0.04).
        let dt = Fixed16::from_raw(0xa3d);
        let hundred = Fixed16::from_int(100);
        assert_eq!(dt.mul(hundred).to_int_floor(), 3);
        // Negative and operator form.
        assert_eq!((-Fixed16::ONE) * half, -half);
    }

    #[test]
    fn add_and_sub_are_modular() {
        assert_eq!(
            Fixed16::from_int(2) + Fixed16::from_int(3),
            Fixed16::from_int(5)
        );
        assert_eq!(
            Fixed16::from_int(2) - Fixed16::from_int(3),
            Fixed16::from_int(-1)
        );
        // Wrapping at the 32-bit boundary (no panic).
        assert_eq!(
            Fixed16::from_raw(i32::MAX) + Fixed16::from_raw(1),
            Fixed16::from_raw(i32::MIN)
        );
    }

    #[test]
    fn vec3_arithmetic() {
        let a = Vec3::new(Fixed16::ONE, Fixed16::from_int(2), Fixed16::ZERO);
        let b = Vec3::new(Fixed16::from_int(3), Fixed16::ONE, Fixed16::ONE);
        assert_eq!(
            a.add(b),
            Vec3::new(Fixed16::from_int(4), Fixed16::from_int(3), Fixed16::ONE)
        );
        assert_eq!(
            a.sub(b),
            Vec3::new(Fixed16::from_int(-2), Fixed16::ONE, -Fixed16::ONE)
        );
        // Scale by 0.5.
        assert_eq!(
            a.scale(Fixed16::from_raw(0x8000)),
            Vec3::new(Fixed16::from_raw(0x8000), Fixed16::ONE, Fixed16::ZERO)
        );
        // dot = 1*3 + 2*1 + 0*1 = 5.
        assert_eq!(a.dot(b), Fixed16::from_int(5));
    }

    #[test]
    fn sqrt_and_length() {
        // sqrt of perfect squares is exact.
        assert_eq!(Fixed16::from_int(4).sqrt(), Fixed16::from_int(2));
        assert_eq!(Fixed16::from_int(100).sqrt(), Fixed16::from_int(10));
        // sqrt(0.25) = 0.5.
        assert_eq!(Fixed16::from_raw(0x4000).sqrt(), Fixed16::from_raw(0x8000));
        // Negatives clamp to zero (no panic).
        assert_eq!(Fixed16::from_int(-9).sqrt(), Fixed16::ZERO);
        // A 3-4-5 right triangle: length is exactly 5.
        let v = Vec3::new(Fixed16::from_int(3), Fixed16::from_int(4), Fixed16::ZERO);
        assert_eq!(v.length(), Fixed16::from_int(5));
    }

    #[test]
    fn bam_trig_table_hits_the_cardinal_angles() {
        let table = build_sine_table();
        // sin(0) = 0, sin(90) = 1, sin(180) = 0, sin(270) = -1.
        assert_eq!(sin_bam(&table, 0), Fixed16::ZERO);
        assert_eq!(sin_bam(&table, BAM_QUARTER_TURN), Fixed16::ONE);
        assert!(sin_bam(&table, 0x800).raw().abs() <= 1); // 180 deg ~ 0
        assert_eq!(sin_bam(&table, 0xc00), -Fixed16::ONE);
        // cos(0) = 1, cos(90) = 0.
        assert_eq!(cos_bam(&table, 0), Fixed16::ONE);
        assert!(cos_bam(&table, BAM_QUARTER_TURN).raw().abs() <= 1);
        // Index wraps modulo a full turn.
        assert_eq!(sin_bam(&table, BAM_FULL_TURN), sin_bam(&table, 0));
    }

    #[test]
    fn built_sine_table_matches_every_exact_legacy_lookup() {
        let table = build_sine_table();
        for (phase, value) in table.into_iter().enumerate() {
            assert_eq!(value, crate::legacy_math::sin_q16(phase as u16).raw());
        }
        assert_eq!(
            table[1], 100,
            "startup conversion truncates instead of rounding"
        );
    }
}
