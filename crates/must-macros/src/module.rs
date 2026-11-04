// SPDX-License-Identifier: LGPL-3.0-or-later

//! `#[module(...)]`: a struct with `param!`, `command!` and `property!` fields becomes
//! a typed module.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream, Parser};
use syn::spanned::Spanned;
use syn::{Expr, Fields, Ident, Item, ItemStruct, LitStr, Path, Token, Type};

use crate::datainfo::Datainfo;

const INTERFACE_CLASSES: &[&str] = &["Readable", "Writable", "Drivable"];

/// The arguments of `#[module(...)]`.
struct Args {
    interface_class: Option<Ident>,
    description: Option<LitStr>,
    krate: Option<Path>,
}

impl Parse for Args {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = Self {
            interface_class: None,
            description: None,
            krate: None,
        };
        let mut first = true;
        while !input.is_empty() {
            let key = Ident::parse_any(input)?;
            if input.peek(Token![=]) {
                input.parse::<Token![=]>()?;
                match key.to_string().as_str() {
                    "description" if args.description.is_none() => {
                        args.description = Some(input.parse()?);
                    }
                    "crate" if args.krate.is_none() => args.krate = Some(input.parse()?),
                    other => {
                        return Err(syn::Error::new(
                            key.span(),
                            format!(
                                "unknown or repeated argument `{other}`; use an interface class, `description = \"...\"` or `crate = path`"
                            ),
                        ));
                    }
                }
            } else if first {
                if !INTERFACE_CLASSES.contains(&key.to_string().as_str()) {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "`{key}` is not a supported interface class; use {}",
                            INTERFACE_CLASSES.join(", ")
                        ),
                    ));
                }
                args.interface_class = Some(key);
            } else {
                return Err(syn::Error::new(
                    key.span(),
                    "the interface class comes first; other arguments are `key = value`",
                ));
            }
            first = false;
            if input.is_empty() {
                break;
            }
            input.parse::<Token![,]>()?;
        }
        Ok(args)
    }
}

/// A flag of a declaration: `name` or `name = value`.
struct Flag {
    name: Ident,
    value: Option<Expr>,
}

/// Parses `description, datainfo?, flags...`; `datainfo` says whether one is expected.
fn parse_decl(
    input: ParseStream,
    with_datainfo: bool,
) -> syn::Result<(Expr, Option<Datainfo>, Vec<Flag>)> {
    let description: Expr = input.parse()?;
    let mut info = None;
    if with_datainfo {
        input.parse::<Token![,]>()?;
        info = Some(input.parse()?);
    }
    let mut flags: Vec<Flag> = Vec::new();
    while !input.is_empty() {
        input.parse::<Token![,]>()?;
        if input.is_empty() {
            break;
        }
        let name = Ident::parse_any(input)?;
        let value = if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            Some(input.parse()?)
        } else {
            None
        };
        if flags.iter().any(|f| f.name == name) {
            return Err(syn::Error::new(
                name.span(),
                format!("`{name}` is given twice"),
            ));
        }
        flags.push(Flag { name, value });
    }
    Ok((description, info, flags))
}

/// `argument = datainfo` / `result = datainfo` of a command, parsed separately because a
/// datainfo is not an expression.
fn parse_command(input: ParseStream) -> syn::Result<(Expr, CommandDecl)> {
    let description: Expr = input.parse()?;
    let mut decl = CommandDecl::default();
    while !input.is_empty() {
        input.parse::<Token![,]>()?;
        if input.is_empty() {
            break;
        }
        let name = Ident::parse_any(input)?;
        let key = name.to_string();
        let repeated = decl.seen.contains(&key);
        decl.seen.push(key.clone());
        if repeated {
            return Err(syn::Error::new(
                name.span(),
                format!("`{key}` is given twice"),
            ));
        }
        match key.as_str() {
            "argument" | "result" => {
                input.parse::<Token![=]>()?;
                let info: Datainfo = input.parse()?;
                if key == "argument" {
                    decl.argument = Some(info);
                } else {
                    decl.result = Some(info);
                }
            }
            "checkable" => decl.checkable = true,
            _ => {
                if !(key == "group" || key == "visibility" || key.starts_with('_')) {
                    return Err(syn::Error::new(
                        name.span(),
                        format!(
                            "unknown command flag `{key}`; use argument, result, group, visibility, checkable or a custom `_name`"
                        ),
                    ));
                }
                input.parse::<Token![=]>()?;
                let value: Expr = input.parse()?;
                decl.props.push((name, value));
            }
        }
    }
    Ok((description, decl))
}

#[derive(Default)]
struct CommandDecl {
    argument: Option<Datainfo>,
    result: Option<Datainfo>,
    checkable: bool,
    props: Vec<(Ident, Expr)>,
    seen: Vec<String>,
}

struct ParamDecl {
    info: Datainfo,
    writable: bool,
    checkable: bool,
    constant: Option<Expr>,
    default: Option<Expr>,
    poll_interval: Option<Expr>,
    props: Vec<(Ident, Expr)>,
}

fn param_decl(info: Datainfo, flags: Vec<Flag>) -> syn::Result<ParamDecl> {
    let mut decl = ParamDecl {
        info,
        writable: false,
        checkable: false,
        constant: None,
        default: None,
        poll_interval: None,
        props: Vec::new(),
    };
    let mut constant_span = None;
    for Flag { name, value } in flags {
        let key = name.to_string();
        match (key.as_str(), value) {
            ("writable", None) => decl.writable = true,
            ("checkable", None) => decl.checkable = true,
            ("constant", Some(v)) => {
                constant_span = Some(name.span());
                decl.constant = Some(v);
            }
            ("default", Some(v)) => decl.default = Some(v),
            ("poll_interval", Some(v)) => decl.poll_interval = Some(v),
            ("group" | "visibility", Some(v)) => decl.props.push((name, v)),
            (k, Some(v)) if k.starts_with('_') => decl.props.push((name, v)),
            ("writable" | "checkable", Some(_)) => {
                return Err(syn::Error::new(
                    name.span(),
                    format!("`{key}` takes no value"),
                ));
            }
            ("constant" | "default" | "poll_interval" | "group" | "visibility", None) => {
                return Err(syn::Error::new(
                    name.span(),
                    format!("`{key}` needs a value: `{key} = ...`"),
                ));
            }
            _ => {
                return Err(syn::Error::new(
                    name.span(),
                    format!(
                        "unknown parameter flag `{key}`; use writable, constant, default, group, visibility, checkable, poll_interval or a custom `_name`"
                    ),
                ));
            }
        }
    }
    if let Some(span) = constant_span
        && (decl.writable || decl.default.is_some())
    {
        return Err(syn::Error::new(
            span,
            "a constant parameter cannot be `writable` or have a `default`",
        ));
    }
    Ok(decl)
}

enum FieldKind {
    Param(Box<ParamDecl>),
    Command(Box<CommandDecl>),
    Property(Box<Datainfo>, Option<Expr>),
    State,
}

struct Field {
    ident: Ident,
    description: Option<Expr>,
    kind: FieldKind,
}

/// The macro of a field type, if it is `param!`, `command!` or `property!`.
fn field_macro(ty: &Type) -> Option<(&'static str, &syn::Macro)> {
    let Type::Macro(m) = ty else {
        return None;
    };
    let last = m.mac.path.segments.last()?.ident.to_string();
    let kind = match last.as_str() {
        "param" => "param",
        "command" => "command",
        "property" => "property",
        _ => return None,
    };
    Some((kind, &m.mac))
}

fn parse_field(field: &syn::Field) -> syn::Result<Field> {
    let ident = field.ident.clone().expect("named fields");
    let Some((kind, mac)) = field_macro(&field.ty) else {
        return Ok(Field {
            ident,
            description: None,
            kind: FieldKind::State,
        });
    };
    match kind {
        "param" => {
            let (description, info, flags) =
                (|input: ParseStream| parse_decl(input, true)).parse2(mac.tokens.clone())?;
            Ok(Field {
                ident,
                description: Some(description),
                kind: FieldKind::Param(Box::new(param_decl(
                    info.expect("parsed with a datainfo"),
                    flags,
                )?)),
            })
        }
        "command" => {
            let (description, decl) = parse_command.parse2(mac.tokens.clone())?;
            Ok(Field {
                ident,
                description: Some(description),
                kind: FieldKind::Command(Box::new(decl)),
            })
        }
        _ => {
            let (description, info, flags) =
                (|input: ParseStream| parse_decl(input, true)).parse2(mac.tokens.clone())?;
            let mut default = None;
            for Flag { name, value } in flags {
                match (name.to_string().as_str(), value) {
                    ("default", Some(v)) => default = Some(v),
                    _ => {
                        return Err(syn::Error::new(
                            name.span(),
                            format!(
                                "unknown property flag `{name}`; a property takes only `default = ...`"
                            ),
                        ));
                    }
                }
            }
            Ok(Field {
                ident,
                description: Some(description),
                kind: FieldKind::Property(Box::new(info.expect("parsed with a datainfo")), default),
            })
        }
    }
}

/// A value for a field of Rust type `T`: string literals become `String`s, so that
/// `default = "A"` works for a `string` datainfo.
fn value_expr(e: &Expr) -> TokenStream {
    if let Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Str(_),
        ..
    }) = e
    {
        quote!(::std::string::String::from(#e))
    } else {
        quote!(#e)
    }
}

/// Checks the declarations against the interface class.
fn interface_checks(name: &Ident, class: Option<&Ident>, fields: &[Field]) -> syn::Result<()> {
    let Some(class) = class else {
        return Ok(());
    };
    let class_name = class.to_string();
    let level = INTERFACE_CLASSES
        .iter()
        .position(|c| *c == class_name)
        .expect("checked when parsing");
    let find = |n: &str| fields.iter().find(|f| f.ident == n);
    let missing = |what: &str| syn::Error::new(name.span(), format!("`{class_name}` needs {what}"));
    match find("value") {
        Some(Field {
            kind: FieldKind::Param(_),
            ..
        }) => {}
        Some(f) => {
            return Err(syn::Error::new(
                f.ident.span(),
                format!("`value` must be a parameter for `{class_name}`"),
            ));
        }
        None => return Err(missing("a parameter `value`")),
    }
    let status = match find("status") {
        Some(Field {
            kind: FieldKind::Param(p),
            ident,
            ..
        }) => {
            let kind = p.info.kind.to_string();
            if !matches!(kind.as_str(), "status" | "tuple" | "datainfo") {
                return Err(syn::Error::new(
                    ident.span(),
                    "`status` needs a status datainfo, e.g. `status(IDLE, WARN, BUSY, ERROR)`",
                ));
            }
            p
        }
        Some(f) => {
            return Err(syn::Error::new(
                f.ident.span(),
                format!("`status` must be a parameter for `{class_name}`"),
            ));
        }
        None => return Err(missing("a parameter `status`")),
    };
    if level >= 1 {
        match find("target") {
            Some(Field {
                kind: FieldKind::Param(p),
                ..
            }) if p.writable => {}
            Some(f) => {
                return Err(syn::Error::new(
                    f.ident.span(),
                    format!(
                        "`target` must be a writable parameter for `{class_name}`: add the flag `writable`"
                    ),
                ));
            }
            None => return Err(missing("a writable parameter `target`")),
        }
    }
    if level >= 2 {
        match find("stop") {
            Some(Field {
                kind: FieldKind::Command(c),
                ..
            }) if c.argument.is_none() && c.result.is_none() => {}
            Some(f) => {
                return Err(syn::Error::new(
                    f.ident.span(),
                    "`stop` must be a command without argument and result for `Drivable`",
                ));
            }
            None => return Err(missing("a command `stop`")),
        }
        if status.info.kind == "status" && !status.info.codes.iter().any(|c| c == "BUSY") {
            return Err(syn::Error::new(
                status.info.kind.span(),
                "the status of a `Drivable` needs the code `BUSY`",
            ));
        }
    }
    Ok(())
}

/// Expands `#[module]`; after an error it still defines the callback macro, so that the
/// methods block does not report a second, confusing error.
pub fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    let name = syn::parse2::<ItemStruct>(item.clone())
        .ok()
        .map(|s| s.ident);
    match expand_checked(attr, item) {
        Ok(tokens) => tokens,
        Err(e) => {
            let error = e.into_compile_error();
            let Some(name) = name else { return error };
            let callback = format_ident!("__must_module_{}", name);
            quote! {
                #error
                #[doc(hidden)]
                #[allow(unused_macros)]
                macro_rules! #callback { ($($methods:tt)*) => {}; }
                #[doc(hidden)]
                #[allow(unused_imports)]
                pub(crate) use #callback;
            }
        }
    }
}

#[allow(clippy::too_many_lines)]
fn expand_checked(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let args: Args = syn::parse2(attr)?;
    let item: Item = syn::parse2(item).map_err(|_| {
        syn::Error::new(
            Span::call_site(),
            "`#[must_server::module]` applies to a struct with named fields",
        )
    })?;
    let Item::Struct(mut item) = item else {
        return Err(syn::Error::new(
            Span::call_site(),
            "`#[must_server::module]` applies to a struct with named fields",
        ));
    };
    if !matches!(item.fields, Fields::Named(_)) {
        return Err(syn::Error::new(
            item.ident.span(),
            "`#[must_server::module]` applies to a struct with named fields",
        ));
    }
    if !item.generics.params.is_empty() {
        return Err(syn::Error::new(
            item.generics.span(),
            "a module type cannot have generic parameters",
        ));
    }
    let krate = args
        .krate
        .as_ref()
        .map_or_else(|| quote!(::must_server), |p| quote!(#p));
    let mut fields = Vec::new();
    let mut errors: Option<syn::Error> = None;
    for field in &item.fields {
        match parse_field(field) {
            Ok(f) => fields.push(f),
            Err(e) => match &mut errors {
                Some(all) => all.combine(e),
                None => errors = Some(e),
            },
        }
    }
    if let Some(e) = errors {
        return Err(e);
    }
    interface_checks(&item.ident, args.interface_class.as_ref(), &fields)?;

    let property_names: Vec<String> = fields
        .iter()
        .filter(|f| matches!(f.kind, FieldKind::Property(..)))
        .map(|f| f.ident.to_string())
        .collect();
    for f in &fields {
        if let FieldKind::Property(info, _) = &f.kind
            && let Some((name, span)) = info.uses(&property_names).first()
        {
            return Err(syn::Error::new(
                *span,
                format!(
                    "the datainfo of a property cannot use the property `{name}`: properties are read first"
                ),
            ));
        }
    }

    let name = &item.ident;
    let mut checks = TokenStream::new();
    let mut properties = TokenStream::new();
    let mut params = TokenStream::new();
    let mut commands = TokenStream::new();
    let mut inits = Vec::new();
    let mut param_refs = Vec::new();
    let mut callback_params = Vec::new();
    let mut callback_commands = Vec::new();

    for f in &fields {
        let ident = &f.ident;
        let key = ident.to_string();
        let description = &f.description;
        match &f.kind {
            FieldKind::State => {
                inits.push(quote!(#ident: ::core::default::Default::default()));
            }
            FieldKind::Property(info, default) => {
                checks.extend(info.type_checks(&krate));
                let ty = info.rust_type(&krate);
                let builder = info.builder(&krate);
                let default = default.as_ref().map_or_else(
                    || quote!(::core::option::Option::None),
                    |d| {
                        let d = value_expr(d);
                        quote!(::core::option::Option::Some(#d))
                    },
                );
                properties.extend(quote! {
                    let #ident: #ty = __declaring.property::<#ty>(#key, #builder.build(), #default);
                });
                inits.push(quote!(#ident));
            }
            FieldKind::Param(p) => {
                checks.extend(p.info.type_checks(&krate));
                let ty = p.info.rust_type(&krate);
                let builder = p.info.builder(&krate);
                let local = format_ident!("__param_{}", ident);
                let mut spec = quote! {
                    #krate::typed::ParamSpec::<#ty>::new(
                        #key,
                        #description,
                        #builder.build(),
                        #krate::typed::has(<Self as #krate::typed::Methods>::READ, #key),
                        #krate::typed::has(<Self as #krate::typed::Methods>::WRITE, #key),
                    )
                };
                if p.writable {
                    spec = quote!(#spec.writable());
                }
                if p.checkable {
                    spec = quote!(#spec.checkable());
                }
                if let Some(c) = &p.constant {
                    let c = value_expr(c);
                    spec = quote!(#spec.constant(#c));
                }
                if let Some(d) = &p.default {
                    let d = value_expr(d);
                    spec = quote!(#spec.default(#d));
                }
                if let Some(i) = &p.poll_interval {
                    spec = quote!(#spec.poll_interval(#i));
                }
                for (k, v) in &p.props {
                    let kt = k.to_string();
                    spec = match kt.as_str() {
                        "group" => quote!(#spec.group(#v)),
                        "visibility" => quote!(#spec.visibility(#v)),
                        _ => quote!(#spec.custom(#kt, #v)),
                    };
                }
                params.extend(quote! {
                    let #local = __declaring.param(#spec);
                });
                inits.push(quote!(#ident: #local));
                param_refs.push(quote!(&self.#ident as &dyn #krate::typed::AnyParam));
                let writable = if p.writable {
                    quote!(writable)
                } else {
                    quote!(readonly)
                };
                callback_params.push(quote!(#ident: #writable));
            }
            FieldKind::Command(c) => {
                let mut spec = quote!(#krate::typed::CommandSpec::new(#key, #description));
                let arg_ty = c
                    .argument
                    .as_ref()
                    .map_or_else(|| quote!(()), |i| i.rust_type(&krate));
                let res_ty = c
                    .result
                    .as_ref()
                    .map_or_else(|| quote!(()), |i| i.rust_type(&krate));
                if let Some(a) = &c.argument {
                    checks.extend(a.type_checks(&krate));
                    let b = a.builder(&krate);
                    spec = quote!(#spec.argument::<#arg_ty>(#b.build()));
                }
                if let Some(r) = &c.result {
                    checks.extend(r.type_checks(&krate));
                    let b = r.builder(&krate);
                    spec = quote!(#spec.result::<#res_ty>(#b.build()));
                }
                if c.checkable {
                    spec = quote!(#spec.checkable());
                }
                for (k, v) in &c.props {
                    let kt = k.to_string();
                    spec = match kt.as_str() {
                        "group" => quote!(#spec.group(#v)),
                        "visibility" => quote!(#spec.visibility(#v)),
                        _ => quote!(#spec.custom(#kt, #v)),
                    };
                }
                commands.extend(quote!(__declaring.command(#spec);));
                inits.push(quote!(#ident: ::core::default::Default::default()));
                let arg = if c.argument.is_some() {
                    quote!(argument)
                } else {
                    quote!(no_argument)
                };
                let res = if c.result.is_some() {
                    quote!(result)
                } else {
                    quote!(no_result)
                };
                callback_commands.push(quote!(#ident(#arg, #res)));
            }
        }
    }

    // the struct with its field types replaced
    if let Fields::Named(named) = &mut item.fields {
        for (field, decl) in named.named.iter_mut().zip(&fields) {
            match &decl.kind {
                FieldKind::Param(p) => {
                    let ty = p.info.rust_type(&krate);
                    field.ty = syn::parse_quote!(#krate::typed::Param<#ty>);
                }
                FieldKind::Command(c) => {
                    let a = c
                        .argument
                        .as_ref()
                        .map_or_else(|| quote!(()), |i| i.rust_type(&krate));
                    let r = c
                        .result
                        .as_ref()
                        .map_or_else(|| quote!(()), |i| i.rust_type(&krate));
                    field.ty = syn::parse_quote!(#krate::typed::Command<#a, #r>);
                }
                FieldKind::Property(info, _) => {
                    let ty = info.rust_type(&krate);
                    field.ty = syn::parse_quote!(#ty);
                }
                FieldKind::State => {}
            }
        }
    }

    let class = args
        .interface_class
        .as_ref()
        .map(Ident::to_string)
        .into_iter();
    let default_description = args.description.as_ref().map_or_else(
        || quote!(::core::option::Option::None),
        |d| quote!(::core::option::Option::Some(#d)),
    );
    let callback = format_ident!("__must_module_{}", name);
    let impl_span = Span::call_site();
    let typed_impl = quote_spanned! {impl_span=>
        impl #krate::typed::TypedModule for #name {
            const DEFAULT_DESCRIPTION: ::core::option::Option<&'static str> = #default_description;

            #[allow(clippy::too_many_lines, unused_variables, unused_mut)]
            fn declare(
                __name: &str,
                __description: &str,
                __settings: &mut dyn #krate::typed::Settings,
            ) -> ::core::result::Result<
                #krate::typed::Declared<Self>,
                #krate::typed::SettingsErrors,
            > {
                let mut __declaring = #krate::typed::Declaring::new(
                    __name,
                    __description,
                    &[#(#class),*],
                    __settings,
                );
                #properties
                #params
                #commands
                let __instance = Self { #(#inits),* };
                __declaring.finish(__instance)
            }

            fn params(&self) -> ::std::vec::Vec<&dyn #krate::typed::AnyParam> {
                ::std::vec![#(#param_refs),*]
            }
        }
    };

    Ok(quote! {
        #item

        #checks

        #typed_impl

        #[doc(hidden)]
        #[allow(unused_macros)]
        macro_rules! #callback {
            ($($methods:tt)*) => {
                #krate::__methods_impl! {
                    crate = #krate;
                    module = #name;
                    params = [#(#callback_params),*];
                    commands = [#(#callback_commands),*];
                    $($methods)*
                }
            };
        }
        #[doc(hidden)]
        #[allow(unused_imports)]
        pub(crate) use #callback;
    })
}
