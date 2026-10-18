pub type Quadric = [f64; 10];

pub fn from_plane(normal: [f64; 3], d: f64, weight: f64) -> Quadric {
    let [a, b, c] = normal;
    [a * a, a * b, a * c, a * d, b * b, b * c, b * d, c * c, c * d, d * d].map(|v| v * weight)
}

pub fn add(into: &mut Quadric, other: &Quadric) {
    for (x, y) in into.iter_mut().zip(other) {
        *x += y;
    }
}

pub fn eval_pair(a: &Quadric, b: &Quadric, p: [f64; 3]) -> f64 {
    let q: [f64; 10] = std::array::from_fn(|i| a[i] + b[i]);
    let [x, y, z] = p;
    q[0] * x * x + 2.0 * q[1] * x * y + 2.0 * q[2] * x * z + 2.0 * q[3] * x + q[4] * y * y + 2.0 * q[5] * y * z + 2.0 * q[6] * y
        + q[7] * z * z
        + 2.0 * q[8] * z
        + q[9]
}

pub fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

pub fn corner_angle(p: [[f64; 3]; 3], i: usize) -> f64 {
    let a = sub(p[(i + 1) % 3], p[i]);
    let b = sub(p[(i + 2) % 3], p[i]);
    let denominator = (length(a) * length(b)).max(1e-30);
    (dot(a, b) / denominator).clamp(-1.0, 1.0).acos().to_degrees()
}

pub fn corner_angles(p: [[f64; 3]; 3]) -> [f64; 3] {
    std::array::from_fn(|i| corner_angle(p, i))
}
