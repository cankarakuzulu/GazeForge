use std::io::{BufWriter, Write};
use std::path::Path;

use gf_geom::decimate::Decimated;

pub fn write(path: &Path, game_positions: &[[f64; 3]], mesh: &Decimated) -> Result<(), String> {
    write_triangles(path, game_positions, &mesh.triangles, &mesh.triangle_uvs)
}

pub fn write_triangles(path: &Path, game_positions: &[[f64; 3]], triangles: &[[u32; 3]], triangle_uvs: &[[gf_geom::Uv; 3]]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = BufWriter::new(file);
    let io = |e: std::io::Error| e.to_string();
    writeln!(out, "o GameHead").map_err(io)?;
    for p in game_positions {
        writeln!(out, "v {:.7} {:.7} {:.7}", p[0], p[1], p[2]).map_err(io)?;
    }
    for uvs in triangle_uvs {
        for uv in uvs {
            writeln!(out, "vt {:.7} {:.7}", uv[0], uv[1]).map_err(io)?;
        }
    }
    writeln!(out, "s 1").map_err(io)?;
    for (t, tri) in triangles.iter().enumerate() {
        let corner = |i: usize| format!("{}/{}", tri[i] + 1, t * 3 + i + 1);
        writeln!(out, "f {} {} {}", corner(0), corner(1), corner(2)).map_err(io)?;
    }
    Ok(())
}

pub struct ObjMesh {
    pub positions: Vec<[f64; 3]>,
    pub uvs: Vec<[f64; 2]>,
    pub faces: Vec<[[usize; 2]; 3]>,
}

impl ObjMesh {
    pub fn triangles(&self) -> Vec<[u32; 3]> {
        self.faces.iter().map(|f| f.map(|c| c[0] as u32)).collect()
    }

    pub fn corner_uvs(&self) -> Vec<[[f64; 2]; 3]> {
        self.faces.iter().map(|f| f.map(|c| self.uvs[c[1]])).collect()
    }
}

pub fn read(path: &Path) -> Result<ObjMesh, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut mesh = ObjMesh { positions: Vec::new(), uvs: Vec::new(), faces: Vec::new() };
    let bad = |n: usize, what: &str| format!("{} line {}: {what}", path.display(), n + 1);
    for (n, line) in text.lines().enumerate() {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("v") => {
                let v: Vec<f64> = parts.take(3).map(|x| x.parse::<f64>()).collect::<Result<_, _>>().map_err(|_| bad(n, "bad v"))?;
                if v.len() != 3 {
                    return Err(bad(n, "v needs 3 numbers"));
                }
                mesh.positions.push([v[0], v[1], v[2]]);
            }
            Some("vt") => {
                let v: Vec<f64> = parts.take(2).map(|x| x.parse::<f64>()).collect::<Result<_, _>>().map_err(|_| bad(n, "bad vt"))?;
                if v.len() != 2 {
                    return Err(bad(n, "vt needs 2 numbers"));
                }
                mesh.uvs.push([v[0], v[1]]);
            }
            Some("f") => {
                let corners: Vec<&str> = parts.collect();
                if corners.len() != 3 {
                    return Err(bad(n, "only triangles are read"));
                }
                let mut face = [[0usize; 2]; 3];
                for (c, corner) in corners.iter().enumerate() {
                    let ids: Vec<usize> = corner.split('/').take(2).map(|x| x.parse::<usize>()).collect::<Result<_, _>>().map_err(|_| bad(n, "bad f"))?;
                    if ids.len() != 2 || ids[0] == 0 || ids[1] == 0 {
                        return Err(bad(n, "each corner needs v/vt"));
                    }
                    face[c] = [ids[0] - 1, ids[1] - 1];
                }
                mesh.faces.push(face);
            }
            _ => {}
        }
    }
    if mesh.faces.iter().any(|f| f.iter().any(|c| c[0] >= mesh.positions.len() || c[1] >= mesh.uvs.len())) {
        return Err(format!("{}: a face corner points past the vertex or uv list", path.display()));
    }
    Ok(mesh)
}
