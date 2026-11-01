//! C3D spherical environment-map (reflection) UV generation.
//!
//! Reconstructed from `FUN_0049f6b0`, the translucent-node vertex-prep routine
//! the scene-graph walker ([`crate::c3d_scene_graph`]) invokes for reflective
//! meshes. For such a node the shipped routine:
//!
//! 1. Inverts the camera-world matrix ([`invert_affine_q16`], from the view
//!    camera object at `DAT_00b643b4 + 0xd4`).
//! 2. Concatenates the node's world matrix (`+0xd4`) with that inverse on the
//!    CPU ([`matmul_q16`]) to get a node-local-to-view-space transform - the
//!    *reflection matrix*. This is the one place the engine concatenates
//!    matrices itself; the opaque render path lets Direct3D do it per vertex.
//! 3. Per vertex, normalizes the stored normal (via the shared reciprocal-
//!    magnitude LUT `DAT_0082b380`), rotates it into view space
//!    ([`transform_direction_q16`]), and writes a spherical environment-map UV
//!    into the vertex's `tu`/`tv` slots (`+0x10`/`+0x14`) as
//!    `uv = (n_view * 2^-16 + 1) * 0.5`.
//!
//! The reciprocal-magnitude LUT is runtime data and is not reproduced here:
//! [`spherical_env_uv`] and [`reflection_vertex_uv`] take an already-normalized
//! Q16.16 normal, consistent with the object-sort and terrain-cull recoveries
//! that likewise accept pre-scaled inputs rather than vendoring the table.

use crate::c3d_camera::{invert_affine_q16, matmul_q16, transform_direction_q16};

/// The Q16.16-to-`f32` scale `FUN_0049f6b0` applies to a view-space normal
/// component: the literal `1.5258789e-05` (`2^-16`) immediate in the shipped
/// routine, kept verbatim so the UV math matches bit for bit.
const Q16_TO_F32: f32 = 1.525_878_9e-5;

/// Builds the node-local-to-view-space reflection matrix: the node's world
/// matrix concatenated with the inverse of the camera-world matrix
/// (`FUN_0049f6b0`). Feed its rotation part to [`transform_direction_q16`] to
/// carry a node-local normal into view space for the spherical map.
pub fn reflection_uv_matrix(node_world_q16: &[i32; 16], camera_world_q16: &[i32; 16]) -> [i32; 16] {
    matmul_q16(node_world_q16, &invert_affine_q16(camera_world_q16))
}

/// Maps a view-space normal (Q16.16) to a spherical environment-map UV
/// (`FUN_0049f6b0`): `u = (n.x * 2^-16 + 1) * 0.5`, `v = (n.y * 2^-16 + 1) *
/// 0.5`. With a unit normal (components in `[-1, 1]`) this yields UV in
/// `[0, 1]`; the Z component is unused. The pair is written to the vertex
/// `tu`/`tv` slots (`+0x10`/`+0x14`).
pub fn spherical_env_uv(view_normal_q16: [i32; 3]) -> [f32; 2] {
    let axis = |q: i32| (q as f32 * Q16_TO_F32 + 1.0) * 0.5;
    [axis(view_normal_q16[0]), axis(view_normal_q16[1])]
}

/// Generates a vertex's reflection UV end to end: rotate the (already
/// normalized) node-local normal into view space with `reflection_matrix`, then
/// apply the spherical map. See the module note on the reciprocal-magnitude LUT.
pub fn reflection_vertex_uv(reflection_matrix: &[i32; 16], normal_q16: [i32; 3]) -> [f32; 2] {
    spherical_env_uv(transform_direction_q16(reflection_matrix, normal_q16))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::c3d_camera::MATRIX_IDENTITY_Q16;

    const ONE: i32 = 0x1_0000;

    fn approx(a: [f32; 2], b: [f32; 2]) {
        assert!((a[0] - b[0]).abs() < 1e-4, "u: {a:?} vs {b:?}");
        assert!((a[1] - b[1]).abs() < 1e-4, "v: {a:?} vs {b:?}");
    }

    #[test]
    fn spherical_map_centers_a_zero_normal_and_spans_the_unit_range() {
        // A normal with zero X/Y maps to the texture center regardless of Z.
        approx(spherical_env_uv([0, 0, ONE]), [0.5, 0.5]);
        // +/-1 along an axis reaches the UV extremes.
        approx(spherical_env_uv([ONE, 0, 0]), [1.0, 0.5]);
        approx(spherical_env_uv([-ONE, 0, 0]), [0.0, 0.5]);
        approx(spherical_env_uv([0, ONE, 0]), [0.5, 1.0]);
        approx(spherical_env_uv([0, -ONE, 0]), [0.5, 0.0]);
    }

    #[test]
    fn identity_camera_and_node_pass_the_normal_through_unchanged() {
        let m = reflection_uv_matrix(&MATRIX_IDENTITY_Q16, &MATRIX_IDENTITY_Q16);
        assert_eq!(m, MATRIX_IDENTITY_Q16);
        // The node-local normal reaches the spherical map untransformed.
        approx(reflection_vertex_uv(&m, [ONE, 0, 0]), [1.0, 0.5]);
    }

    #[test]
    fn camera_rotation_counter_rotates_the_reflected_normal() {
        // Camera rotated +90 deg about Z (and translated, which a normal ignores).
        let mut camera = MATRIX_IDENTITY_Q16;
        camera[0] = 0;
        camera[1] = ONE;
        camera[4] = -ONE;
        camera[5] = 0;
        camera[12] = 8 * ONE;
        camera[13] = -3 * ONE;

        let m = reflection_uv_matrix(&MATRIX_IDENTITY_Q16, &camera);
        // The view inverse rotates -90 about Z: a +X normal becomes -Y in view
        // space, so u stays centered and v drops to 0.
        approx(reflection_vertex_uv(&m, [ONE, 0, 0]), [0.5, 0.0]);
        // Camera translation must not shift the reflection (direction-only).
        let mut camera_no_t = camera;
        camera_no_t[12] = 0;
        camera_no_t[13] = 0;
        let m2 = reflection_uv_matrix(&MATRIX_IDENTITY_Q16, &camera_no_t);
        approx(
            reflection_vertex_uv(&m, [ONE, 0, 0]),
            reflection_vertex_uv(&m2, [ONE, 0, 0]),
        );
    }

    #[test]
    fn node_rotation_composes_before_the_view_inverse() {
        // Node rotated +90 about Z, identity camera: reflection matrix carries the
        // node-local +X normal to +Y in view space (u center, v max).
        let mut node = MATRIX_IDENTITY_Q16;
        node[0] = 0;
        node[1] = ONE;
        node[4] = -ONE;
        node[5] = 0;
        let m = reflection_uv_matrix(&node, &MATRIX_IDENTITY_Q16);
        approx(reflection_vertex_uv(&m, [ONE, 0, 0]), [0.5, 1.0]);
    }
}
