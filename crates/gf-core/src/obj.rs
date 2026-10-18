use std::io::{BufWriter, Write};
use std::path::Path;

use crate::decimate::Decimated;

pub fn write(path: &Path, game_positions: &[[f64; 3]], mesh: &Decimated) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = BufWriter::new(file);
    let io = |e: std::io::Error| e.to_string();
    writeln!(out, "o GameHead").map_err(io)?;
    for p in game_positions {
        writeln!(out, "v {:.7} {:.7} {:.7}", p[0], p[1], p[2]).map_err(io)?;
    }
    for uvs in &mesh.triangle_uvs {
        for uv in uvs {
            writeln!(out, "vt {:.7} {:.7}", uv[0], uv[1]).map_err(io)?;
        }
    }
    writeln!(out, "s 1").map_err(io)?;
    for (t, tri) in mesh.triangles.iter().enumerate() {
        let corner = |i: usize| format!("{}/{}", tri[i] + 1, t * 3 + i + 1);
        writeln!(out, "f {} {} {}", corner(0), corner(1), corner(2)).map_err(io)?;
    }
    Ok(())
}
