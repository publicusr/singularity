//! The black hole on the GPU (ADR-0004): wgpu compute off-screen → RGBA8 readback. `SINGULARITY_CPU=1` disables it.

use super::blackhole::{Frame, PIXEL_LEVELS, PIXEL_RAMP};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    size: [f32; 4],
    center: [f32; 4],
    look0: [f32; 4],
    look1: [f32; 4],
    look2: [f32; 4],
    look3: [f32; 4],
    tint: [f32; 4],
    ramp: [[f32; 4]; 4],
}

struct Target {
    w: usize,
    h: usize,
    storage: wgpu::Buffer,
    readback: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    uniform: wgpu::Buffer,
    target: Option<Target>,
    pub adapter: String,
}

impl Gpu {
    /// `high_performance`: the dedicated card; else the built-in graphics (wgpu's low-power choice).
    pub fn new(high_performance: bool) -> Option<Gpu> {
        if std::env::var("SINGULARITY_CPU").is_ok_and(|v| v != "0") {
            return None;
        }
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        desc.backends = wgpu::Backends::PRIMARY;
        let instance = wgpu::Instance::new(desc);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: if high_performance { wgpu::PowerPreference::HighPerformance } else { wgpu::PowerPreference::LowPower }, ..Default::default() })).ok()?;
        let name = adapter.get_info().name;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("blackhole"), source: wgpu::ShaderSource::Wgsl(include_str!("blackhole.wgsl").into()) });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("blackhole"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Some(Gpu { device, queue, pipeline, uniform, target: None, adapter: name })
    }

    fn ensure_target(&mut self, w: usize, h: usize) {
        if self.target.as_ref().is_none_or(|t| t.w != w || t.h != h) {
            let bytes = (w * h * 4) as u64;
            let buffer = |label, usage| self.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: bytes, usage, mapped_at_creation: false });
            let storage = buffer("out", wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
            let readback = buffer("readback", wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: storage.as_entire_binding() },
                ],
            });
            self.target = Some(Target { w, h, storage, readback, bind });
        }
    }

    /// Render `w`×`h` RGBA8 pixels; None if the GPU failed this frame.
    pub fn render(&mut self, f: &Frame, w: usize, h: usize) -> Option<Vec<u8>> {
        let l = f.look;
        let r = PIXEL_RAMP;
        let levels = if l.pixel > 0 { PIXEL_LEVELS } else { 0.0 };
        let params = Params {
            size: [w as f32, h as f32, f.t, f.gain],
            center: [0.5, 0.5, f.rh, 0.0],
            look0: [l.temp, l.incl, l.roll, l.inner],
            look1: [l.outer, l.opac, l.dopp, l.beam],
            look2: [l.gain, l.contr, l.wind, l.speed],
            look3: [l.expo, l.star, 0.0, levels],
            tint: [0.0; 4],
            ramp: r.map(|c| [c[0], c[1], c[2], 0.0]),
        };
        self.queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&params));
        self.ensure_target(w, h);
        let t = self.target.as_ref()?;
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &t.bind, &[]);
            pass.dispatch_workgroups((w as u32).div_ceil(8), (h as u32).div_ceil(8), 1);
        }
        enc.copy_buffer_to_buffer(&t.storage, 0, &t.readback, 0, (w * h * 4) as u64);
        self.queue.submit([enc.finish()]);
        let slice = t.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        let out = slice.get_mapped_range().ok().map(|d| d.to_vec());
        t.readback.unmap();
        out
    }
}
