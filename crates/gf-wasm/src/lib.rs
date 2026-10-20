use gf_geom::placement::angle_weighted_normals;

pub struct Kernel {
    vertices: usize,
    triangles: Vec<[u32; 3]>,
    offsets: Vec<u32>,
    index: Vec<u32>,
    delta: Vec<i16>,
    scale: Vec<f32>,
    weights: Vec<f32>,
    anchor: Vec<f32>,
    extra: Vec<f32>,
    reference: Vec<f32>,
    out: Vec<f32>,
    normals: Vec<f32>,
    work: Vec<[f64; 3]>,
}

impl Kernel {
    pub fn new(vertices: usize, triangles: Vec<[u32; 3]>, offsets: Vec<u32>, index: Vec<u32>, delta: Vec<i16>, scale: Vec<f32>) -> Result<Kernel, String> {
        let targets = scale.len();
        if offsets.len() != targets + 1 || offsets[0] != 0 {
            return Err(format!("offsets must be {} values starting at 0", targets + 1));
        }
        if offsets.windows(2).any(|w| w[1] < w[0]) || offsets[targets] as usize != index.len() || delta.len() != index.len() * 3 {
            return Err("offsets, index and delta lengths disagree".into());
        }
        if index.iter().any(|&i| i as usize >= vertices) || triangles.iter().flatten().any(|&i| i as usize >= vertices) {
            return Err("an index is outside the vertex range".into());
        }
        let n = vertices * 3;
        Ok(Kernel {
            vertices,
            triangles,
            offsets,
            index,
            delta,
            scale,
            weights: vec![0.0; targets],
            anchor: vec![0.0; n],
            extra: vec![0.0; n],
            reference: vec![0.0; n],
            out: vec![0.0; n],
            normals: vec![0.0; n],
            work: vec![[0.0; 3]; vertices],
        })
    }

    pub fn weights(&mut self) -> &mut [f32] {
        &mut self.weights
    }

    pub fn anchor(&mut self) -> &mut [f32] {
        &mut self.anchor
    }

    pub fn extra(&mut self) -> &mut [f32] {
        &mut self.extra
    }

    pub fn reference(&mut self) -> &mut [f32] {
        &mut self.reference
    }

    pub fn positions(&self) -> &[f32] {
        &self.out
    }

    pub fn normals(&self) -> &[f32] {
        &self.normals
    }

    pub fn preview(&mut self, with_extra: bool) -> usize {
        self.out.copy_from_slice(&self.anchor);
        if with_extra {
            for (o, e) in self.out.iter_mut().zip(&self.extra) {
                *o += *e;
            }
        }
        let mut used = 0;
        for t in 0..self.scale.len() {
            let w = self.weights[t];
            if w == 0.0 {
                continue;
            }
            used += 1;
            let s = w * self.scale[t];
            let (a, b) = (self.offsets[t] as usize, self.offsets[t + 1] as usize);
            for e in a..b {
                let v = self.index[e] as usize * 3;
                let d = &self.delta[e * 3..e * 3 + 3];
                self.out[v] += s * d[0] as f32;
                self.out[v + 1] += s * d[1] as f32;
                self.out[v + 2] += s * d[2] as f32;
            }
        }
        for (g, w) in self.work.iter_mut().enumerate() {
            *w = [self.out[g * 3] as f64, self.out[g * 3 + 1] as f64, self.out[g * 3 + 2] as f64];
        }
        let normals = angle_weighted_normals(&self.work, &self.triangles);
        for (g, n) in normals.iter().enumerate() {
            self.normals[g * 3] = n[0] as f32;
            self.normals[g * 3 + 1] = n[1] as f32;
            self.normals[g * 3 + 2] = n[2] as f32;
        }
        used
    }

    pub fn flips_against(&self, other: &[f32]) -> usize {
        let at = |p: &[f32], i: u32| [p[i as usize * 3] as f64, p[i as usize * 3 + 1] as f64, p[i as usize * 3 + 2] as f64];
        let face = |p: &[f32], t: &[u32; 3]| {
            let (a, b, c) = (at(p, t[0]), at(p, t[1]), at(p, t[2]));
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
        };
        self.triangles
            .iter()
            .filter(|t| {
                let (m, n) = (face(&self.out, t), face(other, t));
                m[0] * n[0] + m[1] * n[1] + m[2] * n[2] < 0.0
            })
            .count()
    }

    pub fn vertices(&self) -> usize {
        self.vertices
    }
}

fn copy<T: Copy>(ptr: *const T, len: usize) -> Vec<T> {
    if len == 0 {
        return Vec::new();
    }
    unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
}

/// # Safety
/// The returned buffer is freed with `gf_free` and the same length.
#[unsafe(no_mangle)]
pub extern "C" fn gf_alloc(len: usize) -> *mut u8 {
    let mut buffer = vec![0u8; len.max(1)];
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}

/// # Safety
/// `ptr` and `len` come from one `gf_alloc` call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gf_free(ptr: *mut u8, len: usize) {
    drop(unsafe { Vec::from_raw_parts(ptr, len.max(1), len.max(1)) });
}

/// # Safety
/// Every pointer addresses the element count its length argument implies, as written by the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gf_kernel_new(
    vertices: usize,
    triangles: *const u32,
    triangle_count: usize,
    targets: usize,
    offsets: *const u32,
    index: *const u32,
    entries: usize,
    delta: *const i16,
    scale: *const f32,
) -> *mut Kernel {
    let flat = copy(triangles, triangle_count * 3);
    let tris = flat.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
    match Kernel::new(vertices, tris, copy(offsets, targets + 1), copy(index, entries), copy(delta, entries * 3), copy(scale, targets)) {
        Ok(k) => Box::into_raw(Box::new(k)),
        Err(_) => std::ptr::null_mut(),
    }
}

fn kernel<'a>(k: *mut Kernel) -> &'a mut Kernel {
    unsafe { &mut *k }
}

/// # Safety
/// `k` comes from `gf_kernel_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gf_weights(k: *mut Kernel) -> *mut f32 {
    kernel(k).weights().as_mut_ptr()
}

/// # Safety
/// `k` comes from `gf_kernel_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gf_anchor(k: *mut Kernel) -> *mut f32 {
    kernel(k).anchor().as_mut_ptr()
}

/// # Safety
/// `k` comes from `gf_kernel_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gf_extra(k: *mut Kernel) -> *mut f32 {
    kernel(k).extra().as_mut_ptr()
}

/// # Safety
/// `k` comes from `gf_kernel_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gf_reference(k: *mut Kernel) -> *mut f32 {
    kernel(k).reference().as_mut_ptr()
}

/// # Safety
/// `k` comes from `gf_kernel_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gf_positions(k: *mut Kernel) -> *const f32 {
    kernel(k).positions().as_ptr()
}

/// # Safety
/// `k` comes from `gf_kernel_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gf_normals(k: *mut Kernel) -> *const f32 {
    kernel(k).normals().as_ptr()
}

/// # Safety
/// `k` comes from `gf_kernel_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gf_preview(k: *mut Kernel, with_extra: u32) -> u32 {
    kernel(k).preview(with_extra != 0) as u32
}

/// # Safety
/// `k` comes from `gf_kernel_new`; `against` is 0 for the anchor, 1 for the reference buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gf_flips(k: *mut Kernel, against: u32) -> u32 {
    let k = kernel(k);
    let other = if against == 0 { k.anchor.clone() } else { k.reference.clone() };
    k.flips_against(&other) as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn gf_chart_cell(u_deg: f64, v_mm: f64) -> u32 {
    gf_geom::face_chart::pack(gf_geom::face_chart::cell_at([u_deg, v_mm]))
}

#[unsafe(no_mangle)]
pub extern "C" fn gf_chart_point_cell(u0: f64, v0: f64, u1: f64, v1: f64, u2: f64, v2: f64, b0: f64, b1: f64, b2: f64) -> u32 {
    use gf_geom::face_chart as fc;
    fc::pack(fc::point_cell([[u0, v0], [u1, v1], [u2, v2]], [b0, b1, b2]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Kernel {
        let tris = vec![[0, 1, 2], [0, 2, 3]];
        Kernel::new(4, tris, vec![0, 2, 3], vec![1, 2, 3], vec![100, 0, 0, 0, 200, 0, 0, 0, -50], vec![1e-3, 2e-3]).unwrap()
    }

    #[test]
    fn preview_is_anchor_plus_weighted_deltas() {
        let mut k = square();
        k.anchor().copy_from_slice(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0]);
        k.weights().copy_from_slice(&[0.5, 0.0]);
        assert_eq!(k.preview(false), 1);
        assert!((k.positions()[3] - 1.05).abs() < 1e-6);
        assert!((k.positions()[7] - 1.1).abs() < 1e-6);
        k.weights().copy_from_slice(&[0.0, 1.0]);
        k.preview(false);
        assert!((k.positions()[11] + 0.1).abs() < 1e-6);
        assert!((k.normals()[2].abs() - 1.0).abs() < 1e-3 || k.normals()[2].abs() > 0.9);
    }

    #[test]
    fn a_folded_preview_counts_a_flip() {
        let mut k = square();
        let flat = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0];
        k.anchor().copy_from_slice(&flat);
        k.preview(false);
        assert_eq!(k.flips_against(&flat), 0);
        k.anchor()[9] = 2.0;
        k.anchor()[10] = 0.5;
        k.preview(false);
        assert_eq!(k.flips_against(&flat), 1);
    }

    #[test]
    fn bad_tables_are_refused() {
        assert!(Kernel::new(2, vec![], vec![0, 1], vec![5], vec![0, 0, 0], vec![1.0]).is_err());
        assert!(Kernel::new(2, vec![], vec![1, 1], vec![0], vec![0, 0, 0], vec![1.0]).is_err());
    }
}
