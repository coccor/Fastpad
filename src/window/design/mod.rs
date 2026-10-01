//! FastPad's design tokens: the shared metrics, the type ramp, and (in tests) the contrast
//! helpers that guard the palettes. One owner per decision, so a look can change in one place.

pub(crate) mod metrics;
pub(crate) mod round;

#[cfg(test)]
pub(crate) mod contrast;

pub(crate) mod type_ramp;
