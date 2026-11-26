//! Application icons of the remembered windows.
//!
//! Windows: the real icons of the EXEs (`native`). Elsewhere (screenshot
//! demo only): generated colored badges (`badge`).

#[cfg(windows)]
mod native;
#[cfg(windows)]
pub use native::Icons;

#[cfg(not(windows))]
mod badge;
#[cfg(not(windows))]
pub use badge::Icons;
