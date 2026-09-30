//! FastPad's design tokens: the shared metrics, the type ramp, and (in tests) the contrast
//! helpers that guard the palettes. One owner per decision, so a look can change in one place.

pub(crate) mod metrics;

#[cfg(test)]
pub(crate) mod contrast;
