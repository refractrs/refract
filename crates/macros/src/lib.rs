//! Optional authoring sugar over Refract's ordinary view runtime.
use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::quote;
use std::collections::HashSet;
use syn::{fold::Fold, parse_quote, punctuated::Punctuated, visit::Visit, Token};

/// Turn a function into a reusable view with mounted state, actions, and effects.
#[proc_macro_attribute]
pub fn view(args: TokenStream, input: TokenStream) -> TokenStream {
    expand(args.into(), input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
fn error<T: syn::spanned::Spanned>(node: &T, message: &str) -> syn::Error {
    syn::Error::new(node.span(), message)
}
fn ident(pat: &syn::Pat) -> syn::Result<&syn::Ident> {
    match pat {
        syn::Pat::Ident(p)
            if p.mutability.is_none() && p.by_ref.is_none() && p.subpat.is_none() =>
        {
            Ok(&p.ident)
        }
        _ => Err(error(pat, "use a single immutable binding name")),
    }
}
fn helper(attr: &syn::Attribute) -> bool {
    [
        "state",
        "action",
        "effect",
        "environment",
        "environment_object",
    ]
    .iter()
    .any(|n| attr.path().is_ident(n))
}
fn expand(args: Tokens, input: Tokens) -> syn::Result<Tokens> {
    let kit: syn::Path = if args.is_empty() {
        parse_quote!(::refract)
    } else {
        let meta: syn::MetaNameValue = syn::parse2(args)?;
        if !meta.path.is_ident("crate_path") {
            return Err(error(&meta, "expected crate_path = path"));
        }
        match meta.value {
            syn::Expr::Path(p) => p.path,
            _ => return Err(error(&meta, "crate_path must be a Rust path")),
        }
    };
    let mut f: syn::ItemFn = syn::parse2(input)?;
    if f.sig.asyncness.is_some() || f.sig.constness.is_some() || f.sig.unsafety.is_some() {
        return Err(error(
            &f.sig,
            "view functions must be synchronous, safe, and non-const",
        ));
    }
    let first = f
        .sig
        .inputs
        .first()
        .ok_or_else(|| error(&f.sig, "first argument must be &mut ViewContext<'_>"))?;
    let syn::FnArg::Typed(first) = first else {
        return Err(error(first, "view functions cannot have a self receiver"));
    };
    let cx = ident(&first.pat)?.clone();
    match first.ty.as_ref() {
        syn::Type::Reference(r) if r.mutability.is_some() => match r.elem.as_ref() {
            syn::Type::Path(p)
                if p.path
                    .segments
                    .last()
                    .is_some_and(|s| s.ident == "ViewContext") => {}
            _ => return Err(error(first, "first argument must be &mut ViewContext<'_>")),
        },
        _ => return Err(error(first, "first argument must be &mut ViewContext<'_>")),
    }
    let valid_output = matches!(&f.sig.output,syn::ReturnType::Type(_,ty) if matches!(ty.as_ref(),syn::Type::ImplTrait(t) if t.bounds.iter().any(|b|matches!(b,syn::TypeParamBound::Trait(t) if t.path.segments.last().is_some_and(|s|s.ident=="View")))));
    if !valid_output {
        return Err(error(&f.sig.output, "view functions must return impl View"));
    }
    f.sig.inputs = f.sig.inputs.into_iter().skip(1).collect();
    let mut props = vec![];
    for arg in &f.sig.inputs {
        let syn::FnArg::Typed(arg) = arg else {
            return Err(error(arg, "owned props required"));
        };
        struct Borrowed(bool);
        impl<'a> Visit<'a> for Borrowed {
            fn visit_type_reference(&mut self, _: &'a syn::TypeReference) {
                self.0 = true
            }
        }
        let mut borrowed = Borrowed(false);
        borrowed.visit_type(&arg.ty);
        if borrowed.0 {
            return Err(error(
                &arg.ty,
                "view props must be owned; use Rc for shared data",
            ));
        }
        props.push(ident(&arg.pat)?.clone());
    }
    let mut slots = HashSet::new();
    let mut effects = HashSet::new();
    for statement in &mut f.block.stmts {
        let syn::Stmt::Local(local) = statement else {
            continue;
        };
        let helpers: Vec<_> = local.attrs.iter().filter(|a| helper(a)).cloned().collect();
        if helpers.is_empty() {
            continue;
        }
        if helpers.len() != 1 {
            return Err(error(local, "use one Refract attribute per declaration"));
        }
        local.attrs.retain(|a| !helper(a));
        let attr = &helpers[0];
        let (name, ty) = match &local.pat {
            syn::Pat::Type(p) => (ident(&p.pat)?.clone(), Some(p.ty.clone())),
            p => (ident(p)?.clone(), None),
        };
        if attr.path().is_ident("state") {
            let mut slot = name.to_string();
            if !matches!(attr.meta, syn::Meta::Path(_)) {
                attr.parse_nested_meta(|meta| {
                    if !meta.path.is_ident("id") {
                        return Err(meta.error("expected id = \"name\""));
                    }
                    slot = meta.value()?.parse::<syn::LitStr>()?.value();
                    Ok(())
                })?;
            }
            if slot.is_empty() || !slots.insert(slot.clone()) {
                return Err(error(
                    local,
                    "state IDs must be nonempty and unique within a view",
                ));
            }
            let init = local
                .init
                .as_ref()
                .ok_or_else(|| error(local, "state requires an initializer"))?;
            if init.diverge.is_some() {
                return Err(error(local, "state does not support let-else"));
            }
            let value = &init.expr;
            let annotation = ty.map(|ty| quote!(: #kit::Binding<#ty>));
            *statement = parse_quote!(let #name #annotation = #cx.state(#slot, || #value););
        } else if attr.path().is_ident("effect") {
            if ty.is_some() {
                return Err(error(
                    local,
                    "effect declarations do not bind a value; omit the type",
                ));
            }
            let mut slot = name.to_string();
            let mut dependencies: syn::Expr = parse_quote!(());
            let mut captures = Vec::<syn::Ident>::new();
            let mut options = HashSet::new();
            if !matches!(attr.meta, syn::Meta::Path(_)) {
                attr.parse_nested_meta(|meta| {
                    let option = meta
                        .path
                        .get_ident()
                        .map(ToString::to_string)
                        .ok_or_else(|| meta.error("expected id, deps, or captures"))?;
                    if !options.insert(option.clone()) {
                        return Err(meta.error("duplicate effect option"));
                    }
                    match option.as_str() {
                        "id" => slot = meta.value()?.parse::<syn::LitStr>()?.value(),
                        "deps" => dependencies = meta.value()?.parse()?,
                        "captures" => {
                            let content;
                            syn::parenthesized!(content in meta.input);
                            captures = content
                                .parse_terminated(|input| input.parse::<syn::Ident>(), Token![,])?
                                .into_iter()
                                .collect();
                            let mut seen = HashSet::new();
                            for capture in &captures {
                                if !seen.insert(capture.to_string()) {
                                    return Err(error(capture, "duplicate effect capture"));
                                }
                            }
                        }
                        _ => return Err(meta.error("expected id, deps, or captures")),
                    }
                    Ok(())
                })?;
            }
            if slot.is_empty() || !effects.insert(slot.clone()) {
                return Err(error(
                    local,
                    "effect IDs must be nonempty and unique within a view",
                ));
            }
            let init = local
                .init
                .as_ref()
                .ok_or_else(|| error(local, "effect requires a setup closure"))?;
            if init.diverge.is_some() {
                return Err(error(local, "effect does not support let-else"));
            }
            let syn::Expr::Closure(original) = init.expr.as_ref() else {
                return Err(error(&init.expr, "effect requires a setup closure"));
            };
            let mut closure = original.clone();
            if closure.asyncness.is_some() || closure.inputs.len() != 1 {
                return Err(error(
                    &closure,
                    "effect requires one synchronous Runtime argument",
                ));
            }
            closure.capture = Some(Default::default());
            // The closure is passed directly so Runtime and Cleanup are inferred,
            // including coercion of a boxed cleanup closure to the Cleanup alias.
            *statement = parse_quote!({
                #(let #captures = ::core::clone::Clone::clone(&#captures);)*
                #cx.effect(#slot, #dependencies, #closure);
            });
        } else if attr.path().is_ident("environment") || attr.path().is_ident("environment_object")
        {
            if local.init.is_some() {
                return Err(error(
                    local,
                    "environment declarations cannot have initializers",
                ));
            }
            let ty = ty.ok_or_else(|| error(local, "environment declaration requires a type"))?;
            if attr.path().is_ident("environment") {
                let key: syn::Path = attr.parse_args()?;
                *statement = parse_quote!(let #name: #ty = #cx.environment::<#key>(););
            } else {
                *statement = parse_quote!(let #name: #ty = #cx.object()?;);
            }
        } else {
            let captures =
                attr.parse_args_with(Punctuated::<syn::Ident, Token![,]>::parse_terminated)?;
            let mut seen = HashSet::new();
            for c in &captures {
                if !seen.insert(c.to_string()) {
                    return Err(error(c, "duplicate action capture"));
                }
            }
            let init = local
                .init
                .as_ref()
                .ok_or_else(|| error(local, "action requires a closure"))?;
            let syn::Expr::Closure(original) = init.expr.as_ref() else {
                return Err(error(&init.expr, "action requires a closure"));
            };
            let mut closure = original.clone();
            if closure.asyncness.is_some() || closure.inputs.len() != 1 {
                return Err(error(
                    &closure,
                    "action requires one synchronous ActionContext argument",
                ));
            }
            let parameter = closure.inputs.first().unwrap();
            if !matches!(parameter, syn::Pat::Type(_)) {
                let parameter = parameter.clone();
                closure.inputs[0] = syn::Pat::Type(syn::PatType {
                    attrs: vec![],
                    pat: Box::new(parameter),
                    colon_token: Default::default(),
                    ty: Box::new(parse_quote!(&mut #kit::ActionContext<'_>)),
                });
            }
            closure.capture = Some(Default::default());
            let captures: Vec<_> = captures.iter().collect();
            *statement = parse_quote!(let #name = {#(let #captures = ::core::clone::Clone::clone(&#captures);)* #closure};);
        }
    }
    struct Nested(Option<syn::Error>);
    impl<'a> Visit<'a> for Nested {
        fn visit_attribute(&mut self, a: &'a syn::Attribute) {
            if helper(a) {
                self.0 = Some(error(
                    a,
                    "Refract declarations must be unconditional top-level statements in a view",
                ));
            }
        }
    }
    let mut nested = Nested(None);
    nested.visit_block(&f.block);
    if let Some(e) = nested.0 {
        return Err(e);
    }
    struct Returns;
    impl Fold for Returns {
        fn fold_expr(&mut self, e: syn::Expr) -> syn::Expr {
            match e {
                syn::Expr::Closure(_) => e,
                syn::Expr::Return(mut r) => {
                    if let Some(value) = r.expr {
                        r.expr = Some(Box::new(parse_quote!(::core::result::Result::Ok(#value))));
                    }
                    syn::Expr::Return(r)
                }
                _ => syn::fold::fold_expr(self, e),
            }
        }
        fn fold_item(&mut self, item: syn::Item) -> syn::Item {
            item
        }
    }
    let mut body = Returns.fold_block(*f.block);
    match body.stmts.last_mut() {
        Some(syn::Stmt::Expr(expression, None)) => {
            let value = expression.clone();
            *expression = parse_quote!(::core::result::Result::Ok(#value));
        }
        _ => return Err(error(&body, "view body must end with a view expression")),
    }
    f.sig.output = parse_quote!(-> impl #kit::View);
    f.block = Box::new(
        parse_quote!({#kit::view_fn(move |#cx: &mut #kit::ViewContext<'_>|{#(let #props = ::core::clone::Clone::clone(&#props);)* #body})}),
    );
    Ok(quote!(#f))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_effect_declarations() {
        let cases = [
            (quote!(#[effect] let timer = start();), "setup closure"),
            (quote!(#[effect] let timer = || {};), "Runtime argument"),
            (quote!(#[effect] let timer = async |_| {};), "synchronous"),
            (quote!(#[effect] let timer: () = |_| {};), "omit the type"),
            (quote!(#[effect(id = "")] let timer = |_| {};), "nonempty"),
            (
                quote!(#[effect(captures(a, a))] let timer = |_| {};),
                "duplicate effect capture",
            ),
            (
                quote!(#[effect(deps = (), deps = ())] let timer = |_| {};),
                "duplicate effect option",
            ),
            (
                quote!(#[effect(unknown = ())] let timer = |_| {};),
                "expected id, deps, or captures",
            ),
            (
                quote!(#[effect] let timer = |_| {}; #[effect(id = "timer")] let other = |_| {};),
                "unique",
            ),
            (
                quote!(if true {
                    #[effect]
                    let timer = |_| {};
                }),
                "unconditional",
            ),
        ];
        for (declaration, expected) in cases {
            let result = expand(
                quote!(),
                quote! {
                    fn test(cx: &mut ViewContext<'_>) -> impl View {
                        #declaration
                        text("test")
                    }
                },
            );
            let error = result.unwrap_err().to_string();
            assert!(error.contains(expected), "{error}; expected {expected}");
        }
    }

    #[test]
    fn rejects_conditional_state() {
        let error = expand(
            quote!(),
            quote!(
                fn test(cx: &mut ViewContext<'_>) -> impl View {
                    if true {
                        #[state]
                        let x = 1;
                    }
                    text("x")
                }
            ),
        )
        .unwrap_err();
        assert!(error.to_string().contains("unconditional"));
    }
    #[test]
    fn rejects_duplicate_state() {
        assert!(expand(
            quote!(),
            quote!(
                fn test(cx: &mut ViewContext<'_>) -> impl View {
                    #[state]
                    let x = 1;
                    #[state(id = "x")]
                    let y = 2;
                    text("x")
                }
            )
        )
        .unwrap_err()
        .to_string()
        .contains("unique"));
    }
    #[test]
    fn rejects_borrowed_props() {
        assert!(expand(
            quote!(),
            quote!(
                fn test(cx: &mut ViewContext<'_>, prop: &str) -> impl View {
                    text(prop)
                }
            )
        )
        .unwrap_err()
        .to_string()
        .contains("owned"));
    }
    #[test]
    fn expansion_supports_actions_and_environment() {
        assert!(expand(
            quote!(),
            quote!(
                fn test(cx: &mut ViewContext<'_>) -> impl View {
                    #[state]
                    let n: i32 = 0;
                    #[environment(Theme)]
                    let theme: i32;
                    #[action(n)]
                    let add = |cx| n.update(cx.runtime(), |v| *v += 1);
                    h_stack()
                        .alignment(Alignment::Center)
                        .justify(Alignment::Center)
                        .child(text(theme.to_string()))
                        .on_press(add)
                }
            )
        )
        .is_ok());
    }
}
