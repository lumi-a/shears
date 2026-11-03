//! WASM bindings for shears.
//!
//! Build with `wasm-bindgen`.

#![expect(
    clippy::missing_inline_in_public_items,
    reason = "wasm-bindgen exported functions can't be #[inline]: it's ignored on externally \
              exported functions, and emits its own warning if added"
)]

use shears_core::rules::RuleSet;
use wasm_bindgen::prelude::wasm_bindgen;

/// Just a placeholder
#[must_use]
#[wasm_bindgen]
pub fn parses_empty_ruleset() -> bool {
    RuleSet::parse("").is_ok()
}
