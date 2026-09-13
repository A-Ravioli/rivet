//! `#[derive(Randomize)]`: constrained-random construction.
//!
//! ```ignore
//! #[derive(Randomize, Debug)]
//! #[rand(constraint = |t: &Self| t.lo < t.hi)]
//! struct Txn {
//!     #[rand(range = 0..256u32)]           lo: u32,
//!     #[rand(range = 0..=255u32)]          hi: u32,
//!     #[rand(one_of = [1, 2, 4, 8])]       burst: u8,
//!     #[rand(weighted = [(0, 9), (1, 1)])] err: u8,
//!     #[rand(with = |r| r.gen_range(0..4) * 4)] aligned: u32,
//!     #[rand(skip)]                        note: String,   // Default
//!     data: [u8; 4],                                       // Random
//!     kind: Kind,                                          // Randomize
//! }
//!
//! #[derive(Randomize)]
//! enum Kind { #[rand(weight = 3)] Read, Write { #[rand(range = 0..4)] strb: u8 }, #[rand(skip)] Never }
//! ```
//!
//! Constraints are satisfied by rejection sampling (`tries`, default 1000);
//! exceeding it panics with the type name so the test fails loudly.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Expr, Fields, LitInt, Path};

struct FieldOpts {
    range: Option<Expr>,
    one_of: Option<Expr>,
    weighted: Option<Expr>,
    with: Option<Expr>,
    skip: bool,
}

struct TypeOpts {
    constraints: Vec<Expr>,
    tries: u64,
    krate: Path,
}

struct VariantOpts {
    weight: u32,
    skip: bool,
}

fn parse_field_opts(attrs: &[Attribute]) -> syn::Result<FieldOpts> {
    let mut o = FieldOpts { range: None, one_of: None, weighted: None, with: None, skip: false };
    for a in attrs.iter().filter(|a| a.path().is_ident("rand")) {
        a.parse_nested_meta(|m| {
            let key = m.path.get_ident().map(|i| i.to_string()).unwrap_or_default();
            match key.as_str() {
                "range" => o.range = Some(m.value()?.parse()?),
                "one_of" => o.one_of = Some(m.value()?.parse()?),
                "weighted" => o.weighted = Some(m.value()?.parse()?),
                "with" => o.with = Some(m.value()?.parse()?),
                "skip" => o.skip = true,
                _ => return Err(m.error("expected range, one_of, weighted, with, or skip")),
            }
            Ok(())
        })?;
    }
    let n = [o.range.is_some(), o.one_of.is_some(), o.weighted.is_some(), o.with.is_some(), o.skip]
        .iter()
        .filter(|b| **b)
        .count();
    if n > 1 {
        return Err(syn::Error::new_spanned(&attrs[0], "only one of range, one_of, weighted, with, skip per field"));
    }
    Ok(o)
}

fn parse_type_opts(attrs: &[Attribute]) -> syn::Result<TypeOpts> {
    let mut o = TypeOpts { constraints: Vec::new(), tries: 1000, krate: syn::parse_quote!(::rivet) };
    for a in attrs.iter().filter(|a| a.path().is_ident("rand")) {
        a.parse_nested_meta(|m| {
            let key = m.path.get_ident().map(|i| i.to_string()).unwrap_or_default();
            match key.as_str() {
                "constraint" => o.constraints.push(m.value()?.parse()?),
                "tries" => o.tries = m.value()?.parse::<LitInt>()?.base10_parse()?,
                "crate" => o.krate = m.value()?.parse()?,
                _ => return Err(m.error("expected constraint, tries, or crate")),
            }
            Ok(())
        })?;
    }
    Ok(o)
}

fn parse_variant_opts(attrs: &[Attribute]) -> syn::Result<VariantOpts> {
    let mut o = VariantOpts { weight: 1, skip: false };
    for a in attrs.iter().filter(|a| a.path().is_ident("rand")) {
        a.parse_nested_meta(|m| {
            let key = m.path.get_ident().map(|i| i.to_string()).unwrap_or_default();
            match key.as_str() {
                "weight" => o.weight = m.value()?.parse::<LitInt>()?.base10_parse()?,
                "skip" => o.skip = true,
                _ => return Err(m.error("expected weight or skip")),
            }
            Ok(())
        })?;
    }
    Ok(o)
}

/// Expression producing one field's value from `rng`.
fn field_value(f: &syn::Field, krate: &Path) -> syn::Result<TokenStream> {
    let o = parse_field_opts(&f.attrs)?;
    let ty = &f.ty;
    Ok(if o.skip {
        quote! { ::core::default::Default::default() }
    } else if let Some(r) = o.range {
        quote! { rng.gen_range(#r) }
    } else if let Some(items) = o.one_of {
        quote! { ::core::clone::Clone::clone(rng.choose(&#items).expect("one_of: empty list")) }
    } else if let Some(items) = o.weighted {
        quote! { ::core::clone::Clone::clone(rng.choose_weighted(&#items).expect("weighted: all weights zero")) }
    } else if let Some(w) = o.with {
        quote! { (#w)(rng) }
    } else {
        quote! { <#ty as #krate::Randomize>::randomize(rng) }
    })
}

fn build_fields(fields: &Fields, krate: &Path) -> syn::Result<TokenStream> {
    Ok(match fields {
        Fields::Named(named) => {
            let inits = named
                .named
                .iter()
                .map(|f| {
                    let name = f.ident.as_ref().unwrap();
                    let v = field_value(f, krate)?;
                    Ok(quote! { #name: #v })
                })
                .collect::<syn::Result<Vec<_>>>()?;
            quote! { { #(#inits),* } }
        }
        Fields::Unnamed(unnamed) => {
            let inits = unnamed.unnamed.iter().map(|f| field_value(f, krate)).collect::<syn::Result<Vec<_>>>()?;
            quote! { ( #(#inits),* ) }
        }
        Fields::Unit => quote! {},
    })
}

pub fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let opts = parse_type_opts(&input.attrs)?;
    let krate = &opts.krate;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let build: TokenStream = match &input.data {
        Data::Struct(s) => {
            let body = build_fields(&s.fields, krate)?;
            quote! { #name #body }
        }
        Data::Enum(e) => {
            let mut arms = Vec::new();
            let mut weights = Vec::new();
            for (i, v) in e.variants.iter().enumerate() {
                let vo = parse_variant_opts(&v.attrs)?;
                if vo.skip || vo.weight == 0 {
                    continue;
                }
                let vname = &v.ident;
                let body = build_fields(&v.fields, krate)?;
                let idx = i as u32;
                let w = vo.weight;
                arms.push(quote! { #idx => #name::#vname #body });
                weights.push(quote! { (#idx, #w) });
            }
            if arms.is_empty() {
                return Err(syn::Error::new_spanned(name, "Randomize: enum has no selectable variants"));
            }
            quote! {
                {
                    const WEIGHTS: &[(u32, u32)] = &[#(#weights),*];
                    match *rng.choose_weighted(WEIGHTS).unwrap() {
                        #(#arms,)*
                        _ => unreachable!(),
                    }
                }
            }
        }
        Data::Union(_) => return Err(syn::Error::new_spanned(name, "Randomize cannot be derived for unions")),
    };

    let tries = opts.tries;
    let constraints = &opts.constraints;
    let name_str = name.to_string();
    let check = if constraints.is_empty() {
        quote! { true }
    } else {
        quote! { #( (#constraints)(&candidate) )&&* }
    };
    Ok(quote! {
        impl #impl_generics #krate::Randomize for #name #ty_generics #where_clause {
            fn randomize(rng: &mut #krate::Rng) -> Self {
                #[allow(unused_imports)]
                use #krate::Random as _;
                for _ in 0..#tries {
                    let candidate: Self = #build;
                    if #check {
                        return candidate;
                    }
                }
                panic!("Randomize for {}: no value satisfied the constraints after {} tries", #name_str, #tries)
            }
        }
    })
}
