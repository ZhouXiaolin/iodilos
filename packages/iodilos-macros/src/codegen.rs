//! Codegen for the iodilos `view!` macro.

use crate::ir::{DynNode, Node, Prop, PropType, Root, TagIdent, TagNode, TextNode};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Expr, Pat};

macro_rules! rt {
    () => {
        quote! { ::iodilos::rt }
    };
}

pub struct Codegen;

impl Codegen {
    pub fn new() -> Self {
        Self
    }

    pub fn root(&self, root: &Root) -> TokenStream {
        let rt = rt!();
        match &root.0[..] {
            [] => quote! { #rt::View::new() },
            [node] => self.node(node),
            nodes => {
                let nodes = nodes.iter().map(|node| self.node(node));
                quote! {
                    ::std::convert::Into::<#rt::View>::into(::std::vec![#(#nodes),*])
                }
            }
        }
    }

    pub fn node(&self, node: &Node) -> TokenStream {
        let rt = rt!();
        match node {
            Node::Tag(tag) => {
                if is_component(&tag.ident) {
                    self.component(tag)
                } else {
                    self.element(tag)
                }
            }
            Node::Text(TextNode { value }) => {
                quote! { ::std::convert::Into::<#rt::View>::into(#value) }
            }
            Node::Dyn(DynNode { value }) => {
                if is_dyn(value) {
                    quote! {
                        #rt::View::from_dynamic(
                            move || ::std::convert::Into::<#rt::View>::into(#value)
                        )
                    }
                } else {
                    quote! { ::std::convert::Into::<#rt::View>::into(#value) }
                }
            }
        }
    }

    pub fn element(&self, element: &TagNode) -> TokenStream {
        let rt = rt!();
        let TagNode {
            ident,
            props,
            children,
        } = element;

        let attributes = props.iter().map(|attr| self.attribute(attr));
        let children = children
            .0
            .iter()
            .map(|child| self.node(child))
            .collect::<Vec<_>>();

        match ident {
            TagIdent::Path(tag) => {
                assert!(tag.get_ident().is_some(), "elements must be an ident");
                quote! {
                    #rt::View::from(
                        #rt::tags::#tag().children(::std::vec![#(#children),*])#(#attributes)*
                    )
                }
            }
            TagIdent::Hyphenated(tag) => quote! {
                #rt::View::from(
                    #rt::custom_element(#tag).children(::std::vec![#(#children),*])#(#attributes)*
                )
            },
        }
    }

    pub fn attribute(&self, attr: &Prop) -> TokenStream {
        let rt = rt!();
        let value = &attr.value;
        let dyn_value = if is_dyn(value) {
            quote! { move || #value }
        } else {
            quote! { #value }
        };

        match &attr.ty {
            PropType::Plain { ident } => quote! { .#ident(#dyn_value) },
            PropType::PlainHyphenated { ident } | PropType::PlainQuoted { ident } => {
                quote! { .attr(#ident, #dyn_value) }
            }
            PropType::Directive { dir, ident } => match dir.to_string().as_str() {
                "on" => quote! { .on(#rt::events::#ident, #value) },
                "prop" => {
                    let ident = ident.to_string();
                    quote! { .prop(#ident, #dyn_value) }
                }
                "bind" => quote! { .bind(#rt::bind::#ident, #value) },
                _ => syn::Error::new(dir.span(), format!("unknown directive `{dir}`"))
                    .to_compile_error(),
            },
            PropType::Ref => quote! { .r#ref(#value) },
            PropType::Spread => quote! { .spread(#value) },
        }
    }

    pub fn component(
        &self,
        TagNode {
            ident,
            props,
            children,
        }: &TagNode,
    ) -> TokenStream {
        let rt = rt!();
        let ident = match ident {
            TagIdent::Path(path) => path,
            TagIdent::Hyphenated(_) => unreachable!("hyphenated tags are not components"),
        };

        let plain = props
            .iter()
            .filter_map(|prop| match &prop.ty {
                PropType::Plain { ident } => Some((ident, prop.value.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        let plain_names = plain.iter().map(|(ident, _)| ident);
        let plain_values = plain.iter().map(|(_, value)| value);

        let other_props = props
            .iter()
            .filter(|prop| !matches!(&prop.ty, PropType::Plain { .. }))
            .collect::<Vec<_>>();
        let other_attributes = other_props.iter().map(|prop| self.attribute(prop));

        let children_quoted = if children.0.is_empty() {
            quote! {}
        } else {
            let children = Codegen::new().root(children);
            quote! {
                .children(#rt::Children::new(move || { #children }))
            }
        };

        quote! {{
            let __component = &#ident;
            #rt::component_scope(move || #rt::Component::create(
                __component,
                #rt::element_like_component_builder(__component)
                    #(.#plain_names(#plain_values))*
                    #(#other_attributes)*
                    #children_quoted
                    .build()
            ))
        }}
    }
}

impl Default for Codegen {
    fn default() -> Self {
        Self::new()
    }
}

fn is_component(ident: &TagIdent) -> bool {
    match ident {
        TagIdent::Path(path) => {
            path.get_ident().is_none()
                || path
                    .get_ident()
                    .unwrap()
                    .to_string()
                    .chars()
                    .next()
                    .unwrap()
                    .is_ascii_uppercase()
        }
        TagIdent::Hyphenated(_) => false,
    }
}

fn is_dyn(ex: &Expr) -> bool {
    match ex {
        Expr::Lit(_) | Expr::Closure(_) | Expr::Path(_) => false,
        Expr::Field(f) => is_dyn(&f.base),
        Expr::Paren(p) => is_dyn(&p.expr),
        Expr::Group(g) => is_dyn(&g.expr),
        Expr::Tuple(t) => t.elems.iter().any(is_dyn),
        Expr::Array(a) => a.elems.iter().any(is_dyn),
        Expr::Repeat(r) => is_dyn(&r.expr) || is_dyn(&r.len),
        Expr::Struct(s) => s.fields.iter().any(|fv| is_dyn(&fv.expr)),
        Expr::Cast(c) => is_dyn(&c.expr),
        Expr::Macro(m) => is_dyn_macro(&m.mac),
        Expr::Block(b) => is_dyn_block(&b.block),
        Expr::Const(_) => false,
        Expr::Loop(l) => is_dyn_block(&l.body),
        Expr::While(w) => is_dyn(&w.cond) || is_dyn_block(&w.body),
        Expr::ForLoop(f) => is_dyn_pattern(&f.pat) || is_dyn(&f.expr) || is_dyn_block(&f.body),
        Expr::Break(_) | Expr::Continue(_) => false,
        Expr::Let(e) => is_dyn_pattern(&e.pat) || is_dyn(&e.expr),
        Expr::Match(m) => {
            is_dyn(&m.expr)
                || m.arms.iter().any(|arm| {
                    is_dyn_pattern(&arm.pat)
                        || arm.guard.as_ref().is_some_and(|(_, expr)| is_dyn(expr))
                        || is_dyn(&arm.body)
                })
        }
        Expr::If(i) => {
            is_dyn(&i.cond)
                || is_dyn_block(&i.then_branch)
                || i.else_branch.as_ref().is_some_and(|(_, expr)| is_dyn(expr))
        }
        Expr::Unary(u) => is_dyn(&u.expr),
        Expr::Binary(b) => is_dyn(&b.left) || is_dyn(&b.right),
        Expr::Index(i) => is_dyn(&i.expr) || is_dyn(&i.index),
        Expr::Range(r) => {
            r.start.as_deref().is_some_and(is_dyn) || r.end.as_deref().is_some_and(is_dyn)
        }
        _ => true,
    }
}

fn is_dyn_pattern(pat: &Pat) -> bool {
    match pat {
        Pat::Wild(_) | Pat::Lit(_) | Pat::Path(_) | Pat::Rest(_) | Pat::Type(_) | Pat::Const(_) => {
            false
        }
        Pat::Paren(p) => is_dyn_pattern(&p.pat),
        Pat::Or(o) => o.cases.iter().any(is_dyn_pattern),
        Pat::Tuple(t) => t.elems.iter().any(is_dyn_pattern),
        Pat::TupleStruct(s) => s.elems.iter().any(is_dyn_pattern),
        Pat::Slice(s) => s.elems.iter().any(is_dyn_pattern),
        Pat::Range(r) => {
            r.start.as_deref().is_some_and(is_dyn) || r.end.as_deref().is_some_and(is_dyn)
        }
        Pat::Reference(r) => r.mutability.is_some(),
        Pat::Ident(id) => {
            (id.by_ref.is_some() && id.mutability.is_some())
                || id
                    .subpat
                    .as_ref()
                    .is_some_and(|(_, pat)| is_dyn_pattern(pat))
        }
        Pat::Struct(s) => s.fields.iter().any(|field| is_dyn_pattern(&field.pat)),
        _ => true,
    }
}

fn is_dyn_macro(mac: &syn::Macro) -> bool {
    mac.path.get_ident().is_none_or(|ident| ident != "view")
}

fn is_dyn_block(block: &syn::Block) -> bool {
    block.stmts.iter().any(|stmt| match stmt {
        syn::Stmt::Expr(ex, _) => is_dyn(ex),
        syn::Stmt::Macro(m) => is_dyn_macro(&m.mac),
        syn::Stmt::Local(loc) => {
            is_dyn_pattern(&loc.pat)
                || loc.init.as_ref().is_some_and(|init| {
                    is_dyn(&init.expr)
                        || init.diverge.as_ref().is_some_and(|(_, expr)| is_dyn(expr))
                })
        }
        syn::Stmt::Item(_) => false,
    })
}
