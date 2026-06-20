//! Reactive primitives, vendored from `sycamore-reactive`.
//!
//! This module is an inlined copy of the `sycamore-reactive` crate (0.9.2) so that
//! iodilos is self-contained with no external `sycamore-*` runtime dependency.
//!
//! ```rust
//! use iodilos::reactive::*;
//!
//! create_root(|| {
//!     let greeting = create_signal("Hello");
//!     let name = create_signal("World");
//!
//!     let display_text = create_memo(move || format!("{greeting} {name}!"));
//!     assert_eq!(display_text.get_clone(), "Hello World!");
//!
//!     name.set("Sycamore");
//!     assert_eq!(display_text.get_clone(), "Hello Sycamore!");
//! });
//! ```

mod context;
mod effects;
mod iter;
mod maybe_dyn;
mod memos;
mod node;
mod root;
mod signals;
mod utils;

pub use context::*;
pub use effects::*;
pub use iter::*;
pub use maybe_dyn::*;
pub use memos::*;
pub use node::*;
pub use root::*;
pub use signals::*;
pub use utils::*;
