use eframe::wgpu;
use eframe::wgpu::util::DeviceExt;

use crate::session::Head;
use crate::viewer::{self, Camera, Matrices, Rect, SlotState, V3, ViewerState, add, cross, dot, mul, norm, sub};

pub const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;
const SAMPLES: u32 = 4;
const UFLOATS: usize = 152;
const USTRIDE: u64 = 768;
const USLOTS: u64 = 256;

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
}

impl Gpu {
    pub fn headless() -> Result<Gpu, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .map_err(|e| format!("no GPU adapter: {e}"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("gazeforge-studio"), ..Default::default() }))
            .map_err(|e| format!("no GPU device: {e}"))?;
        Ok(Gpu { device, queue, info: adapter.get_info() })
    }
}

pub struct FrameRequest<'a> {
    pub slot: usize,
    pub camera: &'a Camera,
    pub matrices: &'a Matrices,
    pub width: u32,
    pub height: u32,
    pub mode: usize,
    pub lod: usize,
    pub head_revision: u64,
    pub head: Option<&'a Head>,
}

pub trait ExternalFrames {
    fn name(&self) -> &str;
    fn frame(&mut self, request: &FrameRequest) -> Option<std::sync::Arc<Vec<u8>>>;
    fn status(&self) -> Option<String> {
        None
    }
}

pub struct TestPattern;

impl ExternalFrames for TestPattern {
    fn name(&self) -> &str {
        "test-pattern"
    }
    fn frame(&mut self, r: &FrameRequest) -> Option<std::sync::Arc<Vec<u8>>> {
        let mut out = vec![0u8; (r.width * r.height * 4) as usize];
        let shift = (r.camera.cur.yaw * 64.0) as i64;
        let cell = if r.matrices.ortho { 16 } else { 32 };
        let tint = (r.mode as u64 * 20 + r.head_revision) as u8;
        for y in 0..r.height {
            for x in 0..r.width {
                let k = (((x as i64 + shift).div_euclid(cell)) + (y as i64).div_euclid(cell)).rem_euclid(2);
                let i = ((y * r.width + x) * 4) as usize;
                let v = if k == 0 { 60 } else { 200 };
                out[i..i + 4].copy_from_slice(&[v, (40 * r.slot as u8).wrapping_add(v / 2), (255 - v).wrapping_add(tint), 255]);
            }
        }
        Some(std::sync::Arc::new(out))
    }
}

pub enum Source {
    Native,
    External(Box<dyn ExternalFrames>),
}

impl Source {
    pub fn name(&self) -> &str {
        match self {
            Source::Native => "native",
            Source::External(e) => e.name(),
        }
    }
}

struct GpuLod {
    tri: u32,
    verts: u32,
    uvs: wgpu::Buffer,
    flags: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

struct GpuHead {
    pos: wgpu::Buffer,
    nrm: wgpu::Buffer,
    refp: wgpu::Buffer,
    has_ref: bool,
    lods: Vec<GpuLod>,
    revision: u64,
    wind: f32,
    n: usize,
    eyes: Option<GpuEyes>,
}

struct GpuEyes {
    tri: u32,
    bind: wgpu::BindGroup,
    pos: wgpu::Buffer,
    nrm: wgpu::Buffer,
}

struct SlotTarget {
    w: u32,
    h: u32,
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
    holds: Option<std::sync::Arc<Vec<u8>>>,
}

pub struct Composite {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub w: u32,
    pub h: u32,
    pub generation: u64,
}

#[derive(Clone, Copy, PartialEq)]
enum MeshPipe {
    Plain,
    Biased,
    Stencil,
    Xray,
}

#[derive(Clone, Copy, PartialEq)]
enum LinePipe {
    Lines,
    Cap,
}

enum Draw {
    Mesh(MeshPipe, u32, usize),
    Eyes(u32),
    Points(u32, usize),
    Lines(LinePipe, u32, u32, u32),
}

struct PassPlan {
    first: bool,
    clear_stencil: bool,
    resolve: bool,
    draws: Vec<Draw>,
}

pub struct FrameStats {
    pub encode_ms: f64,
    pub draws: usize,
    pub passes: usize,
}

pub struct Renderer {
    g1: wgpu::BindGroupLayout,
    mesh: [wgpu::RenderPipeline; 4],
    point: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    cap: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    uniform_bg: wgpu::BindGroup,
    line_buf: wgpu::Buffer,
    line_cap: u64,
    head: Option<GpuHead>,
    slots: Vec<Option<SlotTarget>>,
    pub composite: Option<Composite>,
    pub sources: Vec<Source>,
    pub dpr: f64,
    generation: u64,
    epoch: u64,
    head_generation: u64,
}

fn f32_bytes(v: &[f32]) -> Vec<u8> {
    let mut out: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
    while out.len() < 16 {
        out.push(0);
    }
    out
}

fn u32_bytes(v: &[u32]) -> Vec<u8> {
    let mut out: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
    while out.len() < 16 {
        out.push(0);
    }
    out
}

fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

impl Renderer {
    pub fn new(device: &wgpu::Device) -> Renderer {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("view.wgsl"), source: wgpu::ShaderSource::Wgsl(include_str!("view.wgsl").into()) });
        let g0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new((UFLOATS * 4) as u64) },
                count: None,
            }],
        });
        let g1 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("mesh"), entries: &(0..7).map(storage_entry).collect::<Vec<_>>() });
        let mesh_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("mesh"), bind_group_layouts: &[Some(&g0), Some(&g1)], immediate_size: 0 });
        let line_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("lines"), bind_group_layouts: &[Some(&g0)], immediate_size: 0 });
        let face = |compare, pass_op| wgpu::StencilFaceState { compare, fail_op: wgpu::StencilOperation::Keep, depth_fail_op: wgpu::StencilOperation::Keep, pass_op };
        let ds = |write: bool, compare: wgpu::CompareFunction, stencil: wgpu::StencilState, bias: wgpu::DepthBiasState| {
            Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: Some(write), depth_compare: Some(compare), stencil, bias })
        };
        let no_stencil = wgpu::StencilState::default();
        let no_bias = wgpu::DepthBiasState::default();
        let ms = wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false };
        let target = |blend: Option<wgpu::BlendState>, write_mask| [Some(wgpu::ColorTargetState { format: COLOR, blend, write_mask })];
        let gl_alpha = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
        let blend = Some(wgpu::BlendState { color: gl_alpha, alpha: gl_alpha });
        let make = |label: &str, layout: &wgpu::PipelineLayout, vs: &str, fs: &str, topology, buffers: &[wgpu::VertexBufferLayout], depth, targets: &[Option<wgpu::ColorTargetState>]| {
            let mut desc = wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module: &module, entry_point: Some(vs), compilation_options: Default::default(), buffers },
                primitive: Default::default(),
                depth_stencil: depth,
                multisample: ms,
                fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some(fs), compilation_options: Default::default(), targets }),
                multiview_mask: None,
                cache: None,
            };
            desc.primitive.topology = topology;
            desc.primitive.front_face = wgpu::FrontFace::Ccw;
            desc.primitive.cull_mode = None;
            device.create_render_pipeline(&desc)
        };
        let tri = wgpu::PrimitiveTopology::TriangleList;
        let all = wgpu::ColorWrites::ALL;
        let mesh = [
            make("mesh", &mesh_layout, "vs_mesh", "fs_mesh", tri, &[], ds(true, wgpu::CompareFunction::LessEqual, no_stencil.clone(), no_bias), &target(None, all)),
            make("mesh biased", &mesh_layout, "vs_mesh", "fs_mesh", tri, &[], ds(true, wgpu::CompareFunction::LessEqual, no_stencil.clone(), wgpu::DepthBiasState { constant: 1, slope_scale: 1.0, clamp: 0.0 }), &target(None, all)),
            make(
                "mesh stencil",
                &mesh_layout,
                "vs_mesh",
                "fs_mesh",
                tri,
                &[],
                ds(false, wgpu::CompareFunction::Always, wgpu::StencilState { front: face(wgpu::CompareFunction::Always, wgpu::StencilOperation::Invert), back: face(wgpu::CompareFunction::Always, wgpu::StencilOperation::Invert), read_mask: 0xff, write_mask: 0xff }, no_bias),
                &target(None, wgpu::ColorWrites::empty()),
            ),
            make("mesh xray", &mesh_layout, "vs_mesh", "fs_mesh", tri, &[], ds(false, wgpu::CompareFunction::Greater, no_stencil.clone(), no_bias), &target(blend, all)),
        ];
        let point = make("points", &mesh_layout, "vs_point", "fs_point", tri, &[], ds(true, wgpu::CompareFunction::LessEqual, no_stencil.clone(), no_bias), &target(None, all));
        let vb = [wgpu::VertexBufferLayout { array_stride: 12, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3] }];
        let lines = make("lines", &line_layout, "vs_line", "fs_line", wgpu::PrimitiveTopology::LineList, &vb, ds(false, wgpu::CompareFunction::Always, no_stencil.clone(), no_bias), &target(None, all));
        let cap = make(
            "cap",
            &line_layout,
            "vs_line",
            "fs_line",
            tri,
            &vb,
            ds(true, wgpu::CompareFunction::Less, wgpu::StencilState { front: face(wgpu::CompareFunction::NotEqual, wgpu::StencilOperation::Keep), back: face(wgpu::CompareFunction::NotEqual, wgpu::StencilOperation::Keep), read_mask: 0xff, write_mask: 0 }, no_bias),
            &target(None, all),
        );
        let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("uniforms"), size: USTRIDE * USLOTS, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let uniform_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("uniforms"),
            layout: &g0,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &uniform, offset: 0, size: wgpu::BufferSize::new((UFLOATS * 4) as u64) }) }],
        });
        let line_cap = 1 << 16;
        let line_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("lines"), size: line_cap, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        Renderer {
            g1,
            mesh,
            point,
            lines,
            cap,
            uniform,
            uniform_bg,
            line_buf,
            line_cap,
            head: None,
            slots: (0..4).map(|_| None).collect(),
            composite: None,
            sources: (0..4).map(|_| Source::Native).collect(),
            dpr: 1.0,
            generation: 0,
            epoch: 0,
            head_generation: 0,
        }
    }

    pub fn clear_head(&mut self) {
        self.head = None;
    }

    pub fn sync_world(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, world: &mut crate::world::World) {
        if world.epoch != self.epoch || world.head_generation != self.head_generation {
            self.clear_head();
            self.epoch = world.epoch;
            self.head_generation = world.head_generation;
        }
        if let Some(s) = &world.session {
            self.sync_head(device, queue, &s.head, world.uvs_dirty);
            world.uvs_dirty = false;
        }
    }

    pub fn sync_head(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, head: &Head, uvs_changed: bool) {
        let fresh = self.head.as_ref().is_none_or(|g| g.n != head.n || g.lods.len() != head.lods.len());
        if fresh {
            let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
            let buf = |label: &str, bytes: Vec<u8>| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: &bytes, usage });
            let pos = buf("positions", f32_bytes(&head.positions));
            let nrm = buf("normals", f32_bytes(&head.normals));
            let refp = buf("reference", f32_bytes(head.reference.as_deref().unwrap_or(&vec![0.0; head.n * 3])));
            let lods = head
                .lods
                .iter()
                .map(|l| {
                    let idx = buf("indices", u32_bytes(&l.indices));
                    let uvs = buf("uvs", f32_bytes(&l.uvs));
                    let flags = buf("flags", u32_bytes(&l.flags));
                    let verts = buf("verts", u32_bytes(&l.verts));
                    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("lod"),
                        layout: &self.g1,
                        entries: &[&idx, &pos, &nrm, &refp, &uvs, &flags, &verts]
                            .iter()
                            .enumerate()
                            .map(|(i, b)| wgpu::BindGroupEntry { binding: i as u32, resource: b.as_entire_binding() })
                            .collect::<Vec<_>>(),
                    });
                    GpuLod { tri: l.triangles() as u32, verts: l.verts.len() as u32, uvs, flags, bind }
                })
                .collect();
            let eyes = head.eyes.as_ref().filter(|e| !e.indices.is_empty()).map(|e| {
                let (tri, verts) = (e.indices.len() / 3, e.positions.len() / 3);
                let entries = [
                    buf("eye indices", u32_bytes(&e.indices)),
                    buf("eye positions", f32_bytes(&e.positions)),
                    buf("eye normals", f32_bytes(&e.normals)),
                    buf("eye reference", f32_bytes(&e.positions)),
                    buf("eye uvs", f32_bytes(&vec![0.0; tri * 6])),
                    buf("eye flags", u32_bytes(&vec![0; tri])),
                    buf("eye verts", u32_bytes(&(0..verts as u32).collect::<Vec<_>>())),
                ];
                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("eyes"),
                    layout: &self.g1,
                    entries: &entries.iter().enumerate().map(|(i, b)| wgpu::BindGroupEntry { binding: i as u32, resource: b.as_entire_binding() }).collect::<Vec<_>>(),
                });
                let [_, pos, nrm, ..] = entries;
                GpuEyes { tri: tri as u32, bind, pos, nrm }
            });
            self.head = Some(GpuHead { pos, nrm, refp, has_ref: head.reference.is_some(), lods, revision: head.revision, wind: head.wind, n: head.n, eyes });
            return;
        }
        let g = self.head.as_mut().expect("checked above");
        if g.revision != head.revision {
            queue.write_buffer(&g.pos, 0, &f32_bytes(&head.positions));
            queue.write_buffer(&g.nrm, 0, &f32_bytes(&head.normals));
            if let (Some(ge), Some(e)) = (&g.eyes, &head.eyes) {
                queue.write_buffer(&ge.pos, 0, &f32_bytes(&e.positions));
                queue.write_buffer(&ge.nrm, 0, &f32_bytes(&e.normals));
            }
            g.revision = head.revision;
            g.wind = head.wind;
        }
        if let (Some(r), false) = (&head.reference, g.has_ref) {
            queue.write_buffer(&g.refp, 0, &f32_bytes(r));
            g.has_ref = true;
        }
        if uvs_changed {
            for (gl, l) in g.lods.iter().zip(&head.lods) {
                queue.write_buffer(&gl.uvs, 0, &f32_bytes(&l.uvs));
                queue.write_buffer(&gl.flags, 0, &u32_bytes(&l.flags));
            }
        }
    }

    fn slot_target(&mut self, device: &wgpu::Device, slot: usize, w: u32, h: u32) {
        if self.slots[slot].as_ref().is_some_and(|s| s.w == w && s.h == h) {
            return;
        }
        let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
        let tex = |label: &str, format, samples, usage| {
            device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size, mip_level_count: 1, sample_count: samples, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] })
        };
        let msaa = tex("msaa", COLOR, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
        let depth = tex("depth", DEPTH, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
        let resolve = tex("resolve", COLOR, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST);
        let resolve_view = resolve.create_view(&Default::default());
        self.slots[slot] = Some(SlotTarget { w, h, msaa, depth, resolve, resolve_view, holds: None });
    }

    fn ensure_composite(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        if self.composite.as_ref().is_some_and(|c| c.w == w && c.h == h) {
            return;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("composite"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.generation += 1;
        self.composite = Some(Composite { texture, view, w, h, generation: self.generation });
    }

    fn base_uniforms(st: &ViewerState, slot: SlotState, m: &Matrices, wind: f32, has_ref: bool, rect: &Rect, canvas_h: u32) -> [f32; UFLOATS] {
        let o = &st.opts;
        let mut u = [0.0f32; UFLOATS];
        u[..16].copy_from_slice(&m.mvp);
        let put = |u: &mut [f32; UFLOATS], at: usize, v: V3, w: f64| {
            u[at] = v[0] as f32;
            u[at + 1] = v[1] as f32;
            u[at + 2] = v[2] as f32;
            u[at + 3] = w as f32;
        };
        put(&mut u, 16, m.eye, 0.0);
        put(&mut u, 20, m.s, 0.0);
        put(&mut u, 24, m.u, 0.0);
        put(&mut u, 28, m.f, 0.0);
        let (dirs, cols) = viewer::lights_world(o, m);
        for (i, (d, c)) in dirs.iter().zip(&cols).enumerate() {
            put(&mut u, 32 + i * 4, *d, 0.0);
            put(&mut u, 48 + i * 4, *c, 0.0);
        }
        let sky = [0.92, 0.96, 1.04];
        put(&mut u, 64, mul(sky, o.ambient), 0.0);
        let earth = [0.42, 0.36, 0.32];
        put(&mut u, 68, mul([0, 1, 2].map(|i| earth[i] + (sky[i] - earth[i]) * o.ground), o.ambient), 0.0);
        put(&mut u, 72, [0.80f64, 0.74, 0.68].map(|x| x.powf(2.2)), 0.0);
        put(&mut u, 76, if o.dark { [0.92, 0.92, 0.95] } else { [0.08, 0.10, 0.14] }, 0.0);
        put(&mut u, 80, background(o.dark), 0.0);
        put(&mut u, 84, [0.95, 0.55, 0.15], 0.0);
        let heat = match slot.mode {
            8 => o.heat.quality,
            9 => o.heat.displacement,
            10 => o.heat.curvature,
            _ => 1.0,
        };
        let heat = if heat == 0.0 || heat.is_nan() { 1.0 } else { heat };
        let b = |x: bool| if x { 1.0 } else { 0.0 };
        u[88..92].copy_from_slice(&[slot.mode as f32, b(m.ortho), dirs.len() as f32, o.aniso as f32]);
        u[92..96].copy_from_slice(&[b(o.wire), o.wire_w as f32, b(o.wire_fade), 0.0]);
        u[96..100].copy_from_slice(&[b(o.back_hi), b(o.seams), o.exposure as f32, heat.max(1e-6) as f32]);
        u[100..104].copy_from_slice(&[wind, b(has_ref), 1.0, 1.0]);
        let planes = viewer::planes(o, &st.bounds);
        u[104..108].copy_from_slice(&[planes.len() as f32, -1.0, b(o.clip_box.on), 1.0]);
        u[108..112].copy_from_slice(&[rect.w as f32, rect.h as f32, 0.0, canvas_h as f32]);
        u[112..116].copy_from_slice(&[rect.x as f32, rect.y as f32, b(o.bad), 0.0]);
        for (i, p) in planes.iter().enumerate() {
            put(&mut u, 116 + i * 4, p.n, p.d);
        }
        let (lo, hi) = viewer::box_world(o, &st.bounds);
        put(&mut u, 140, lo, 0.0);
        put(&mut u, 144, hi, 0.0);
        u
    }

    pub fn render(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, st: &ViewerState, head: Option<&Head>, width: u32, height: u32) -> FrameStats {
        let started = std::time::Instant::now();
        let (width, height) = (width.max(1), height.max(1));
        self.ensure_composite(device, width, height);
        let rects = viewer::rects(st.layout, width, height);
        for r in &rects {
            self.slot_target(device, r.slot, r.w, r.h);
        }
        let mut uni: Vec<f32> = Vec::new();
        let mut lines: Vec<f32> = Vec::new();
        let chunk = |u: [f32; UFLOATS], uni: &mut Vec<f32>| -> u32 {
            let i = (uni.len() / (USTRIDE as usize / 4)) as u32;
            uni.extend_from_slice(&u);
            uni.resize(uni.len() + (USTRIDE as usize / 4 - UFLOATS), 0.0);
            i
        };
        let mut plans: Vec<(Rect, Vec<PassPlan>)> = Vec::new();
        let mut externals: Vec<(Rect, std::sync::Arc<Vec<u8>>)> = Vec::new();
        let (centre, rad) = (st.bounds.centre, st.bounds.radius);
        for r in &rects {
            let cam = &st.cams[r.cam];
            let slot = st.vp[r.slot];
            let m = cam.matrices(r.w as f64 / r.h as f64);
            if let Source::External(ext) = &mut self.sources[r.slot] {
                let req = FrameRequest { slot: r.slot, camera: cam, matrices: &m, width: r.w, height: r.h, mode: slot.mode, lod: slot.lod, head_revision: head.map_or(0, |h| h.revision), head };
                if let Some(px) = ext.frame(&req).filter(|p| p.len() == (r.w * r.h * 4) as usize) {
                    externals.push((*r, px));
                    continue;
                }
            }
            let (wind, has_ref) = self.head.as_ref().map_or((1.0, false), |g| (g.wind, g.has_ref));
            let base = Self::base_uniforms(st, slot, &m, wind, has_ref, r, height);
            let lod = self.head.as_ref().map_or(0, |g| slot.lod.min(g.lods.len().saturating_sub(1)));
            let has_head = self.head.is_some();
            let o = &st.opts;
            let mut passes = Vec::new();
            let mut first = PassPlan { first: true, clear_stencil: true, resolve: false, draws: Vec::new() };
            let base_chunk = chunk(base, &mut uni);
            if has_head {
                first.draws.push(Draw::Mesh(if o.points { MeshPipe::Biased } else { MeshPipe::Plain }, base_chunk, lod));
            }
            if o.eyes && self.head.as_ref().is_some_and(|g| g.eyes.is_some()) {
                let mut u = base;
                u[72..75].copy_from_slice(&[0.93f32, 0.93, 0.95].map(|x| x.powf(2.2)));
                u[88] = 0.0;
                u[92] = 0.0;
                u[96..98].copy_from_slice(&[0.0, 0.0]);
                u[100..104].copy_from_slice(&[1.0, 0.0, 1.0, 0.0]);
                first.draws.push(Draw::Eyes(chunk(u, &mut uni)));
            }
            passes.push(first);
            let planes = viewer::planes(o, &st.bounds);
            let line = |data: &[V3], col: V3, pipe: LinePipe, cap: Option<usize>, uni: &mut Vec<f32>, lines: &mut Vec<f32>| -> Draw {
                let mut u = base;
                u[148..151].copy_from_slice(&col.map(|x| x as f32));
                if let Some(j) = cap {
                    u[110] = 1.0;
                    u[105] = j as f32;
                }
                let c = chunk(u, uni);
                let first = (lines.len() / 3) as u32;
                for p in data {
                    lines.extend(p.iter().map(|&x| x as f32));
                }
                Draw::Lines(pipe, c, first, data.len() as u32)
            };
            if o.caps && !planes.is_empty() && has_head {
                for (j, q) in planes.iter().enumerate() {
                    let t1 = norm(if q.n[2].abs() < 0.9 { cross(q.n, [0.0, 0.0, 1.0]) } else { cross(q.n, [1.0, 0.0, 0.0]) });
                    let t2 = cross(q.n, t1);
                    let c0 = sub(centre, mul(q.n, dot(q.n, centre) - q.d));
                    let s = rad * 4.0;
                    let corner = |a: f64, b: f64| add(c0, add(mul(t1, a * s), mul(t2, b * s)));
                    let quad = [corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)];
                    let cap = line(&quad, [0.36, 0.45, 0.62], LinePipe::Cap, Some(j), &mut uni, &mut lines);
                    passes.push(PassPlan { first: false, clear_stencil: true, resolve: false, draws: vec![Draw::Mesh(MeshPipe::Stencil, base_chunk, lod), cap] });
                }
            }
            let mut last = PassPlan { first: false, clear_stencil: false, resolve: true, draws: Vec::new() };
            if has_head && o.wire && o.xray {
                let mut u = base;
                u[95] = 1.0;
                last.draws.push(Draw::Mesh(MeshPipe::Xray, chunk(u, &mut uni), lod));
            }
            if has_head && o.points {
                let mut u = base;
                u[148..151].copy_from_slice(&[0.95, 0.35, 0.10]);
                u[107] = (o.point_size * self.dpr) as f32;
                last.draws.push(Draw::Points(chunk(u, &mut uni), lod));
            }
            if o.light_gizmo {
                let (dirs, cols) = viewer::lights_world(o, &m);
                for (d, c) in dirs.iter().zip(&cols) {
                    let col = c.map(|v| v.powf(1.0 / 2.2).min(1.0));
                    last.draws.push(line(&[add(centre, mul(*d, rad * 0.55)), add(centre, mul(*d, rad * 1.25))], col, LinePipe::Lines, None, &mut uni, &mut lines));
                }
            }
            if o.plane_gizmo && !planes.is_empty() {
                for q in &planes {
                    let t1 = norm(if q.n[2].abs() < 0.9 { cross(q.n, [0.0, 0.0, 1.0]) } else { cross(q.n, [1.0, 0.0, 0.0]) });
                    let t2 = cross(q.n, t1);
                    let c0 = sub(centre, mul(q.n, dot(q.n, centre) - q.d));
                    let s = rad * 0.9;
                    let k = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]].map(|[a, b]| add(c0, add(mul(t1, a * s), mul(t2, b * s))));
                    let seg = [k[0], k[1], k[1], k[2], k[2], k[3], k[3], k[0], c0, add(c0, mul(q.n, rad * 0.3))];
                    last.draws.push(line(&seg, [0.25, 0.55, 0.95], LinePipe::Lines, None, &mut uni, &mut lines));
                }
            }
            if o.clip_box.on {
                let (lo, hi) = viewer::box_world(o, &st.bounds);
                let v = |i: usize| [if i & 1 != 0 { hi[0] } else { lo[0] }, if i & 2 != 0 { hi[1] } else { lo[1] }, if i & 4 != 0 { hi[2] } else { lo[2] }];
                let mut e = Vec::new();
                for i in 0..8 {
                    for bit in [1, 2, 4] {
                        if i & bit == 0 {
                            e.push(v(i));
                            e.push(v(i | bit));
                        }
                    }
                }
                last.draws.push(line(&e, [0.9, 0.3, 0.6], LinePipe::Lines, None, &mut uni, &mut lines));
            }
            if st.pivot_flash > 0.0 && r.slot == st.active {
                let t = cam.cur.target;
                let s = rad * 0.06;
                let seg = [
                    [t[0] - s, t[1], t[2]],
                    [t[0] + s, t[1], t[2]],
                    [t[0], t[1] - s, t[2]],
                    [t[0], t[1] + s, t[2]],
                    [t[0], t[1], t[2] - s],
                    [t[0], t[1], t[2] + s],
                ];
                last.draws.push(line(&seg, [1.0, 0.85, 0.2], LinePipe::Lines, None, &mut uni, &mut lines));
            }
            passes.push(last);
            plans.push((*r, passes));
        }
        if !uni.is_empty() {
            queue.write_buffer(&self.uniform, 0, &f32_bytes(&uni));
        }
        if !lines.is_empty() {
            let need = (lines.len() * 4) as u64;
            if need > self.line_cap {
                self.line_cap = need.next_power_of_two();
                self.line_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("lines"), size: self.line_cap, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            }
            queue.write_buffer(&self.line_buf, 0, &f32_bytes(&lines));
        }
        for (r, _) in &plans {
            if let Some(t) = self.slots[r.slot].as_mut() {
                t.holds = None;
            }
        }
        for (r, px) in &externals {
            let t = self.slots[r.slot].as_mut().expect("slot target");
            if t.holds.as_ref().is_some_and(|h| std::sync::Arc::ptr_eq(h, px)) {
                continue;
            }
            t.holds = Some(px.clone());
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &t.resolve, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                px,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(r.w * 4), rows_per_image: Some(r.h) },
                wgpu::Extent3d { width: r.w, height: r.h, depth_or_array_layers: 1 },
            );
        }
        let bg = background(st.opts.dark);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        let (mut draws, mut pass_count) = (0usize, 0usize);
        for (r, passes) in &plans {
            let t = self.slots[r.slot].as_ref().expect("slot target");
            for p in passes {
                pass_count += 1;
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("view"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &t.msaa,
                        depth_slice: None,
                        resolve_target: if p.resolve { Some(&t.resolve_view) } else { None },
                        ops: wgpu::Operations {
                            load: if p.first { wgpu::LoadOp::Clear(wgpu::Color { r: bg[0], g: bg[1], b: bg[2], a: 1.0 }) } else { wgpu::LoadOp::Load },
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &t.depth,
                        depth_ops: Some(wgpu::Operations { load: if p.first { wgpu::LoadOp::Clear(1.0) } else { wgpu::LoadOp::Load }, store: wgpu::StoreOp::Store }),
                        stencil_ops: Some(wgpu::Operations { load: if p.clear_stencil { wgpu::LoadOp::Clear(0) } else { wgpu::LoadOp::Load }, store: wgpu::StoreOp::Store }),
                    }),
                    ..Default::default()
                });
                for d in &p.draws {
                    draws += 1;
                    match d {
                        Draw::Mesh(pipe, c, lod) => {
                            let g = self.head.as_ref().expect("head");
                            let l = &g.lods[*lod];
                            let i = match pipe {
                                MeshPipe::Plain => 0,
                                MeshPipe::Biased => 1,
                                MeshPipe::Stencil => 2,
                                MeshPipe::Xray => 3,
                            };
                            pass.set_pipeline(&self.mesh[i]);
                            pass.set_bind_group(0, &self.uniform_bg, &[(*c as u64 * USTRIDE) as u32]);
                            pass.set_bind_group(1, &l.bind, &[]);
                            pass.draw(0..l.tri * 3, 0..1);
                        }
                        Draw::Eyes(c) => {
                            let e = self.head.as_ref().and_then(|g| g.eyes.as_ref()).expect("eyes");
                            pass.set_pipeline(&self.mesh[0]);
                            pass.set_bind_group(0, &self.uniform_bg, &[(*c as u64 * USTRIDE) as u32]);
                            pass.set_bind_group(1, &e.bind, &[]);
                            pass.draw(0..e.tri * 3, 0..1);
                        }
                        Draw::Points(c, lod) => {
                            let g = self.head.as_ref().expect("head");
                            let l = &g.lods[*lod];
                            pass.set_pipeline(&self.point);
                            pass.set_bind_group(0, &self.uniform_bg, &[(*c as u64 * USTRIDE) as u32]);
                            pass.set_bind_group(1, &l.bind, &[]);
                            pass.draw(0..l.verts * 6, 0..1);
                        }
                        Draw::Lines(pipe, c, first, count) => {
                            pass.set_pipeline(if *pipe == LinePipe::Cap { &self.cap } else { &self.lines });
                            pass.set_bind_group(0, &self.uniform_bg, &[(*c as u64 * USTRIDE) as u32]);
                            pass.set_vertex_buffer(0, self.line_buf.slice(..));
                            pass.set_stencil_reference(0);
                            pass.draw(*first..*first + *count, 0..1);
                        }
                    }
                }
            }
        }
        let comp = self.composite.as_ref().expect("composite");
        for r in &rects {
            let t = self.slots[r.slot].as_ref().expect("slot target");
            enc.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo { texture: &t.resolve, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyTextureInfo { texture: &comp.texture, mip_level: 0, origin: wgpu::Origin3d { x: r.x, y: r.y, z: 0 }, aspect: wgpu::TextureAspect::All },
                wgpu::Extent3d { width: r.w, height: r.h, depth_or_array_layers: 1 },
            );
        }
        queue.submit([enc.finish()]);
        FrameStats { encode_ms: started.elapsed().as_secs_f64() * 1000.0, draws, passes: pass_count }
    }

    pub fn read_rgba(&self, device: &wgpu::Device, queue: &wgpu::Queue, x: u32, y: u32, w: u32, h: u32) -> Result<Vec<u8>, String> {
        let comp = self.composite.as_ref().ok_or("nothing rendered yet")?;
        read_texture(device, queue, &comp.texture, x, y, w, h)
    }

    pub fn wait(&self, device: &wgpu::Device) -> Result<(), String> {
        device.poll(wgpu::PollType::wait_indefinitely()).map(|_| ()).map_err(|e| format!("poll: {e}"))
    }
}

pub fn read_texture(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture, x: u32, y: u32, w: u32, h: u32) -> Result<Vec<u8>, String> {
    let row = (w * 4).div_ceil(256) * 256;
    let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("readback"), size: (row * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("readback") });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit([enc.finish()]);
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| format!("poll: {e}"))?;
    rx.recv().map_err(|e| format!("map: {e}"))?.map_err(|e| format!("map: {e}"))?;
    let data = slice.get_mapped_range();
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for r in 0..h {
        let at = (r * row) as usize;
        out.extend_from_slice(&data[at..at + (w * 4) as usize]);
    }
    drop(data);
    buf.unmap();
    Ok(out)
}

pub fn background(dark: bool) -> [f64; 3] {
    if dark { [0.11, 0.11, 0.12] } else { [0.93, 0.93, 0.92] }
}

pub fn pick(st: &ViewerState, head: &Head, rect: &Rect, px: f64, py: f64) -> Option<(f64, V3)> {
    let nx = (px / rect.w as f64) * 2.0 - 1.0;
    let ny = 1.0 - (py / rect.h as f64) * 2.0;
    let (o, d) = st.cams[rect.cam].ray(nx, ny, rect.w as f64 / rect.h as f64);
    let planes = viewer::planes(&st.opts, &st.bounds);
    let (lo, hi) = viewer::box_world(&st.opts, &st.bounds);
    let cut = |p: V3| planes.iter().any(|q| dot(q.n, p) > q.d) || (st.opts.clip_box.on && (0..3).all(|k| p[k] > lo[k] && p[k] < hi[k]));
    let lod = &head.lods[st.vp[rect.slot].lod.min(head.lods.len() - 1)];
    let pos = &head.positions;
    let g = |i: u32| [pos[i as usize * 3] as f64, pos[i as usize * 3 + 1] as f64, pos[i as usize * 3 + 2] as f64];
    let mut best: Option<(f64, V3)> = None;
    for t in lod.indices.chunks_exact(3) {
        let (a, b, c) = (g(t[0]), g(t[1]), g(t[2]));
        let (e1, e2) = (sub(b, a), sub(c, a));
        let pv = cross(d, e2);
        let det = dot(e1, pv);
        if det.abs() < 1e-14 {
            continue;
        }
        let inv = 1.0 / det;
        let tv = sub(o, a);
        let uu = dot(tv, pv) * inv;
        if !(0.0..=1.0).contains(&uu) {
            continue;
        }
        let qv = cross(tv, e1);
        let vv = dot(d, qv) * inv;
        if vv < 0.0 || uu + vv > 1.0 {
            continue;
        }
        let dist = dot(e2, qv) * inv;
        if dist <= 0.0 || best.is_some_and(|(bt, _)| dist >= bt) {
            continue;
        }
        let hit = add(o, mul(d, dist));
        if cut(hit) {
            continue;
        }
        best = Some((dist, hit));
    }
    best
}
