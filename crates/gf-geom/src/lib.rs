// Style lints only: the released arithmetic is kept as written so every placement stays byte-identical.
#![allow(clippy::too_many_arguments, clippy::needless_range_loop, clippy::type_complexity, clippy::neg_cmp_op_on_partial_ord, clippy::needless_borrow)]

pub mod cancel;
pub mod decimate;
pub mod eyeball;
pub mod face_chart;
pub mod lids;
pub mod placement;
pub mod placement_f2;
pub mod placement_f3;
pub mod placement_f5;
pub mod prepared;
pub mod quadric;
pub mod topology;
pub mod uv_chart;

pub type Uv = [f32; 2];

pub fn check_finite(points: &[[f64; 3]]) -> Result<(), String> {
    let bad: Vec<usize> = points.iter().enumerate().filter(|(_, p)| p.iter().any(|x| !x.is_finite())).map(|(i, _)| i).collect();
    match bad.first() {
        None => Ok(()),
        Some(first) => Err(format!("{} vertices have a NaN or infinite coordinate (first: vertex {first})", bad.len())),
    }
}

pub fn open_boundary(triangles: &[[u32; 3]]) -> Vec<u32> {
    let mut edges: Vec<(u32, u32)> = Vec::with_capacity(triangles.len() * 3);
    for t in triangles {
        for i in 0..3 {
            let (a, b) = (t[i], t[(i + 1) % 3]);
            edges.push((a.min(b), a.max(b)));
        }
    }
    edges.sort_unstable();
    let mut out: Vec<u32> = Vec::new();
    let mut i = 0;
    while i < edges.len() {
        let mut j = i + 1;
        while j < edges.len() && edges[j] == edges[i] {
            j += 1;
        }
        if j - i == 1 {
            out.push(edges[i].0);
            out.push(edges[i].1);
        }
        i = j;
    }
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(feature = "parallel")]
pub(crate) fn map_each<T: Send, F: Fn(usize) -> T + Sync + Send>(items: &[usize], f: F) -> Vec<T> {
    use rayon::prelude::*;
    items.par_iter().map(|&i| f(i)).collect()
}

#[cfg(not(feature = "parallel"))]
pub(crate) fn map_each<T, F: Fn(usize) -> T>(items: &[usize], f: F) -> Vec<T> {
    items.iter().map(|&i| f(i)).collect()
}

#[cfg(feature = "parallel")]
pub fn map_range<T: Send, F: Fn(usize) -> T + Sync + Send>(count: usize, f: F) -> Vec<T> {
    use rayon::prelude::*;
    (0..count).into_par_iter().map(f).collect()
}

#[cfg(not(feature = "parallel"))]
pub fn map_range<T, F: Fn(usize) -> T>(count: usize, f: F) -> Vec<T> {
    (0..count).map(f).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_points_pass() {
        assert!(check_finite(&[[0.0, 1.0, -2.0], [1e-9, 3.0, 4.0]]).is_ok());
    }

    #[test]
    fn nan_and_infinite_points_are_refused() {
        let error = check_finite(&[[0.0; 3], [0.0, f64::NAN, 0.0], [f64::INFINITY, 0.0, 0.0]]).unwrap_err();
        assert!(error.contains("2 vertices") && error.contains("vertex 1"), "{error}");
        assert!(check_finite(&[[f64::NEG_INFINITY, 0.0, 0.0]]).is_err());
    }

    #[test]
    fn the_open_boundary_of_two_triangles_is_all_four_corners() {
        assert_eq!(open_boundary(&[[0, 1, 2], [0, 2, 3]]), vec![0, 1, 2, 3]);
        let closed = [[0, 1, 2], [0, 3, 1], [1, 3, 2], [2, 3, 0]];
        assert!(open_boundary(&closed).is_empty());
    }

    #[test]
    fn map_each_keeps_the_order() {
        let items: Vec<usize> = (0..1000).rev().collect();
        assert_eq!(map_each(&items, |i| i * 2), items.iter().map(|i| i * 2).collect::<Vec<_>>());
        assert_eq!(map_range(1000, |i| i * 3), (0..1000).map(|i| i * 3).collect::<Vec<_>>());
    }
}
