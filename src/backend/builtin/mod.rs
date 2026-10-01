//! Built-in backends shipped with the crate.

pub mod ene_i2c;
pub mod openrgb;

use super::Backend;

/// All compiled-in backends. Order here is only a fallback; the applied order
/// comes from `backends.order` in the config.
#[must_use]
pub fn builtin_backends() -> Vec<Box<dyn Backend>> {
    vec![
        Box::<openrgb::OpenRgbBackend>::default(),
        Box::<ene_i2c::EneI2cBackend>::default(),
    ]
}
