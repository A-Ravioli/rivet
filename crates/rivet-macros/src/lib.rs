//! Proc macros for Rivet: `#[rivet::test]`.

use proc_macro::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{parse_macro_input, Expr, Ident, ItemFn, LitBool, LitInt, Token};

struct TestArgs {
    timeout: Option<Expr>,
    skip: bool,
    expect_fail: bool,
    stage: i32,
}

impl Parse for TestArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = TestArgs { timeout: None, skip: false, expect_fail: false, stage: 0 };
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            match key.to_string().as_str() {
                "timeout" => {
                    input.parse::<Token![=]>()?;
                    args.timeout = Some(input.parse()?);
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
                        args.expect_fail = input.parse::<LitBool>()?.value;
                    } else {
                        args.expect_fail = true;
                    }
                }
                "stage" => {
                    input.parse::<Token![=]>()?;
                    args.stage = input.parse::<LitInt>()?.base10_parse()?;
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
/// Attributes: `timeout = <Duration expr>`, `skip`, `expect_fail`,
/// `stage = <i32>`.
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
    let stage = args.stage;
    // The dut parameter may be `Module` or any type implementing `rivet::Bind`.
    let dut_ty = func.sig.inputs.first().and_then(|arg| match arg {
        syn::FnArg::Typed(pt) => Some((*pt.ty).clone()),
        _ => None,
    });
    let call = match &dut_ty {
        Some(ty) => quote! {
            async move {
                let dut = <#ty as ::rivet::Bind>::bind(dut)?;
                #name(dut).await
            }
        },
        None => quote! { #name() },
    };
    let expanded = quote! {
        #func

        ::rivet::inventory::submit! {
            ::rivet::TestDesc {
                name: #name_str,
                module: module_path!(),
                run: |dut: ::rivet::Module| ::rivet::test::boxed(#call),
                timeout: || {
                    #[allow(unused_imports)]
                    use ::rivet::TimeExt as _;
                    #timeout
                },
                skip: #skip,
                expect_fail: #expect_fail,
                stage: #stage,
            }
        }
    };
    expanded.into()
}
