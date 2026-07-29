use std::path::PathBuf;

use thiserror::Error;

pub type Result<T, E = SqueezeError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum SqueezeError {
    #[error("I/O failure on `{path}`")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to decode `{path}` as an image")]
    Decode {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },

    #[error("failed to parse `{path}` as a DDS container")]
    DdsParse {
        path: PathBuf,
        #[source]
        source: image_dds::ddsfile::Error,
    },

    #[error("failed to decompress the DDS surface in `{path}`")]
    DdsDecode {
        path: PathBuf,
        #[source]
        source: image_dds::error::CreateImageError,
    },

    #[error("block compression failed for `{path}`")]
    Encode {
        path: PathBuf,
        #[source]
        source: image_dds::CreateDdsError,
    },

    #[error("the block compression pipeline produced no output for `{path}`")]
    EncodeFailed { path: PathBuf },

    #[error("failed to serialize the DDS container for `{path}`")]
    DdsWrite {
        path: PathBuf,
        #[source]
        source: image_dds::ddsfile::Error,
    },

    #[error("re-encoding `{path}` in its source format failed")]
    ReEncode {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },

    #[error("RSC7 container error in `{path}`: {detail}")]
    Rsc7 { path: PathBuf, detail: String },

    #[error("key extraction failed for `{path}`: {detail}")]
    Keys { path: PathBuf, detail: String },

    #[error("no registered squeezer claims `{0}`")]
    NoSqueezer(PathBuf),

    #[error("`{path}` resolves outside the workspace root; refusing to touch it")]
    OutsideRoot { path: PathBuf },
}

impl SqueezeError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
