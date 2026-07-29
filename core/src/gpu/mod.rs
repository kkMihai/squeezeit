#[cfg(feature = "gpu")]
mod imp;
#[cfg(feature = "gpu")]
pub use imp::*;

#[cfg(not(feature = "gpu"))]
mod dummy;
#[cfg(not(feature = "gpu"))]
pub use dummy::*;
