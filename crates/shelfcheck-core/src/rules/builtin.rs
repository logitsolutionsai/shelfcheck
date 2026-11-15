//! The built-in rule packs, embedded at build time from `rules/`.

use super::Pack;
use crate::error::Result;

const OPENAI: &str = include_str!("../../rules/openai-product-feed/checked-2026-11-14.toml");
const ACP: &str = include_str!("../../rules/acp-feed/2026-04-17.toml");
const GOOGLE: &str = include_str!("../../rules/google-merchant/checked-2026-11-14.toml");
const GS1: &str = include_str!("../../rules/gs1-gtin/genspecs-26.0.toml");
const CROSS: &str = include_str!("../../rules/cross-format/checked-2026-11-14.toml");

/// Built-in packs in report order: OpenAI, ACP, Google, GS1, cross-format.
pub fn packs() -> Result<Vec<Pack>> {
    let mut out = Vec::with_capacity(5);
    for (text, name) in [
        (OPENAI, "rules/openai-product-feed/checked-2026-11-14.toml"),
        (ACP, "rules/acp-feed/2026-04-17.toml"),
        (GOOGLE, "rules/google-merchant/checked-2026-11-14.toml"),
        (GS1, "rules/gs1-gtin/genspecs-26.0.toml"),
        (CROSS, "rules/cross-format/checked-2026-11-14.toml"),
    ] {
        let mut p = Pack::parse(text, name)
            .map_err(|e| crate::error::Error::internal(format!("built-in pack: {e}")))?;
        p.origin = "built-in".to_string();
        out.push(p);
    }
    Ok(out)
}
