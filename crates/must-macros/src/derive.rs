// SPDX-License-Identifier: LGPL-3.0-or-later

//! `#[derive(SecopEnum)]` and `#[derive(SecopStruct)]`.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{Data, DeriveInput, Fields, GenericArgument, PathArguments, Type};

use crate::datainfo::Datainfo;

fn krate() -> TokenStream {
    quote!(::must_server)
}

pub fn secop_enum(input: TokenStream) -> syn::Result<TokenStream> {
    let input: DeriveInput = syn::parse2(input)?;
    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new(
            Span::call_site(),
            "`SecopEnum` derives for a field-less enum",
        ));
    };
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "`SecopEnum` cannot derive for a generic enum",
        ));
    }
    if data.variants.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "a SECoP enum needs at least one member",
        ));
    }
    for v in &data.variants {
        if !matches!(v.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(
                &v.ident,
                "`SecopEnum` derives for a field-less enum: this variant has fields",
            ));
        }
    }
    let krate = krate();
    let name = &input.ident;
    let variants: Vec<_> = data.variants.iter().map(|v| &v.ident).collect();
    let names: Vec<String> = variants.iter().map(ToString::to_string).collect();
    let first = variants[0];
    Ok(quote! {
        impl #krate::typed::SecopType for #name {
            const KINDS: &'static [#krate::typed::Kind] = &[#krate::typed::Kind::Enum];

            fn to_value(
                &self,
                _info: &#krate::typed::DataInfo,
            ) -> ::core::result::Result<#krate::typed::Value, #krate::typed::SecopError> {
                ::core::result::Result::Ok(#krate::typed::Value::Enum(match self {
                    #(Self::#variants => Self::#variants as i64,)*
                }))
            }

            fn from_value(
                value: &#krate::typed::Value,
                info: &#krate::typed::DataInfo,
            ) -> ::core::result::Result<Self, #krate::typed::SecopError> {
                match value {
                    #(#krate::typed::Value::Enum(code) if *code == Self::#variants as i64 => {
                        ::core::result::Result::Ok(Self::#variants)
                    })*
                    _ => ::core::result::Result::Err(
                        #krate::typed::mismatch(::core::stringify!(#name), value, info),
                    ),
                }
            }

            fn fallback() -> Self {
                Self::#first
            }

            fn datainfo() -> ::core::option::Option<#krate::typed::DataInfo> {
                #krate::typed::DataInfo::enumeration([#((#names, Self::#variants as i64)),*])
                    .build()
                    .ok()
            }
        }
    })
}

/// The inner type of `Option<T>`.
fn option_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(p) = ty else { return None };
    let last = p.path.segments.last()?;
    if last.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    match args.args.first()? {
        GenericArgument::Type(t) if args.args.len() == 1 => Some(t),
        _ => None,
    }
}

#[allow(clippy::too_many_lines)]
pub fn secop_struct(input: TokenStream) -> syn::Result<TokenStream> {
    let input: DeriveInput = syn::parse2(input)?;
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new(
            Span::call_site(),
            "`SecopStruct` derives for a struct with named fields",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "`SecopStruct` derives for a struct with named fields",
        ));
    };
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "`SecopStruct` cannot derive for a generic struct",
        ));
    }
    let krate = krate();
    let name = &input.ident;
    let mut members = Vec::new();
    let mut optional = Vec::new();
    let mut to_items = Vec::new();
    let mut from_items = Vec::new();
    let mut fallbacks = Vec::new();
    let mut checks = TokenStream::new();
    for field in &fields.named {
        let ident = field.ident.as_ref().expect("named fields");
        let key = ident.to_string();
        let inner = option_inner(&field.ty);
        let ty = inner.unwrap_or(&field.ty);
        let mut info: Option<Datainfo> = None;
        for attr in &field.attrs {
            if attr.path().is_ident("datainfo") {
                info = Some(attr.parse_args()?);
            }
        }
        let builder = if let Some(i) = &info {
            checks.extend(i.type_checks(&krate));
            let b = i.builder(&krate);
            quote!(#b.build())
        } else {
            let text = quote!(#ty).to_string();
            quote!(#krate::typed::Di::of::<#ty>(#text).build())
        };
        members.push(quote!((#key, #builder)));
        fallbacks.push(if inner.is_some() {
            quote!(#ident: ::core::option::Option::None)
        } else {
            quote!(#ident: <#ty as #krate::typed::SecopType>::fallback())
        });
        let member_info = quote!(#krate::typed::support::struct_member(info, #key)?);
        if inner.is_some() {
            optional.push(key.clone());
            to_items.push(quote! {
                if let ::core::option::Option::Some(v) = &self.#ident {
                    map.insert(#key.to_owned(), #krate::typed::SecopType::to_value(v, #member_info)?);
                }
            });
            from_items.push(quote! {
                #ident: match map.get(#key) {
                    ::core::option::Option::Some(v) => ::core::option::Option::Some(
                        <#ty as #krate::typed::SecopType>::from_value(v, #member_info)?,
                    ),
                    ::core::option::Option::None => ::core::option::Option::None,
                }
            });
        } else {
            to_items.push(quote! {
                map.insert(#key.to_owned(), #krate::typed::SecopType::to_value(&self.#ident, #member_info)?);
            });
            from_items.push(quote! {
                #ident: <#ty as #krate::typed::SecopType>::from_value(
                    map.get(#key).ok_or_else(|| #krate::typed::mismatch(#key, value, info))?,
                    #member_info,
                )?
            });
        }
    }
    Ok(quote! {
        #checks

        impl #krate::typed::SecopType for #name {
            const KINDS: &'static [#krate::typed::Kind] = &[#krate::typed::Kind::Struct];

            fn to_value(
                &self,
                info: &#krate::typed::DataInfo,
            ) -> ::core::result::Result<#krate::typed::Value, #krate::typed::SecopError> {
                let mut map = #krate::typed::support::IndexMap::new();
                #(#to_items)*
                ::core::result::Result::Ok(#krate::typed::Value::Struct(map))
            }

            fn from_value(
                value: &#krate::typed::Value,
                info: &#krate::typed::DataInfo,
            ) -> ::core::result::Result<Self, #krate::typed::SecopError> {
                let #krate::typed::Value::Struct(map) = value else {
                    return ::core::result::Result::Err(
                        #krate::typed::mismatch(::core::stringify!(#name), value, info),
                    );
                };
                ::core::result::Result::Ok(Self { #(#from_items),* })
            }

            fn fallback() -> Self {
                Self { #(#fallbacks),* }
            }

            fn datainfo() -> ::core::option::Option<#krate::typed::DataInfo> {
                #krate::typed::support::struct_datainfo(
                    ::std::vec![#(#members),*],
                    &[#(#optional),*],
                )
                .ok()
            }
        }
    })
}
