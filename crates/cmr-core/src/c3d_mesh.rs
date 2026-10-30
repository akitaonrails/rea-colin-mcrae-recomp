//! C3D runtime mesh layout and the per-node draw-call expansion
//! (reconstruction item E06/E07).
//!
//! When the draw walker `FUN_004a28a0` reaches a drawable node (its render-pass
//! bit is set, its kind `node+0x180` is `0`, and its mesh pointer `node+0x0c` is
//! non-null), it submits the node's world matrix and then expands the node's
//! mesh into Direct3D `DrawIndexedPrimitive` calls:
//!
//! - The active level of detail is `mesh+0x80`. Each LOD has a fixed-stride
//!   (`0x18`) record pair: a **draw-group** header at `mesh+0x20 + lod*0x18`
//!   (its `+0x10` is the `DrawIndexedPrimitive` vertex type) and a **prim-list
//!   head** pointer at `mesh+0x34 + lod*0x18`.
//! - The prim list is singly linked through each record's `+0x18`. Each prim
//!   record carries the resource ordinal (`+0x00`, `-1` = untextured), the
//!   vertex count (`+0x08`) and buffer (`+0x0c`), and the index count (`+0x10`)
//!   and buffer (`+0x14`) - the five `DrawIndexedPrimitive` arguments after the
//!   primitive type.
//! - Each prim is drawn as `DrawIndexedPrimitive(D3DPT_TRIANGLELIST, vertex_type,
//!   vbuf, vcount, ibuf, icount, D3DDP_DONOTUPDATEEXTENTS)`, subject to the
//!   translucent per-prim gate [`crate::c3d_scene_graph::node_prims_draw`], and
//!   fog is baked per prim when the global fog flag is on and the vertex type is
//!   not `1`.
//!
//! This module recovers that layout and the parameter assembly as pure data; the
//! buffers themselves (`vertex_buffer`/`index_buffer`) are opaque runtime handles
//! produced by the batch builder (see [`crate::c3d_batch`]).
//!
//! # Runtime mesh-record assembly (the loader node loop)
//!
//! The draw-time layout above is populated when the C3D loader `FUN_004a2b70`
//! instantiates a mesh. For each on-disk group-1 mesh record it allocates a
//! `0xa4`-byte runtime record (`FUN_004ae1d0`) and fills:
//!
//! - `mesh+0x0c` the vertex count and `mesh+0x10` the triangle count (from the
//!   group-1 `+0x10`/`+0x12` fields). The triangle count is **doubled** for the
//!   two-sided menu mode (`3`) when the renderer reports no hardware two-sided
//!   handling (`renderer+0x28 == 0`) - see [`runtime_face_count`].
//! - `mesh+0x18` the runtime triangle array (stride `0x1c`), each entry built
//!   from one on-disk face: see [`RuntimeTriangle`].
//! - `mesh+0x00` the dequantized/staging vertex array (stride `0x20`), each
//!   component `raw * 2^-16`. For the stage/car modes (`5`/`6`/`7`/`8`) the
//!   first two LODs' vertex streams are also preserved at `mesh+0x04` (LOD 0)
//!   and `mesh+0x08` (LOD 1) - see [`preserves_lod_vertex_streams`] and
//!   [`lod_stream_backup_offset`].
//!
//! The per-LOD finalize `FUN_004b1e10` then writes the draw-group vertex type
//! (`1` vs `2`, see [`vertex_type_has_per_vertex_diffuse`]) and the prim list is
//! built by `FUN_004b2340`, whose group-by-material-run / vertex-dedup / compact
//! `u16` index topology is **exactly** [`crate::c3d_batch::plan_consecutive_batches`]
//! keyed on each face's material ordinal - that builder is reused here rather
//! than re-derived.
//!
//! Decisive constraint: `FUN_004b2340` allocates the vbuf (`prim+0x0c`), ibuf
//! (`prim+0x14`) and per-vertex normal array (`prim+0x04`, the on-disk
//! normals, stride `0xc`; only the damage chain reads it) from the
//! system-memory arena `FUN_004990d0`, not from a Direct3D
//! vertex/index buffer. C3D geometry is therefore drawn from system-memory
//! streams (user-pointer draw calls); the loader creates a Direct3D resource
//! only for textures (`FUN_004b01d0`). The compatibility interface models
//! geometry as plain slices, which is why the software path consumes
//! [`crate::c3d_batch`] vertices/indices directly.
//!
//! See evidence `rea.official-eu.c3d-rasterizer`, `rea.official-eu.c3d-mesh-instantiate`
//! and `impl.c3d-mesh-draw`.

use std::fmt;

use cmr_formats::c3d::{C3dGroup0Record, C3dGroup1Record};

use crate::c3d_batch::{C3dBatchPlanError, C3dConsecutiveBatch, plan_consecutive_batches};
use crate::c3d_render::{DP_DONOTUPDATEEXTENTS, PRIMTYPE_TRIANGLELIST, VERTEXTYPE_LVERTEX};

/// `mesh+0x80`: the active level-of-detail index selecting the draw-group pair.
pub const MESH_LOD_INDEX_OFFSET: usize = 0x80;
/// Base of the per-LOD draw-group header table (`mesh+0x20 + lod*stride`).
pub const MESH_DRAW_GROUP_BASE: usize = 0x20;
/// Base of the per-LOD prim-list-head table (`mesh+0x34 + lod*stride`).
pub const MESH_PRIM_HEAD_BASE: usize = 0x34;
/// Stride between successive LOD records in both tables.
pub const MESH_LOD_STRIDE: usize = 0x18;
/// Offset of the `DrawIndexedPrimitive` vertex type within a draw-group header.
pub const DRAW_GROUP_VERTEX_TYPE_OFFSET: usize = 0x10;

/// Prim record: resource ordinal (`-1` = untextured).
pub const PRIM_RESOURCE_ORDINAL_OFFSET: usize = 0x00;
/// Prim record: vertex count.
pub const PRIM_VERTEX_COUNT_OFFSET: usize = 0x08;
/// Prim record: vertex buffer handle.
pub const PRIM_VERTEX_BUFFER_OFFSET: usize = 0x0c;
/// Prim record: index count.
pub const PRIM_INDEX_COUNT_OFFSET: usize = 0x10;
/// Prim record: index buffer handle.
pub const PRIM_INDEX_BUFFER_OFFSET: usize = 0x14;
/// Prim record: next-prim link (singly linked list).
pub const PRIM_NEXT_OFFSET: usize = 0x18;

/// The sentinel resource ordinal marking an untextured prim (`prim+0x00 == -1`).
pub const PRIM_UNTEXTURED_ORDINAL: i32 = -1;

/// Byte offset of a LOD's draw-group header within the mesh.
pub fn draw_group_offset(lod: u8) -> usize {
    MESH_DRAW_GROUP_BASE + usize::from(lod) * MESH_LOD_STRIDE
}

/// Byte offset of a LOD's prim-list-head pointer within the mesh.
pub fn prim_head_offset(lod: u8) -> usize {
    MESH_PRIM_HEAD_BASE + usize::from(lod) * MESH_LOD_STRIDE
}

/// One prim record's draw fields (the five `DrawIndexedPrimitive` arguments after
/// the primitive type, plus the resource ordinal that selects the texture bind).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrimBatch {
    /// `prim+0x00`: resource ordinal, or [`PRIM_UNTEXTURED_ORDINAL`].
    pub resource_ordinal: i32,
    /// `prim+0x08`: vertex count.
    pub vertex_count: u32,
    /// `prim+0x0c`: opaque vertex-buffer handle.
    pub vertex_buffer: u32,
    /// `prim+0x10`: index count.
    pub index_count: u32,
    /// `prim+0x14`: opaque index-buffer handle.
    pub index_buffer: u32,
}

impl PrimBatch {
    /// Whether this prim is untextured (its ordinal is the `-1` sentinel).
    pub fn is_untextured(&self) -> bool {
        self.resource_ordinal == PRIM_UNTEXTURED_ORDINAL
    }
}

/// The resolved `DrawIndexedPrimitive` call for one prim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrawIndexedPrimitive {
    /// `D3DPRIMITIVETYPE` - always `D3DPT_TRIANGLELIST`.
    pub primitive_type: u32,
    /// `D3DVERTEXTYPE` from the LOD draw-group header (`+0x10`).
    pub vertex_type: u32,
    /// The vertex buffer and its count.
    pub vertex_buffer: u32,
    pub vertex_count: u32,
    /// The index buffer and its count.
    pub index_buffer: u32,
    pub index_count: u32,
    /// `dwFlags` - always `D3DDP_DONOTUPDATEEXTENTS`.
    pub flags: u32,
}

/// Assembles the `DrawIndexedPrimitive` call for one prim under a LOD draw-group's
/// `vertex_type` (`FUN_004a28a0`: `DrawIndexedPrimitive(TRIANGLELIST, vertex_type,
/// prim.vbuf, prim.vcount, prim.ibuf, prim.icount, DONOTUPDATEEXTENTS)`).
pub fn draw_call(vertex_type: u32, prim: &PrimBatch) -> DrawIndexedPrimitive {
    DrawIndexedPrimitive {
        primitive_type: PRIMTYPE_TRIANGLELIST,
        vertex_type,
        vertex_buffer: prim.vertex_buffer,
        vertex_count: prim.vertex_count,
        index_buffer: prim.index_buffer,
        index_count: prim.index_count,
        flags: DP_DONOTUPDATEEXTENTS,
    }
}

/// Whether per-vertex fog is baked into this prim before drawing (`FUN_004a28a0`:
/// the global fog query `FUN_004b01c0()` is set and the draw-group vertex type is
/// not `1`). The vertex type `1` (pre-lit/unfogged) skips the fog bake.
pub fn prim_takes_fog(global_fog_enabled: bool, vertex_type: u32) -> bool {
    global_fog_enabled && vertex_type != 1
}

/// Whether a draw-group vertex type carries per-vertex diffuse color unpacked
/// from the triangle's color bytes. The lit vertex type (`2`, [`VERTEXTYPE_LVERTEX`])
/// does; the pre-lit type (`1`) instead forces an opaque specular (`0xff000000`)
/// and rescales only the UVs (`FUN_004b1e10`), which is also why type `1` skips
/// the fog bake in [`prim_takes_fog`].
pub fn vertex_type_has_per_vertex_diffuse(vertex_type: u32) -> bool {
    vertex_type == VERTEXTYPE_LVERTEX
}

// --- Runtime mesh-record assembly (the loader node loop) ---

/// Loader mode selecting a two-sided menu mesh (`FUN_004a2b70` `param_2 == 3`).
pub const C3D_MODE_MENU_TWO_SIDED: i32 = 3;

/// `mesh+0x00`: dequantized/staging vertex array (`vertex_count * 0x20` bytes).
pub const MESH_STAGING_VERTEX_ARRAY_OFFSET: usize = 0x00;
/// `mesh+0x04`: preserved LOD-0 vertex stream (stage/car modes only).
pub const MESH_LOD0_VERTEX_STREAM_OFFSET: usize = 0x04;
/// `mesh+0x08`: preserved LOD-1 vertex stream (stage/car modes only).
pub const MESH_LOD1_VERTEX_STREAM_OFFSET: usize = 0x08;
/// `mesh+0x0c`: vertex count (from group-1 `+0x10`).
pub const MESH_VERTEX_COUNT_OFFSET: usize = 0x0c;
/// `mesh+0x10`: triangle count (from group-1 `+0x12`, doubled when two-sided).
pub const MESH_FACE_COUNT_OFFSET: usize = 0x10;
/// `mesh+0x18`: runtime triangle array (`triangle_count * 0x1c` bytes).
pub const MESH_TRIANGLE_ARRAY_OFFSET: usize = 0x18;
/// `mesh+0x81`: number of LODs stored in the record.
pub const MESH_LOD_COUNT_OFFSET: usize = 0x81;

/// Stride of one vertex in the staging and preserved LOD vertex streams.
pub const MESH_VERTEX_STRIDE: usize = 0x20;
/// Stride of one runtime triangle record in the `mesh+0x18` array.
pub const MESH_TRIANGLE_STRIDE: usize = 0x1c;

/// Runtime triangle record: triangle flag/color word.
pub const RUNTIME_TRI_FLAG_OFFSET: usize = 0x00;
/// Runtime triangle record: material ordinal (`-1` = untextured).
pub const RUNTIME_TRI_MATERIAL_OFFSET: usize = 0x04;
/// Runtime triangle record: first per-vertex ARGB color.
pub const RUNTIME_TRI_COLOR0_OFFSET: usize = 0x08;
/// Runtime triangle record: second per-vertex ARGB color.
pub const RUNTIME_TRI_COLOR1_OFFSET: usize = 0x0c;
/// Runtime triangle record: third per-vertex ARGB color.
pub const RUNTIME_TRI_COLOR2_OFFSET: usize = 0x10;
/// Runtime triangle record: first vertex index.
pub const RUNTIME_TRI_INDEX0_OFFSET: usize = 0x14;
/// Runtime triangle record: second vertex index.
pub const RUNTIME_TRI_INDEX1_OFFSET: usize = 0x16;
/// Runtime triangle record: third vertex index.
pub const RUNTIME_TRI_INDEX2_OFFSET: usize = 0x18;

/// The sentinel material ordinal marking an untextured runtime triangle, which
/// takes the per-mode default diffuse instead of a texture bind.
pub const RUNTIME_TRI_UNTEXTURED_MATERIAL: i32 = -1;

/// One runtime triangle record (`mesh+0x18` array, stride `0x1c`), built from an
/// on-disk face (entry24) in the loader node loop. The prim list that the draw
/// walker expands is grouped from this array by [`material_ref`](Self::material_ref)
/// via [`crate::c3d_batch::plan_consecutive_batches`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeTriangle {
    /// `+0x00`: triangle flag/color word.
    pub flag: u16,
    /// `+0x04`: material ordinal, or [`RUNTIME_TRI_UNTEXTURED_MATERIAL`].
    pub material_ref: i32,
    /// `+0x08`/`+0x0c`/`+0x10`: the three per-vertex ARGB colors.
    pub vertex_colors: [u32; 3],
    /// `+0x14`/`+0x16`/`+0x18`: the three vertex indices into the mesh vertices.
    pub vertex_indices: [u16; 3],
}

impl RuntimeTriangle {
    /// Whether this triangle is untextured (its ordinal is the `-1` sentinel).
    pub fn is_untextured(&self) -> bool {
        self.material_ref == RUNTIME_TRI_UNTEXTURED_MATERIAL
    }
}

/// Whether the loader duplicates each face to make the mesh two-sided. Only the
/// two-sided menu mode (`3`) does so, and only when the renderer reports no
/// hardware two-sided handling (`renderer+0x28 == 0`, i.e. `renderer_backface_flag`
/// is `0`).
pub fn mode_duplicates_back_faces(mode: i32, renderer_backface_flag: u32) -> bool {
    mode == C3D_MODE_MENU_TWO_SIDED && renderer_backface_flag == 0
}

/// The runtime triangle-array length: the group-1 face count, doubled when
/// [`mode_duplicates_back_faces`] holds.
pub fn runtime_face_count(group1_face_count: u16, mode: i32, renderer_backface_flag: u32) -> usize {
    let base = usize::from(group1_face_count);
    if mode_duplicates_back_faces(mode, renderer_backface_flag) {
        base * 2
    } else {
        base
    }
}

/// Whether this loader mode preserves the first two LODs' vertex streams in the
/// mesh record (`mesh+0x04`/`+0x08`). The stage/car tree-builder modes
/// (`5`/`6`/`7`/`8`) do; the menu/instance modes (`0`/`1`/`2`/`3`) do not.
pub fn preserves_lod_vertex_streams(mode: i32) -> bool {
    matches!(mode, 5..=8)
}

/// The mesh-record offset that preserves this LOD's vertex stream, when
/// [`preserves_lod_vertex_streams`] holds for the mode. Only LOD `0`
/// (`mesh+0x04`) and LOD `1` (`mesh+0x08`) are backed up; higher LODs are not.
pub fn lod_stream_backup_offset(lod: u8) -> Option<usize> {
    match lod {
        0 => Some(MESH_LOD0_VERTEX_STREAM_OFFSET),
        1 => Some(MESH_LOD1_VERTEX_STREAM_OFFSET),
        _ => None,
    }
}

// --- Per-LOD draw-group assembly (parse -> batch -> draw call) ---

/// One LOD's assembled draw group: the draw-group vertex type (the value
/// `FUN_004b1e10` writes - `2` = lit LVERTEX, `1` = pre-lit) paired with the
/// prim batches the draw walker expands.
///
/// Each prim is a consecutive run of faces sharing a resolved selector, carrying
/// the deduplicated vertices and compact triangle indices directly. Those are
/// the recovered system-memory vertex/index streams (see the module docs), not
/// Direct3D buffers, so the prim *is* a [`C3dConsecutiveBatch`]. The batch
/// builder `FUN_004b2340` is exactly [`plan_consecutive_batches`], so this type
/// only pairs that output with the LOD's vertex type.
#[derive(Debug, Eq, PartialEq)]
pub struct MeshLodDrawGroup<'a, S> {
    vertex_type: u32,
    draw_flags: u8,
    prims: Vec<C3dConsecutiveBatch<'a, S>>,
}

impl<'a, S: Copy> MeshLodDrawGroup<'a, S> {
    /// The draw-group vertex type passed to [`draw_call`] for every prim.
    pub fn vertex_type(&self) -> u32 {
        self.vertex_type
    }

    /// The draw-group flags byte (`mesh + lod*0x18 + 0x20`, copied from Group-1
    /// record offset `+0x14` by `FUN_004b1e10`).
    pub fn draw_flags(&self) -> u8 {
        self.draw_flags
    }

    /// Whether this LOD disables back-face culling (`D3DCULL_NONE`,
    /// [`crate::c3d_camera::TWO_SIDED_FLAG`]).
    pub fn is_two_sided(&self) -> bool {
        self.draw_flags & crate::c3d_camera::TWO_SIDED_FLAG != 0
    }

    /// Whether this LOD draws through the cylindrical Y-billboard drawer
    /// `FUN_004a18e0` ([`crate::c3d_camera::Y_BILLBOARD_FLAG`]).
    pub fn is_y_billboard(&self) -> bool {
        self.draw_flags & crate::c3d_camera::Y_BILLBOARD_FLAG != 0
    }

    /// The prim batches, in submission order.
    pub fn prims(&self) -> &[C3dConsecutiveBatch<'a, S>] {
        &self.prims
    }

    /// Whether the per-prim fog bake applies to this LOD (see [`prim_takes_fog`]).
    pub fn takes_fog(&self, global_fog_enabled: bool) -> bool {
        prim_takes_fog(global_fog_enabled, self.vertex_type)
    }

    /// Whether this LOD's vertices carry per-vertex diffuse color (see
    /// [`vertex_type_has_per_vertex_diffuse`]).
    pub fn has_per_vertex_diffuse(&self) -> bool {
        vertex_type_has_per_vertex_diffuse(self.vertex_type)
    }
}

/// Expands a triangle-list index stream with reversed-winding duplicates
/// (`[i2, i1, i0]` after the original triangles, matching `FUN_004a2b70` at
/// `0x4a3e9e..0x4a3f42`) so a two-sided draw group (`D3DCULL_NONE`) renders from
/// both sides under a single-sided `D3DCULL_CCW` rasterizer.
#[must_use]
pub fn two_sided_indices(indices: &[u16]) -> Vec<u16> {
    let mut doubled = Vec::with_capacity(indices.len() * 2);
    doubled.extend_from_slice(indices);
    for &[i0, i1, i2] in indices.as_chunks::<3>().0 {
        doubled.extend_from_slice(&[i2, i1, i0]);
    }
    doubled
}

/// Assembles one LOD's draw group from its on-disk group-1 mesh record, the LOD
/// draw-group vertex type, and the resolved per-face selectors.
///
/// There is one selector per 24-byte face: the face's group-2 material ordinal
/// resolved to a render resource, already reflecting the `-1` untextured
/// fallback, exactly as [`plan_consecutive_batches`] requires. This mirrors the
/// loader finalize `FUN_004b1e10` -> prim builder `FUN_004b2340`: the prim list
/// is grouped by consecutive selector run, with per-run vertex deduplication and
/// compact `u16` indices. Keying on the resolved selector (not the raw on-disk
/// ordinal) is deliberate - the loader batches on the runtime material reference.
pub fn assemble_lod_draw_group<'a, S: Copy + Eq>(
    record: C3dGroup1Record<'a>,
    vertex_type: u32,
    selectors: &[S],
) -> Result<MeshLodDrawGroup<'a, S>, C3dBatchPlanError> {
    let prims = plan_consecutive_batches(record, selectors)?;
    Ok(MeshLodDrawGroup {
        vertex_type,
        draw_flags: record.draw_flags(),
        prims,
    })
}

/// The vertex and index counts one prim submits in its `DrawIndexedPrimitive`
/// call: the prim's deduplicated vertex count and its `3 * triangles` indices.
pub fn prim_draw_counts<S: Copy>(prim: &C3dConsecutiveBatch<'_, S>) -> (u32, u32) {
    let vertex_count = u32::try_from(prim.unique_entries_32().len())
        .expect("a C3D prim's deduplicated vertex count fits in u32");
    let index_count =
        u32::try_from(prim.indices().len()).expect("a C3D prim's index count fits in u32");
    (vertex_count, index_count)
}

// --- Node multi-LOD mesh assembly (node mesh-index list -> per-LOD meshes) ---

/// The mesh-index value marking a skipped LOD slot (the node value `0xffff`).
pub const MESH_LOD_INDEX_SENTINEL: u16 = 0xffff;

/// The draw-group vertex type the loader assigns to every LOD of a node's mesh,
/// chosen by bit 0 of the load mode flag `param_5` of `FUN_004a2b70`: `2`
/// ([`VERTEXTYPE_LVERTEX`], lit, per-vertex diffuse) when the bit is clear, `1`
/// (pre-lit, opaque + UV-only) when it is set. It is fixed per load call and
/// shared by all LODs.
pub fn node_mesh_vertex_type(load_flags: u8) -> u32 {
    if load_flags & 1 == 0 {
        VERTEXTYPE_LVERTEX
    } else {
        1
    }
}

/// A scene node's assembled multi-LOD mesh: one draw group per LOD in the node's
/// mesh-index-list order, plus the active LOD the draw walker reads.
///
/// A LOD slot is `None` when the node's mesh-index entry is the
/// [`MESH_LOD_INDEX_SENTINEL`] or its group-1 record has no geometry: the loader
/// advances the LOD index but finalizes no draw group, so the slot is preserved
/// (keeping later LOD indices aligned) but empty. The active LOD is `0` at load
/// (`mesh+0x80`); the runtime distance-based LOD selection that later updates it
/// is a separate subsystem not modeled here.
#[derive(Debug, Eq, PartialEq)]
pub struct AssembledMesh<'a> {
    active_lod: u8,
    lods: Vec<Option<MeshLodDrawGroup<'a, i16>>>,
}

impl<'a> AssembledMesh<'a> {
    /// The load-time active LOD index (`0`), the slot the draw walker expands.
    pub fn active_lod(&self) -> u8 {
        self.active_lod
    }

    /// The number of LOD slots (`mesh+0x81`), equal to the node's mesh-index
    /// count including any skipped (`None`) slots.
    pub fn lod_count(&self) -> usize {
        self.lods.len()
    }

    /// The draw group for LOD `lod`, or `None` for a skipped slot or an index
    /// past the last LOD.
    pub fn lod(&self, lod: usize) -> Option<&MeshLodDrawGroup<'a, i16>> {
        self.lods.get(lod).and_then(Option::as_ref)
    }

    /// The draw group for the active LOD, if that slot carries geometry.
    pub fn active(&self) -> Option<&MeshLodDrawGroup<'a, i16>> {
        self.lod(usize::from(self.active_lod))
    }
}

/// Failure to assemble a node's multi-LOD mesh.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssembleNodeMeshError {
    /// A mesh index addressed past the end of the group-1 record table (mesh LUT).
    MeshIndexOutOfBounds {
        lod: usize,
        index: u16,
        record_count: usize,
    },
    /// One LOD's draw-group assembly failed (an out-of-range vertex ordinal).
    Lod {
        lod: usize,
        source: C3dBatchPlanError,
    },
}

impl fmt::Display for AssembleNodeMeshError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MeshIndexOutOfBounds {
                lod,
                index,
                record_count,
            } => write!(
                formatter,
                "C3D node LOD {lod} mesh index {index} is outside the {record_count} group-1 records"
            ),
            Self::Lod { lod, source } => write!(formatter, "C3D node LOD {lod}: {source}"),
        }
    }
}

impl std::error::Error for AssembleNodeMeshError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lod { source, .. } => Some(source),
            Self::MeshIndexOutOfBounds { .. } => None,
        }
    }
}

/// Assembles a scene node's multi-LOD mesh from its mesh-index list and the
/// file's group-1 records (the mesh LUT, in file order).
///
/// Each mesh index is one LOD (`FUN_004a2b70` node loop): `mesh_indices[k]`
/// selects `group1_records[mesh_indices[k]]` for LOD `k`. A [`MESH_LOD_INDEX_SENTINEL`]
/// index, or a record with zero vertices or zero faces, leaves LOD `k`'s slot
/// empty. `vertex_type` is the fixed per-load value (see [`node_mesh_vertex_type`])
/// shared by every LOD. Each LOD's faces are batched by their group-2 material
/// ordinal (the loader keys the prim builder on the on-disk material id;
/// resolving an ordinal to a bound texture is a separate step).
pub fn assemble_node_mesh<'a>(
    mesh_indices: &[u16],
    group1_records: &[C3dGroup1Record<'a>],
    vertex_type: u32,
) -> Result<AssembledMesh<'a>, AssembleNodeMeshError> {
    let mut lods = Vec::with_capacity(mesh_indices.len());
    for (lod, &index) in mesh_indices.iter().enumerate() {
        if index == MESH_LOD_INDEX_SENTINEL {
            lods.push(None);
            continue;
        }
        let record = *group1_records.get(usize::from(index)).ok_or(
            AssembleNodeMeshError::MeshIndexOutOfBounds {
                lod,
                index,
                record_count: group1_records.len(),
            },
        )?;
        if record.entry32_count() == 0 || record.entry24_count() == 0 {
            lods.push(None);
            continue;
        }
        let selectors: Vec<i16> = record
            .entries_24()
            .map(|face| face.group2_ordinal())
            .collect();
        let group = assemble_lod_draw_group(record, vertex_type, &selectors)
            .map_err(|source| AssembleNodeMeshError::Lod { lod, source })?;
        lods.push(Some(group));
    }
    Ok(AssembledMesh {
        active_lod: 0,
        lods,
    })
}

/// Assembles every scene node's mesh from a parsed C3D file's node records and
/// group-1 records (the mesh LUT, in file order).
///
/// Returns one entry per group-0 node record, aligned with the scene-graph node
/// index that [`crate::c3d_scene_graph::SceneGraph::from_c3d_group0`] assigns
/// (records map 1:1 to nodes). A node with no mesh-index list (`value_count == 0`,
/// the root marker and transform-only nodes) yields `None`; a drawable node
/// yields its [`AssembledMesh`] via [`assemble_node_mesh`] over its
/// [`C3dGroup0Record::values`]. `vertex_type` is the fixed per-load value (see
/// [`node_mesh_vertex_type`]). This is the capstone that turns a parsed stage or
/// car `.c3d` into per-node geometry.
pub fn assemble_scene_meshes<'a>(
    group0_records: impl IntoIterator<Item = C3dGroup0Record<'a>>,
    group1_records: &[C3dGroup1Record<'a>],
    vertex_type: u32,
) -> Result<Vec<Option<AssembledMesh<'a>>>, AssembleNodeMeshError> {
    group0_records
        .into_iter()
        .map(|node| {
            if node.value_count() == 0 {
                return Ok(None);
            }
            let indices: Vec<u16> = node.values().collect();
            assemble_node_mesh(&indices, group1_records, vertex_type).map(Some)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::c3d_render::{DP_DONOTUPDATEEXTENTS, PRIMTYPE_TRIANGLELIST};

    #[test]
    fn lod_table_offsets_step_by_the_record_stride() {
        assert_eq!(draw_group_offset(0), 0x20);
        assert_eq!(draw_group_offset(1), 0x38);
        assert_eq!(prim_head_offset(0), 0x34);
        assert_eq!(prim_head_offset(1), 0x4c);
        // The draw-group header and prim-list head for a LOD are 0x14 apart.
        assert_eq!(prim_head_offset(3) - draw_group_offset(3), 0x14);
    }

    #[test]
    fn draw_call_maps_the_prim_fields_to_draw_indexed_primitive() {
        let prim = PrimBatch {
            resource_ordinal: 7,
            vertex_count: 24,
            vertex_buffer: 0x1000,
            index_count: 36,
            index_buffer: 0x2000,
        };
        let call = draw_call(2, &prim);
        assert_eq!(
            call,
            DrawIndexedPrimitive {
                primitive_type: PRIMTYPE_TRIANGLELIST,
                vertex_type: 2,
                vertex_buffer: 0x1000,
                vertex_count: 24,
                index_buffer: 0x2000,
                index_count: 36,
                flags: DP_DONOTUPDATEEXTENTS,
            }
        );
        assert!(!prim.is_untextured());
    }

    #[test]
    fn the_minus_one_ordinal_marks_an_untextured_prim() {
        let prim = PrimBatch {
            resource_ordinal: PRIM_UNTEXTURED_ORDINAL,
            vertex_count: 3,
            vertex_buffer: 1,
            index_count: 3,
            index_buffer: 2,
        };
        assert!(prim.is_untextured());
    }

    #[test]
    fn fog_is_baked_unless_disabled_or_vertex_type_is_one() {
        assert!(prim_takes_fog(true, 2));
        assert!(!prim_takes_fog(false, 2)); // global fog off
        assert!(!prim_takes_fog(true, 1)); // pre-lit vertex type skips fog
    }

    #[test]
    fn only_the_lit_vertex_type_carries_per_vertex_diffuse() {
        assert!(vertex_type_has_per_vertex_diffuse(2)); // VERTEXTYPE_LVERTEX
        assert!(!vertex_type_has_per_vertex_diffuse(1)); // pre-lit forces opaque
    }

    #[test]
    fn only_two_sided_menu_mode_doubles_faces_and_only_without_hw_two_sided() {
        // Mode 3 with no hardware two-sided handling (flag 0) doubles.
        assert!(mode_duplicates_back_faces(C3D_MODE_MENU_TWO_SIDED, 0));
        assert_eq!(runtime_face_count(10, C3D_MODE_MENU_TWO_SIDED, 0), 20);
        // Mode 3 but the renderer handles two-sided: no doubling.
        assert!(!mode_duplicates_back_faces(C3D_MODE_MENU_TWO_SIDED, 1));
        assert_eq!(runtime_face_count(10, C3D_MODE_MENU_TWO_SIDED, 1), 10);
        // Other modes never double, regardless of the renderer flag.
        for mode in [0, 1, 2, 5, 6, 7, 8] {
            assert!(!mode_duplicates_back_faces(mode, 0));
            assert_eq!(runtime_face_count(7, mode, 0), 7);
        }
    }

    #[test]
    fn runtime_triangle_untextured_sentinel() {
        let tri = RuntimeTriangle {
            flag: 0,
            material_ref: 5,
            vertex_colors: [0xff00_0000, 0xff00_0000, 0xff00_0000],
            vertex_indices: [0, 1, 2],
        };
        assert!(!tri.is_untextured());
        let untextured = RuntimeTriangle {
            material_ref: RUNTIME_TRI_UNTEXTURED_MATERIAL,
            ..tri
        };
        assert!(untextured.is_untextured());
    }

    #[test]
    fn only_stage_and_car_modes_preserve_lod_vertex_streams() {
        for mode in [5, 6, 7, 8] {
            assert!(preserves_lod_vertex_streams(mode));
        }
        for mode in [0, 1, 2, 3] {
            assert!(!preserves_lod_vertex_streams(mode));
        }
    }

    #[test]
    fn only_the_first_two_lods_back_up_their_vertex_stream() {
        assert_eq!(lod_stream_backup_offset(0), Some(0x04));
        assert_eq!(lod_stream_backup_offset(1), Some(0x08));
        assert_eq!(lod_stream_backup_offset(2), None);
        assert_eq!(lod_stream_backup_offset(255), None);
    }

    #[test]
    fn lod_draw_group_batches_faces_by_consecutive_material_run() {
        // Three vertices; three faces all referencing [0, 1, 2]; materials form
        // a run of 7, then the untextured sentinel -1 => two prim batches.
        let file = mesh_record_file(3, &[(7, [0, 1, 2]), (7, [0, 1, 2]), (-1, [0, 1, 2])]);
        let record = file.group1_records().next().unwrap();
        let selectors: Vec<i16> = record
            .entries_24()
            .map(|face| face.group2_ordinal())
            .collect();

        let group = assemble_lod_draw_group(record, VERTEXTYPE_LVERTEX, &selectors).unwrap();

        assert_eq!(group.vertex_type(), VERTEXTYPE_LVERTEX);
        assert!(group.has_per_vertex_diffuse());
        assert!(group.takes_fog(true));
        assert!(!group.takes_fog(false));
        assert_eq!(group.draw_flags(), 0);
        assert!(!group.is_two_sided());
        assert!(!group.is_y_billboard());
        assert_eq!(
            two_sided_indices(&[0, 1, 2, 3, 4, 5]),
            vec![0, 1, 2, 3, 4, 5, 2, 1, 0, 5, 4, 3]
        );

        let prims = group.prims();
        assert_eq!(prims.len(), 2);
        // Prim 0: the two material-7 faces, three unique vertices, six indices.
        assert_eq!(prims[0].selector(), 7);
        assert_eq!(prim_draw_counts(&prims[0]), (3, 6));
        // Prim 1: the single untextured face.
        assert_eq!(prims[1].selector(), -1);
        assert_eq!(prim_draw_counts(&prims[1]), (3, 3));
    }

    #[test]
    fn lod_draw_group_rejects_a_mismatched_selector_count() {
        let file = mesh_record_file(3, &[(0, [0, 1, 2])]);
        let record = file.group1_records().next().unwrap();

        let error = assemble_lod_draw_group::<i16>(record, VERTEXTYPE_LVERTEX, &[]).unwrap_err();
        assert_eq!(
            error,
            crate::c3d_batch::C3dBatchPlanError::SelectorCountMismatch {
                entry24_count: 1,
                selector_count: 0,
            }
        );
    }

    #[test]
    fn node_mesh_vertex_type_follows_bit_zero_of_the_load_flags() {
        assert_eq!(node_mesh_vertex_type(0), VERTEXTYPE_LVERTEX); // bit clear => lit
        assert_eq!(node_mesh_vertex_type(0b1110), VERTEXTYPE_LVERTEX); // other bits ignored
        assert_eq!(node_mesh_vertex_type(1), 1); // bit set => pre-lit
        assert_eq!(node_mesh_vertex_type(0b1011), 1);
    }

    #[test]
    fn node_mesh_assembles_one_draw_group_per_lod_in_index_order() {
        // LOD 0: two material-7 faces over 3 verts. LOD 1: one face over 4 verts.
        let lod0 = mesh_record_file(3, &[(7, [0, 1, 2]), (7, [0, 1, 2])]);
        let lod1 = mesh_record_file(4, &[(5, [0, 1, 3])]);
        let lut = [
            lod0.group1_records().next().unwrap(),
            lod1.group1_records().next().unwrap(),
        ];

        let mesh = assemble_node_mesh(&[0, 1], &lut, VERTEXTYPE_LVERTEX).unwrap();

        assert_eq!(mesh.active_lod(), 0);
        assert_eq!(mesh.lod_count(), 2);
        let lod0_group = mesh.lod(0).unwrap();
        assert_eq!(lod0_group.prims().len(), 1);
        assert_eq!(prim_draw_counts(&lod0_group.prims()[0]), (3, 6));
        assert_eq!(mesh.lod(1).unwrap().prims()[0].selector(), 5);
        // The active LOD is the first slot.
        assert_eq!(mesh.active(), mesh.lod(0));
    }

    #[test]
    fn node_mesh_leaves_sentinel_and_empty_lod_slots_vacant_but_aligned() {
        let geometry = mesh_record_file(3, &[(0, [0, 1, 2])]);
        let empty = mesh_record_file(3, &[]); // a record with no faces
        let lut = [
            geometry.group1_records().next().unwrap(),
            empty.group1_records().next().unwrap(),
        ];

        // LOD 0 = sentinel (skipped), LOD 1 = real geometry, LOD 2 = empty record.
        let mesh =
            assemble_node_mesh(&[MESH_LOD_INDEX_SENTINEL, 0, 1], &lut, VERTEXTYPE_LVERTEX).unwrap();

        assert_eq!(mesh.lod_count(), 3); // slots stay aligned with the index list
        assert!(mesh.lod(0).is_none()); // sentinel
        assert!(mesh.lod(1).is_some()); // geometry
        assert!(mesh.lod(2).is_none()); // zero-face record
        // Active LOD 0 is a skipped slot, so nothing is drawn at load.
        assert!(mesh.active().is_none());
    }

    #[test]
    fn node_mesh_rejects_a_mesh_index_past_the_lut() {
        let only = mesh_record_file(3, &[(0, [0, 1, 2])]);
        let lut = [only.group1_records().next().unwrap()];

        let error = assemble_node_mesh(&[0, 2], &lut, VERTEXTYPE_LVERTEX).unwrap_err();
        assert_eq!(
            error,
            AssembleNodeMeshError::MeshIndexOutOfBounds {
                lod: 1,
                index: 2,
                record_count: 1,
            }
        );
    }

    #[test]
    fn scene_meshes_align_with_node_index_and_resolve_values_through_the_lut() {
        // Node 0 = root marker (no mesh), node 1 = two LODs (indices 0, 1),
        // node 2 = transform-only (no mesh).
        let nodes = nodes_file(&[&[], &[0, 1], &[]]);
        let lod0 = mesh_record_file(3, &[(7, [0, 1, 2])]);
        let lod1 = mesh_record_file(4, &[(5, [0, 1, 3])]);
        let lut = [
            lod0.group1_records().next().unwrap(),
            lod1.group1_records().next().unwrap(),
        ];

        let meshes =
            assemble_scene_meshes(nodes.group0_records(), &lut, VERTEXTYPE_LVERTEX).unwrap();

        assert_eq!(meshes.len(), 3); // one entry per node record
        assert!(meshes[0].is_none()); // marker, no mesh-index list
        assert!(meshes[2].is_none()); // transform-only
        let node1 = meshes[1].as_ref().unwrap();
        assert_eq!(node1.lod_count(), 2);
        assert_eq!(node1.lod(0).unwrap().prims()[0].selector(), 7);
        assert_eq!(node1.lod(1).unwrap().prims()[0].selector(), 5);
    }

    #[test]
    fn scene_meshes_propagate_a_bad_mesh_index() {
        let nodes = nodes_file(&[&[], &[5]]); // node 1 references LUT index 5
        let only = mesh_record_file(3, &[(0, [0, 1, 2])]);
        let lut = [only.group1_records().next().unwrap()];

        let error =
            assemble_scene_meshes(nodes.group0_records(), &lut, VERTEXTYPE_LVERTEX).unwrap_err();
        assert_eq!(
            error,
            AssembleNodeMeshError::MeshIndexOutOfBounds {
                lod: 0,
                index: 5,
                record_count: 1,
            }
        );
    }

    /// Builds a group-0-only C3D file whose node records carry the given
    /// mesh-index (`values`) lists. Node 0 is the root marker (`field_04 = -1`),
    /// the rest reference parent 0; only the fields this module reads are set.
    fn nodes_file(nodes: &[&[u16]]) -> cmr_formats::c3d::C3dFile {
        use cmr_formats::c3d::{C3dLimits, parse_c3d};

        let mut body = Vec::new();
        for (index, values) in nodes.iter().enumerate() {
            let declared = 0x50 + values.len() * 2;
            let mut record = vec![0u8; declared];
            record[0..4].copy_from_slice(&u32::try_from(declared).unwrap().to_le_bytes());
            let field_04: i16 = if index == 0 { -1 } else { 0 };
            record[4..6].copy_from_slice(&field_04.to_le_bytes());
            record[0x4c] = u8::try_from(values.len()).unwrap();
            for (k, &value) in values.iter().enumerate() {
                let start = 0x50 + k * 2;
                record[start..start + 2].copy_from_slice(&value.to_le_bytes());
            }
            body.extend_from_slice(&record);
        }

        let mut buf = vec![0u8; 32];
        buf[0..4].copy_from_slice(b"CM3D");
        buf[4..8].copy_from_slice(&100u32.to_le_bytes());
        buf[12..14].copy_from_slice(&u16::try_from(nodes.len()).unwrap().to_le_bytes());
        buf[20..24].copy_from_slice(&32u32.to_le_bytes()); // group0 at offset 32
        let end = u32::try_from(32 + body.len()).unwrap();
        buf[24..28].copy_from_slice(&end.to_le_bytes()); // empty group1 at end
        buf[28..32].copy_from_slice(&end.to_le_bytes()); // empty blob at end
        buf.extend_from_slice(&body);

        parse_c3d(&buf, C3dLimits::default()).unwrap()
    }

    /// Builds a minimal C3D file with one group-1 mesh record: `vertex_count`
    /// distinct 32-byte vertices and one 24-byte face per `(material, ordinals)`,
    /// with the material ordinal written to the face `+0x08` field.
    fn mesh_record_file(
        vertex_count: usize,
        faces: &[(i16, [u16; 3])],
    ) -> cmr_formats::c3d::C3dFile {
        use cmr_formats::c3d::{
            C3D_FORMAT_VERSION, C3D_GROUP1_ENTRY24_LEN, C3D_GROUP1_ENTRY32_LEN,
            C3D_GROUP1_FIXED_LEN, C3D_HEADER_LEN, C3dLimits, parse_c3d,
        };

        let record_len = C3D_GROUP1_FIXED_LEN
            + vertex_count * C3D_GROUP1_ENTRY32_LEN
            + faces.len() * C3D_GROUP1_ENTRY24_LEN;
        let payload_len = C3D_HEADER_LEN + record_len;
        let mut input = vec![0u8; payload_len];
        input[0..4].copy_from_slice(b"CM3D");
        input[4..8].copy_from_slice(&C3D_FORMAT_VERSION.to_le_bytes());
        input[8..12].copy_from_slice(&(C3D_HEADER_LEN as u32).to_le_bytes());
        input[14..16].copy_from_slice(&1u16.to_le_bytes());
        input[20..24].copy_from_slice(&(C3D_HEADER_LEN as u32).to_le_bytes());
        input[24..28].copy_from_slice(&(C3D_HEADER_LEN as u32).to_le_bytes());
        input[28..32].copy_from_slice(&(payload_len as u32).to_le_bytes());

        let record_offset = C3D_HEADER_LEN;
        input[record_offset + 12..record_offset + 16]
            .copy_from_slice(&(record_len as u32).to_le_bytes());
        input[record_offset + 16..record_offset + 18]
            .copy_from_slice(&u16::try_from(vertex_count).unwrap().to_le_bytes());
        input[record_offset + 18..record_offset + 20]
            .copy_from_slice(&u16::try_from(faces.len()).unwrap().to_le_bytes());

        let entries32_offset = record_offset + C3D_GROUP1_FIXED_LEN;
        for index in 0..vertex_count {
            let start = entries32_offset + index * C3D_GROUP1_ENTRY32_LEN;
            input[start..start + C3D_GROUP1_ENTRY32_LEN]
                .copy_from_slice(&[u8::try_from(index + 1).unwrap(); C3D_GROUP1_ENTRY32_LEN]);
        }

        let entries24_offset = entries32_offset + vertex_count * C3D_GROUP1_ENTRY32_LEN;
        // Face layout: ordinals at +0x02/+0x04/+0x06, material ordinal at +0x08.
        for (face_index, (material, ordinals)) in faces.iter().enumerate() {
            let start = entries24_offset + face_index * C3D_GROUP1_ENTRY24_LEN;
            for (corner, ordinal) in ordinals.iter().enumerate() {
                let field = start + 2 + corner * 2;
                input[field..field + 2].copy_from_slice(&ordinal.to_le_bytes());
            }
            input[start + 8..start + 10].copy_from_slice(&material.to_le_bytes());
        }

        parse_c3d(&input, C3dLimits::default()).unwrap()
    }
}
