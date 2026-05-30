// i18n moved to hydra-core::i18n so core renderers (e.g.
// SetupReport::render in coding_plan/setup.rs) can localise their
// output. This module is a re-export shim so all existing
// `crate::i18n::{t, Msg, Locale, …}` call sites keep compiling
// unchanged.
pub use hydra_core::i18n::*;
