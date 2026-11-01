//! C3D camera, projection, viewport, clipping, and culling (item E05).
//!
//! # Scope
//!
//! The shipped game renders with Direct3D 5 Immediate Mode (see
//! [`crate::c3d_render`]). This module reconstructs the view/projection stage
//! that feeds the E06 submission: the perspective projection matrix builder
//! (`FUN_004a71b0`) and its parallel Q16.16 coefficient set, the D3DVIEWPORT2
//! clip volume (`FUN_004a6ed0`), the 2D top-down (XZ) frustum cull
//! (`FUN_004a1c00`), the cull-mode / winding rule (`FUN_004a2460`), the LOD
//! record selection, and the camera VIEW matrix. All values are the recovered
//! formulas/constants.
//!
//! The camera VIEW matrix is `inverse(camera-world)` ([`invert_affine_q16`],
//! `FUN_0048f460`) - a Q16.16 affine matrix inverse, not a look-at and not a
//! BAM-angle composition. The recovered routine is pure integer arithmetic (an
//! `i64` multiply reduced by `>> 16`, a `2^32 / det` integer reciprocal), so it
//! reproduces byte-exact with no driver floating-point dependency.
//!
//! Matrices are `[i32; 16]` column-major Q16.16: the 3x3 basis occupies columns
//! `0..=2`/`4..=6`/`8..=10`, the translation `12..=14`, and the homogeneous row
//! `3`/`7`/`11 = 0`, `15 = 1.0`. This matches the affine transform `FUN_0048fa60`
//! (`out[i] = m[12+i] + sum_j (m[4*j+i] * v[j])`).
//!
//! See evidence `rea.official-eu.c3d-camera`.

use crate::display::DisplayExtent;
use crate::root_menu_c3d_transform::{
    C3dMatrixWords, C3dViewportWords, RootMenuC3dViewportError, root_menu_c3d_viewport,
};

// --- Perspective projection (FUN_004a71b0) ------------------------------------

/// Baseline projection inputs `(w, h, z_near, z_far)` the D3D init sets
/// (`FUN_004a6950`): a fixed ~90 degree horizontal FOV and a 1.0..250.0 depth
/// range. Track load and the mirror/menu/sky callers vary these.
pub const PROJECTION_BASELINE: (f32, f32, f32, f32) = (2.0, 2.4, 1.0, 250.0);
/// The near plane, fixed at 1.0 at every call site.
pub const Z_NEAR: f32 = 1.0;
/// The default far plane (overridden per track and scaled by the view-distance
/// setting).
pub const Z_FAR_DEFAULT: f32 = 250.0;
/// Temporary far plane used while drawing the camera-centered horizon root
/// (`FUN_004a1780`, Q16.16 `0x61a8_0000`).
pub const HORIZON_Z_FAR: f32 = 25_000.0;

/// Per-detail-level far-plane multipliers (`FUN_00435d10`, cases 0..6).
pub const DETAIL_FAR_MULTIPLIERS: [f32; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

/// Default gameplay chase-camera FOV scalar (`cam+0x54` / `DAT_005507e8[car]`,
/// set to `0xa000` = `0.625` Q16.16 by `FUN_0041a3e0` at `0x41a5d1`).
pub const GAMEPLAY_FOV_DEFAULT_Q16: i32 = 0xa000;

/// Per-frame gameplay viewport projection `(w, h, z_near, z_far)` (`FUN_00435d10`
/// at `0x435d18..0x435d96`).
///
/// Whereas [`PROJECTION_BASELINE`] (`2.0, 2.4, 1.0, 250.0`) is the one-time
/// D3D device initialization before a race (`FUN_004a6950`), `FUN_00435d10`
/// recomputes the perspective width and height each gameplay frame from the
/// camera's FOV scalar (`fov_q16`, `0xa000` for chase cameras) and the
/// viewport aspect (`width / height`):
/// - `scale_q16 = fmul_q16(fov_q16, 0x275c2)` (`0x18999` = `1.5375` for `0xa000`)
/// - `aspect_q16 = ((width as f64 / height as f64) * 65536.0) as i32` (`0x15555` for `640x480`)
/// - `w_q16 = fmul_q16(0x123d7, fmul_q16(scale_q16, aspect_q16))` (`0x25643` = `2.3369598`)
/// - `h_q16 = fmul_q16(0x2bae1, scale_q16)` (`0x43285` = `4.197342`)
///
/// Combined with [`viewport`] (`clip_width = 2.0`, `clip_height = 2.0 * (width / height)`),
/// this gives a ~1:1 horizontal-to-vertical pixel aspect ratio (`373.9 px/unit` vs
/// `377.8 px/unit` at `640x480`) instead of squishing the scene vertically to `67.5%`.
#[must_use]
pub fn gameplay_projection(
    width: u16,
    height: u16,
    fov_q16: i32,
    z_near: f32,
    z_far: f32,
) -> (f32, f32, f32, f32) {
    if width == 0 || height == 0 {
        return (PROJECTION_BASELINE.0, PROJECTION_BASELINE.1, z_near, z_far);
    }
    let scale_q16 = fmul_q16(fov_q16, 0x2_75c2);
    let aspect_q16 = ((f64::from(width) / f64::from(height)) * 65_536.0) as i32;
    let w_q16 = fmul_q16(0x1_23d7, fmul_q16(scale_q16, aspect_q16));
    let h_q16 = fmul_q16(0x2_bae1, scale_q16);
    (
        w_q16 as f32 / 65_536.0,
        h_q16 as f32 / 65_536.0,
        z_near,
        z_far,
    )
}

/// Default gameplay projection `(w, h, z_near, z_far)` for a `width x height`
/// viewport (`FUN_00435d10` with [`GAMEPLAY_FOV_DEFAULT_Q16`], [`Z_NEAR`], and
/// [`Z_FAR_DEFAULT`]).
#[must_use]
pub fn gameplay_projection_default(width: u16, height: u16) -> (f32, f32, f32, f32) {
    gameplay_projection(
        width,
        height,
        GAMEPLAY_FOV_DEFAULT_Q16,
        Z_NEAR,
        Z_FAR_DEFAULT,
    )
}

/// Builds the left-handed D3D perspective matrix (`FUN_004a71b0`) as a row-major
/// `D3DMATRIX`: `m00 = w/2`, `m11 = h/2`, `m22 = zf/(zf-zn)`, `_34 = 1`,
/// `_43 = -zn*zf/(zf-zn)`, `_44 = 0`.
pub fn perspective_lh(w: f32, h: f32, z_near: f32, z_far: f32) -> [f32; 16] {
    let mut m = [0.0f32; 16];
    let range = z_far - z_near;
    m[0] = w * 0.5; // _11
    m[5] = h * 0.5; // _22
    m[10] = z_far / range; // _33
    m[11] = 1.0; // _34 (w = z)
    m[14] = -z_near * z_far / range; // _43
    m[15] = 0.0; // _44
    m
}

/// A mirror/reflection view flips the horizontal scale, realized by negating the
/// projection width (`FUN_00435d10`: `uVar3 = -uVar3`).
pub fn mirror_width(w: f32) -> f32 {
    -w
}

/// Truncates a float coefficient to Q16.16 the way the engine stores its
/// parallel fixed-point copy (`__ftol(coef * 65536)`, truncating toward zero).
pub fn coeff_to_q16(coef: f32) -> i32 {
    (coef * 65_536.0) as i32
}

/// The sparse Q16.16 projection coefficients the engine keeps alongside the
/// float matrix for its own fixed-point vertex/distance path: `[m00, m11, m22,
/// 1.0, m32]` (stored at `ctx+0x40c0/40d4/40e8/40ec/40f8`).
pub fn projection_coeffs_q16(w: f32, h: f32, z_near: f32, z_far: f32) -> [i32; 5] {
    let m = perspective_lh(w, h, z_near, z_far);
    [
        coeff_to_q16(m[0]),
        coeff_to_q16(m[5]),
        coeff_to_q16(m[10]),
        0x1_0000, // the constant 1.0 (w = z) coefficient
        coeff_to_q16(m[14]),
    ]
}

// --- Viewport (FUN_004a6ed0, D3DVIEWPORT2) ------------------------------------

/// The recovered D3DVIEWPORT2 fields: full back buffer at the origin, a clip
/// volume whose X span is fixed and whose Y span carries the `W/H` aspect, and
/// depth `0..1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub clip_x: f32,
    pub clip_width: f32,
    pub clip_y: f32,
    pub clip_height: f32,
    pub min_z: f32,
    pub max_z: f32,
}

/// Builds the viewport for a logical render size (e.g. 640x480). Aspect enters
/// only through the clip rectangle; the projection FOV is independent.
pub fn viewport(width: u32, height: u32) -> Viewport {
    let aspect = width as f32 / height as f32;
    Viewport {
        x: 0,
        y: 0,
        width,
        height,
        clip_x: -1.0,
        clip_width: 2.0,
        clip_y: aspect,
        clip_height: 2.0 * aspect,
        min_z: 0.0,
        max_z: 1.0,
    }
}

// --- 2D top-down frustum cull (FUN_004a1c00) ----------------------------------

/// Default near/far extents of the two XZ frustum triangles (`DAT_005022e8` /
/// `DAT_005022ec`); overridden per track.
pub const FRUSTUM_NEAR_EXTENT: f32 = 100.0;
pub const FRUSTUM_FAR_EXTENT: f32 = 500.0;
/// Distance below which a visible node goes to the full-detail deferred near
/// list (`DAT_00502774`), and that list's cap.
pub const NEAR_FAR_SPLIT: f32 = 7.0;
pub const NEAR_LIST_CAP: usize = 25;

/// Point-in-triangle test on the XZ (ground) plane (`FUN_004a1c00`): the point
/// is inside when it is on the non-negative side of all three edges (the
/// original pre-scales the deltas by `1/100` to fit 32-bit; the sign is
/// unchanged, so this computes the edge cross products directly in `i64`).
/// Vertices are `[x, z]`.
pub fn point_in_triangle_xz(point: [i64; 2], tri: [[i64; 2]; 3]) -> bool {
    (0..3).all(|edge| {
        let a = tri[edge];
        let b = tri[(edge + 1) % 3];
        let cross = (b[0] - a[0]) * (point[1] - a[1]) - (b[1] - a[1]) * (point[0] - a[0]);
        cross >= 0
    })
}

// --- Cull mode / winding (FUN_004a2460) ---------------------------------------

/// `D3DCULL_NONE` - two-sided (drawn for a record whose two-sided bit is set).
pub const CULL_NONE: u32 = 1;
/// `D3DCULL_CW`.
pub const CULL_CW: u32 = 2;
/// `D3DCULL_CCW` - the default (`ctx+0x70`), so front faces are wound clockwise.
pub const CULL_CCW: u32 = 3;
/// Bit in a mesh LOD record's byte `+0x20` marking the record two-sided.
pub const TWO_SIDED_FLAG: u8 = 0x01;
/// Bit in a mesh LOD record's byte `+0x20` selecting the cylindrical Y-billboard
/// drawer `FUN_004a18e0` (`FUN_004a07d0` at `0x4a0f6a`, `FUN_004a1ed0` at `0x4a1f0c`).
pub const Y_BILLBOARD_FLAG: u8 = 0x04;

/// The cull mode for a draw record: `CULL_NONE` when its two-sided bit is set,
/// otherwise the default back-face cull.
pub fn cull_mode(record_flag_20: u8) -> u32 {
    if record_flag_20 & TWO_SIDED_FLAG != 0 {
        CULL_NONE
    } else {
        CULL_CCW
    }
}

/// Builds the cylindrical Y-billboard WORLD matrix (`FUN_004a18e0` + `FUN_004a07d0`
/// `0x4a0816..0x4a094f` + `FUN_004a2b70` `0x4a3279..0x4a34d0`) for a node whose
/// active LOD draw group has [`Y_BILLBOARD_FLAG`] (`0x04`) set.
///
/// Trackside foliage and spectator quads (`flag20 & 0x04`) are authored in the
/// local YZ plane (`x == 0`, `z in [-w/2, +w/2]`, `y in [0, h]`). At draw time:
/// - `FUN_004a07d0` normalizes the camera's XZ forward direction `(cam[8], 0, cam[10])`
///   to unit `(fx, 0, fz)` and forms the perpendicular `perp = (fz, 0, -fx)`.
/// - `FUN_004a18e0` rotates each vertex's `z` into `(new_x, new_y, new_z) = (-z * fz, y, z * fx)`
///   and submits a diagonal WORLD matrix using the node's three basis-row Euclidean
///   lengths `(scale_x, scale_y, scale_z)` (`node+0x30/+0x34/+0x38`) plus the node's
///   world translation `node_world_q16[12..15]`.
///
/// Placing `[-fz * scale_x, 0, fx * scale_z, 0]` in row 2 (Z basis) and
/// `[0, scale_y, 0, 0]` in row 1 (Y basis) of the node's WORLD matrix maps any
/// YZ-plane vertex `(0, y, z, 1)` to identical world coordinates without mutating
/// the shared static Group-1 vertex buffer.
#[must_use]
pub fn y_billboard_world_q16(
    node_world_q16: &[i32; 16],
    camera_world_q16: &[i32; 16],
) -> [i32; 16] {
    let row_len_q16 = |x: i32, y: i32, z: i32| -> i32 {
        let len_sq =
            i64::from(fmul_q16(x, x)) + i64::from(fmul_q16(y, y)) + i64::from(fmul_q16(z, z));
        if len_sq <= 0 {
            0
        } else {
            ((len_sq as f64 / 65_536.0).sqrt() * 65_536.0) as i32
        }
    };
    let scale_x = row_len_q16(node_world_q16[0], node_world_q16[1], node_world_q16[2]);
    let scale_y = row_len_q16(node_world_q16[4], node_world_q16[5], node_world_q16[6]);
    let scale_z = row_len_q16(node_world_q16[8], node_world_q16[9], node_world_q16[10]);

    let cam_fx = camera_world_q16[8];
    let cam_fz = camera_world_q16[10];
    let cam_xz_len = row_len_q16(cam_fx, 0, cam_fz);
    let (fx_q16, fz_q16) = if cam_xz_len > 0 {
        (
            (((cam_fx as i64) << 16) / i64::from(cam_xz_len)) as i32,
            (((cam_fz as i64) << 16) / i64::from(cam_xz_len)) as i32,
        )
    } else {
        (0, 0x1_0000)
    };

    [
        fmul_q16(fx_q16, scale_x),
        0,
        fmul_q16(fz_q16, scale_z),
        0,
        0,
        scale_y,
        0,
        0,
        -fmul_q16(fz_q16, scale_x),
        0,
        fmul_q16(fx_q16, scale_z),
        0,
        node_world_q16[12],
        node_world_q16[13],
        node_world_q16[14],
        node_world_q16[15],
    ]
}

// --- LOD / detail selection ---------------------------------------------------

/// Stride of one mesh draw-group (LOD) record (base `mesh+0x20`).
pub const LOD_RECORD_STRIDE: usize = 0x18;
/// Record-relative offset of the D3D vertex type.
pub const LOD_VERTEX_TYPE_OFFSET: usize = 0x10;
/// Record-relative offset of the primitive-list head (`mesh+0x34` = base + 0x14).
pub const LOD_PRIMITIVE_HEAD_OFFSET: usize = 0x14;
/// Base offset of the LOD record array within a mesh.
pub const LOD_RECORD_BASE: usize = 0x20;
/// Global detail cap: a node with detail byte (`node+0x191`) above this is
/// skipped (`DAT_0050278c`).
pub const DETAIL_CAP: u8 = 3;
/// Billboard sentinel value: a mesh whose `+0x84/+0x88/+0x8c` all equal `-1.0`
/// Q16.16 draws with an identity WORLD transform instead of its node matrix.
pub const BILLBOARD_SENTINEL_Q16: i32 = -0x1_0000;

/// Mesh-relative byte offset of LOD record `lod`.
pub fn lod_record_offset(lod: usize) -> usize {
    LOD_RECORD_BASE + lod * LOD_RECORD_STRIDE
}

/// Whether a mesh's `+0x84/+0x88/+0x8c` triple marks it as a billboard (all
/// three equal to the `-1.0` sentinel).
pub fn is_billboard(xyz_q16: [i32; 3]) -> bool {
    xyz_q16 == [BILLBOARD_SENTINEL_Q16; 3]
}

/// Whether a node is within the global detail cap (drawn) or above it (skipped).
pub fn node_within_detail_cap(node_detail: u8) -> bool {
    node_detail <= DETAIL_CAP
}

// --- Camera VIEW matrix: Q16.16 affine inverse (FUN_0048f460) -----------------

/// A Q16.16 column-major identity matrix (`FUN_0048ed80`), returned for a
/// singular (zero-determinant) basis.
pub const MATRIX_IDENTITY_Q16: [i32; 16] = [
    0x1_0000, 0, 0, 0, //
    0, 0x1_0000, 0, 0, //
    0, 0, 0x1_0000, 0, //
    0, 0, 0, 0x1_0000,
];

/// Q16.16 fixed-point multiply: `(a * b) >> 16` with an `i64` intermediate and
/// truncation toward zero, matching the shipped `imul`/`shrd` sequence.
fn fmul_q16(a: i32, b: i32) -> i32 {
    ((a as i64 * b as i64) >> 16) as i32
}

/// Inverts a Q16.16 affine matrix (`FUN_0048f460`) to produce the camera VIEW
/// matrix from the camera-world matrix. The 3x3 basis is inverted via the
/// cofactor adjugate over the determinant (reciprocal `2^32 / det`), and the
/// translation becomes `inverse_basis * (-translation)`. A singular basis
/// (`det == 0`) yields [`MATRIX_IDENTITY_Q16`], exactly as the shipped routine.
pub fn invert_affine_q16(m: &[i32; 16]) -> [i32; 16] {
    // Cofactor row paired with basis column 0/1/2 for the determinant.
    let cof0 = fmul_q16(m[5], m[10]) - fmul_q16(m[9], m[6]);
    let cof1 = fmul_q16(m[9], m[2]) - fmul_q16(m[1], m[10]);
    let cof2 = fmul_q16(m[1], m[6]) - fmul_q16(m[5], m[2]);
    let det = fmul_q16(cof2, m[8]) + fmul_q16(cof1, m[4]) + fmul_q16(cof0, m[0]);
    if det == 0 {
        return MATRIX_IDENTITY_Q16;
    }
    let recip = (0x1_0000_0000_i64 / det as i64) as i32;

    let mut out = [0_i32; 16];
    // Inverse basis = adjugate (transposed cofactors) scaled by 1/det.
    out[0] = fmul_q16(recip, cof0);
    out[1] = fmul_q16(recip, cof1);
    out[2] = fmul_q16(recip, cof2);
    out[4] = fmul_q16(recip, fmul_q16(m[8], m[6]) - fmul_q16(m[4], m[10]));
    out[5] = fmul_q16(recip, fmul_q16(m[0], m[10]) - fmul_q16(m[8], m[2]));
    out[6] = fmul_q16(recip, fmul_q16(m[4], m[2]) - fmul_q16(m[0], m[6]));
    out[8] = fmul_q16(recip, fmul_q16(m[4], m[9]) - fmul_q16(m[8], m[5]));
    out[9] = fmul_q16(recip, fmul_q16(m[8], m[1]) - fmul_q16(m[0], m[9]));
    out[10] = fmul_q16(recip, fmul_q16(m[0], m[5]) - fmul_q16(m[4], m[1]));
    out[15] = 0x1_0000;

    // Inverse translation = inverse_basis applied to the negated translation.
    let neg = [-m[12], -m[13], -m[14]];
    out[12] = fmul_q16(neg[2], out[8]) + fmul_q16(neg[1], out[4]) + fmul_q16(neg[0], out[0]);
    out[13] = fmul_q16(neg[2], out[9]) + fmul_q16(neg[1], out[5]) + fmul_q16(neg[0], out[1]);
    out[14] = fmul_q16(neg[2], out[10]) + fmul_q16(neg[1], out[6]) + fmul_q16(neg[0], out[2]);
    out
}

/// Applies a Q16.16 column-major affine matrix to a point (`FUN_0048fa60`):
/// `out[i] = m[12+i] + sum_j (m[4*j+i] * v[j])`. The companion to
/// [`invert_affine_q16`], used to transform a point by the VIEW matrix.
pub fn apply_affine_q16(m: &[i32; 16], v: [i32; 3]) -> [i32; 3] {
    let mut out = [0_i32; 3];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot =
            m[12 + i] + fmul_q16(m[i], v[0]) + fmul_q16(m[4 + i], v[1]) + fmul_q16(m[8 + i], v[2]);
    }
    out
}

/// Concatenates two Q16.16 matrices (`FUN_0048ecb0`): `out[4*r + c] =
/// sum_k (A[4*r + k] * B[4*k + c]) >> 16`. With the row-vector convention of
/// [`apply_affine_q16`]/[`transform_direction_q16`] this composes left to right -
/// `transform(v, matmul(A, B))` applies `A` then `B`. Unlike the main render
/// path (where Direct3D concatenates WORLD * VIEW * PROJECTION per vertex), the
/// reflection-UV path performs this concatenation on the CPU.
pub fn matmul_q16(a: &[i32; 16], b: &[i32; 16]) -> [i32; 16] {
    let mut out = [0_i32; 16];
    for r in 0..4 {
        for c in 0..4 {
            let mut acc = 0_i32;
            for k in 0..4 {
                acc = acc.wrapping_add(fmul_q16(a[4 * r + k], b[4 * k + c]));
            }
            out[4 * r + c] = acc;
        }
    }
    out
}

/// Transforms a direction (a normal) by the 3x3 rotation part of a Q16.16
/// matrix (`FUN_0048dfe0`), ignoring translation: `out[c] = sum_r (m[4*r + c] *
/// v[r]) >> 16` for `c` in `0..3`. This is [`apply_affine_q16`] without the
/// translation term, used to carry a vertex normal into view space for the
/// reflection-UV generator.
pub fn transform_direction_q16(m: &[i32; 16], v: [i32; 3]) -> [i32; 3] {
    let mut out = [0_i32; 3];
    for (c, slot) in out.iter_mut().enumerate() {
        *slot = fmul_q16(m[c], v[0]) + fmul_q16(m[4 + c], v[1]) + fmul_q16(m[8 + c], v[2]);
    }
    out
}

// --- Transform-state submission (FUN_004a7d70, FUN_004a71b0) -------------------

/// `D3DTRANSFORMSTATE_WORLD` - the per-node model matrix (`SetTransform(1)`).
pub const TRANSFORMSTATE_WORLD: u32 = 1;
/// `D3DTRANSFORMSTATE_VIEW` - the camera VIEW matrix (`SetTransform(2)`).
pub const TRANSFORMSTATE_VIEW: u32 = 2;
/// `D3DTRANSFORMSTATE_PROJECTION` - the projection matrix (`SetTransform(3)`).
pub const TRANSFORMSTATE_PROJECTION: u32 = 3;

/// Converts a Q16.16 matrix to the float `D3DMATRIX` the engine submits
/// (`FUN_004a7d70`: each word times `1.5258789e-05`, i.e. `2^-16`). The VIEW
/// matrix is computed as [`invert_affine_q16`] of the camera-world matrix, then
/// converted and handed to `SetTransform(VIEW)`; D3D - not the CPU -
/// concatenates WORLD * VIEW * PROJECTION per vertex.
pub fn matrix_q16_to_f32(m: &[i32; 16]) -> [f32; 16] {
    let mut out = [0.0_f32; 16];
    for (slot, &word) in out.iter_mut().zip(m.iter()) {
        *slot = word as f32 / 65_536.0;
    }
    out
}

// --- Gameplay-scene matrix bridge ---------------------------------------------

/// Wraps a row-major `[f32; 16]` as the `C3dMatrixWords` the software transform
/// path consumes (it stores each slot as f32 bits and reads it back with
/// `f32::from_bits`). The row-vector convention matches [`apply_affine_q16`] and
/// [`perspective_lh`], so these matrices feed the per-vertex transform directly.
fn matrix_words_from_f32(m: [f32; 16]) -> C3dMatrixWords {
    C3dMatrixWords::from_words(m.map(f32::to_bits))
}

/// The gameplay-scene PROJECTION matrix as submission words (see
/// [`perspective_lh`]).
pub fn projection_matrix_words(w: f32, h: f32, z_near: f32, z_far: f32) -> C3dMatrixWords {
    matrix_words_from_f32(perspective_lh(w, h, z_near, z_far))
}

/// A node's WORLD matrix as submission words, from its Q16.16 world transform
/// (node `+0x114`).
pub fn world_matrix_words(world_q16: &[i32; 16]) -> C3dMatrixWords {
    matrix_words_from_f32(matrix_q16_to_f32(world_q16))
}

/// The camera VIEW matrix as submission words: the affine inverse of the
/// camera-world matrix ([`invert_affine_q16`]) converted to float. This is the
/// matrix `FUN_004a7d70` builds and submits with `SetTransform(VIEW)`.
pub fn view_matrix_words(camera_world_q16: &[i32; 16]) -> C3dMatrixWords {
    matrix_words_from_f32(matrix_q16_to_f32(&invert_affine_q16(camera_world_q16)))
}

/// The gameplay-scene D3DVIEWPORT2 as submission words. The clip volume is
/// scene-independent - it is the same full-back-buffer viewport the menu path
/// submits (`{clipX=-1, clipW=2, clipY=W/H, clipH=2*(W/H), minZ=0, maxZ=1}`,
/// matching [`viewport`]) - so this reuses the shared builder rather than
/// defining a second representation.
pub fn viewport_words(
    width: u16,
    height: u16,
) -> Result<C3dViewportWords, RootMenuC3dViewportError> {
    root_menu_c3d_viewport(DisplayExtent { width, height })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perspective_matches_the_recovered_baseline() {
        let (w, h, zn, zf) = PROJECTION_BASELINE;
        let m = perspective_lh(w, h, zn, zf);
        assert_eq!(m[0], 1.0); // w/2
        assert_eq!(m[5], 1.2); // h/2
        assert_eq!(m[11], 1.0); // _34
        assert_eq!(m[15], 0.0); // _44
        assert!((m[10] - 250.0 / 249.0).abs() < 1e-6); // zf/(zf-zn)
        assert!((m[14] - (-250.0 / 249.0)).abs() < 1e-6); // -zn*zf/(zf-zn)
    }

    #[test]
    fn gameplay_projection_matches_fun_00435d10_q16_formula_and_preserves_pixel_aspect() {
        let (w, h, zn, zf) = gameplay_projection_default(640, 480);
        // scale = (0xa000 * 0x275c2) >> 16 = 0x18999
        // aspect = int((640/480) * 65536) = 0x15555
        // w_q16 = (0x123d7 * ((0x18999 * 0x15555) >> 16)) >> 16 = 0x25643
        // h_q16 = (0x2bae1 * 0x18999) >> 16 = 0x43285
        assert_eq!(w, 0x2_5643 as f32 / 65_536.0);
        assert_eq!(h, 0x4_3285 as f32 / 65_536.0);
        assert_eq!(zn, Z_NEAR);
        assert_eq!(zf, Z_FAR_DEFAULT);

        // Combined with D3DVIEWPORT2 (clip_width = 2.0, clip_height = 2.0 * 640 / 480),
        // horizontal and vertical screen pixel scales per view unit at z = 1 are ~1:1.
        let vp = viewport(640, 480);
        let px_per_unit_x = (w * 0.5) * (vp.width as f32 / vp.clip_width);
        let px_per_unit_y = (h * 0.5) * (vp.height as f32 / vp.clip_height);
        let aspect_ratio = px_per_unit_y / px_per_unit_x;
        assert!(
            (aspect_ratio - 1.0104).abs() < 0.005,
            "unexpected pixel aspect ratio: {aspect_ratio}"
        );
    }

    #[test]
    fn projection_q16_coeffs_include_the_unit_w_term() {
        let (w, h, zn, zf) = PROJECTION_BASELINE;
        let c = projection_coeffs_q16(w, h, zn, zf);
        assert_eq!(c[0], 0x1_0000); // m00 = 1.0
        assert_eq!(c[1], coeff_to_q16(1.2)); // m11 = 1.2
        assert_eq!(c[3], 0x1_0000); // the constant 1.0 coefficient
        assert!(c[4] < 0); // m32 is negative
    }

    #[test]
    fn mirror_negates_width_only() {
        assert_eq!(mirror_width(2.0), -2.0);
        let m = perspective_lh(mirror_width(2.0), 2.4, 1.0, 250.0);
        assert_eq!(m[0], -1.0); // flipped X scale
        assert_eq!(m[5], 1.2); // Y unchanged
    }

    #[test]
    fn viewport_puts_aspect_in_the_clip_rectangle_only() {
        let v = viewport(640, 480);
        assert_eq!((v.x, v.y, v.width, v.height), (0, 0, 640, 480));
        assert_eq!(v.clip_x, -1.0);
        assert_eq!(v.clip_width, 2.0);
        assert_eq!(v.clip_y, 640.0 / 480.0);
        assert_eq!(v.clip_height, 2.0 * (640.0 / 480.0));
        assert_eq!((v.min_z, v.max_z), (0.0, 1.0));
    }

    #[test]
    fn point_in_triangle_xz_accepts_inside_rejects_outside() {
        // CCW triangle on the XZ plane.
        let tri = [[0, 0], [10, 0], [0, 10]];
        assert!(point_in_triangle_xz([1, 1], tri));
        assert!(point_in_triangle_xz([0, 0], tri)); // on a vertex (>= 0)
        assert!(!point_in_triangle_xz([9, 9], tri)); // outside the hypotenuse
        assert!(!point_in_triangle_xz([-1, 1], tri));
    }

    #[test]
    fn cull_mode_is_none_only_for_two_sided_records() {
        assert_eq!(cull_mode(0x00), CULL_CCW);
        assert_eq!(cull_mode(TWO_SIDED_FLAG), CULL_NONE);
        assert_eq!(cull_mode(0xfe), CULL_CCW); // the two-sided bit is clear
    }

    #[test]
    fn y_billboard_aligns_xz_basis_with_camera_forward_and_preserves_scale_and_translation() {
        let mut node = MATRIX_IDENTITY_Q16;
        node[0] = 2 * ONE; // X scale = 2
        node[5] = 3 * ONE; // Y scale = 3
        node[10] = 4 * ONE; // Z scale = 4
        node[12] = 10 * ONE;
        node[13] = 20 * ONE;
        node[14] = 30 * ONE;

        // Camera forward (+Z row = indices 8..11) pointing along +X (1, 0, 0).
        let mut cam = MATRIX_IDENTITY_Q16;
        cam[8] = ONE;
        cam[9] = 0;
        cam[10] = 0;

        let out = y_billboard_world_q16(&node, &cam);
        // fx = 1, fz = 0 => row0 = (2, 0, 0), row1 = (0, 3, 0), row2 = (0, 0, 4).
        assert_eq!([out[0], out[1], out[2]], [2 * ONE, 0, 0]);
        assert_eq!([out[4], out[5], out[6]], [0, 3 * ONE, 0]);
        assert_eq!([out[8], out[9], out[10]], [0, 0, 4 * ONE]);
        assert_eq!([out[12], out[13], out[14]], [10 * ONE, 20 * ONE, 30 * ONE]);

        // Purely vertical camera forward (cam_fx = 0, cam_fz = 0) falls back to (0, 1).
        cam[8] = 0;
        cam[9] = ONE;
        cam[10] = 0;
        let fallback = y_billboard_world_q16(&node, &cam);
        assert_eq!([fallback[0], fallback[2]], [0, 4 * ONE]);
        assert_eq!([fallback[8], fallback[10]], [-2 * ONE, 0]);
    }

    #[test]
    fn lod_offsets_and_billboard_sentinel() {
        assert_eq!(lod_record_offset(0), 0x20);
        assert_eq!(lod_record_offset(2), 0x20 + 2 * 0x18);
        // Primitive head sits at mesh+0x34 for LOD 0.
        assert_eq!(lod_record_offset(0) + LOD_PRIMITIVE_HEAD_OFFSET, 0x34);
        assert!(is_billboard([BILLBOARD_SENTINEL_Q16; 3]));
        assert!(!is_billboard([
            BILLBOARD_SENTINEL_Q16,
            0,
            BILLBOARD_SENTINEL_Q16
        ]));
        assert!(node_within_detail_cap(3));
        assert!(!node_within_detail_cap(4));
    }

    const ONE: i32 = 0x1_0000;

    #[test]
    fn inverse_of_identity_is_identity() {
        assert_eq!(invert_affine_q16(&MATRIX_IDENTITY_Q16), MATRIX_IDENTITY_Q16);
    }

    #[test]
    fn inverse_of_pure_translation_negates_it() {
        let mut m = MATRIX_IDENTITY_Q16;
        m[12] = 2 * ONE;
        m[13] = -3 * ONE;
        m[14] = 5 * ONE;
        let inv = invert_affine_q16(&m);
        // Basis stays identity; translation is negated.
        assert_eq!(&inv[0..12], &MATRIX_IDENTITY_Q16[0..12]);
        assert_eq!([inv[12], inv[13], inv[14]], [-2 * ONE, 3 * ONE, -5 * ONE]);
    }

    #[test]
    fn inverse_of_uniform_scale_is_the_reciprocal_scale() {
        let mut m = MATRIX_IDENTITY_Q16;
        m[0] = 2 * ONE;
        m[5] = 2 * ONE;
        m[10] = 2 * ONE;
        let inv = invert_affine_q16(&m);
        assert_eq!([inv[0], inv[5], inv[10]], [ONE / 2, ONE / 2, ONE / 2]);
    }

    #[test]
    fn singular_basis_falls_back_to_identity() {
        let mut m = MATRIX_IDENTITY_Q16;
        m[0] = 0; // zero the (0,0) basis entry -> det collapses to 0
        m[5] = 0;
        m[10] = 0;
        assert_eq!(invert_affine_q16(&m), MATRIX_IDENTITY_Q16);
    }

    #[test]
    fn inverse_undoes_a_rotation_and_translation_round_trip() {
        // +90 degrees about Z (column-major basis), translated by (2, 3, 0).
        let mut world = MATRIX_IDENTITY_Q16;
        world[0] = 0;
        world[1] = ONE; // col0 = (0, 1, 0)
        world[4] = -ONE;
        world[5] = 0; // col1 = (-1, 0, 0)
        world[12] = 2 * ONE;
        world[13] = 3 * ONE;

        let view = invert_affine_q16(&world);
        let point = [ONE, 0, 0];
        let transformed = apply_affine_q16(&world, point);
        // (1,0,0) rotated +90 about Z = (0,1,0), plus (2,3,0) = (2,4,0).
        assert_eq!(transformed, [2 * ONE, 4 * ONE, 0]);
        // The inverse VIEW returns it exactly (integer-exact for this matrix).
        assert_eq!(apply_affine_q16(&view, transformed), point);
    }

    /// The exact row-vector multiply the software raster applies
    /// (`multiply_row_vector`): `out[c] = sum_r v[r] * m[4*r + c]`.
    fn row_vector_mul(v: [f32; 4], words: C3dMatrixWords) -> [f32; 4] {
        let m = words.words().map(f32::from_bits);
        std::array::from_fn(|c| v[0] * m[c] + v[1] * m[4 + c] + v[2] * m[8 + c] + v[3] * m[12 + c])
    }

    #[test]
    fn projection_words_map_near_to_zero_and_far_to_one() {
        let (w, h, zn, zf) = PROJECTION_BASELINE;
        let proj = projection_matrix_words(w, h, zn, zf);
        // A point on the near plane maps to NDC z = 0, the far plane to z = 1.
        let near = row_vector_mul([0.0, 0.0, zn, 1.0], proj);
        let far = row_vector_mul([0.0, 0.0, zf, 1.0], proj);
        assert!((near[2] / near[3]).abs() < 1e-5); // zn -> 0
        assert!((far[2] / far[3] - 1.0).abs() < 1e-5); // zf -> 1
        // w' carries z (the LH perspective divide).
        assert!((far[3] - zf).abs() < 1e-3);
    }

    #[test]
    fn view_words_move_the_world_into_camera_space() {
        // Camera translated to (2, 3, 5), identity orientation.
        let mut camera_world = MATRIX_IDENTITY_Q16;
        camera_world[12] = 2 * ONE;
        camera_world[13] = 3 * ONE;
        camera_world[14] = 5 * ONE;
        let view = view_matrix_words(&camera_world);
        // A world point at the camera origin lands at the view-space origin.
        let at_camera = row_vector_mul([2.0, 3.0, 5.0, 1.0], view);
        assert!(at_camera[0].abs() < 1e-3);
        assert!(at_camera[1].abs() < 1e-3);
        assert!(at_camera[2].abs() < 1e-3);
        // A world point one unit ahead (+Z) stays one unit ahead in view space.
        let ahead = row_vector_mul([2.0, 3.0, 6.0, 1.0], view);
        assert!((ahead[2] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn viewport_words_match_the_e05_clip_volume() {
        let v = viewport(640, 480);
        let words = viewport_words(640, 480).unwrap().words();
        assert_eq!(words[0], 44); // dwSize
        assert_eq!((words[1], words[2]), (0, 0)); // origin
        assert_eq!((words[3], words[4]), (640, 480)); // width, height
        assert_eq!(f32::from_bits(words[5]), v.clip_x);
        assert_eq!(f32::from_bits(words[6]), v.clip_y);
        assert_eq!(f32::from_bits(words[7]), v.clip_width);
        assert_eq!(f32::from_bits(words[8]), v.clip_height);
        assert_eq!(f32::from_bits(words[9]), v.min_z);
        assert_eq!(f32::from_bits(words[10]), v.max_z);
        // A zero or oversized extent is rejected.
        assert!(viewport_words(0, 480).is_err());
        assert!(viewport_words(640, 0).is_err());
    }

    #[test]
    fn world_words_round_trip_the_q16_matrix() {
        let mut world = MATRIX_IDENTITY_Q16;
        world[12] = -4 * ONE;
        let words = world_matrix_words(&world);
        // Translation survives the Q16 -> f32 -> bits wrapping.
        let moved = row_vector_mul([1.0, 0.0, 0.0, 1.0], words);
        assert!((moved[0] - (1.0 - 4.0)).abs() < 1e-3);
    }

    #[test]
    fn view_matrix_converts_to_float_via_2_pow_neg16() {
        let mut m = MATRIX_IDENTITY_Q16;
        m[12] = 3 * ONE; // +3.0 translation
        m[1] = ONE / 2; // +0.5
        let f = matrix_q16_to_f32(&m);
        assert_eq!(f[0], 1.0);
        assert_eq!(f[1], 0.5);
        assert_eq!(f[12], 3.0);
        assert_eq!(f[15], 1.0);
        assert_eq!(f[3], 0.0);
        // Transform states match the D3DTRANSFORMSTATETYPE enum.
        assert_eq!(
            [
                TRANSFORMSTATE_WORLD,
                TRANSFORMSTATE_VIEW,
                TRANSFORMSTATE_PROJECTION
            ],
            [1, 2, 3]
        );
    }

    #[test]
    fn matmul_identity_is_the_other_operand() {
        let mut m = MATRIX_IDENTITY_Q16;
        m[0] = 2 * ONE;
        m[12] = 7 * ONE;
        assert_eq!(matmul_q16(&MATRIX_IDENTITY_Q16, &m), m);
        assert_eq!(matmul_q16(&m, &MATRIX_IDENTITY_Q16), m);
    }

    #[test]
    fn matmul_composes_left_to_right_like_sequential_transforms() {
        // A: +90 deg about Z, then translate (2, 3, 0). B: translate (0, 0, 5).
        let mut a = MATRIX_IDENTITY_Q16;
        a[0] = 0;
        a[1] = ONE;
        a[4] = -ONE;
        a[5] = 0;
        a[12] = 2 * ONE;
        a[13] = 3 * ONE;
        let mut b = MATRIX_IDENTITY_Q16;
        b[14] = 5 * ONE;

        let composed = matmul_q16(&a, &b);
        let v = [ONE, 0, 0];
        // transform(v, A*B) must equal applying A then B (translation included).
        let via_composed = apply_affine_q16(&composed, v);
        let via_steps = apply_affine_q16(&b, apply_affine_q16(&a, v));
        assert_eq!(via_composed, via_steps);
        // (1,0,0) -> rotate -> (0,1,0) -> +(2,3,0) -> (2,4,0) -> +(0,0,5).
        assert_eq!(via_composed, [2 * ONE, 4 * ONE, 5 * ONE]);
    }

    #[test]
    fn transform_direction_ignores_translation() {
        // +90 deg about Z with a large translation that a direction must not see.
        let mut m = MATRIX_IDENTITY_Q16;
        m[0] = 0;
        m[1] = ONE;
        m[4] = -ONE;
        m[5] = 0;
        m[12] = 100 * ONE;
        m[13] = -50 * ONE;
        m[14] = 9 * ONE;
        // (1,0,0) rotated +90 about Z = (0,1,0); no translation leaks in.
        assert_eq!(transform_direction_q16(&m, [ONE, 0, 0]), [0, ONE, 0]);
        // The same matrix with translation zeroed gives the identical result.
        let mut rot = m;
        rot[12] = 0;
        rot[13] = 0;
        rot[14] = 0;
        assert_eq!(
            transform_direction_q16(&m, [ONE, ONE, 0]),
            transform_direction_q16(&rot, [ONE, ONE, 0])
        );
    }
}
