//! Proc-macros used by `iodilos`.
//!
//! This crate provides the `view!` macro, the `#[derive(Props)]` builder
//! derive, and the `#[component]` attribute (ported from `sycamore-macro` for
//! strict API alignment). The view-syntax parser (originally
//! `sycamore-view-parser`) is inlined so that iodilos-macros is self-contained.

#![deny(missing_debug_implementations)]

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

mod codegen;
mod component;
mod inline_props;
mod ir;
mod parse;
mod props;
mod util;

/// Create a TUI view using the declarative `view!` syntax.
#[proc_macro]
pub fn view(input: TokenStream) -> TokenStream {
    let root = parse_macro_input!(input as crate::ir::Root);
    codegen::Codegen::new().root(&root).into()
}

/// A macro for creating components from functions.
///
/// Add this attribute to a `fn` to mark it as a component that can be invoked
/// from the [`view!`](macro@view) macro. An optional `inline_props` argument
/// (`#[component(inline_props)]`) synthesises a `{}_Props` struct from the fn
/// parameters so that no separate `#[derive(Props)]` struct is needed.
///
/// Ported from `sycamore-macro`'s `#[component]`, with the async-component
/// branch dropped: iodilos components are synchronous `View` constructors; use
/// [`iodilos::use_future`] for async work.
#[proc_macro_attribute]
pub fn component(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as component::ComponentArgs);

    component::component_impl(args, item.clone().into())
        .unwrap_or_else(|err| {
            // If proc-macro errors, emit the original function for better IDE support.
            let error_tokens = err.into_compile_error();
            let body_input = proc_macro2::TokenStream::from(item);
            quote::quote! {
                #body_input
                #error_tokens
            }
        })
        .into()
}

/// The derive macro for `Props`. Generates the typed builder API used by the
/// [`view!`](macro@view) macro when invoking components.
///
/// Ported from `sycamore-macro`'s `Props` derive (credited to
/// <https://github.com/idanarye/rust-typed-builder>), with the HTML/SVG
/// `#[prop(attributes(..))]` machinery stripped — iodilos components do not
/// carry an `attributes` field.
#[proc_macro_derive(Props, attributes(prop))]
pub fn derive_props(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    props::impl_derive_props(&input)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}
