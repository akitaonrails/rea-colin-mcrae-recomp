//! The car's crash damage: the per-part dents (`FUN_0041f880`), the part
//! detach with its breakage kind (`FUN_00420120`) and the per-player part
//! setup the race load runs (`FUN_0041efe0`'s loop over the body node's
//! children). The severity is [`crate::prang_sound::impact_severity`]
//! (`FUN_0041f630`), the handling follow-up
//! [`crate::car_setup::derive_car_setup`] (`FUN_00420740`).
//!
//! A car body is node 5 of its `.C3D`; its direct children with ids
//! `6..=0x6e` are the parts (`id - 6`), each with three LODs: LOD 0 is the
//! drawn ("live") mesh, LOD 1 the fully crushed target and LOD 2 the rest
//! shape. The part nodes carry identity matrices, so their vertices are
//! car-local. An impact moves every live vertex within unit distance of the
//! car-local contact point toward its crushed position, weighted by
//! `(1 - r^2) * severity`; a part whose accumulated deformation passes its
//! threshold takes the crushed mesh's UVs and its damaged material. The
//! arithmetic runs over each LOD's prim runs (`FUN_004b2340`'s compacted
//! copies, so a vertex shared by two runs counts twice) in the binary's
//! prepended chain order.
//!
//! Not modelled, because nothing observable reads them: the normal arrays
//! (the draw path does not read them), the per-part bounds `+0x8dc/+0xa44`
//! and `+0x784`. The night headlight pod-lamp breakage (`FUN_0044a770`) is
//! reconstructed in [`CarDamage::break_pod_lamp_44a770`]. A description of
//! disassembly, not original source.
//! Evidence: `rea.official-eu.car-crash-damage`, `.work/car-damage-notes.md`,
//! `.work/car-parts-notes.md`.

use cmr_formats::c3d::{C3dFile, C3dGroup1Record};

use crate::c3d_batch::plan_consecutive_batches;

/// The parts a setup block has room for (`+0x788[30]`).
pub const MAX_PARTS: usize = 30;
/// The body node's id; its children are the parts.
pub const BODY_NODE_ID: u16 = 5;
/// The special part's id (`S +0x2d0`).
pub const SPECIAL_PART_ID: u16 = 0x6f;

fn qmul(a: i32, b: i32) -> i32 {
    ((i64::from(a) * i64::from(b)) >> 16) as i32
}

fn qdiv(a: i32, b: i32) -> i32 {
    ((i64::from(a) << 16) / i64::from(b)) as i32
}

/// One prim run of a part LOD (`FUN_004b2340`'s record): its compacted
/// vertices as Q16 words, its material and where it came from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DamagePrim {
    pub positions: Vec<[i32; 3]>,
    pub uvs: Vec<[i32; 2]>,
    /// The faces' group-2 ordinal (prim `+0x00`).
    pub material: i16,
    /// The source vertex of each compacted vertex, and the run's faces, in
    /// the LOD's group-1 record.
    pub entries: Vec<u16>,
    pub faces: std::ops::Range<usize>,
}

/// One part's three LOD chains, in the binary's prepended (reversed) order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DamagePart {
    /// The node id (`6 + part`, or [`SPECIAL_PART_ID`]).
    pub node_id: u16,
    /// LOD 2 (rest), LOD 1 (crushed), LOD 0 (live, drawn).
    pub rest: Vec<DamagePrim>,
    pub crush: Vec<DamagePrim>,
    pub live: Vec<DamagePrim>,
    /// LOD 0's group-1 record, and the node's group-0 record (its scene
    /// node).
    pub live_record: usize,
    pub node_index: usize,
    /// The material walk's stop, in original prim order: the live prim
    /// that takes the crushed prim's material on detach.
    material_prim: usize,
}

impl DamagePart {
    /// Build a part from its three LODs' prims in their original order.
    /// `None` when a LOD has no prims (the setup skips such a part).
    #[must_use]
    pub fn new(
        node_id: u16,
        live_record: usize,
        lod0: Vec<DamagePrim>,
        lod1: Vec<DamagePrim>,
        lod2: Vec<DamagePrim>,
    ) -> Option<Self> {
        if lod0.is_empty() || lod1.is_empty() || lod2.is_empty() {
            return None;
        }
        // The walk sets the slots, stops at the first rest/crushed pair
        // whose ordinals differ, else advances to the end of the rest list.
        let mut material_prim = 0;
        for (k, a) in lod2.iter().enumerate() {
            material_prim = k;
            if lod1.get(k).is_none_or(|b| b.material != a.material) {
                break;
            }
        }
        let reversed = |mut v: Vec<DamagePrim>| {
            v.reverse();
            v
        };
        Some(Self {
            node_id,
            rest: reversed(lod2),
            crush: reversed(lod1),
            live: reversed(lod0),
            live_record,
            node_index: 0,
            material_prim,
        })
    }

    /// The live prims with their place in the node's draw group (original
    /// order; the chains are reversed).
    pub fn live_in_draw_order(&self) -> impl Iterator<Item = (usize, &DamagePrim)> {
        let len = self.live.len();
        self.live
            .iter()
            .enumerate()
            .map(move |(c, prim)| (len - 1 - c, prim))
    }

    /// The chain-order index of an original-order prim.
    fn chain_index(len: usize, original: usize) -> Option<usize> {
        (original < len).then(|| len - 1 - original)
    }
}

/// The breakage cue the detach sets (`FUN_0046a2a0`): 1 headlight, 2
/// screen, 3 mirror.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakageKind {
    Headlight = 1,
    Screen = 2,
    Mirror = 3,
}

/// The index bytes `+0xcc1..+0xccd` and the detach thresholds
/// `+0xc30..+0xc58` the part setup writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageLayout {
    /// `+0xcc1..+0xcc6`: the parts the handling reads (`FUN_00420740`).
    pub handling_cc1: [u8; 6],
    /// `+0xcc7..+0xcca` screens, `+0xccb` mirror, `+0xccc..+0xccd` lights.
    pub screens_cc7: [u8; 4],
    pub mirror_ccb: u8,
    pub lights_ccc: [u8; 2],
    /// `+0xc30..`: parts 0..10; parts 11 and up keep 0.
    pub thresholds_c30: [i32; 11],
}

impl DamageLayout {
    /// `FUN_0041efe0`'s tables: high detail, or low detail with the screens
    /// keyed on the car model `car +0x739`.
    #[must_use]
    pub fn new(low_detail: bool, model: u8) -> Self {
        if !low_detail {
            return Self {
                handling_cc1: [7, 0xa, 1, 2, 6, 3],
                screens_cc7: [1, 2, 4, 5],
                mirror_ccb: 1,
                lights_ccc: [8, 9],
                thresholds_c30: [
                    0x8000, 0x3333, 0x3333, 0x8000, 0x3333, 0x3333, 0x8000, 0x8000, 0x1999, 0x1999,
                    0x8000,
                ],
            };
        }
        let screens_cc7 = match model {
            0 | 1 | 8 => [3, 3, 3, 3],
            2 | 3 | 6 | 10 => [4, 4, 4, 4],
            5 | 9 | 11 => [1, 1, 1, 1],
            7 => [1, 1, 2, 1],
            _ => [4, 4, 4, 2],
        };
        Self {
            handling_cc1: [7, 8, 5, 6, 7, 1],
            screens_cc7,
            mirror_ccb: 1,
            lights_ccc: [1, 1],
            thresholds_c30: [0x3333; 11],
        }
    }
}

/// One car's damage words of the setup block and its part meshes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CarDamage {
    /// The parts by index (`id - 6`; the special part at the count when
    /// it was met); `count` of them are processed (`S +0xbb0`).
    pub parts: Vec<Option<DamagePart>>,
    pub count: usize,
    /// `+0x2d0`: the special part, whose positions live in the Q16 list
    /// `+0x2d4` instead of its mesh.
    special: Option<usize>,
    special_list: Vec<[i32; 3]>,
    /// `+0x788`: the accumulated deformation per part, `+0x864` the last
    /// impact's intensity, `+0xbb8` the detach latches.
    pub deform_788: [i32; MAX_PARTS],
    pub impact_864: [i32; MAX_PARTS],
    pub detached_bb8: [bool; MAX_PARTS],
    pub layout: DamageLayout,
    /// `DAT_00828e28`: the low-detail car models.
    low_detail: bool,
    /// `S + 0xca8[6]`: broken night pod-lamp latches (`FUN_0044a770`).
    pub broken_pod_lamps_ca8: [bool; 6],
    /// `S + 0xcc0`: number of broken night pod lamps (`0..=2`).
    pub broken_pod_count_cc0: u8,
    /// Body-frame offsets of the model's night pod lamps (`0x4f0738`).
    pub pod_lamp_offsets: Vec<[i32; 3]>,
    /// `FUN_0046b240() == 8`: night stage gate for `FUN_0044a770`.
    pub night_stage: bool,
}

impl CarDamage {
    /// `FUN_0041efe0`'s per-player part setup over the body's children in
    /// child order, then its tail.
    #[must_use]
    pub fn new(children: Vec<DamagePart>, low_detail: bool, model: u8) -> Self {
        let mut parts: Vec<Option<DamagePart>> = vec![None; MAX_PARTS];
        let mut count = 0;
        let mut special = None;
        for part in children {
            let id = part.node_id;
            let index = if id == SPECIAL_PART_ID {
                special = Some(count);
                count
            } else if (6..SPECIAL_PART_ID).contains(&id) {
                usize::from(id - 6)
            } else {
                continue;
            };
            if index >= MAX_PARTS {
                continue;
            }
            parts[index] = Some(part);
            count += 1;
        }
        // `FUN_0041ef30`: the special part's live positions (skipped for a
        // special part at index 0).
        let special_list = special
            .filter(|&i| i != 0)
            .and_then(|i| parts[i].as_ref())
            .map(|part| {
                part.live
                    .iter()
                    .flat_map(|prim| prim.positions.iter().copied())
                    .collect()
            })
            .unwrap_or_default();
        Self {
            parts,
            count: count.min(MAX_PARTS),
            special,
            special_list,
            deform_788: [0; MAX_PARTS],
            impact_864: [0; MAX_PARTS],
            detached_bb8: [false; MAX_PARTS],
            layout: DamageLayout::new(low_detail, model),
            low_detail,
            broken_pod_lamps_ca8: [false; 6],
            broken_pod_count_cc0: 0,
            pod_lamp_offsets: Vec::new(),
            night_stage: false,
        }
    }

    /// Configure the night stage gate and breakable pod-lamp offsets for `FUN_0044a770`.
    pub fn set_night_pod_lamps(&mut self, night_stage: bool, pod_lamp_offsets: Vec<[i32; 3]>) {
        self.night_stage = night_stage;
        self.pod_lamp_offsets = pod_lamp_offsets;
        self.broken_pod_lamps_ca8 = [false; 6];
        self.broken_pod_count_cc0 = 0;
    }

    /// `FUN_0041d710`: `[L, R]` detached latches (`S+0xbb8[S.ccc]`, `S+0xbb8[S.ccd]`).
    #[must_use]
    pub fn detached_lights_lr(&self) -> [bool; 2] {
        self.layout.lights_ccc.map(|idx| {
            self.detached_bb8
                .get(usize::from(idx))
                .copied()
                .unwrap_or(false)
        })
    }

    /// Build the parts from a car `.C3D`: the direct children of its body
    /// node, each LOD batched by face material as `FUN_004b2340` does.
    /// `None` without a body node.
    #[must_use]
    pub fn from_c3d(scene: &C3dFile, low_detail: bool, model: u8) -> Option<Self> {
        let nodes: Vec<_> = scene.group0_records().collect();
        let records: Vec<_> = scene.group1_records().collect();
        let body = nodes.iter().position(|n| n.node_id() == BODY_NODE_ID)?;
        let children = nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.parent_index() == Some(body))
            .filter_map(|(node_index, node)| {
                let lods: Vec<usize> = node.values().map(usize::from).collect();
                let prims = |lod: usize| {
                    lods.get(lod)
                        .and_then(|&r| records.get(r))
                        .map(|&record| lod_prims(record))
                        .unwrap_or_default()
                };
                let mut part =
                    DamagePart::new(node.node_id(), *lods.first()?, prims(0), prims(1), prims(2))?;
                part.node_index = node_index;
                Some(part)
            })
            .collect();
        Some(Self::new(children, low_detail, model))
    }

    fn part_mut(&mut self, i: usize) -> Option<&mut DamagePart> {
        self.parts.get_mut(i).and_then(Option::as_mut)
    }

    /// `FUN_00420c70` at race start: the impact table zeroed.
    pub fn clear_impacts(&mut self) {
        self.impact_864 = [0; MAX_PARTS];
    }

    /// `FUN_0041f880(car, sev)`: dent the live meshes around the car-local
    /// contact `car +0x3f0`. At night (`0x41f7be`), if the front part
    /// deformation `S+0x788[S+0xcc1]` grew, runs [`Self::break_pod_lamp_44a770`].
    pub fn dent(&mut self, contact: [i32; 3], severity: i32) {
        let front_idx = usize::from(self.layout.handling_cc1[0]);
        let old_front_deform = self.deform_788.get(front_idx).copied().unwrap_or(0);
        let mut cnt = [0u16; 4];
        let mut g = [0i32; 4];
        let mut sp: isize = -1;
        let low_detail = self.low_detail;
        for i in 0..self.count {
            let special = self.special == Some(i);
            let mut all = 0i32;
            let mut moved = 0i32;
            let mut nverts = 0i32;
            let Some(part) = self.parts.get_mut(i).and_then(Option::as_mut) else {
                continue;
            };
            let chains = part.live.len().min(part.rest.len()).min(part.crush.len());
            for c in 0..chains {
                let count = part.live[c].positions.len();
                for k in 0..count {
                    let p = if special {
                        sp += 1;
                        self.special_list
                            .get(sp as usize)
                            .copied()
                            .unwrap_or_default()
                    } else {
                        part.live[c].positions[k]
                    };
                    let q = usize::from(p[0] < 0) * 2 + usize::from(p[2] < 0);
                    if low_detail {
                        cnt[q] = cnt[q].wrapping_add(2);
                    }
                    let d: [i32; 3] = std::array::from_fn(|a| p[a].wrapping_sub(contact[a]));
                    let r2 = d.iter().fold(0i32, |s, &v| s.wrapping_add(qmul(v, v)));
                    if r2 >= 0x1_0000 {
                        continue;
                    }
                    let w = qmul(0x1_0000 - qmul(r2, 0x1_0000), severity);
                    if w <= 0 {
                        continue;
                    }
                    all = all.wrapping_add(w);
                    if low_detail {
                        g[q] = g[q].wrapping_add(w);
                    }
                    let a = part.rest[c].positions.get(k).copied().unwrap_or_default();
                    let b = part.crush[c].positions.get(k).copied().unwrap_or_default();
                    if (0..3).all(|ax| b[ax].wrapping_sub(p[ax]).wrapping_abs() < 1) {
                        continue;
                    }
                    let dl: [i32; 3] = std::array::from_fn(|ax| qmul(b[ax].wrapping_sub(a[ax]), w));
                    let mut np: [i32; 3] = std::array::from_fn(|ax| p[ax].wrapping_add(dl[ax]));
                    for ax in 0..3 {
                        let over = (dl[ax] > 0 && np[ax] > b[ax]) || (dl[ax] < 0 && np[ax] < b[ax]);
                        if over {
                            np = b;
                        }
                    }
                    if special {
                        if let Some(slot) = self.special_list.get_mut(sp as usize) {
                            *slot = np;
                        }
                    } else {
                        part.live[c].positions[k] = np;
                    }
                    moved = moved.wrapping_add(w);
                }
                nverts = nverts.wrapping_add(count as i32);
            }
            let r = if nverts << 16 == 0 {
                -1
            } else {
                qdiv(0x1_0000, nverts << 16)
            };
            self.deform_788[i] = self.deform_788[i]
                .wrapping_add(qmul(moved, r))
                .min(0x1_0000);
            self.impact_864[i] = qmul(all, r);
        }
        if low_detail {
            for q in 0..4 {
                g[q] = if cnt[q] == 0 {
                    0
                } else {
                    qdiv(g[q], i32::from(cnt[q]) << 16)
                };
            }
            let l = self.layout.handling_cc1;
            for (index, value) in [
                (l[2], g[0] + g[2]),
                (l[3], g[1] + g[3]),
                (l[0], g[0] + g[1]),
                (l[1], g[2] + g[3]),
            ] {
                if let Some(slot) = self.impact_864.get_mut(usize::from(index)) {
                    *slot = value;
                }
            }
        }
        let new_front_deform = self.deform_788.get(front_idx).copied().unwrap_or(0);
        if self.night_stage && new_front_deform > old_front_deform {
            self.break_pod_lamp_44a770(contact);
        }
    }

    /// `FUN_0044a770(car)`: break the nearest intact night pod lamp when the
    /// front-part deformation crosses `0x3333` (first lamp) or `0x6666` (second lamp).
    pub fn break_pod_lamp_44a770(&mut self, contact: [i32; 3]) {
        let front_idx = usize::from(self.layout.handling_cc1[0]);
        let v = self.deform_788.get(front_idx).copied().unwrap_or(0);
        let eligible = (v > 0x3333 && self.broken_pod_count_cc0 == 0)
            || (v > 0x6666 && self.broken_pod_count_cc0 == 1);
        if !eligible {
            return;
        }
        let mut best = 0x2710_0000;
        let mut chosen = None;
        for (k, &o) in self.pod_lamp_offsets.iter().enumerate().take(6) {
            if self.broken_pod_lamps_ca8[k] {
                continue;
            }
            let d: [i32; 3] = std::array::from_fn(|ax| contact[ax].wrapping_sub(o[ax]));
            let sum = d.iter().fold(0i32, |s, &c| s.wrapping_add(qmul(c, c)));
            let dist = crate::legacy_math::sqrt_q16(sum as u32) as i32;
            if dist < best {
                best = dist;
                chosen = Some(k);
            }
        }
        if let Some(k) = chosen {
            self.broken_pod_count_cc0 = self.broken_pod_count_cc0.wrapping_add(1);
            self.broken_pod_lamps_ca8[k] = true;
        }
    }

    /// `FUN_00420120`: parts past their thresholds take the crushed UVs and
    /// their damaged material, once; the last such part sets the kind.
    pub fn detach(&mut self) -> Option<BreakageKind> {
        let mut kind = None;
        for i in 0..self.count {
            let threshold = self.layout.thresholds_c30.get(i).copied().unwrap_or(0);
            if self.special == Some(i) || self.detached_bb8[i] || self.deform_788[i] <= threshold {
                continue;
            }
            let Some(part) = self.part_mut(i) else {
                continue;
            };
            for (live, crush) in part.live.iter_mut().zip(&part.crush) {
                for (uv, crushed) in live.uvs.iter_mut().zip(&crush.uvs) {
                    *uv = *crushed;
                }
            }
            let to = DamagePart::chain_index(part.live.len(), part.material_prim);
            let from = DamagePart::chain_index(part.crush.len(), part.material_prim);
            if let (Some(to), Some(from)) = (to, from) {
                part.live[to].material = part.crush[from].material;
            }
            self.detached_bb8[i] = true;
            let layout = &self.layout;
            let i = i as u8;
            if i == layout.mirror_ccb {
                kind = Some(BreakageKind::Mirror);
            } else if layout.screens_cc7.contains(&i) {
                kind = Some(BreakageKind::Screen);
            } else if layout.lights_ccc.contains(&i) {
                kind = Some(BreakageKind::Headlight);
            }
        }
        kind
    }

    /// Write the live meshes back into the car's `.C3D` (positions, UVs and
    /// the swapped materials), which the renderer draws from.
    pub fn write_live(&self, scene: &mut C3dFile) {
        for part in self.parts.iter().flatten() {
            for prim in &part.live {
                for ((&entry, &position), &uv) in
                    prim.entries.iter().zip(&prim.positions).zip(&prim.uvs)
                {
                    scene.set_entry32_position_uv(
                        part.live_record,
                        usize::from(entry),
                        position,
                        uv,
                    );
                }
                for face in prim.faces.clone() {
                    scene.set_entry24_group2_ordinal(part.live_record, face, prim.material);
                }
            }
        }
    }
}

/// One LOD's prim runs: consecutive faces of one material, vertices
/// compacted per run (`FUN_004b2340`, as [`plan_consecutive_batches`]).
fn lod_prims(record: C3dGroup1Record<'_>) -> Vec<DamagePrim> {
    let selectors: Vec<i16> = record.entries_24().map(|f| f.group2_ordinal()).collect();
    let Ok(batches) = plan_consecutive_batches(record, &selectors) else {
        return Vec::new();
    };
    let mut first_face = 0;
    batches
        .iter()
        .map(|batch| {
            let faces = first_face..first_face + batch.entries_24().len();
            first_face = faces.end;
            DamagePrim {
                positions: batch
                    .unique_entries_32()
                    .iter()
                    .map(|e| e.position_q16())
                    .collect(),
                uvs: batch
                    .unique_entries_32()
                    .iter()
                    .map(|e| e.uv_q16())
                    .collect(),
                material: batch.selector(),
                entries: batch.unique_entry32_ordinals().to_vec(),
                faces,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HALF: i32 = 0x8000;
    const QUARTER: i32 = 0x4000;

    fn prim(points: &[[i32; 3]], u: i32, material: i16) -> DamagePrim {
        DamagePrim {
            positions: points.to_vec(),
            uvs: vec![[u, u]; points.len()],
            material,
            entries: (0..points.len() as u16).collect(),
            faces: 0..1,
        }
    }

    /// A one-prim part at x = 0.5 whose crushed shape sits 0.25 inward.
    fn part(id: u16, crushed_material: i16) -> DamagePart {
        let rest = [[HALF, 0, 0], [HALF, 0, HALF]];
        DamagePart::new(
            id,
            0,
            vec![prim(&rest, 0, 1)],
            vec![prim(
                &[[QUARTER, 0, 0], [QUARTER, 0, HALF]],
                HALF,
                crushed_material,
            )],
            vec![prim(&rest, 0, 1)],
        )
        .unwrap()
    }

    fn live(damage: &CarDamage, i: usize) -> &DamagePrim {
        &damage.parts[i].as_ref().unwrap().live[0]
    }

    #[test]
    fn a_hit_dents_the_near_vertices_toward_the_crushed_shape() {
        let mut damage = CarDamage::new(vec![part(6, 2)], false, 0);
        damage.dent([HALF, 0, 0], HALF);
        let positions = &live(&damage, 0).positions;
        // On the contact w = sev = 0.5: half way to the crushed x.
        assert_eq!(positions[0][0], 0x6000);
        // 0.5 away w = (1 - 0.25) * 0.5, so it moves less.
        assert!(positions[1][0] > 0x6000 && positions[1][0] < HALF);
        assert!(damage.deform_788[0] > 0);
        assert!(damage.impact_864[0] > 0);
        // A far hit moves nothing and leaves the intensity 0.
        let before = positions.clone();
        damage.dent([0x5_0000, 0, 0], 0x1_0000);
        assert_eq!(live(&damage, 0).positions, before);
        assert_eq!(damage.impact_864[0], 0);
    }

    #[test]
    fn a_dent_never_overshoots_the_crushed_shape() {
        let mut damage = CarDamage::new(vec![part(6, 2)], false, 0);
        for _ in 0..10 {
            damage.dent([HALF, 0, 0], 0x1_0000);
        }
        assert_eq!(live(&damage, 0).positions[0], [QUARTER, 0, 0]);
        assert!(damage.deform_788[0] <= 0x1_0000);
    }

    #[test]
    fn a_part_past_its_threshold_detaches_once_with_its_cue() {
        // Part 1 (node 7) is the mirror in high detail, threshold 0x3333.
        let mut damage = CarDamage::new(vec![part(6, 2), part(7, 9)], false, 0);
        damage.dent([HALF, 0, QUARTER], 0x1_0000);
        assert_eq!(damage.detach(), Some(BreakageKind::Mirror));
        assert_eq!(live(&damage, 1).material, 9);
        assert_eq!(live(&damage, 1).uvs[0], [HALF, HALF]);
        assert_eq!(damage.detach(), None, "latched");
        // Part 0's threshold is 0x8000.
        assert_eq!(damage.detached_bb8[0], damage.deform_788[0] > 0x8000);
    }

    #[test]
    fn parts_sit_at_their_ids_and_others_are_skipped() {
        let damage = CarDamage::new(
            vec![part(7, 2), part(5, 2), part(6, 2), part(0x70, 2)],
            false,
            0,
        );
        assert_eq!(damage.count, 2);
        assert_eq!(damage.parts[0].as_ref().unwrap().node_id, 6);
        assert_eq!(damage.parts[1].as_ref().unwrap().node_id, 7);
    }

    #[test]
    fn the_detach_material_is_the_first_prim_whose_ordinals_differ() {
        let p = |m| prim(&[[0, 0, 0]], 0, m);
        let part = DamagePart::new(
            6,
            0,
            vec![p(1), p(2), p(3)],
            vec![p(1), p(5), p(6)],
            vec![p(1), p(2), p(3)],
        )
        .unwrap();
        assert_eq!(part.material_prim, 1);
        let same =
            DamagePart::new(6, 0, vec![p(1), p(2)], vec![p(1), p(2)], vec![p(1), p(2)]).unwrap();
        assert_eq!(same.material_prim, 1, "the last when none differ");
        // The chains run in reversed order.
        assert_eq!(part.live[0].material, 3);
    }

    #[test]
    fn the_low_detail_layout_follows_the_model() {
        assert_eq!(DamageLayout::new(true, 7).screens_cc7, [1, 1, 2, 1]);
        assert_eq!(DamageLayout::new(true, 13).screens_cc7, [4, 4, 4, 2]);
        assert_eq!(DamageLayout::new(false, 7).lights_ccc, [8, 9]);
    }

    #[test]
    fn low_detail_impacts_split_into_the_car_halves() {
        let mut damage = CarDamage::new(vec![part(6, 2)], true, 0);
        damage.dent([HALF, 0, 0], 0x1_0000);
        // Both vertices are x >= 0, z >= 0: quadrant 0 feeds cc1 (7) and cc3 (5).
        assert!(damage.impact_864[7] > 0);
        assert!(damage.impact_864[5] > 0);
        assert_eq!(damage.impact_864[8], 0);
    }

    #[test]
    fn night_pod_lamps_break_nearest_first_above_thresholds() {
        // Front part is cc1 = 7 -> node_id = 6 + 7 = 13.
        let mut damage = CarDamage::new((6..=13).map(|id| part(id, 2)).collect(), false, 0);
        damage.set_night_pod_lamps(
            true,
            vec![[HALF, 0, -0x4000], [HALF, 0, 0x1000], [HALF, 0, 0x4000]],
        );
        // First hit pushes front deform_788[7] above 0x3333 -> breaks nearest pod lamp (index 1).
        damage.dent([HALF, 0, 0], 0x8000);
        assert!(damage.deform_788[7] > 0x3333);
        assert_eq!(damage.broken_pod_count_cc0, 1);
        assert_eq!(damage.broken_pod_lamps_ca8[..3], [false, true, false]);

        // Second hit pushes deform_788[7] above 0x6666 -> breaks next nearest pod lamp (index 0, tie keeps first).
        damage.dent([HALF, 0, 0], 0x1_0000);
        assert!(damage.deform_788[7] > 0x6666);
        assert_eq!(damage.broken_pod_count_cc0, 2);
        assert_eq!(damage.broken_pod_lamps_ca8[..3], [true, true, false]);
    }
}
