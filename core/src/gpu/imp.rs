use crate::settings::{Backend, Quality};
use block_compression::{BC7Settings, CompressionVariant, GpuBlockCompressor};
use image_dds::ImageFormat;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::{Pin, pin};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Once, OnceLock, mpsc};
use std::task::{Context, Poll, Waker};
use thiserror::Error;

fn queue_depth() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get() / 2)
        .unwrap_or(4)
        .clamp(2, 8)
}

const MAX_IN_FLIGHT: usize = 2;
const MAX_BATCH_JOBS: usize = 8;
const MAX_BATCH_OUTPUT_BYTES: u64 = 48 << 20;
const MAX_BATCH_INPUT_BYTES: u64 = 128 << 20;
const MAX_BATCH_COST: u64 = 6 << 20;

fn cost_weight(format: ImageFormat, quality: Quality) -> u64 {
    match (format, quality) {
        (ImageFormat::BC7RgbaUnorm, Quality::Fast) => 4,
        (ImageFormat::BC7RgbaUnorm, Quality::Normal) => 8,
        (ImageFormat::BC7RgbaUnorm, Quality::Slow) => 16,
        _ => 1,
    }
}

const MAX_CONSECUTIVE_FAILURES: u32 = 3;
const SUBMISSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
const WARM_UP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);
const MAX_POOLED_TEXTURE_BYTES: u64 = 512 << 20;

#[derive(Debug, Error)]
pub enum GpuError {
    #[error("no suitable GPU adapter: {0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    #[error("only the software rasterizer `{0}` is available; the CPU pipeline is faster")]
    SoftwareOnly(String),
    #[error("GPU device request failed: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    #[error("format {0:?} has no GPU compression kernel")]
    UnsupportedFormat(ImageFormat),
    #[error("GPU validation failure: {0}")]
    Validation(String),
    #[error("GPU out of memory: {0}")]
    OutOfMemory(String),
    #[error("compressed buffer readback failed: {0}")]
    Readback(String),
    #[error("GPU compressor busy")]
    Busy,
}

const BLIT_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32(vertex_index) / 2) * 2.0;
    let y = f32(i32(vertex_index) & 1) * 2.0;
    out.tex_coords = vec2<f32>(x, y);
    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    return out;
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(source, source_sampler, in.tex_coords);
}
"#;

struct Shared {
    device: wgpu::Device,
    queue: wgpu::Queue,
    blit_pipeline: wgpu::RenderPipeline,
    blit_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

pub struct GpuContext {
    shared: Arc<Shared>,
    worker: OnceLock<mpsc::SyncSender<Job>>,
    ready: Arc<std::sync::atomic::AtomicBool>,
    bc_supported: bool,
}

impl GpuContext {
    pub fn new((device, queue): (wgpu::Device, wgpu::Queue)) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("squeezeit blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
        });

        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("squeezeit blit layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("squeezeit blit pipeline layout"),
            bind_group_layouts: &[Some(&blit_layout)],
            immediate_size: 0,
        });

        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("squeezeit blit pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("squeezeit blit sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let bc_supported = device
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC);

        Self {
            shared: Arc::new(Shared {
                device,
                queue,
                blit_pipeline,
                blit_layout,
                sampler,
            }),
            worker: OnceLock::new(),
            ready: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            bc_supported,
        }
    }

    pub(crate) fn supports_bc_source(&self, format: ImageFormat) -> bool {
        self.bc_supported && bc_source_format(format).is_some()
    }

    pub fn create_standalone() -> Result<Self, GpuError> {
        let adapter = pick_adapter()?;
        let features = adapter.features() & wgpu::Features::TEXTURE_COMPRESSION_BC;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: features,
            ..Default::default()
        }))?;
        Ok(Self::new((device, queue)))
    }

    pub fn begin_warm_up(&self) {
        let _ = self.sender();
    }

    pub fn wait_until_ready(&self) -> bool {
        self.begin_warm_up();
        let deadline = std::time::Instant::now() + WARM_UP_TIMEOUT;
        while !self.ready.load(Ordering::Acquire) {
            if std::time::Instant::now() >= deadline {
                tracing::warn!(
                    seconds = WARM_UP_TIMEOUT.as_secs(),
                    "GPU compressor did not finish warming up in time"
                );
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        true
    }

    fn sender(&self) -> &mpsc::SyncSender<Job> {
        self.worker.get_or_init(|| {
            let (tx, rx) = mpsc::sync_channel(queue_depth());
            let shared = Arc::clone(&self.shared);
            let ready = Arc::clone(&self.ready);
            std::thread::Builder::new()
                .name("squeezeit-gpu".into())
                .spawn(move || worker_loop(shared, rx, ready))
                .expect("spawn GPU worker thread");
            tx
        })
    }
}

fn pick_adapter() -> Result<wgpu::Adapter, GpuError> {
    let mut fallback: Option<GpuError> = None;

    for backends in [
        wgpu::Backends::VULKAN,
        wgpu::Backends::from_env().unwrap_or_default(),
    ] {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        descriptor.backends = backends;
        let instance = wgpu::Instance::new(descriptor);
        let requested = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }));

        match requested {
            Ok(adapter) => {
                let info = adapter.get_info();
                if info.device_type == wgpu::DeviceType::Cpu {
                    tracing::debug!(adapter = %info.name, "ignoring software rasterizer");
                    fallback.get_or_insert(GpuError::SoftwareOnly(info.name));
                    continue;
                }
                tracing::debug!(adapter = %info.name, backend = ?info.backend, "GPU adapter selected");
                return Ok(adapter);
            }
            Err(e) => {
                fallback.get_or_insert(GpuError::Adapter(e));
            }
        }
    }

    Err(fallback.unwrap_or(GpuError::SoftwareOnly("none".into())))
}

pub fn select(backend: Backend, gpu: Option<&GpuContext>) -> Option<&GpuContext> {
    match backend {
        Backend::Cpu => None,
        Backend::Gpu => {
            if gpu.is_none() {
                static WARN_ONCE: Once = Once::new();
                WARN_ONCE.call_once(|| {
                    tracing::warn!(
                        "GPU backend selected but no GPU context is available, \
                         falling back to the CPU pipeline"
                    );
                });
            }
            gpu
        }
    }
}

static GPU_COMPRESSED: AtomicU64 = AtomicU64::new(0);
static GPU_BUSY: AtomicU64 = AtomicU64::new(0);
static GPU_FAILURES: AtomicU64 = AtomicU64::new(0);

pub fn counters() -> (u64, u64, u64) {
    (
        GPU_COMPRESSED.load(Ordering::Relaxed),
        GPU_BUSY.load(Ordering::Relaxed),
        GPU_FAILURES.load(Ordering::Relaxed),
    )
}

pub fn supported_mip_levels(width: u32, height: u32, requested: u32) -> u32 {
    let mut levels = 0;
    while levels < requested {
        let (w, h) = (width >> levels, height >> levels);
        if w < 4 || h < 4 || w % 4 != 0 || h % 4 != 0 {
            break;
        }
        levels += 1;
    }
    levels
}

fn bc_source_format(format: ImageFormat) -> Option<wgpu::TextureFormat> {
    Some(match format {
        ImageFormat::BC1RgbaUnorm => wgpu::TextureFormat::Bc1RgbaUnorm,
        ImageFormat::BC2RgbaUnorm => wgpu::TextureFormat::Bc2RgbaUnorm,
        ImageFormat::BC3RgbaUnorm => wgpu::TextureFormat::Bc3RgbaUnorm,
        ImageFormat::BC4RUnorm => wgpu::TextureFormat::Bc4RUnorm,
        ImageFormat::BC5RgUnorm => wgpu::TextureFormat::Bc5RgUnorm,
        ImageFormat::BC7RgbaUnorm => wgpu::TextureFormat::Bc7RgbaUnorm,
        _ => return None,
    })
}

fn bc_block_bytes(format: ImageFormat) -> u32 {
    match format {
        ImageFormat::BC1RgbaUnorm | ImageFormat::BC4RUnorm => 8,
        _ => 16,
    }
}

fn variant_for(format: ImageFormat, quality: Quality) -> Option<CompressionVariant> {
    let bc7 = match quality {
        Quality::Fast => BC7Settings::alpha_fast(),
        Quality::Normal => BC7Settings::alpha_basic(),
        Quality::Slow => BC7Settings::alpha_slow(),
    };
    Some(match format {
        ImageFormat::BC1RgbaUnorm => CompressionVariant::BC1,
        ImageFormat::BC2RgbaUnorm => CompressionVariant::BC2,
        ImageFormat::BC3RgbaUnorm => CompressionVariant::BC3,
        ImageFormat::BC4RUnorm => CompressionVariant::BC4,
        ImageFormat::BC5RgUnorm => CompressionVariant::BC5,
        ImageFormat::BC7RgbaUnorm => CompressionVariant::BC7(bc7),
        _ => return None,
    })
}

struct Job {
    pixels: Vec<u8>,
    source_compressed: Option<(wgpu::TextureFormat, u32)>,
    source_w: u32,
    source_h: u32,
    target_w: u32,
    target_h: u32,
    variant: CompressionVariant,
    mip_levels: u32,
    output_bytes: u64,
    compression_cost: u64,
    reply: mpsc::Sender<JobReply>,
}

type JobReply = Result<Vec<u8>, (GpuError, Vec<u8>)>;

impl Job {
    fn needs_chain(&self) -> bool {
        self.source_compressed.is_some()
            || (self.source_w, self.source_h) != (self.target_w, self.target_h)
            || self.mip_levels > 1
    }

    fn input_vram_bytes(&self) -> u64 {
        let mut bytes = self.pixels.len() as u64;
        if self.needs_chain() {
            bytes += texture_bytes(self.target_w, self.target_h, self.mip_levels);

            let steps = downscale_steps(self.source_w, self.source_h, self.target_w, self.target_h);
            for &(w, h) in &steps[..steps.len() - 1] {
                bytes += texture_bytes(w, h, 1);
            }
        }
        bytes
    }
}

#[allow(clippy::too_many_arguments)]
pub fn resize_and_compress(
    ctx: &GpuContext,
    rgba: Vec<u8>,
    source_w: u32,
    source_h: u32,
    target_w: u32,
    target_h: u32,
    format: ImageFormat,
    quality: Quality,
    mip_levels: u32,
) -> Result<Vec<u8>, (GpuError, Vec<u8>)> {
    if rgba.len() as u64 != 4 * source_w as u64 * source_h as u64 {
        return Err((
            GpuError::Validation("RGBA source size mismatch".into()),
            rgba,
        ));
    }
    dispatch(
        ctx, rgba, None, source_w, source_h, target_w, target_h, format, quality, mip_levels,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn resize_and_compress_bc(
    ctx: &GpuContext,
    blocks: Vec<u8>,
    src_format: ImageFormat,
    source_w: u32,
    source_h: u32,
    target_w: u32,
    target_h: u32,
    out_format: ImageFormat,
    quality: Quality,
    mip_levels: u32,
) -> Result<Vec<u8>, (GpuError, Vec<u8>)> {
    let Some(wgpu_format) = bc_source_format(src_format) else {
        return Err((GpuError::UnsupportedFormat(src_format), blocks));
    };
    let block_bytes = bc_block_bytes(src_format);
    let expected = (source_w as u64 / 4) * (source_h as u64 / 4) * block_bytes as u64;
    if !source_w.is_multiple_of(4) || !source_h.is_multiple_of(4) || blocks.len() as u64 != expected
    {
        return Err((
            GpuError::Validation("BC source dimensions/length unsupported".into()),
            blocks,
        ));
    }
    dispatch(
        ctx,
        blocks,
        Some((wgpu_format, block_bytes)),
        source_w,
        source_h,
        target_w,
        target_h,
        out_format,
        quality,
        mip_levels,
    )
}

#[allow(clippy::too_many_arguments)]
fn dispatch(
    ctx: &GpuContext,
    pixels: Vec<u8>,
    source_compressed: Option<(wgpu::TextureFormat, u32)>,
    source_w: u32,
    source_h: u32,
    target_w: u32,
    target_h: u32,
    format: ImageFormat,
    quality: Quality,
    mip_levels: u32,
) -> Result<Vec<u8>, (GpuError, Vec<u8>)> {
    let Some(variant) = variant_for(format, quality) else {
        return Err((GpuError::UnsupportedFormat(format), pixels));
    };
    let mip_levels = mip_levels.max(1);
    if supported_mip_levels(target_w, target_h, mip_levels) != mip_levels {
        return Err((
            GpuError::Validation("dimensions unsupported by the GPU pipeline".into()),
            pixels,
        ));
    }
    let output_bytes: u64 = (0..mip_levels)
        .map(|level| variant.blocks_byte_size(target_w >> level, target_h >> level) as u64)
        .sum();
    let texels: u64 = (0..mip_levels)
        .map(|level| ((target_w >> level) as u64) * ((target_h >> level) as u64))
        .sum();
    let compression_cost = texels * cost_weight(format, quality);

    let (reply_tx, reply_rx) = mpsc::channel();
    let job = Job {
        pixels,
        source_compressed,
        source_w,
        source_h,
        target_w,
        target_h,
        variant,
        mip_levels,
        output_bytes,
        compression_cost,
        reply: reply_tx,
    };
    let sender = ctx.sender();
    if !ctx.ready.load(Ordering::Acquire) {
        GPU_BUSY.fetch_add(1, Ordering::Relaxed);
        return Err((GpuError::Busy, job.pixels));
    }
    match sender.try_send(job) {
        Ok(()) => {}
        Err(mpsc::TrySendError::Full(job)) => {
            GPU_BUSY.fetch_add(1, Ordering::Relaxed);
            return Err((GpuError::Busy, job.pixels));
        }
        Err(mpsc::TrySendError::Disconnected(job)) => {
            GPU_FAILURES.fetch_add(1, Ordering::Relaxed);
            return Err((GpuError::Readback("GPU worker exited".into()), job.pixels));
        }
    }
    match reply_rx.recv() {
        Ok(Ok(blocks)) => {
            GPU_COMPRESSED.fetch_add(1, Ordering::Relaxed);
            Ok(blocks)
        }
        Ok(Err(failure)) => {
            GPU_FAILURES.fetch_add(1, Ordering::Relaxed);
            Err(failure)
        }
        Err(_) => {
            GPU_FAILURES.fetch_add(1, Ordering::Relaxed);
            Err((GpuError::Readback("GPU worker exited".into()), Vec::new()))
        }
    }
}

fn downscale_steps(source_w: u32, source_h: u32, target_w: u32, target_h: u32) -> Vec<(u32, u32)> {
    let mut steps = Vec::new();
    let (mut w, mut h) = (source_w, source_h);
    loop {
        let next = ((w / 2).max(target_w), (h / 2).max(target_h));
        if next == (w, h) || next == (target_w, target_h) {
            break;
        }
        steps.push(next);
        (w, h) = next;
    }
    steps.push((target_w, target_h));
    steps
}

fn blit(
    device: &wgpu::Device,
    shared: &Shared,
    encoder: &mut wgpu::CommandEncoder,
    source: &wgpu::TextureView,
    target: &wgpu::TextureView,
) {
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("squeezeit blit bind group"),
        layout: &shared.blit_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&shared.sampler),
            },
        ],
    });

    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("squeezeit bilinear blit"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(&shared.blit_pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    pass.draw(0..3, 0..1);
}

fn texture_bytes(width: u32, height: u32, mip_levels: u32) -> u64 {
    (0..mip_levels.max(1))
        .map(|level| 4 * ((width >> level).max(1) as u64) * ((height >> level).max(1) as u64))
        .sum()
}

#[derive(Default)]
struct ResourcePool {
    textures: HashMap<(u32, u32, u32), Vec<wgpu::Texture>>,
    texture_bytes: u64,
    storage: Vec<wgpu::Buffer>,
    staging: Vec<wgpu::Buffer>,
}

impl ResourcePool {
    fn acquire_texture(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        mip_levels: u32,
    ) -> wgpu::Texture {
        if let Some(texture) = self
            .textures
            .get_mut(&(width, height, mip_levels))
            .and_then(Vec::pop)
        {
            self.texture_bytes -= texture_bytes(width, height, mip_levels);
            return texture;
        }
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("squeezeit pooled texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: mip_levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
    }

    fn return_texture(&mut self, key: (u32, u32, u32), texture: wgpu::Texture) {
        let bytes = texture_bytes(key.0, key.1, key.2);
        if self.texture_bytes + bytes > MAX_POOLED_TEXTURE_BYTES {
            return;
        }
        self.texture_bytes += bytes;
        self.textures.entry(key).or_default().push(texture);
    }

    fn acquire_storage(&mut self, device: &wgpu::Device, size: u64) -> wgpu::Buffer {
        Self::acquire_buffer(
            &mut self.storage,
            device,
            size,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            "squeezeit blocks",
        )
    }

    fn acquire_staging(&mut self, device: &wgpu::Device, size: u64) -> wgpu::Buffer {
        Self::acquire_buffer(
            &mut self.staging,
            device,
            size,
            wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            "squeezeit readback",
        )
    }

    fn acquire_buffer(
        pool: &mut Vec<wgpu::Buffer>,
        device: &wgpu::Device,
        size: u64,
        usage: wgpu::BufferUsages,
        label: &str,
    ) -> wgpu::Buffer {
        if let Some(index) = (0..pool.len())
            .filter(|&i| pool[i].size() >= size)
            .min_by_key(|&i| pool[i].size())
        {
            return pool.swap_remove(index);
        }
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: size.next_power_of_two(),
            usage,
            mapped_at_creation: false,
        })
    }

    fn return_storage(&mut self, buffer: wgpu::Buffer) {
        if self.storage.len() <= MAX_IN_FLIGHT {
            self.storage.push(buffer);
        } else if let Some((idx, min_buf)) = self
            .storage
            .iter()
            .enumerate()
            .min_by_key(|(_, b)| b.size())
            && min_buf.size() < buffer.size()
        {
            self.storage[idx] = buffer;
        }
    }

    fn return_staging(&mut self, buffer: wgpu::Buffer) {
        if self.staging.len() <= MAX_IN_FLIGHT {
            self.staging.push(buffer);
        } else if let Some((idx, min_buf)) = self
            .staging
            .iter()
            .enumerate()
            .min_by_key(|(_, b)| b.size())
            && min_buf.size() < buffer.size()
        {
            self.staging[idx] = buffer;
        }
    }
}

struct PendingJob {
    job: Job,
    offset: u64,
    size: u64,
}

type ErrorScopeFuture = Pin<Box<dyn Future<Output = Option<wgpu::Error>>>>;

struct InFlight {
    submitted_at: std::time::Instant,
    jobs: Vec<PendingJob>,
    used: u64,
    staging: wgpu::Buffer,
    blocks: wgpu::Buffer,
    textures: Vec<((u32, u32, u32), wgpu::Texture)>,
    submission: wgpu::SubmissionIndex,
    map_rx: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    validation: ErrorScopeFuture,
    oom: ErrorScopeFuture,
}

struct JobSource {
    rx: mpsc::Receiver<Job>,
    carry: Option<Job>,
    closed: bool,
}

impl JobSource {
    fn next(&mut self, block: bool) -> Option<Job> {
        if let Some(job) = self.carry.take() {
            return Some(job);
        }
        if self.closed {
            return None;
        }
        if block {
            match self.rx.recv() {
                Ok(job) => Some(job),
                Err(_) => {
                    self.closed = true;
                    None
                }
            }
        } else {
            match self.rx.try_recv() {
                Ok(job) => Some(job),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.closed = true;
                    None
                }
            }
        }
    }
}

fn worker_loop(
    shared: Arc<Shared>,
    rx: mpsc::Receiver<Job>,
    ready: Arc<std::sync::atomic::AtomicBool>,
) {
    let started = std::time::Instant::now();
    let mut compressor = GpuBlockCompressor::new(shared.device.clone(), shared.queue.clone());
    ready.store(true, Ordering::Release);
    tracing::debug!(
        elapsed_ms = started.elapsed().as_millis(),
        "GPU compressor warmed up"
    );

    let mut source = JobSource {
        rx,
        carry: None,
        closed: false,
    };
    let mut pool = ResourcePool::default();
    let mut in_flight: VecDeque<InFlight> = VecDeque::new();
    let mut consecutive_failures = 0u32;

    loop {
        while in_flight.len() < MAX_IN_FLIGHT {
            match collect_batch(&mut source, in_flight.is_empty()) {
                Some(jobs) => {
                    let batch = submit_batch(&shared, &mut compressor, &mut pool, jobs);
                    in_flight.push_back(batch);
                }
                None => break,
            }
        }
        match in_flight.pop_front() {
            Some(batch) => {
                let retire_started = std::time::Instant::now();
                let jobs = batch.jobs.len();
                let submitted = batch.submitted_at;
                let ok = retire_batch(&shared, &mut pool, batch);
                tracing::debug!(
                    jobs,
                    wait_ms = retire_started.elapsed().as_millis() as u64,
                    total_ms = submitted.elapsed().as_millis() as u64,
                    ok,
                    "GPU batch retired"
                );
                if ok {
                    consecutive_failures = 0;
                } else {
                    consecutive_failures += 1;
                }
            }
            None if source.closed => return,
            None => {}
        }

        if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
            ready.store(false, Ordering::Release);
            tracing::error!(
                failures = consecutive_failures,
                "GPU worker shutting down after repeated batch failures, \
                 device likely lost; remaining textures run on the CPU"
            );
            for batch in in_flight.drain(..) {
                retire_batch(&shared, &mut pool, batch);
            }
            return;
        }
    }
}

fn collect_batch(source: &mut JobSource, block: bool) -> Option<Vec<Job>> {
    let first = source.next(block)?;
    let mut output_bytes = first.output_bytes;
    let mut vram_bytes = first.input_vram_bytes();
    let mut cost = first.compression_cost;
    let mut jobs = vec![first];

    while jobs.len() < MAX_BATCH_JOBS {
        let Some(job) = source.next(false) else { break };
        if output_bytes + job.output_bytes > MAX_BATCH_OUTPUT_BYTES
            || vram_bytes + job.input_vram_bytes() > MAX_BATCH_INPUT_BYTES
            || cost + job.compression_cost > MAX_BATCH_COST
        {
            source.carry = Some(job);
            break;
        }
        output_bytes += job.output_bytes;
        vram_bytes += job.input_vram_bytes();
        cost += job.compression_cost;
        jobs.push(job);
    }
    Some(jobs)
}

fn mip_view(texture: &wgpu::Texture, level: u32) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        base_mip_level: level,
        mip_level_count: Some(1),
        ..Default::default()
    })
}

fn submit_batch(
    shared: &Shared,
    compressor: &mut GpuBlockCompressor,
    pool: &mut ResourcePool,
    jobs: Vec<Job>,
) -> InFlight {
    let device = &shared.device;
    let oom_scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let validation_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

    let total: u64 = jobs.iter().map(|job| job.output_bytes).sum();
    let blocks = pool.acquire_storage(device, total);
    let staging = pool.acquire_staging(device, total);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("squeezeit gpu batch"),
    });

    let mut pending = Vec::with_capacity(jobs.len());
    let mut textures = Vec::new();
    let mut offset = 0u64;

    for job in jobs {
        let source_texture = match job.source_compressed {
            Some((format, block_bytes)) => {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("squeezeit bc source"),
                    size: wgpu::Extent3d {
                        width: job.source_w,
                        height: job.source_h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                shared.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &job.pixels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some((job.source_w / 4) * block_bytes),
                        rows_per_image: Some(job.source_h / 4),
                    },
                    wgpu::Extent3d {
                        width: job.source_w,
                        height: job.source_h,
                        depth_or_array_layers: 1,
                    },
                );
                texture
            }
            None => {
                let texture = pool.acquire_texture(device, job.source_w, job.source_h, 1);
                shared.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &job.pixels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4 * job.source_w),
                        rows_per_image: Some(job.source_h),
                    },
                    wgpu::Extent3d {
                        width: job.source_w,
                        height: job.source_h,
                        depth_or_array_layers: 1,
                    },
                );
                texture
            }
        };

        let chain_texture = job
            .needs_chain()
            .then(|| pool.acquire_texture(device, job.target_w, job.target_h, job.mip_levels));

        if let Some(chain) = &chain_texture {
            let steps = downscale_steps(job.source_w, job.source_h, job.target_w, job.target_h);
            let (intermediate, final_step) = steps.split_at(steps.len() - 1);

            let mut current = source_texture.create_view(&Default::default());
            for &(w, h) in intermediate {
                let scratch = pool.acquire_texture(device, w, h, 1);
                let view = scratch.create_view(&Default::default());
                blit(device, shared, &mut encoder, &current, &view);
                current = view;
                textures.push(((w, h, 1), scratch));
            }
            debug_assert_eq!(final_step, [(job.target_w, job.target_h)]);
            blit(device, shared, &mut encoder, &current, &mip_view(chain, 0));

            for level in 1..job.mip_levels {
                blit(
                    device,
                    shared,
                    &mut encoder,
                    &mip_view(chain, level - 1),
                    &mip_view(chain, level),
                );
            }
        }

        let job_start = offset;
        for level in 0..job.mip_levels {
            let (w, h) = (job.target_w >> level, job.target_h >> level);
            let view = match &chain_texture {
                Some(chain) => mip_view(chain, level),
                None => source_texture.create_view(&Default::default()),
            };
            compressor.add_compression_task(
                job.variant,
                &view,
                w,
                h,
                &blocks,
                None,
                Some(offset as u32),
            );
            offset += job.variant.blocks_byte_size(w, h) as u64;
        }

        match job.source_compressed {
            Some(_) => {}
            None => textures.push(((job.source_w, job.source_h, 1), source_texture)),
        }
        if let Some(chain) = chain_texture {
            textures.push(((job.target_w, job.target_h, job.mip_levels), chain));
        }
        pending.push(PendingJob {
            offset: job_start,
            size: offset - job_start,
            job,
        });
    }

    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("squeezeit block compression"),
            timestamp_writes: None,
        });
        compressor.compress(&mut pass);
    }

    encoder.copy_buffer_to_buffer(&blocks, 0, &staging, 0, total);
    let submission = shared.queue.submit([encoder.finish()]);

    let validation = Box::pin(validation_scope.pop()) as ErrorScopeFuture;
    let oom = Box::pin(oom_scope.pop()) as ErrorScopeFuture;

    let (map_tx, map_rx) = mpsc::channel();
    staging
        .slice(0..total)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = map_tx.send(result);
        });

    InFlight {
        submitted_at: std::time::Instant::now(),
        jobs: pending,
        used: total,
        staging,
        blocks,
        textures,
        submission,
        map_rx,
        validation,
        oom,
    }
}

fn retire_batch(shared: &Shared, pool: &mut ResourcePool, batch: InFlight) -> bool {
    let InFlight {
        submitted_at: _,
        jobs,
        used,
        staging,
        blocks,
        textures,
        submission,
        map_rx,
        validation,
        oom,
    } = batch;

    let waited = shared.device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: Some(SUBMISSION_TIMEOUT),
    });
    let mapped = match &waited {
        Ok(_) => map_rx.recv(),
        Err(_) => map_rx.try_recv().map_err(|_| mpsc::RecvError),
    };

    let error = if let Some(e) = block_on(validation) {
        Some(GpuError::Validation(e.to_string()))
    } else if let Some(e) = block_on(oom) {
        Some(GpuError::OutOfMemory(e.to_string()))
    } else if let Err(e) = waited {
        Some(GpuError::Readback(e.to_string()))
    } else {
        match &mapped {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(GpuError::Readback(e.to_string())),
            Err(_) => Some(GpuError::Readback("map callback dropped".into())),
        }
    };

    let succeeded = error.is_none();
    match error {
        Some(error) => {
            tracing::warn!(%error, jobs = jobs.len(), "GPU batch failed, jobs fall back to CPU");
            for pending in jobs {
                let failure = clone_error(&error);
                let _ = pending.job.reply.send(Err((failure, pending.job.pixels)));
            }
            if matches!(mapped, Ok(Ok(()))) {
                staging.unmap();
            }
        }
        None => {
            {
                let data = staging.slice(0..used).get_mapped_range();
                for pending in jobs {
                    let start = pending.offset as usize;
                    let end = start + pending.size as usize;
                    let _ = pending.job.reply.send(Ok(data[start..end].to_vec()));
                }
            }
            staging.unmap();
        }
    }

    if succeeded {
        for (key, texture) in textures {
            pool.return_texture(key, texture);
        }
        pool.return_storage(blocks);
        pool.return_staging(staging);
    }
    succeeded
}

fn clone_error(error: &GpuError) -> GpuError {
    match error {
        GpuError::Validation(m) => GpuError::Validation(m.clone()),
        GpuError::OutOfMemory(m) => GpuError::OutOfMemory(m.clone()),
        GpuError::Busy => GpuError::Busy,
        other => GpuError::Readback(other.to_string()),
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::squeezers::codec;
    use image::{Rgba, RgbaImage};

    fn context() -> Option<&'static GpuContext> {
        static CTX: std::sync::OnceLock<Option<GpuContext>> = std::sync::OnceLock::new();
        CTX.get_or_init(|| GpuContext::create_standalone().ok())
            .as_ref()
    }

    #[allow(clippy::too_many_arguments)]
    fn compress_retrying(
        ctx: &GpuContext,
        rgba: &[u8],
        sw: u32,
        sh: u32,
        tw: u32,
        th: u32,
        format: ImageFormat,
        quality: Quality,
        mips: u32,
    ) -> Result<Vec<u8>, GpuError> {
        let mut pixels = rgba.to_vec();
        loop {
            match resize_and_compress(ctx, pixels, sw, sh, tw, th, format, quality, mips) {
                Err((GpuError::Busy, returned)) => {
                    pixels = returned;
                    std::thread::yield_now();
                }
                Err((error, _)) => return Err(error),
                Ok(blocks) => return Ok(blocks),
            }
        }
    }

    #[test]
    fn gpu_bc_source_decode_roundtrip() {
        let Some(ctx) = context() else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        if !ctx.bc_supported {
            eprintln!("no TEXTURE_COMPRESSION_BC feature, skipping");
            return;
        }
        let src = RgbaImage::from_pixel(64, 64, Rgba([200, 40, 60, 255]));
        let blocks = codec::encode_block(&src, ImageFormat::BC3RgbaUnorm, Quality::Normal, 1)
            .expect("cpu bc3 encode")
            .data;

        let mut data = blocks;
        let out = loop {
            match resize_and_compress_bc(
                ctx,
                data,
                ImageFormat::BC3RgbaUnorm,
                64,
                64,
                32,
                32,
                ImageFormat::BC1RgbaUnorm,
                Quality::Normal,
                1,
            ) {
                Err((GpuError::Busy, back)) => {
                    data = back;
                    std::thread::yield_now();
                }
                Err((error, _)) => panic!("bc source pipeline: {error}"),
                Ok(blocks) => break blocks,
            }
        };
        assert_eq!(out.len(), 8 * 8 * 8);
        let decoded = codec::decode_block(&out, ImageFormat::BC1RgbaUnorm, 32, 32).unwrap();
        let px = decoded.get_pixel(16, 16);
        assert!(
            px.0[0] > 150 && px.0[1] < 100 && px.0[2] < 110,
            "decoded color off: {px:?}"
        );
    }

    #[test]
    fn a_downscale_never_takes_more_than_half_in_one_step() {
        for (sw, sh, tw, th) in [
            (4096, 4096, 1024, 1024),
            (4096, 4096, 512, 512),
            (2048, 1024, 512, 256),
            (2048, 2048, 1024, 1024),
            (1500, 1500, 1024, 1024),
            (3000, 3000, 1024, 1024),
            (4096, 8, 512, 4),
            (512, 512, 512, 512),
        ] {
            let steps = downscale_steps(sw, sh, tw, th);
            assert_eq!(
                *steps.last().unwrap(),
                (tw, th),
                "{sw}x{sh} to {tw}x{th} did not land on the target"
            );

            let mut from = (sw, sh);
            for &to in &steps {
                assert!(
                    to.0 * 2 >= from.0 && to.1 * 2 >= from.1,
                    "{sw}x{sh} to {tw}x{th}: step {from:?} to {to:?} drops more than half, \
                     which bilinear cannot filter"
                );
                assert!(to.0 <= from.0 && to.1 <= from.1, "a step grew the texture");
                from = to;
            }
        }
    }

    #[test]
    fn gpu_downscale_filters_rather_than_samples() {
        let Some(ctx) = context() else {
            eprintln!("no GPU adapter, skipping");
            return;
        };

        let mut seed = 0x9E37_79B9u32;
        let source = RgbaImage::from_fn(512, 512, |_, _| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let bit = if seed >> 31 == 0 { 0 } else { 255 };
            Rgba([bit, bit, bit, 255])
        });

        let data = compress_retrying(
            ctx,
            source.as_raw(),
            512,
            512,
            64,
            64,
            ImageFormat::BC1RgbaUnorm,
            Quality::Slow,
            1,
        )
        .expect("GPU pipeline");
        let gpu = codec::decode_block(&data, ImageFormat::BC1RgbaUnorm, 64, 64).unwrap();
        let cpu = codec::resize(source, 64, 64);

        let spread = |img: &RgbaImage| -> f64 {
            let values: Vec<f64> = img
                .as_raw()
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| f64::from(p[0]))
                .collect();
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64).sqrt()
        };

        let (gpu_spread, cpu_spread) = (spread(&gpu), spread(&cpu));
        assert!(
            gpu_spread < cpu_spread * 2.0,
            "GPU spread {gpu_spread:.1} against CPU {cpu_spread:.1}: the GPU is keeping \
             contrast a filtered downscale would have averaged away, so it is sampling"
        );
    }

    #[test]
    fn gpu_resize_and_bc1_roundtrip() {
        let Some(ctx) = context() else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        let image = RgbaImage::from_pixel(64, 64, Rgba([255, 0, 0, 255]));
        let data = compress_retrying(
            ctx,
            image.as_raw(),
            64,
            64,
            32,
            32,
            ImageFormat::BC1RgbaUnorm,
            Quality::Normal,
            1,
        )
        .expect("GPU pipeline");
        assert_eq!(data.len(), 8 * 8 * 8);

        let decoded = codec::decode_block(&data, ImageFormat::BC1RgbaUnorm, 32, 32).unwrap();
        let px = decoded.get_pixel(16, 16);
        assert!(px.0[0] > 200 && px.0[1] < 60 && px.0[2] < 60, "{:?}", px);
    }

    #[test]
    fn gpu_mip_chain_sizes_match_cpu_layout() {
        let Some(ctx) = context() else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        let image = RgbaImage::from_pixel(128, 128, Rgba([0, 255, 0, 255]));
        let mip_levels = 4;
        let data = compress_retrying(
            ctx,
            image.as_raw(),
            128,
            128,
            64,
            64,
            ImageFormat::BC7RgbaUnorm,
            Quality::Fast,
            mip_levels,
        )
        .expect("GPU pipeline");
        let blocks: u32 = [64u32, 32, 16, 8].iter().map(|d| (d / 4) * (d / 4)).sum();
        assert_eq!(data.len(), blocks as usize * 16);
    }

    #[test]
    fn gpu_concurrent_jobs_batch_correctly() {
        let Some(ctx) = context() else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        std::thread::scope(|scope| {
            for round in 0u8..24 {
                scope.spawn(move || {
                    let color = Rgba([round * 10, 255 - round * 10, 40, 255]);
                    let image = RgbaImage::from_pixel(64, 64, color);
                    let data = compress_retrying(
                        ctx,
                        image.as_raw(),
                        64,
                        64,
                        32,
                        32,
                        ImageFormat::BC1RgbaUnorm,
                        Quality::Normal,
                        1,
                    )
                    .expect("GPU pipeline");
                    let decoded =
                        codec::decode_block(&data, ImageFormat::BC1RgbaUnorm, 32, 32).unwrap();
                    let px = decoded.get_pixel(16, 16);
                    for (got, want) in px.0.iter().zip(color.0.iter()) {
                        assert!(
                            (*got as i16 - *want as i16).abs() < 24,
                            "round {round}: {:?} vs {:?}",
                            px,
                            color
                        );
                    }
                });
            }
        });
    }

    #[test]
    fn never_accepts_a_software_rasterizer() {
        if let Ok(adapter) = pick_adapter() {
            let info = adapter.get_info();
            assert_ne!(
                info.device_type,
                wgpu::DeviceType::Cpu,
                "picked software rasterizer {}, which is slower than the CPU pipeline",
                info.name
            );
        }
    }

    #[test]
    fn mip_clamp_stops_at_4x4_and_rejects_misaligned() {
        assert_eq!(supported_mip_levels(1024, 1024, u32::MAX), 9);
        assert_eq!(supported_mip_levels(1024, 512, u32::MAX), 8);
        assert_eq!(supported_mip_levels(64, 64, 4), 4);
        assert_eq!(supported_mip_levels(1000, 1000, u32::MAX), 2);
        assert_eq!(supported_mip_levels(6, 6, u32::MAX), 0);
        assert_eq!(supported_mip_levels(4, 4, u32::MAX), 1);
    }
}
