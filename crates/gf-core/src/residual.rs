use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;

pub const ENCODING: &str = "i16-planar-delta-zlib/1";
pub const DEFAULT_QUANTUM_M: f64 = 5e-6;
const MAGIC: &[u8; 4] = b"GFRS";
const HEADER: usize = 4 + 1 + 4 + 8;

#[derive(Debug)]
pub struct Encoded {
    pub bytes: Vec<u8>,
    pub raw_bytes: usize,
    pub max_error_m: f64,
}

pub fn encode(residual: &[[f64; 3]], quantum_m: f64) -> Result<Encoded, String> {
    if !(quantum_m.is_finite() && quantum_m > 0.0) {
        return Err(format!("the residual quantum must be finite and positive, got {quantum_m}"));
    }
    gf_geom::check_finite(residual)?;
    let limit = i16::MAX as f64;
    let mut planes: Vec<u8> = Vec::with_capacity(residual.len() * 6);
    let mut overflow = 0usize;
    let mut worst = 0.0f64;
    let mut max_error = 0.0f64;
    for axis in 0..3 {
        let mut previous: i16 = 0;
        for p in residual {
            let q = (p[axis] / quantum_m).round();
            if q.abs() > limit {
                overflow += 1;
                worst = worst.max(p[axis].abs());
                continue;
            }
            let q = q as i16;
            let back: f64 = (q as f64) * quantum_m;
            max_error = max_error.max((back - p[axis]).abs());
            planes.extend_from_slice(&q.wrapping_sub(previous).to_le_bytes());
            previous = q;
        }
    }
    if overflow > 0 {
        return Err(format!(
            "{overflow} residual components exceed the i16 range at {} mm (largest {:.4} mm, limit {:.4} mm)",
            quantum_m * 1000.0,
            worst * 1000.0,
            limit * quantum_m * 1000.0
        ));
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(&planes).map_err(|e| e.to_string())?;
    let packed = encoder.finish().map_err(|e| e.to_string())?;
    let mut bytes = Vec::with_capacity(HEADER + packed.len());
    bytes.extend_from_slice(MAGIC);
    bytes.push(1);
    bytes.extend_from_slice(&(residual.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&quantum_m.to_le_bytes());
    bytes.extend_from_slice(&packed);
    Ok(Encoded { bytes, raw_bytes: planes.len(), max_error_m: max_error })
}

pub fn decode(bytes: &[u8]) -> Result<(Vec<[f64; 3]>, f64), String> {
    if bytes.len() < HEADER || &bytes[..4] != MAGIC {
        return Err("not a residual blob (bad magic)".into());
    }
    if bytes[4] != 1 {
        return Err(format!("residual blob version {} is not supported (1 is)", bytes[4]));
    }
    let count = u32::from_le_bytes(bytes[5..9].try_into().unwrap()) as usize;
    let quantum = f64::from_le_bytes(bytes[9..17].try_into().unwrap());
    if !(quantum.is_finite() && quantum > 0.0) {
        return Err(format!("residual blob has a bad quantum {quantum}"));
    }
    let mut planes = Vec::with_capacity(count * 6);
    ZlibDecoder::new(&bytes[HEADER..]).read_to_end(&mut planes).map_err(|e| format!("residual blob: {e}"))?;
    if planes.len() != count * 6 {
        return Err(format!("residual blob holds {} bytes of planes, {count} vertices need {}", planes.len(), count * 6));
    }
    let mut out = vec![[0.0f64; 3]; count];
    for axis in 0..3 {
        let mut value: i16 = 0;
        for (v, point) in out.iter_mut().enumerate() {
            let at = (axis * count + v) * 2;
            value = value.wrapping_add(i16::from_le_bytes([planes[at], planes[at + 1]]));
            point[axis] = (value as f64) * quantum;
        }
    }
    Ok((out, quantum))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<[f64; 3]> {
        (0..5000).map(|i| {
            let t = i as f64 * 0.37;
            [t.sin() * 0.03, (t * 1.3).cos() * 0.0001, if i % 2 == 0 { 0.16 } else { -0.16 }]
        }).collect()
    }

    #[test]
    fn round_trip_stays_within_half_a_quantum_per_axis() {
        let r = sample();
        let encoded = encode(&r, DEFAULT_QUANTUM_M).unwrap();
        let (back, q) = decode(&encoded.bytes).unwrap();
        assert_eq!(q, DEFAULT_QUANTUM_M);
        let worst = r.iter().zip(&back).flat_map(|(a, b)| (0..3).map(move |k| (a[k] - b[k]).abs())).fold(0.0, f64::max);
        assert!(worst <= DEFAULT_QUANTUM_M / 2.0 + 1e-15, "{worst}");
        assert!((encoded.max_error_m - worst).abs() < 1e-15);
    }

    #[test]
    fn alternating_extremes_survive_the_wrapping_delta() {
        let r = sample();
        let (back, _) = decode(&encode(&r, DEFAULT_QUANTUM_M).unwrap().bytes).unwrap();
        assert!((back[0][2] - 0.16).abs() < 3e-6 && (back[1][2] + 0.16).abs() < 3e-6);
    }

    #[test]
    fn overflow_and_bad_input_are_refused() {
        assert!(encode(&[[0.2, 0.0, 0.0]], DEFAULT_QUANTUM_M).unwrap_err().contains("exceed"));
        assert!(encode(&[[f64::NAN, 0.0, 0.0]], DEFAULT_QUANTUM_M).is_err());
        assert!(encode(&[[0.0; 3]], 0.0).is_err());
        let mut bytes = encode(&sample(), DEFAULT_QUANTUM_M).unwrap().bytes;
        bytes[0] = b'X';
        assert!(decode(&bytes).is_err());
        let good = encode(&sample(), DEFAULT_QUANTUM_M).unwrap().bytes;
        assert!(decode(&good[..good.len() - 10]).is_err());
    }
}
