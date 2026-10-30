//! The stage's moving objects: knockable props (the rally school's cones,
//! signs, hay bales, boxes) taken from the stage `.obj` at the load
//! (`FUN_00482140`), bucketed in the terrain cell grid (`FUN_0043c010`,
//! `FUN_0043c410`), simulated each tick while hit or moving
//! (`FUN_00450d40`) and drawn through their node's local matrix
//! (`FUN_00439ee0`). The car's contact with them is
//! [`crate::moving_object_collision`].
//!
//! A `.obj` node is a moving object when its class has bit 29 clear and a
//! mass class `(class >> 16) & 3` (1, 2 or 3, mass index 2, 1, 0: 5000, 500
//! and 100), exactly the classes the static scenery skips. Objects rest
//! until a car hits them; a hit object integrates its impulses, falls with
//! gravity 0.04 a tick, slides on the ground without bouncing, damps by 0.8
//! a tick and settles onto one of its allowed faces, after which a light
//! object other than a cone stays where it lies (inert).
//!
//! A description of disassembly, not original source. Evidence:
//! `rea.official-eu.moving-objects`, `.work/moving-objects-notes.md`.

use cmr_formats::c3d::C3dFile;

use crate::c3d_camera::matmul_q16;
use crate::c3d_terrain_cull::CellGrid;
use crate::compiled_car_initializer::normalize_compiled_lane_vector;
use crate::fixed_point::{Fixed16, Vec3};
use crate::legacy_math::{signed_ratio_angle, sqrt_q16};

/// One block's object capacity (`0x1f4`).
pub const MAX_OBJECTS: usize = 500;
/// `0x4f14f4`: the masses by mass index (100, 500, 5000).
pub const MASS: [i32; 3] = [100 << 16, 500 << 16, 5000 << 16];
/// The objects re-bucketed per tick at most (`0x552c08`, 50).
pub const MAX_ACTIVE: usize = 50;
/// The cone material (plastic).
pub const CONE_MATERIAL: u8 = 0xd;

/// Object states (`+0x96` bits 3..5).
pub mod state {
    pub const REST: u8 = 0;
    pub const HIT: u8 = 1;
    pub const MOVING: u8 = 2;
    pub const INERT: u8 = 3;
}

const AIRBORNE: u8 = 4;
const ROTATING: u8 = 0x40;
const TRANSLATING: u8 = 0x80;

pub(crate) fn q(a: i32, b: i32) -> i32 {
    ((i64::from(a) * i64::from(b)) >> 16) as i32
}

pub(crate) fn qdiv(a: i32, b: i32) -> i32 {
    ((i64::from(a) << 16) / i64::from(b)) as i32
}

pub(crate) fn add(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    std::array::from_fn(|k| a[k].wrapping_add(b[k]))
}

pub(crate) fn sub(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    std::array::from_fn(|k| a[k].wrapping_sub(b[k]))
}

pub(crate) fn scale(v: [i32; 3], s: i32) -> [i32; 3] {
    v.map(|c| q(c, s))
}

pub(crate) fn dot(a: [i32; 3], b: [i32; 3]) -> i32 {
    q(a[0], b[0])
        .wrapping_add(q(a[1], b[1]))
        .wrapping_add(q(a[2], b[2]))
}

pub(crate) fn cross(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [
        q(a[1], b[2]).wrapping_sub(q(a[2], b[1])),
        q(a[2], b[0]).wrapping_sub(q(a[0], b[2])),
        q(a[0], b[1]).wrapping_sub(q(a[1], b[0])),
    ]
}

pub(crate) fn length(v: [i32; 3]) -> i32 {
    sqrt_q16(dot(v, v) as u32) as i32
}

pub(crate) fn norm(v: [i32; 3]) -> [i32; 3] {
    normalize_compiled_lane_vector(v)
}

/// `FUN_0048e100`: the rows' dots (world to body).
pub(crate) fn rows_dot(rows: &[[i32; 3]; 3], v: [i32; 3]) -> [i32; 3] {
    rows.map(|r| dot(r, v))
}

/// `FUN_0048dfe0`: `row0 * x + row1 * y + row2 * z` (body to world).
pub(crate) fn rows_combine(rows: &[[i32; 3]; 3], v: [i32; 3]) -> [i32; 3] {
    add(
        add(scale(rows[0], v[0]), scale(rows[1], v[1])),
        scale(rows[2], v[2]),
    )
}

fn vec3(v: [i32; 3]) -> Vec3 {
    Vec3::new(
        Fixed16::from_raw(v[0]),
        Fixed16::from_raw(v[1]),
        Fixed16::from_raw(v[2]),
    )
}

fn raw(v: Vec3) -> [i32; 3] {
    [v.x.raw(), v.y.raw(), v.z.raw()]
}

/// `FUN_0043ce70`: the eight corners in native order.
pub(crate) fn corners(position: [i32; 3], rows: &[[i32; 3]; 3], half: [i32; 3]) -> [[i32; 3]; 8] {
    crate::static_world_collision::build_static_obb_points(
        vec3(position),
        rows.map(vec3),
        vec3(half),
    )
    .map(raw)
}

/// One moving object (`0x703e58 + i * 0xa0`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovingObject {
    /// The `.obj` group-0 record whose node draws it.
    pub node: usize,
    /// `+0x04` centre, `+0x10` half extents, `+0x1c` velocity, `+0x28`
    /// angular velocity (body), `+0x34` pending torque (body), `+0x40`
    /// pending impulse (world), `+0x4c` inverse inertia.
    pub position: [i32; 3],
    pub half: [i32; 3],
    pub velocity: [i32; 3],
    pub angular: [i32; 3],
    pub torque: [i32; 3],
    pub impulse: [i32; 3],
    pub inverse_inertia: [i32; 3],
    /// `+0x58/+0x64/+0x70`: the orientation rows; `+0x7c` the drawn
    /// origin (centre less `row1 * hy`).
    pub rows: [[i32; 3]; 3],
    pub base: [i32; 3],
    /// `+0x88` bounding-sphere radius, `+0x8c` ground height cache.
    pub radius: i32,
    pub ground_height: i32,
    /// `+0x90`: `index << 5 | material`; `+0x92` the ground triangle cache.
    pub id_material: u16,
    pub ground_handle: i16,
    /// `+0x94`: the grid cell.
    pub cell: i32,
    /// `+0x96`: mass index (bits 0-1), airborne (2), state (3-5),
    /// rotating (6), translating (7); `+0x97` the allowed rest axes.
    pub flags: u8,
    pub rest_axes: u8,
    /// `node +0x30..+0x38`: the authored local matrix's row lengths.
    pub row_scale: [i32; 3],
    /// The last ground normal (the probe's stack local when no triangle
    /// is found).
    pub(crate) ground_normal: [i32; 3],
    pub(crate) next: Option<usize>,
    pub(crate) prev: Option<usize>,
}

impl MovingObject {
    #[must_use]
    pub const fn state(&self) -> u8 {
        (self.flags >> 3) & 7
    }

    fn set_state(&mut self, state: u8) {
        self.flags = (self.flags & 0xc7) | (state << 3);
    }

    #[must_use]
    pub const fn mass_index(&self) -> usize {
        (self.flags & 3) as usize
    }

    #[must_use]
    pub const fn material(&self) -> u8 {
        (self.id_material & 0x1f) as u8
    }

    /// A cone: plastic, allowed to rest on its base (`rest & 2`).
    #[must_use]
    pub const fn is_cone(&self) -> bool {
        self.material() == CONE_MATERIAL && self.rest_axes & 2 != 0
    }

    #[must_use]
    pub const fn airborne(&self) -> bool {
        self.flags & AIRBORNE != 0
    }

    /// `FUN_00439ee0`: the node's local matrix, each row scaled component
    /// by component by the authored row lengths, with the base point.
    #[must_use]
    pub fn node_local(&self) -> [i32; 16] {
        let mut m = [0; 16];
        for (r, row) in self.rows.iter().enumerate() {
            for c in 0..3 {
                m[r * 4 + c] = q(row[c], self.row_scale[c]);
            }
        }
        m[12..15].copy_from_slice(&self.base);
        m[15] = 0x1_0000;
        m
    }

    /// Hit by a car (`FUN_0043f790`'s `state = 1`).
    pub fn mark_hit(&mut self) {
        self.set_state(state::HIT);
    }
}

/// The ground under a point (`FUN_00446c40` with its triangle and height
/// caches): the height and, when a triangle was found, its normal.
pub trait ObjectGround {
    fn query(&mut self, point: [i32; 3], handle: &mut i16, height: i32) -> (i32, Option<[i32; 3]>);
}

/// A flat ground at a fixed height (tests and stages without ground data).
#[derive(Clone, Copy, Debug)]
pub struct FlatGround(pub i32);

impl ObjectGround for FlatGround {
    fn query(&mut self, _: [i32; 3], _: &mut i16, _: i32) -> (i32, Option<[i32; 3]>) {
        (self.0, Some([0, 0x1_0000, 0]))
    }
}

/// The stage's ground through the compiled lane provider.
pub struct CompiledObjectGround<P>(pub P);

impl<P: crate::compiled_car_initializer::CompiledLaneGeometryProvider> ObjectGround
    for CompiledObjectGround<P>
{
    fn query(&mut self, point: [i32; 3], handle: &mut i16, height: i32) -> (i32, Option<[i32; 3]>) {
        let input = crate::compiled_car_initializer::CompiledLaneDispatchInput {
            source_vector: point,
            initial_handle: *handle,
            fallback: height,
        };
        let Ok(done) = crate::compiled_car_initializer::dispatch_compiled_lane(input, &mut self.0)
        else {
            return (height, None);
        };
        let writes = done.handle_writes;
        if writes.normalized_to_minus_one || writes.invalidated_after_search_miss {
            *handle = -1;
        }
        if let Some(found) = writes.searched_handle {
            *handle = found;
        }
        (done.lane_word, done.output_vector_write)
    }
}

/// The stage's moving objects, their cell lists and the per-tick active list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovingObjects {
    pub objects: Vec<MovingObject>,
    /// `0x551b90`/`0x556fac`: the cells that hold objects and their list heads.
    cells: Vec<(i32, Option<usize>)>,
    /// `0x552c08`: the objects simulated last tick, re-bucketed next tick.
    active: Vec<usize>,
    /// `0x55d7d8`: the update's scratch rows. A sliding object that does
    /// not rotate reuses the rows of the last object loaded into it.
    scratch_rows: [[i32; 3]; 3],
}

impl MovingObjects {
    /// `FUN_00482140`, then `FUN_0043c010`: the moving objects of a stage
    /// `.obj` in record order, at most [`MAX_OBJECTS`], and their cells.
    #[must_use]
    pub fn load(objects: &C3dFile, grid: &CellGrid) -> Self {
        let records: Vec<_> = objects.group0_records().collect();
        let meshes: Vec<_> = objects.group1_records().collect();
        let mut out = Vec::new();
        for (k, node) in records.iter().enumerate() {
            if out.len() == MAX_OBJECTS {
                break;
            }
            let class = node.node_class();
            let mass_class = (class >> 16) & 3;
            if class & 0x2000_0000 != 0 || mass_class == 0 {
                continue;
            }
            let local = node.affine_matrix_q16();
            let world = match node.parent_index() {
                Some(p) => matmul_q16(&records[p].affine_matrix_q16(), &local),
                None => local,
            };
            let rows_raw = [
                [local[0], local[1], local[2]],
                [local[4], local[5], local[6]],
                [local[8], local[9], local[10]],
            ];
            let Some(bounds) = node
                .values()
                .next()
                .and_then(|r| meshes.get(usize::from(r)))
                .map(|m| m.collision_bounds_q16())
            else {
                continue;
            };
            let row_scale = rows_raw.map(length);
            let e: [i32; 3] = std::array::from_fn(|i| q(bounds[i], row_scale[i]));
            if e.contains(&0) {
                continue;
            }
            let rows = rows_raw.map(norm);
            let half = e.map(|v| v / 2);
            let position = add([world[12], world[13], world[14]], scale(rows[1], half[1]));
            let (index, mass_scale) = match mass_class {
                1 => (2u8, 0x1_0000),
                2 => (1, 5 << 16),
                _ => (0, 0x1_0000),
            };
            let mp = qdiv(MASS[usize::from(index)], mass_scale);
            let limit = if mp <= 15 << 16 {
                0x7fbc_0000
            } else {
                q(qdiv(0x7fbc_0000, mp), 0xc_0000)
            };
            let inertia = |a: i32, b: i32| {
                let s = q(a, a).wrapping_add(q(b, b));
                if s > limit {
                    return 0;
                }
                let i = q(s, qdiv(mp, 12 << 16));
                if i == 0 { -1 } else { qdiv(0x1_0000, i) }
            };
            let rest_axes = (0..6).fold(0u8, |bits, i| {
                bits | ((((class >> (23 - i)) & 1) as u8) << i)
            });
            let mut material = (class & 0x1f) as u16;
            if material >= 0x12 {
                material = 0;
            }
            out.push(MovingObject {
                node: k,
                position,
                half,
                velocity: [0; 3],
                angular: [0; 3],
                torque: [0; 3],
                impulse: [0; 3],
                inverse_inertia: [
                    inertia(e[2], e[1]),
                    inertia(e[2], e[0]),
                    inertia(e[0], e[1]),
                ],
                rows,
                base: [local[12], local[13], local[14]],
                radius: length(scale(e, 0x8000)),
                ground_height: 0,
                id_material: ((out.len() as u16) << 5) | material,
                ground_handle: 0,
                cell: 0,
                flags: index,
                rest_axes,
                row_scale,
                ground_normal: [0, 0x1_0000, 0],
                next: None,
                prev: None,
            });
        }
        let mut objects = Self {
            objects: out,
            cells: Vec::new(),
            active: Vec::new(),
            scratch_rows: [[0; 3]; 3],
        };
        objects.build_cells(grid);
        objects
    }

    #[cfg(test)]
    pub(crate) fn from_objects_for_test(objects: Vec<MovingObject>, grid: &CellGrid) -> Self {
        let mut set = Self {
            objects,
            cells: Vec::new(),
            active: Vec::new(),
            scratch_rows: [[0; 3]; 3],
        };
        set.build_cells(grid);
        set
    }

    /// `FUN_0043c010`: every object appended at the tail of its cell's list.
    fn build_cells(&mut self, grid: &CellGrid) {
        self.cells.clear();
        for i in 0..self.objects.len() {
            let cell = grid.cell_of_point(self.objects[i].position[0], self.objects[i].position[2]);
            let o = &mut self.objects[i];
            o.next = None;
            o.prev = None;
            o.cell = cell;
            match self.cells.iter().position(|&(c, _)| c == cell) {
                Some(slot) => {
                    let mut tail = self.cells[slot].1.expect("a listed cell has a head");
                    while let Some(next) = self.objects[tail].next {
                        tail = next;
                    }
                    self.objects[tail].next = Some(i);
                    self.objects[i].prev = Some(tail);
                }
                None => self.cells.push((cell, Some(i))),
            }
        }
    }

    /// `FUN_0043c410`: last tick's moved objects into their new cells (at
    /// the head of the list).
    pub fn rebucket(&mut self, grid: &CellGrid) {
        for i in std::mem::take(&mut self.active) {
            let p = self.objects[i].position;
            let cell = grid.cell_of_point(p[0], p[2]);
            if cell == self.objects[i].cell {
                continue;
            }
            let old = self.objects[i].cell;
            self.objects[i].cell = cell;
            let (next, prev) = (self.objects[i].next, self.objects[i].prev);
            if let Some(prev) = prev {
                self.objects[prev].next = next;
            }
            if let Some(next) = next {
                self.objects[next].prev = prev;
            }
            if prev.is_none()
                && let Some(slot) = self
                    .cells
                    .iter_mut()
                    .find(|(c, head)| *c == old && *head == Some(i))
            {
                slot.1 = next;
            }
            self.objects[i].next = None;
            self.objects[i].prev = None;
            match self.cells.iter().position(|&(c, _)| c == cell) {
                Some(slot) => {
                    let head = self.cells[slot].1;
                    self.objects[i].next = head;
                    if let Some(head) = head {
                        self.objects[head].prev = Some(i);
                    }
                    self.cells[slot].1 = Some(i);
                }
                None => self.cells.push((cell, Some(i))),
            }
        }
    }

    /// The objects listed in a cell, in list order.
    pub fn in_cell(&self, cell: i32) -> impl Iterator<Item = usize> + '_ {
        let head = self
            .cells
            .iter()
            .find(|&&(c, _)| c == cell)
            .and_then(|&(_, head)| head);
        std::iter::successors(head, |&i| self.objects[i].next)
    }

    /// `FUN_00450d40`: one tick of every hit or moving object. `dt` is the
    /// solver step `DAT_004eac6c`.
    pub fn update(&mut self, ground: &mut impl ObjectGround, dt: i32, trig: &[i32]) {
        self.active.clear();
        for i in 0..self.objects.len() {
            let st = self.objects[i].state();
            if st == state::REST || st == state::INERT {
                self.scratch_rows = self.objects[i].rows;
                continue;
            }
            if !matches!(st, 1 | 2 | 4 | 5) {
                continue;
            }
            let st = self.step(i, st, ground, dt, trig);
            if st == 5 {
                // `0x452640`: no writer of states 4/5 exists.
                continue;
            }
            if self.active.len() < MAX_ACTIVE {
                self.active.push(i);
            }
        }
    }

    /// One object's tick; returns the state the tail sees.
    fn step(
        &mut self,
        i: usize,
        mut st: u8,
        ground: &mut impl ObjectGround,
        dt: i32,
        trig: &[i32],
    ) -> u8 {
        let mut hit_now = false;
        let mut a = [0; 3];
        let o = &mut self.objects[i];
        if st == state::HIT || st == 4 {
            a = o.impulse.map(|c| q(q(c, 0x3333), dt));
            o.velocity = add(o.velocity, a);
            for k in 0..3 {
                let t = q(o.torque[k], o.inverse_inertia[k]).wrapping_neg();
                o.angular[k] = o.angular[k].wrapping_add(q(t, dt));
            }
            o.impulse = [0; 3];
            o.torque = [0; 3];
            st = if st == 4 { 5 } else { state::MOVING };
            o.set_state(st);
            o.flags |= ROTATING | TRANSLATING;
            hit_now = true;
        }
        let mut rotated = false;
        let mut base = o.base;
        if o.flags & TRANSLATING != 0 {
            let d = sub(o.velocity.map(|c| q(c, dt)), a.map(|c| q(c, dt / 2)));
            base = add(base, d);
            o.position = add(o.position, d);
        }
        if o.flags & ROTATING != 0 {
            let angles = o.angular.map(|w| q(q(w, dt), 0x28b) as i16);
            if let Some(rows) =
                crate::vehicle_dynamics::integrate_orientation(o.rows.map(vec3), angles, trig)
            {
                o.rows = rows.map(raw);
            }
            self.scratch_rows = o.rows;
            rotated = true;
        }
        let rows = self.scratch_rows;
        let mut c = corners(o.position, &rows, o.half);
        let (depth, n) = probe(o, &rows, &c, ground);
        let mut settled = false;
        'tail: {
            if depth < -0x1999 {
                o.flags |= AIRBORNE | TRANSLATING;
                o.velocity[1] = o.velocity[1].wrapping_sub(q(0xa3d, dt));
                break 'tail;
            }
            o.flags &= !AIRBORNE;
            if o.mass_index() != 0 || o.is_cone() {
                let t = rows_dot(&rows, n);
                o.angular = scale(t, dot(o.angular, t));
            }
            o.position[1] = o.position[1].wrapping_add(depth);
            base[1] = base[1].wrapping_add(depth);
            for corner in &mut c {
                corner[1] = corner[1].wrapping_add(depth);
            }
            let vn = dot(n, o.velocity);
            if vn < 0 {
                o.velocity = sub(o.velocity, scale(n, vn));
            }
            if hit_now {
                break 'tail;
            }
            if o.flags & TRANSLATING != 0 {
                o.velocity = o.velocity.map(|v| q(v, 0xcccc));
                if length(o.velocity) < 0x41 {
                    o.velocity = [0; 3];
                    o.flags &= !TRANSLATING;
                }
            }
            let mut w = 0;
            if o.flags & ROTATING != 0 {
                o.angular = o.angular.map(|v| q(v, 0xcccc));
                w = length(o.angular);
                if w < 0x28f {
                    o.angular = [0; 3];
                    o.flags &= !ROTATING;
                    w = 0;
                }
            }
            if o.mass_index() == 0 && w > 0 {
                let spin = rows_combine(&rows, o.angular);
                if length(sub(spin, scale(n, dot(n, spin)))) >= 0x28f {
                    break 'tail;
                }
            }
            settled = true;
            let choice = settle(o, n, trig);
            self.scratch_rows = o.rows;
            base = settle_height(o, &choice, base, ground);
        }
        if rotated || settled {
            let rows = self.scratch_rows;
            base = sub(o.position, scale(rows[1], o.half[1]));
        }
        o.base = base;
        st = o.state();
        st
    }

    /// The node locals to draw the objects with (`FUN_00439ee0`), as
    /// `(group-0 record, matrix)`.
    pub fn node_locals(&self) -> impl Iterator<Item = (usize, [i32; 16])> + '_ {
        self.objects.iter().map(|o| (o.node, o.node_local()))
    }
}

/// `FUN_004508b0`: the ground under the object and its penetration depth
/// along the ground normal (positive = into the ground).
fn probe(
    o: &mut MovingObject,
    rows: &[[i32; 3]; 3],
    c: &[[i32; 3]; 8],
    ground: &mut impl ObjectGround,
) -> (i32, [i32; 3]) {
    let (h, normal) = ground.query(o.position, &mut o.ground_handle, o.ground_height);
    o.ground_height = h;
    if let Some(normal) = normal {
        o.ground_normal = normal;
    }
    let n = norm(o.ground_normal);
    let p = if o.is_cone() {
        let mut best = 10_000 << 16;
        let mut point = c[0];
        for corner in &c[..4] {
            let d = dot(sub(*corner, o.position), n);
            if d < best {
                best = d;
                point = *corner;
            }
        }
        let apex: [i32; 3] = std::array::from_fn(|k| c[4][k].wrapping_add(c[7][k]) / 2);
        if dot(sub(apex, o.position), n) < best {
            point = apex;
        }
        point
    } else {
        let d = rows_dot(rows, n.map(i32::wrapping_neg));
        c[usize::from(d[1] >= 0) * 4 + usize::from(d[0] < 0) * 2 + usize::from(d[2] < 0)]
    };
    let depth = if n[1] == 0 {
        -1
    } else {
        qdiv(
            dot(
                [
                    o.position[0].wrapping_sub(p[0]),
                    h.wrapping_sub(p[1]),
                    o.position[2].wrapping_sub(p[2]),
                ],
                n,
            ),
            n[1],
        )
    };
    (depth, n)
}

/// The settle's chosen axis and whether it snapped onto its target.
#[derive(Clone, Copy)]
struct SettleChoice {
    best: usize,
    target: [i32; 3],
    snapped: bool,
}

/// The settle's rest-pose alignment (`0x45182d`): the allowed axis closest
/// to the ground normal (a cone's tilted target) eased toward it by 0.2.
fn settle(o: &mut MovingObject, n: [i32; 3], trig: &[i32]) -> SettleChoice {
    let fpos = |i: usize| o.rest_axes >> i & 1 != 0;
    let fneg = |i: usize| o.rest_axes >> (3 + i) & 1 != 0;
    let cone = o.is_cone();
    let mut best: Option<usize> = None;
    let mut bd = 0i32;
    let target = if cone {
        let a = signed_ratio_angle(o.half[1].wrapping_mul(2), o.half[0]).unwrap_or(0);
        let axis = norm(cross(n, o.rows[1]));
        let tilt =
            crate::vehicle_dynamics::axis_angle_matrix(vec3(axis), 0x400i16.wrapping_sub(a), trig)
                .unwrap_or(crate::c3d_camera::MATRIX_IDENTITY_Q16);
        let m = [
            [tilt[0], tilt[1], tilt[2]],
            [tilt[4], tilt[5], tilt[6]],
            [tilt[8], tilt[9], tilt[10]],
        ];
        let t = norm(rows_combine(&m, n));
        for i in 0..3 {
            if !fpos(i) {
                continue;
            }
            let dd = if i == 1 {
                dot(o.rows[1], n)
            } else {
                dot(o.rows[i], t)
            };
            if dd < 0 && !fneg(i) {
                continue;
            }
            if dd.wrapping_abs() > bd.wrapping_abs() {
                best = Some(i);
                bd = dd;
            }
        }
        if matches!(best, Some(1) | None) { n } else { t }
    } else {
        for i in 0..3 {
            if !fpos(i) {
                continue;
            }
            let dd = dot(o.rows[i], n);
            if dd < 0 && !fneg(i) {
                continue;
            }
            if dd.wrapping_abs() > bd.wrapping_abs() {
                best = Some(i);
                bd = dd;
            }
        }
        n
    };
    let best = best.unwrap_or(1);
    let (i1, i2) = ((best + 1) % 3, (best + 2) % 3);
    let g = if bd > 0 {
        target
    } else {
        target.map(i32::wrapping_neg)
    };
    let mut r = o.rows[best];
    let snapped = length(sub(g, r)) < 0x28f;
    if snapped {
        r = g;
    } else {
        r = norm(add(r, scale(sub(g, r), 0x3333)));
    }
    let r1 = norm(sub(o.rows[i1], scale(r, dot(r, o.rows[i1]))));
    let r2 = norm(cross(r, r1));
    o.rows[best] = r;
    o.rows[i1] = r1;
    o.rows[i2] = r2;
    SettleChoice {
        best,
        target,
        snapped,
    }
}

/// The settle's height correction and rest decision.
fn settle_height(
    o: &mut MovingObject,
    choice: &SettleChoice,
    mut base: [i32; 3],
    ground: &mut impl ObjectGround,
) -> [i32; 3] {
    let SettleChoice {
        best,
        target,
        snapped,
    } = *choice;
    let rows = o.rows;
    let cone = o.is_cone();
    let dy = if snapped && !cone {
        let hb = o.half[best];
        let lift = if target[1] == 0 {
            -1
        } else {
            qdiv(hb, target[1])
        };
        lift.wrapping_add(o.ground_height.wrapping_sub(o.position[1]))
    } else {
        let c = corners(o.position, &rows, o.half);
        probe(o, &rows, &c, ground).0
    };
    o.position[1] = o.position[1].wrapping_add(dy);
    base[1] = base[1].wrapping_add(dy);
    if o.flags & (ROTATING | TRANSLATING) == 0 && snapped {
        let rest = if o.mass_index() != 0 || cone {
            state::REST
        } else {
            state::INERT
        };
        o.set_state(rest);
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: i32 = 0x1_0000;

    fn object(material: u16, rest_axes: u8, mass_index: u8) -> MovingObject {
        MovingObject {
            node: 3,
            position: [0, ONE / 2, 0],
            half: [ONE / 4, ONE / 2, ONE / 4],
            velocity: [0; 3],
            angular: [0; 3],
            torque: [0; 3],
            impulse: [0; 3],
            inverse_inertia: [ONE; 3],
            rows: [[ONE, 0, 0], [0, ONE, 0], [0, 0, ONE]],
            base: [0; 3],
            radius: ONE,
            ground_height: 0,
            id_material: material,
            ground_handle: 0,
            cell: 0,
            flags: mass_index,
            rest_axes,
            row_scale: [ONE; 3],
            ground_normal: [0, ONE, 0],
            next: None,
            prev: None,
        }
    }

    fn set(objects: Vec<MovingObject>) -> MovingObjects {
        MovingObjects {
            objects,
            cells: Vec::new(),
            active: Vec::new(),
            scratch_rows: [[0; 3]; 3],
        }
    }

    fn trig() -> Vec<i32> {
        crate::fixed_point::build_sine_table().to_vec()
    }

    #[test]
    fn a_pushed_cone_slides_damps_and_comes_to_rest_upright() {
        // A cone resting on its base (allowed along +y).
        let mut cone = object(u16::from(CONE_MATERIAL), 0b010, 0);
        cone.impulse = [2 * ONE, 0, 0];
        cone.mark_hit();
        let mut objects = set(vec![cone]);
        let trig = trig();
        objects.update(&mut FlatGround(0), ONE, &trig);
        let o = &objects.objects[0];
        assert_eq!(o.state(), state::MOVING);
        assert!(o.velocity[0] > 0, "0.2 of the impulse");
        for _ in 0..200 {
            objects.update(&mut FlatGround(0), ONE, &trig);
        }
        let o = &objects.objects[0];
        assert_eq!(o.state(), state::REST, "settled");
        assert!(o.position[0] > 0, "it slid");
        // Upright, to the table normalize's precision.
        assert!(o.rows[1][1] > 0xfff0 && o.rows[1][0] == 0 && o.rows[1][2] == 0);
        assert!(
            (o.position[1] - ONE / 2).abs() < 0x100,
            "standing on the ground"
        );
    }

    #[test]
    fn a_light_prop_that_is_not_a_cone_lies_inert_once_settled() {
        let mut prop = object(1, 0b010, 0);
        prop.impulse = [ONE, 0, 0];
        prop.mark_hit();
        let mut objects = set(vec![prop]);
        let trig = trig();
        for _ in 0..200 {
            objects.update(&mut FlatGround(0), ONE, &trig);
        }
        assert_eq!(objects.objects[0].state(), state::INERT);
    }

    #[test]
    fn an_object_off_the_ground_falls_at_four_hundredths_a_tick() {
        let mut crate_box = object(1, 0b010, 2);
        crate_box.position[1] = 5 * ONE;
        crate_box.mark_hit();
        let mut objects = set(vec![crate_box]);
        let trig = trig();
        objects.update(&mut FlatGround(0), ONE, &trig);
        objects.update(&mut FlatGround(0), ONE, &trig);
        let o = &objects.objects[0];
        assert!(o.airborne());
        assert_eq!(o.velocity[1], -2 * 0xa3d);
    }

    #[test]
    fn resting_objects_are_not_simulated() {
        let mut objects = set(vec![object(1, 0b010, 2)]);
        let before = objects.objects[0].clone();
        objects.update(&mut FlatGround(-ONE), ONE, &trig());
        assert_eq!(objects.objects[0], before);
        assert!(objects.active.is_empty());
    }

    #[test]
    fn a_moved_object_changes_cell_at_the_next_tick() {
        let grid = CellGrid {
            width: 4,
            height: 4,
            cell_count: 16,
            cell_size: 0x8_0000,
            min_x: 0,
            half_cell: 0x4_0000,
            max_z: 0x10_0000,
        };
        let mut a = object(1, 0b010, 2);
        let mut b = object(1, 0b010, 2);
        b.position = [0x9_0000, ONE / 2, 0];
        a.id_material = 1;
        b.id_material = (1 << 5) | 1;
        let mut objects = set(vec![a, b]);
        objects.build_cells(&grid);
        let cell_a = objects.objects[0].cell;
        let cell_b = objects.objects[1].cell;
        assert_ne!(cell_a, cell_b);
        assert_eq!(objects.in_cell(cell_a).collect::<Vec<_>>(), [0]);
        // Object 0 is pushed into object 1's cell and re-bucketed.
        objects.objects[0].position[0] = 0x9_8000;
        objects.active = vec![0];
        objects.rebucket(&grid);
        assert_eq!(objects.objects[0].cell, cell_b);
        assert_eq!(
            objects.in_cell(cell_b).collect::<Vec<_>>(),
            [0, 1],
            "pushed at the head"
        );
        assert_eq!(objects.in_cell(cell_a).count(), 0);
    }

    #[test]
    fn the_node_matrix_scales_by_column_and_carries_the_base() {
        let mut o = object(1, 0b010, 2);
        o.row_scale = [2 * ONE, ONE, ONE];
        o.base = [7, 8, 9];
        let m = o.node_local();
        assert_eq!(&m[0..3], &[2 * ONE, 0, 0]);
        assert_eq!(&m[4..7], &[0, ONE, 0]);
        assert_eq!(&m[12..16], &[7, 8, 9, ONE]);
    }
}
