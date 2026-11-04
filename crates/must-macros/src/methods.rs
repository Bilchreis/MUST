// SPDX-License-Identifier: LGPL-3.0-or-later

//! `#[methods]` and the hidden `__methods_impl!`: the behaviour of a typed module.
//!
//! `#[methods]` cannot see the module's declaration, so it hands the impl block to the
//! callback macro `__must_module_<Struct>!` that `#[module]` generated next to the
//! struct. That macro calls `__methods_impl!` with the declaration and the impl block,
//! which sees both and generates the `Methods` implementation.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, ImplItem, ItemImpl, Path, Token, Type, bracketed, parenthesized};

pub fn expand_attribute(attr: &TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    if !attr.is_empty() {
        return Err(syn::Error::new(
            Span::call_site(),
            "`#[must_server::methods]` takes no arguments",
        ));
    }
    let block: ItemImpl = syn::parse2(item)?;
    if let Some((_, path, _)) = &block.trait_ {
        return Err(syn::Error::new_spanned(
            path,
            "`#[must_server::methods]` applies to an inherent impl block (`impl MyModule { ... }`)",
        ));
    }
    let Type::Path(self_ty) = &*block.self_ty else {
        return Err(syn::Error::new_spanned(
            &block.self_ty,
            "the methods block must name a module type declared with `#[must_server::module]`",
        ));
    };
    let mut path = self_ty.path.clone();
    let last = path.segments.last_mut().expect("a type path has a segment");
    last.ident = format_ident!("__must_module_{}", last.ident);
    last.arguments = syn::PathArguments::None;
    Ok(quote! { #path! { #block } })
}

/// A parameter of the declaration: name and whether it is writable.
struct DeclParam {
    name: Ident,
    writable: bool,
}

/// A command of the declaration.
struct DeclCommand {
    name: Ident,
    argument: bool,
    result: bool,
}

struct Input {
    krate: Path,
    module: Ident,
    params: Vec<DeclParam>,
    commands: Vec<DeclCommand>,
    block: ItemImpl,
}

fn keyword(input: ParseStream, word: &str) -> syn::Result<()> {
    let ident: Ident = syn::ext::IdentExt::parse_any(input)?;
    if ident != word {
        return Err(syn::Error::new(ident.span(), format!("expected `{word}`")));
    }
    input.parse::<Token![=]>()?;
    Ok(())
}

impl Parse for DeclParam {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let mode: Ident = input.parse()?;
        Ok(Self {
            name,
            writable: mode == "writable",
        })
    }
}

impl Parse for DeclCommand {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        let content;
        parenthesized!(content in input);
        let argument: Ident = content.parse()?;
        content.parse::<Token![,]>()?;
        let result: Ident = content.parse()?;
        Ok(Self {
            name,
            argument: argument == "argument",
            result: result == "result",
        })
    }
}

impl Parse for Input {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        keyword(input, "crate")?;
        let krate: Path = input.parse()?;
        input.parse::<Token![;]>()?;
        keyword(input, "module")?;
        let module: Ident = input.parse()?;
        input.parse::<Token![;]>()?;
        keyword(input, "params")?;
        let content;
        bracketed!(content in input);
        let params: Punctuated<DeclParam, Token![,]> = Punctuated::parse_terminated(&content)?;
        input.parse::<Token![;]>()?;
        keyword(input, "commands")?;
        let content;
        bracketed!(content in input);
        let commands: Punctuated<DeclCommand, Token![,]> = Punctuated::parse_terminated(&content)?;
        input.parse::<Token![;]>()?;
        let block: ItemImpl = input.parse()?;
        Ok(Self {
            krate,
            module,
            params: params.into_iter().collect(),
            commands: commands.into_iter().collect(),
            block,
        })
    }
}

/// A span that resolves like the generated code but points at `ident` in errors.
fn at(ident: &Ident) -> Span {
    Span::call_site().located_at(ident.span())
}

fn combine(errors: &mut Option<syn::Error>, e: syn::Error) {
    match errors {
        Some(all) => all.combine(e),
        None => *errors = Some(e),
    }
}

#[allow(clippy::too_many_lines)]
pub fn expand_impl(tokens: TokenStream) -> syn::Result<TokenStream> {
    let Input {
        krate,
        module,
        params,
        commands,
        block,
    } = syn::parse2(tokens)?;
    let mut block = block;
    // behaviour methods must be async even if they do not await, e.g. a simulation
    for item in &mut block.items {
        if let ImplItem::Fn(f) = item {
            let name = f.sig.ident.to_string();
            if name == "start"
                || name == "poll"
                || name.starts_with("read_")
                || name.starts_with("write_")
                || name.starts_with("do_")
            {
                f.attrs
                    .push(syn::parse_quote!(#[allow(clippy::unused_async, clippy::unused_async_trait_impl)]));
            }
        }
    }
    let mut errors = None;
    let mut reads = Vec::new();
    let mut writes = Vec::new();
    let mut dos = Vec::new();
    let mut start = None;
    let mut poll = None;
    for item in &block.items {
        let ImplItem::Fn(f) = item else { continue };
        let ident = &f.sig.ident;
        let name = ident.to_string();
        if name == "start" {
            start = Some(ident.clone());
        } else if name == "poll" {
            poll = Some(ident.clone());
        } else if let Some(p) = name.strip_prefix("read_") {
            match params.iter().find(|d| d.name == p) {
                Some(d) => reads.push((d, ident.clone())),
                None => combine(
                    &mut errors,
                    syn::Error::new(
                        ident.span(),
                        format!("`{module}` has no parameter `{p}` for `{name}` to read"),
                    ),
                ),
            }
        } else if let Some(p) = name.strip_prefix("write_") {
            match params.iter().find(|d| d.name == p) {
                Some(d) if d.writable => writes.push((d, ident.clone())),
                Some(_) => combine(
                    &mut errors,
                    syn::Error::new(
                        ident.span(),
                        format!("`{p}` is readonly: `{name}` needs `{p}` declared `writable`"),
                    ),
                ),
                None => combine(
                    &mut errors,
                    syn::Error::new(
                        ident.span(),
                        format!("`{module}` has no parameter `{p}` for `{name}` to write"),
                    ),
                ),
            }
        } else if let Some(c) = name.strip_prefix("do_") {
            match commands.iter().find(|d| d.name == c) {
                Some(d) => dos.push((d, ident.clone())),
                None => combine(
                    &mut errors,
                    syn::Error::new(
                        ident.span(),
                        format!("`{module}` has no command `{c}` for `{name}`"),
                    ),
                ),
            }
        }
    }
    for command in &commands {
        if !dos.iter().any(|(d, _)| d.name == command.name) {
            combine(
                &mut errors,
                syn::Error::new(
                    command.name.span(),
                    format!(
                        "the command `{}` needs a method `do_{}` in the `#[must_server::methods]` block",
                        command.name, command.name
                    ),
                ),
            );
        }
    }
    let self_ty = &block.self_ty;
    if let Some(e) = errors {
        // the block and an empty implementation, so that only the real error shows
        let error = e.into_compile_error();
        return Ok(quote! {
            #error
            #block
            impl #krate::typed::Methods for #self_ty {
                const READ: &'static [&'static str] = &[];
                const WRITE: &'static [&'static str] = &[];
                fn read_param<'__a>(
                    &'__a mut self,
                    _: &str,
                    _: &#krate::typed::DataInfo,
                ) -> ::core::option::Option<
                    #krate::typed::BoxFuture<'__a, #krate::typed::Result<#krate::typed::Value>>,
                > {
                    ::core::option::Option::None
                }
                fn write_param<'__a>(
                    &'__a mut self,
                    _: &str,
                    _: #krate::typed::Value,
                    _: &#krate::typed::DataInfo,
                ) -> ::core::option::Option<
                    #krate::typed::BoxFuture<'__a, #krate::typed::Result<#krate::typed::Value>>,
                > {
                    ::core::option::Option::None
                }
                fn call_command<'__a>(
                    &'__a mut self,
                    _: &str,
                    _: ::core::option::Option<#krate::typed::Value>,
                    _: ::core::option::Option<&#krate::typed::DataInfo>,
                    _: ::core::option::Option<&#krate::typed::DataInfo>,
                ) -> #krate::typed::BoxFuture<
                    '__a,
                    #krate::typed::Result<::core::option::Option<#krate::typed::Value>>,
                > {
                    ::std::boxed::Box::pin(async {
                        ::core::result::Result::Err(#krate::typed::support::unknown("command", "?"))
                    })
                }
            }
        });
    }

    let read_names = reads.iter().map(|(d, _)| d.name.to_string());
    let write_names = writes.iter().map(|(d, _)| d.name.to_string());
    let read_arms = reads.iter().map(|(d, method)| {
        let key = d.name.to_string();
        let field = &d.name;
        let call = quote_spanned! {at(method)=>
            let __value = self.#method().await?;
            #krate::typed::support::encode_as(&self.#field, &__value, &__info)
        };
        quote! {
            #key => ::core::option::Option::Some(::std::boxed::Box::pin(async move { #call })),
        }
    });
    let write_arms = writes.iter().map(|(d, method)| {
        let key = d.name.to_string();
        let field = &d.name;
        let call = quote_spanned! {at(method)=>
            let __argument = #krate::typed::support::decode_as(&self.#field, &__value, &__info)?;
            let __written = self.#method(__argument).await?;
            #krate::typed::support::encode_as(&self.#field, &__written, &__info)
        };
        quote! {
            #key => ::core::option::Option::Some(::std::boxed::Box::pin(async move { #call })),
        }
    });
    let do_arms = dos.iter().map(|(d, method)| {
        let key = d.name.to_string();
        let field = &d.name;
        let argument = if d.argument {
            quote_spanned! {at(method)=>
                let __argument = #krate::typed::support::command_argument(
                    &self.#field, __arg.as_ref(), __arg_info.as_ref())?;
            }
        } else {
            TokenStream::new()
        };
        let call_args = if d.argument {
            quote!(__argument)
        } else {
            TokenStream::new()
        };
        let finish = if d.result {
            quote_spanned! {at(method)=>
                let __result = self.#method(#call_args).await?;
                #krate::typed::support::command_result(&self.#field, &__result, __res_info.as_ref())
            }
        } else {
            quote_spanned! {at(method)=>
                let () = self.#method(#call_args).await?;
                ::core::result::Result::Ok(::core::option::Option::None)
            }
        };
        quote! {
            #key => ::std::boxed::Box::pin(async move { #argument #finish }),
        }
    });
    let start_hook = start.map(|method| {
        quote_spanned! {at(&method)=>
            fn start_hook(&mut self) -> #krate::typed::BoxFuture<'_, #krate::typed::Result<()>> {
                ::std::boxed::Box::pin(async move {
                    let () = self.#method().await?;
                    ::core::result::Result::Ok(())
                })
            }
        }
    });
    let has_poll = poll.is_some();
    let poll_hook = poll.map(|method| {
        quote_spanned! {at(&method)=>
            fn poll_hook<'__a>(
                &'__a mut self,
                due: &'__a [&'__a str],
            ) -> #krate::typed::BoxFuture<'__a, #krate::typed::Result<()>> {
                ::std::boxed::Box::pin(async move {
                    let () = self.#method(due).await?;
                    ::core::result::Result::Ok(())
                })
            }
        }
    });

    Ok(quote! {
        #block

        impl #krate::typed::Methods for #self_ty {
            const READ: &'static [&'static str] = &[#(#read_names),*];
            const WRITE: &'static [&'static str] = &[#(#write_names),*];
            const HAS_POLL: bool = #has_poll;

            #[allow(unused_variables, clippy::match_single_binding)]
            fn read_param<'__a>(
                &'__a mut self,
                parameter: &str,
                info: &#krate::typed::DataInfo,
            ) -> ::core::option::Option<
                #krate::typed::BoxFuture<'__a, #krate::typed::Result<#krate::typed::Value>>,
            > {
                let __info = ::core::clone::Clone::clone(info);
                match parameter {
                    #(#read_arms)*
                    _ => ::core::option::Option::None,
                }
            }

            #[allow(unused_variables, clippy::match_single_binding)]
            fn write_param<'__a>(
                &'__a mut self,
                parameter: &str,
                value: #krate::typed::Value,
                info: &#krate::typed::DataInfo,
            ) -> ::core::option::Option<
                #krate::typed::BoxFuture<'__a, #krate::typed::Result<#krate::typed::Value>>,
            > {
                let (__value, __info) = (value, ::core::clone::Clone::clone(info));
                match parameter {
                    #(#write_arms)*
                    _ => ::core::option::Option::None,
                }
            }

            #[allow(unused_variables, clippy::match_single_binding)]
            fn call_command<'__a>(
                &'__a mut self,
                command: &str,
                argument: ::core::option::Option<#krate::typed::Value>,
                argument_info: ::core::option::Option<&#krate::typed::DataInfo>,
                result_info: ::core::option::Option<&#krate::typed::DataInfo>,
            ) -> #krate::typed::BoxFuture<
                '__a,
                #krate::typed::Result<::core::option::Option<#krate::typed::Value>>,
            > {
                let __arg = argument;
                let __arg_info = argument_info.cloned();
                let __res_info = result_info.cloned();
                let __command = command.to_owned();
                match command {
                    #(#do_arms)*
                    _ => ::std::boxed::Box::pin(async move {
                        ::core::result::Result::Err(
                            #krate::typed::support::unknown("command", &__command),
                        )
                    }),
                }
            }

            #start_hook
            #poll_hook
        }
    })
}
