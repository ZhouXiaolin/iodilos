//! Proc-macros used by `iodilos`.
//!
//! This crate provides the `view!` macro and inlines the view-syntax parser
//! (originally `sycamore-view-parser`) so that iodilos-macros is self-contained.

#![deny(missing_debug_implementations)]

use proc_macro::TokenStream;
use syn::parse_macro_input;

mod codegen;
mod ir;
mod parse;

/// Create a TUI view using the declarative `view!` syntax.
#[proc_macro]
pub fn view(input: TokenStream) -> TokenStream {
    let root = parse_macro_input!(input as crate::ir::Root);
    codegen::Codegen::new().root(&root).into()
}
