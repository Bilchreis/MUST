// SPDX-License-Identifier: LGPL-3.0-or-later

//! The datainfo syntax of the module macros: a SECoP type name with its keys as named
//! arguments, e.g. `double(min = 0.0, unit = "K")`, optionally followed by `as T`.

use proc_macro2::{Span, TokenStream};
use quote::{quote, quote_spanned};
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Expr, Ident, Token, Type, parenthesized};

/// The SECoP status codes `status(...)` may name; must match `typed::STATUS_CODES`.
const STATUS_CODES: &[&str] = &[
    "DISABLED",
    "IDLE",
    "STANDBY",
    "PREPARED",
    "WARN",
    "WARN_STANDBY",
    "WARN_PREPARED",
    "UNSTABLE",
    "BUSY",
    "DISABLING",
    "INITIALIZING",
    "PREPARING",
    "STARTING",
    "RAMPING",
    "STABILIZING",
    "FINALIZING",
    "ERROR",
    "UNKNOWN",
    "ERROR_STANDBY",
    "ERROR_PREPARED",
];

/// A datainfo as written in a declaration.
#[derive(Clone)]
pub struct Datainfo {
    /// The SECoP type name, e.g. `double`.
    pub kind: Ident,
    /// `key = value` arguments.
    pub keys: Vec<(Ident, Expr)>,
    /// Member datainfos of `array` and `tuple`.
    pub members: Vec<Datainfo>,
    /// The type of `enum(T)` and `struct(T)`.
    pub of_type: Option<Type>,
    /// The codes of `status(...)`.
    pub codes: Vec<Ident>,
    /// The expression of `datainfo(expr)`.
    pub expr: Option<Expr>,
    /// `as T`.
    pub cast: Option<Type>,
}

/// The keys each SECoP type has; keys starting with `_` are custom and always allowed.
fn keys_of(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "double" => &[
            "min",
            "max",
            "unit",
            "fmtstr",
            "absolute_resolution",
            "relative_resolution",
        ],
        "scaled" => &[
            "scale",
            "min",
            "max",
            "unit",
            "fmtstr",
            "absolute_resolution",
            "relative_resolution",
        ],
        "int" => &["min", "max", "unit"],
        "string" => &["minchars", "maxchars", "isUTF8"],
        "blob" => &["minbytes", "maxbytes"],
        "array" => &["minlen", "maxlen"],
        "bool" | "tuple" | "enum" | "struct" | "status" | "datainfo" => &[],
        _ => return None,
    })
}

const KINDS: &str = "double, scaled, int, bool, string, blob, array, tuple, enum, struct, status \
                     or datainfo";

impl Parse for Datainfo {
    #[allow(clippy::too_many_lines)]
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let kind = Ident::parse_any(input)?;
        let name = kind.to_string();
        let Some(allowed) = keys_of(&name) else {
            return Err(syn::Error::new(
                kind.span(),
                format!("`{name}` is not a datainfo type; use {KINDS}"),
            ));
        };
        let content;
        parenthesized!(content in input);
        let mut info = Self {
            kind,
            keys: Vec::new(),
            members: Vec::new(),
            of_type: None,
            codes: Vec::new(),
            expr: None,
            cast: None,
        };
        while !content.is_empty() {
            if content.peek(Ident::peek_any)
                && content.peek2(Token![=])
                && !content.peek2(Token![==])
            {
                let key = Ident::parse_any(&content)?;
                content.parse::<Token![=]>()?;
                let value: Expr = content.parse()?;
                let k = key.to_string();
                if !k.starts_with('_') && !allowed.contains(&k.as_str()) {
                    let known = if allowed.is_empty() {
                        "no keys".to_owned()
                    } else {
                        format!("the keys {}", allowed.join(", "))
                    };
                    return Err(syn::Error::new(
                        key.span(),
                        format!("`{name}` has no key `{k}`; it has {known}"),
                    ));
                }
                if info.keys.iter().any(|(existing, _)| *existing == key) {
                    return Err(syn::Error::new(key.span(), format!("`{k}` is given twice")));
                }
                info.keys.push((key, value));
            } else {
                match name.as_str() {
                    "array" if info.members.is_empty() => info.members.push(content.parse()?),
                    "tuple" => info.members.push(content.parse()?),
                    "enum" | "struct" if info.of_type.is_none() => {
                        info.of_type = Some(content.parse()?);
                    }
                    "status" => {
                        let code = Ident::parse_any(&content)?;
                        if !STATUS_CODES.contains(&code.to_string().as_str()) {
                            return Err(syn::Error::new(
                                code.span(),
                                format!(
                                    "`{code}` is not a SECoP status code; use {}",
                                    STATUS_CODES.join(", ")
                                ),
                            ));
                        }
                        info.codes.push(code);
                    }
                    "datainfo" if info.expr.is_none() => info.expr = Some(content.parse()?),
                    _ => {
                        return Err(content.error(format!(
                            "unexpected argument for `{name}`; it takes only `key = value` \
                             arguments"
                        )));
                    }
                }
            }
            if content.is_empty() {
                break;
            }
            content.parse::<Token![,]>()?;
        }
        let missing = match name.as_str() {
            "array" if info.members.is_empty() => Some("its element datainfo"),
            "tuple" if info.members.is_empty() => Some("its member datainfos"),
            "enum" | "struct" if info.of_type.is_none() => Some("the Rust type"),
            "status" if info.codes.is_empty() => Some("its status codes"),
            "datainfo" if info.expr.is_none() => Some("a `DataInfo` expression"),
            _ => None,
        };
        if let Some(missing) = missing {
            return Err(syn::Error::new(
                info.kind.span(),
                format!("`{name}(...)` needs {missing}"),
            ));
        }
        if input.peek(Token![as]) {
            input.parse::<Token![as]>()?;
            info.cast = Some(input.parse()?);
        } else if name == "datainfo" {
            return Err(syn::Error::new(
                info.kind.span(),
                "`datainfo(...)` needs `as T` with the Rust type",
            ));
        }
        Ok(info)
    }
}

/// The SECoP kind of a datainfo type name, as a `typed::Kind` variant; `None` for
/// `datainfo(...)`, whose kind is only known at runtime.
fn kind_variant(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "double" => "Double",
        "scaled" => "Scaled",
        "int" => "Int",
        "bool" => "Bool",
        "enum" => "Enum",
        "string" => "String",
        "blob" => "Blob",
        "array" => "Array",
        "tuple" | "status" => "Tuple",
        "struct" => "Struct",
        _ => return None,
    })
}

impl Datainfo {
    /// The Rust type of values of this datainfo: `as T`, or the default for the type.
    pub fn rust_type(&self, krate: &TokenStream) -> TokenStream {
        if let Some(cast) = &self.cast {
            return quote!(#cast);
        }
        match self.kind.to_string().as_str() {
            "double" | "scaled" => quote!(f64),
            "int" => quote!(i64),
            "bool" => quote!(bool),
            "string" => quote!(::std::string::String),
            "blob" => quote!(::std::vec::Vec<u8>),
            "array" => {
                let element = self.members[0].rust_type(krate);
                quote!(::std::vec::Vec<#element>)
            }
            "tuple" => {
                let members = self.members.iter().map(|m| m.rust_type(krate));
                quote!((#(#members,)*))
            }
            "enum" | "struct" => {
                let ty = self.of_type.as_ref().expect("checked when parsing");
                quote!(#ty)
            }
            "status" => quote!(#krate::typed::Status),
            _ => unreachable!("datainfo(...) always has `as T`"),
        }
    }

    /// Compile-time checks that the Rust types fit: one constant per `as T`, `enum(T)`
    /// and `struct(T)`, nested ones included.
    pub fn type_checks(&self, krate: &TokenStream) -> TokenStream {
        let mut checks = TokenStream::new();
        let name = self.kind.to_string();
        let checked: Option<(&Type, &str)> = match (&self.cast, &self.of_type) {
            (Some(cast), _) => kind_variant(&name).map(|k| (cast, k)),
            (None, Some(ty)) => kind_variant(&name).map(|k| (ty, k)),
            _ => None,
        };
        if let Some((ty, variant)) = checked {
            let variant = Ident::new(variant, Span::call_site());
            let ty_text = quote!(#ty).to_string().replace(' ', "");
            let message = format!("the Rust type `{ty_text}` does not fit a `{name}` datainfo");
            checks.extend(quote_spanned! {ty.span()=>
                const _: () = ::core::assert!(
                    #krate::typed::accepts(
                        <#ty as #krate::typed::SecopType>::KINDS,
                        #krate::typed::Kind::#variant,
                    ),
                    #message,
                );
            });
        }
        for member in &self.members {
            checks.extend(member.type_checks(krate));
        }
        checks
    }

    /// An expression building a `typed::Di` for this datainfo.
    pub fn builder(&self, krate: &TokenStream) -> TokenStream {
        let name = self.kind.to_string();
        let di = quote!(#krate::typed::Di);
        let mut expr = match name.as_str() {
            "enum" | "struct" => {
                let ty = self.of_type.as_ref().expect("checked when parsing");
                let text = quote!(#ty).to_string();
                quote!(#di::of::<#ty>(#text))
            }
            "status" => {
                let codes = self.codes.iter().map(Ident::to_string);
                quote!(#di::status(&[#(#codes),*]))
            }
            "datainfo" => {
                let e = self.expr.as_ref().expect("checked when parsing");
                quote!(#di::from_info(::core::result::Result::<_, ::std::string::String>::Ok(#e)))
            }
            _ => quote!(#di::new(#name)),
        };
        if !self.members.is_empty() {
            let members = self.members.iter().map(|m| m.builder(krate));
            expr = quote!(#expr.members(::std::vec![#(#members),*]));
        }
        for (key, value) in &self.keys {
            let k = key.to_string();
            expr = quote!(#expr.set(#k, ::core::clone::Clone::clone(&(#value))));
        }
        if name == "int" {
            let ty = self.rust_type(krate);
            expr = quote!(#expr.int_bounds::<#ty>());
        }
        expr
    }

    /// The identifiers among `names` that this datainfo's values use.
    pub fn uses<'n>(&self, names: &'n [String]) -> Vec<(&'n str, Span)> {
        struct Finder<'a, 'n> {
            names: &'n [String],
            found: &'a mut Vec<(&'n str, Span)>,
        }
        impl<'ast> Visit<'ast> for Finder<'_, '_> {
            fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
                if let Some(ident) = path.path.get_ident()
                    && let Some(name) = self.names.iter().find(|n| ident == n.as_str())
                {
                    self.found.push((name.as_str(), ident.span()));
                }
                syn::visit::visit_expr_path(self, path);
            }
        }
        let mut found = Vec::new();
        let mut finder = Finder {
            names,
            found: &mut found,
        };
        for (_, value) in &self.keys {
            finder.visit_expr(value);
        }
        if let Some(e) = &self.expr {
            finder.visit_expr(e);
        }
        for member in &self.members {
            found.extend(member.uses(names));
        }
        found
    }
}
