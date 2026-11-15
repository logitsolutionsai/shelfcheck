//! shelfcheck core: the record model, feed readers, column maps, the rule
//! engine with its packs, `diff`, and the results model.
//!
//! Every rule is code and data; nothing here calls a network or a language
//! model, and the same feed always gets the same result.

pub mod attributes;
pub mod check;
pub mod config;
pub mod diff;
pub mod error;
pub mod gtin;
pub mod images;
pub mod map;
pub mod model;
pub mod price;
pub mod read;
pub mod results;
pub mod rules;

pub use error::{Error, Result};
pub use results::{ResultsFile, VERSION};
