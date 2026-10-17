//! Off-screen GPU rendering for the black hole (wgpu compute → readback → kitty).
//! Falls back to the CPU tracer when no adapter is available (SINGULARITY_CPU=1 forces it).

use crate::audio::AudioFrame;
use crate::blackhole::Style;
use crate::gfx::{Fb, Rgb};
use wgpu::util::DeviceExt;

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

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    out: Option<(wgpu::Buffer, wgpu::Buffer, usize)>,
    pub adapter_name: String,
}

/// Pixel-art palette: deep red → orange → amber → pale yellow.
pub const PIXEL_RAMP: [Rgb; 4] = [Rgb(0.35, 0.03, 0.02), Rgb(0.85, 0.22, 0.04), Rgb(1.0, 0.58, 0.12), Rgb(1.0, 0.93, 0.62)];

impl Gpu {
    pub fn new() -> Option<Gpu> {
        if std::env::var("SINGULARITY_CPU").is_ok() {
            return None;
        }
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        desc.backends = wgpu::Backends::PRIMARY;
        let instance = wgpu::Instance::new(desc);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            ..Default::default()
        }))
        .ok()?;
        let adapter_name = adapter.get_info().name;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blackhole"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blackhole.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("blackhole"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let layout = pipeline.get_bind_group_layout(0);
        Some(Gpu { device, queue, pipeline, layout, out: None, adapter_name })
    }

    /// Renders into `fb` (its size decides the resolution). `levels` > 0 = pixel-art quantisation.
    pub fn render(&mut self, fb: &mut Fb, t: f32, a: &AudioFrame, st: &Style, levels: f32) {
        let (w, h) = (fb.w, fb.h);
        let l = st.look;
        let tint = st.tint.unwrap_or(Rgb::BLACK);
        let r = PIXEL_RAMP;
        let params = Params {
            size: [w as f32, h as f32, t, 0.85 + 0.45 * a.bass + 0.25 * a.beat],
            center: [st.center.0, st.center.1, st.rh, if st.alpha { 1.0 } else { 0.0 }],
            look0: [l.temp, l.incl, l.roll, l.inner],
            look1: [l.outer, l.opac, l.dopp, l.beam],
            look2: [l.gain, l.contr, l.wind, l.speed],
            look3: [l.expo, l.star, if st.tint.is_some() { 1.0 } else { 0.0 }, levels],
            tint: [tint.0, tint.1, tint.2, 0.0],
            ramp: [[r[0].0, r[0].1, r[0].2, 0.0], [r[1].0, r[1].1, r[1].2, 0.0], [r[2].0, r[2].1, r[2].2, 0.0], [r[3].0, r[3].1, r[3].2, 0.0]],
        };
        let n = w * h;
        let bytes = (n * 4) as u64;
        if self.out.as_ref().map(|o| o.2) != Some(n) {
            let storage = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("out"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.out = Some((storage, readback, n));
        }
        let (storage, readback, _) = self.out.as_ref().unwrap();
        let uniform = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: storage.as_entire_binding() },
            ],
        });
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups((w as u32).div_ceil(8), (h as u32).div_ceil(8), 1);
        }
        enc.copy_buffer_to_buffer(storage, 0, readback, 0, bytes);
        self.queue.submit([enc.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        {
            let Ok(data) = slice.get_mapped_range() else {
                readback.unmap();
                return;
            };
            fb.raw.clear();
            fb.raw.extend_from_slice(&data);
        }
        readback.unmap();
        // keep the f32 view in sync for the cell tier (premultiplied, like the CPU path)
        let alpha = st.alpha;
        if alpha {
            fb.alpha.resize(n, 0.0);
        }
        for i in 0..n {
            let px = &fb.raw[i * 4..i * 4 + 4];
            let a8 = px[3] as f32 / 255.0;
            let k = if alpha { a8 } else { 1.0 } / 255.0;
            fb.px[i] = Rgb(px[0] as f32 * k, px[1] as f32 * k, px[2] as f32 * k);
            if alpha {
                fb.alpha[i] = a8;
            }
        }
    }
}
