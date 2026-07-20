pub mod error;
pub mod gpu;
pub mod gta_keys;
pub mod pipeline;
pub mod platform;
pub mod settings;
pub mod squeezers;

pub use pipeline::{backup, batch, report};
pub use squeezers::games::{gta5, rpf};
pub use squeezers::{standard, texture};

pub use backup::BackupVault;
pub use error::{Result, SqueezeError};
pub use gpu::{GpuContext, GpuError};
pub use report::{ReportSnapshot, SqueezeReport};
pub use settings::{ProcessingBackend, Quality, SqueezeSettings};
pub use squeezers::{
    HybridSqueezer, SqueezeOutcome, Squeezer, SqueezerRegistry, TextureJob, WithGpu,
};
pub use texture::TextureRole;
