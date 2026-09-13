//! Proc macros for Rivet: `#[rivet::test]` and `#[derive(Randomize)]`.

mod randomize;

use proc_macro::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{parse_macro_input, Expr, Ident, ItemFn, LitBool, LitInt, LitStr, Token};

struct TestArgs {
    timeout: Option<Expr>,
    wall_timeout: Option<Expr>,
    skip: bool,
    expect_fail: bool,
    /// `expect_fail = "message"`: the failure must say this.
    expect_fail_msg: Option<String>,
    /// `expect_timeout`: the test is expected to run out of simulated time.
    expect_timeout: bool,
    stage: i32,
    /// `params = [a, b, c]`: one test per element, passed as the second
    /// argument.
    params: Option<syn::ExprArray>,
    /// `param_sets = ["w8", "w16"]`: only run under these manifest
    /// parameter sets (empty: all).
    param_sets: Vec<String>,
}

impl Parse for TestArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = TestArgs {
            timeout: None,
            wall_timeout: None,
            skip: false,
            expect_fail: false,
            expect_fail_msg: None,
            expect_timeout: false,
            stage: 0,
            params: None,
            param_sets: Vec::new(),
        };
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            match key.to_string().as_str() {
                "timeout" => {
                    input.parse::<Token![=]>()?;
                    args.timeout = Some(input.parse()?);
                }
                "wall_timeout" => {
                    input.parse::<Token![=]>()?;
                    args.wall_timeout = Some(input.parse()?);
                }
                "skip" => {
                    if input.peek(Token![=]) {
                        input.parse::<Token![=]>()?;
                        args.skip = input.parse::<LitBool>()?.value;
                    } else {
                        args.skip = true;
                    }
                }
                "expect_fail" => {
                    if input.peek(Token![=]) {
                        input.parse::<Token![=]>()?;
                        if input.peek(LitStr) {
                            // `expect_fail = "overflow"`: the message must
                            // contain this, so a test cannot pass by failing
                            // for an unrelated reason.
                            args.expect_fail_msg = Some(input.parse::<LitStr>()?.value());
                            args.expect_fail = true;
                        } else {
                            args.expect_fail = input.parse::<LitBool>()?.value;
                        }
                    } else {
                        args.expect_fail = true;
                    }
                }
                "expect_timeout" => {
                    if input.peek(Token![=]) {
                        input.parse::<Token![=]>()?;
                        args.expect_timeout = input.parse::<LitBool>()?.value;
                    } else {
                        args.expect_timeout = true;
                    }
                }
                "stage" => {
                    input.parse::<Token![=]>()?;
                    args.stage = input.parse::<LitInt>()?.base10_parse()?;
                }
                "params" => {
                    input.parse::<Token![=]>()?;
                    args.params = Some(input.parse()?);
                }
                "param_sets" => {
                    input.parse::<Token![=]>()?;
                    let arr: syn::ExprArray = input.parse()?;
                    for e in arr.elems {
                        match e {
                            Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) => args.param_sets.push(s.value()),
                            other => {
                                return Err(syn::Error::new_spanned(
                                    other,
                                    "param_sets entries must be string literals",
                                ))
                            }
                        }
                    }
                }
                other => return Err(syn::Error::new(key.span(), format!("unknown test attribute `{other}`"))),
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(args)
    }
}

/// Mark an `async fn(dut: Module) -> rivet::Result<()>` as a test.
///
/// ```ignore
/// #[rivet::test(timeout = 10.us())]
/// async fn my_test(dut: Module) -> rivet::Result<()> { Ok(()) }
/// ```
///
/// Attributes: `timeout = <Duration expr>` (simulated time),
/// `wall_timeout = <seconds>`, `skip`, `expect_fail` (optionally
/// `expect_fail = "part of the message"`), `expect_timeout`, `stage = <i32>`,
/// `params = [v, ...]` (registers `name[v]` per value, passed as the second
/// argument), `param_sets = ["a", ...]` (only under these `rivet.toml`
/// parameter sets).
#[proc_macro_attribute]
pub fn test(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as TestArgs);
    let func = parse_macro_input!(item as ItemFn);
    let name = &func.sig.ident;
    let name_str = name.to_string();
    if func.sig.asyncness.is_none() {
        return syn::Error::new_spanned(&func.sig, "#[rivet::test] functions must be `async fn`")
            .to_compile_error()
            .into();
    }
    let timeout = match &args.timeout {
        Some(e) => quote! { Some(#e) },
        None => quote! { None },
    };
    let skip = args.skip;
    let expect_fail = args.expect_fail;
    let expect_fail_msg = match &args.expect_fail_msg {
        Some(m) => quote! { Some(#m) },
        None => quote! { None },
    };
    let expect_timeout = args.expect_timeout;
    let stage = args.stage;
    let wall_timeout = match &args.wall_timeout {
        Some(e) => quote! { Some((#e) as f64) },
        None => quote! { None },
    };
    // The dut parameter may be `Module` or any type implementing `rivet::Bind`.
    let dut_ty = func.sig.inputs.first().and_then(|arg| match arg {
        syn::FnArg::Typed(pt) => Some((*pt.ty).clone()),
        _ => None,
    });
    let param_sets = &args.param_sets;
    // One registration per parameter value (or exactly one without params).
    let variants: Vec<(String, Option<Expr>)> = match &args.params {
        Some(arr) => arr
            .elems
            .iter()
            .map(|e| {
                let text: String = quote!(#e).to_string().chars().filter(|c| !c.is_whitespace()).collect();
                (format!("{name_str}[{text}]"), Some(e.clone()))
            })
            .collect(),
        None => vec![(name_str.clone(), None)],
    };
    if args.params.is_some() && func.sig.inputs.len() != 2 {
        return syn::Error::new_spanned(&func.sig, "a test with `params` takes two arguments: (dut, param)")
            .to_compile_error()
            .into();
    }
    let registrations = variants.iter().map(|(vname, param)| {
        let call = match (&dut_ty, param) {
            (Some(ty), Some(p)) => quote! {
                async move {
                    let dut = <#ty as ::rivet::Bind>::bind(dut)?;
                    #name(dut, #p).await
                }
            },
            (Some(ty), None) => quote! {
                async move {
                    let dut = <#ty as ::rivet::Bind>::bind(dut)?;
                    #name(dut).await
                }
            },
            (None, _) => quote! { #name() },
        };
        quote! {
            ::rivet::inventory::submit! {
                ::rivet::TestDesc {
                    name: #vname,
                    module: module_path!(),
                    run: |dut: ::rivet::Module| ::rivet::test::boxed(#call),
                    timeout: || {
                        #[allow(unused_imports)]
                        use ::rivet::TimeExt as _;
                        #timeout
                    },
                    skip: #skip,
                    expect_fail: #expect_fail,
                    expect_fail_msg: #expect_fail_msg,
                    expect_timeout: #expect_timeout,
                    file: file!(),
                    line: line!(),
                    stage: #stage,
                    wall_timeout: #wall_timeout,
                    param_sets: &[#(#param_sets),*],
                }
            }
        }
    });
    let expanded = quote! {
        #func
        #(#registrations)*
    };
    expanded.into()
}

/// Derive [`Randomize`](../rivet/trait.Randomize.html) for a struct or
/// enum. See the `randomize` module docs for the attribute grammar.
#[proc_macro_derive(Randomize, attributes(rand))]
pub fn derive_randomize(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as syn::DeriveInput);
    match randomize::derive(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> syn::Result<TestArgs> {
        syn::parse2::<TestArgs>(s.parse().unwrap())
    }

    // `test` is this crate's own macro, so name the built-in explicitly.
    #[core::prelude::v1::test]
    fn attribute_forms() {
        let a = parse("").unwrap();
        assert!(a.timeout.is_none() && !a.skip && !a.expect_fail && a.stage == 0);
        let a = parse("timeout = 10.us(), skip, expect_fail, stage = -2").unwrap();
        assert!(a.timeout.is_some() && a.skip && a.expect_fail && a.stage == -2);
        let a = parse("skip = false, expect_fail = true,").unwrap();
        assert!(!a.skip && a.expect_fail);
        let a = parse("wall_timeout = 30").unwrap();
        assert!(a.wall_timeout.is_some());
        let a = parse(r#"params = [8, (16, 2)], param_sets = ["w8"]"#).unwrap();
        assert_eq!(a.params.unwrap().elems.len(), 2);
        assert_eq!(a.param_sets, ["w8"]);
        assert!(parse("param_sets = [1]").is_err());
        assert!(parse("bogus = 1").is_err());
        assert!(parse("stage = x").is_err());
    }
}
