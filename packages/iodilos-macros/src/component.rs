//! The `#[component]` attribute macro implementation.
//!
//! Ported from `sycamore-macro`'s `component` module. The async-component
//! branch is intentionally dropped: iodilos components are synchronous `View`
//! constructors, and async work belongs to [`use_future`](::iodilos::use_future).
//! An `async fn` component emits a compile error pointing there.

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    AttrStyle, Attribute, Error, FnArg, Generics, Ident, Item, ItemFn, Meta, Pat, Result,
    ReturnType, Token, Type, TypeTuple, parse_quote,
};

pub struct ComponentFn {
    pub f: ItemFn,
}

impl Parse for ComponentFn {
    fn parse(input: ParseStream) -> Result<Self> {
        // Parse macro body.
        let parsed: Item = input.parse()?;

        match parsed {
            Item::Fn(mut f) => {
                let ItemFn { sig, .. } = &mut f;

                if sig.constness.is_some() {
                    return Err(syn::Error::new(
                        sig.constness.span(),
                        "const functions can't be components",
                    ));
                }

                if sig.abi.is_some() {
                    return Err(syn::Error::new(
                        sig.abi.span(),
                        "extern functions can't be components",
                    ));
                }

                if let ReturnType::Default = sig.output {
                    return Err(syn::Error::new(
                        sig.paren_token.span.close(),
                        "component must return `iodilos::view::View`",
                    ));
                };

                let inputs = sig.inputs.clone().into_iter().collect::<Vec<_>>();

                match &inputs[..] {
                    [] => {}
                    [input] => {
                        if let FnArg::Receiver(_) = input {
                            return Err(syn::Error::new(
                                input.span(),
                                "components can't accept a receiver",
                            ));
                        }

                        if let FnArg::Typed(pat) = input
                            && let Type::Tuple(TypeTuple { elems, .. }) = &*pat.ty
                            && elems.is_empty()
                        {
                            return Err(syn::Error::new(
                                pat.ty.span(),
                                "taking an unit tuple as props is useless",
                            ));
                        }
                    }
                    [..] => {
                        if inputs.len() > 1 {
                            return Err(syn::Error::new(
                                sig.inputs
                                    .clone()
                                    .into_iter()
                                    .skip(2)
                                    .collect::<Punctuated<_, Token![,]>>()
                                    .span(),
                                "component should not take more than 1 parameter",
                            ));
                        }
                    }
                };

                Ok(Self { f })
            }
            item => Err(syn::Error::new_spanned(
                item,
                "the `component` attribute can only be applied to functions",
            )),
        }
    }
}

impl ToTokens for ComponentFn {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let ComponentFn { f } = self;
        let ItemFn { sig, .. } = &f;

        if sig.asyncness.is_some() {
            // iodilos components are synchronous View constructors; async work
            // goes through `use_future`, not an `async fn` component signature.
            tokens.extend(
                Error::new(
                    sig.asyncness.span(),
                    "iodilos components cannot be `async`; use `iodilos::use_future` for async work",
                )
                .to_compile_error(),
            );
            return;
        }

        tokens.extend(quote! {
            #[allow(non_snake_case)]
            #f
        })
    }
}

/// Arguments to the `component` attribute proc-macro.
pub struct ComponentArgs {
    inline_props: Option<Ident>,
    _comma: Option<Token![,]>,
    attrs: Punctuated<Meta, Token![,]>,
}

impl Parse for ComponentArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let inline_props: Option<Ident> = input.parse()?;
        let (comma, attrs) = if let Some(inline_props) = &inline_props {
            // Check if the ident is correct.
            if *inline_props != "inline_props" {
                return Err(Error::new(inline_props.span(), "expected `inline_props`"));
            }

            let comma: Option<Token![,]> = input.parse()?;
            let attrs: Punctuated<Meta, Token![,]> = if comma.is_some() {
                input.parse_terminated(Meta::parse, Token![,])?
            } else {
                Punctuated::new()
            };
            (comma, attrs)
        } else {
            (None, Punctuated::new())
        };
        Ok(Self {
            inline_props,
            _comma: comma,
            attrs,
        })
    }
}

pub fn component_impl(args: ComponentArgs, item: TokenStream) -> Result<TokenStream> {
    if args.inline_props.is_some() {
        let mut item_fn = syn::parse::<ItemFn>(item.into())?;
        let inline_props = inline_props_impl(&mut item_fn, args.attrs)?;
        // TODO: don't parse the function twice.
        let comp = syn::parse::<ComponentFn>(item_fn.to_token_stream().into())?;
        Ok(quote! {
            #inline_props
            #comp
        })
    } else {
        let comp = syn::parse::<ComponentFn>(item.into())?;
        Ok(comp.to_token_stream())
    }
}

/// Codegens the new props struct and modifies the component body to accept this new struct as
/// props.
fn inline_props_impl(item: &mut ItemFn, attrs: Punctuated<Meta, Token![,]>) -> Result<TokenStream> {
    let props_vis = &item.vis;
    let props_struct_ident = format_ident!("{}_Props", item.sig.ident);

    let inputs = item.sig.inputs.clone();
    let props = inputs.clone().into_iter().collect::<Vec<_>>();
    let generics: &mut Generics = &mut item.sig.generics;
    let mut fields = Vec::new();
    for arg in inputs {
        match arg {
            FnArg::Receiver(receiver) => {
                return Err(syn::Error::new(
                    receiver.span(),
                    "`self` cannot be a property",
                ));
            }
            FnArg::Typed(pat_type) => match *pat_type.pat {
                Pat::Ident(ident_pat) => crate::inline_props::push_field(
                    &mut fields,
                    generics,
                    pat_type.attrs,
                    ident_pat.clone().ident,
                    *pat_type.ty,
                ),
                _ => {
                    return Err(syn::Error::new(
                        pat_type.pat.span(),
                        "pattern must contain an identifier, properties cannot be unnamed",
                    ));
                }
            },
        }
    }

    let generics_phantoms = generics.params.iter().enumerate().filter_map(|(i, param)| {
        let phantom_ident = format_ident!("__phantom{i}");
        match param {
            syn::GenericParam::Type(ty) => {
                let ty = &ty.ident;
                Some(quote! {
                    #[prop(default, setter(skip))]
                    #phantom_ident: ::std::marker::PhantomData<#ty>
                })
            }
            syn::GenericParam::Lifetime(lt) => {
                let lt = &lt.lifetime;
                Some(quote! {
                    #[prop(default, setter(skip))]
                    #phantom_ident: ::std::marker::PhantomData<&#lt ()>
                })
            }
            syn::GenericParam::Const(_) => None,
        }
    });

    let doc_comment = format!("Props for [`{}`].", item.sig.ident);

    let attrs = attrs.into_iter().map(|a| Attribute {
        pound_token: Token![#](Span::call_site()),
        style: AttrStyle::Outer,
        bracket_token: Default::default(),
        meta: a,
    });
    let ret = Ok(quote! {
        #[allow(non_camel_case_types)]
        #[doc = #doc_comment]
        #[derive(::iodilos::rt::Props)]
        #(#attrs)*
        #props_vis struct #props_struct_ident #generics {
            #(#fields,)*
            #(#generics_phantoms,)*
        }
    });

    // Rewrite component body.

    // Get the ident (technically, patterns) of each prop.
    let props_pats = props.iter().map(|arg| match arg {
        FnArg::Receiver(_) => unreachable!(),
        FnArg::Typed(arg) => match &*arg.pat {
            Pat::Ident(pat) => {
                if pat.subpat.is_some() {
                    let ident = &pat.ident;
                    quote! { #ident: #pat }
                } else {
                    quote! { #pat }
                }
            }
            _ => unreachable!(),
        },
    });
    // Rewrite function signature.
    let props_struct_generics = generics.split_for_impl().1;
    item.sig.inputs = parse_quote! { __props: #props_struct_ident #props_struct_generics };
    // Rewrite function body.
    let block = item.block.clone();
    item.block = parse_quote! {{
        let #props_struct_ident {
            #(#props_pats,)*
            ..
        } = __props;
        #block
    }};

    ret
}
