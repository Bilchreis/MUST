// SPDX-License-Identifier: LGPL-3.0-or-later

//! Procedural macros of `must-server`. Use them through `must-server`, which re-exports
//! them with their documentation: `#[must_server::module]`, `#[must_server::methods]`,
//! `#[derive(must_server::SecopEnum)]` and `#[derive(must_server::SecopStruct)]`.

mod datainfo;
mod derive;
mod methods;
mod module;

use proc_macro::TokenStream;

fn output(result: syn::Result<proc_macro2::TokenStream>) -> TokenStream {
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}

/// Declares a struct a typed module; see `must_server::module`.
#[proc_macro_attribute]
pub fn module(attr: TokenStream, item: TokenStream) -> TokenStream {
    module::expand(attr.into(), item.into()).into()
}

/// The behaviour methods of a typed module; see `must_server::methods`.
#[proc_macro_attribute]
pub fn methods(attr: TokenStream, item: TokenStream) -> TokenStream {
    output(methods::expand_attribute(&attr.into(), item.into()))
}

/// Generates a typed module's `Methods` implementation from its declaration and its
/// methods block; called by the callback macro that `#[module]` generates.
#[doc(hidden)]
#[proc_macro]
pub fn __methods_impl(input: TokenStream) -> TokenStream {
    output(methods::expand_impl(input.into()))
}

/// `SecopType` for a field-less enum; see `must_server::SecopEnum`.
#[proc_macro_derive(SecopEnum)]
pub fn secop_enum(input: TokenStream) -> TokenStream {
    output(derive::secop_enum(input.into()))
}

/// `SecopType` for a struct with named fields; see `must_server::SecopStruct`.
#[proc_macro_derive(SecopStruct, attributes(datainfo))]
pub fn secop_struct(input: TokenStream) -> TokenStream {
    output(derive::secop_struct(input.into()))
}
