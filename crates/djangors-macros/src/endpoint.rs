use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    parse::{Parse, ParseStream},
    parse2, Attribute, Expr, Ident, ItemFn, Lit, LitInt, LitStr, Meta, Token, Type,
};

#[derive(Default)]
struct EndpointArgs {
    method: Option<String>,
    path: Option<String>,
    response: Option<Type>,
    request: Option<Type>,
    query: Option<Type>,
    status: Option<u16>,
    tag: Option<String>,
    prefix: Option<String>,
    name: Option<String>,
    summary: Option<String>,
}

fn normalize_http_method(raw: &str, span: proc_macro2::Span) -> syn::Result<String> {
    let upper = raw.trim().to_ascii_uppercase();
    match upper.as_str() {
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS" => Ok(upper),
        _ => Err(syn::Error::new(
            span,
            format!(
                "unsupported HTTP method '{raw}' in #[endpoint] (expected GET, POST, PUT, PATCH, DELETE, HEAD, or OPTIONS)"
            ),
        )),
    }
}

impl Parse for EndpointArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut args = EndpointArgs::default();
        while !input.is_empty() {
            if input.peek(Ident) && input.peek2(Token![=]) {
                let key: Ident = input.parse()?;
                let _eq: Token![=] = input.parse()?;
                let key_str = key.to_string();
                match key_str.as_str() {
                    "method" => {
                        let m = if input.peek(LitStr) {
                            let lit: LitStr = input.parse()?;
                            normalize_http_method(&lit.value(), lit.span())?
                        } else {
                            let ident: Ident = input.parse()?;
                            normalize_http_method(&ident.to_string(), ident.span())?
                        };
                        args.method = Some(m);
                    }
                    "path" => {
                        let lit: LitStr = input.parse()?;
                        args.path = Some(lit.value());
                    }
                    "response" => {
                        args.response = Some(input.parse::<Type>()?);
                    }
                    "request" | "body" => {
                        args.request = Some(input.parse::<Type>()?);
                    }
                    "query" => {
                        args.query = Some(input.parse::<Type>()?);
                    }
                    "status" => {
                        let lit: LitInt = input.parse()?;
                        let code: u16 = lit.base10_parse()?;
                        args.status = Some(code);
                    }
                    "tag" | "group" => {
                        let lit: LitStr = input.parse()?;
                        args.tag = Some(lit.value());
                    }
                    "prefix" => {
                        let lit: LitStr = input.parse()?;
                        args.prefix = Some(lit.value());
                    }
                    "name" | "operation_id" => {
                        let lit: LitStr = input.parse()?;
                        args.name = Some(lit.value());
                    }
                    "summary" => {
                        let lit: LitStr = input.parse()?;
                        args.summary = Some(lit.value());
                    }
                    _ => {
                        return Err(syn::Error::new_spanned(
                            key,
                            format!("unknown #[endpoint] option '{key_str}'"),
                        ));
                    }
                }
            } else if args.method.is_none() {
                if input.peek(LitStr) {
                    let lit: LitStr = input.parse()?;
                    args.method = Some(normalize_http_method(&lit.value(), lit.span())?);
                } else if input.peek(Ident) {
                    let ident: Ident = input.parse()?;
                    args.method = Some(normalize_http_method(&ident.to_string(), ident.span())?);
                } else {
                    return Err(input.error(
                        "expected HTTP method (e.g. GET, POST) as first argument to #[endpoint]",
                    ));
                }
            } else if args.path.is_none() && input.peek(LitStr) {
                let lit: LitStr = input.parse()?;
                args.path = Some(lit.value());
            } else {
                return Err(input.error(
                    "unexpected positional argument in #[endpoint(...)] (expected `key = value`)",
                ));
            }

            if !input.is_empty() {
                let _comma: Token![,] = input.parse()?;
            }
        }
        Ok(args)
    }
}

fn first_doc_line(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        if let Meta::NameValue(nv) = &attr.meta {
            if let Expr::Lit(expr_lit) = &nv.value {
                if let Lit::Str(lit) = &expr_lit.lit {
                    let trimmed = lit.value().trim().to_string();
                    if !trimmed.is_empty() {
                        return Some(trimmed);
                    }
                }
            }
        }
    }
    None
}

fn opt_str_tokens(val: Option<&str>) -> TokenStream2 {
    match val {
        Some(s) => quote! { Some(#s) },
        None => quote! { None },
    }
}

pub fn expand_endpoint(attr: TokenStream2, item: TokenStream2) -> syn::Result<TokenStream2> {
    let args = parse2::<EndpointArgs>(attr)?;
    let input_fn = parse2::<ItemFn>(item)?;

    let method = args.method.ok_or_else(|| {
        syn::Error::new_spanned(
            &input_fn.sig.ident,
            "#[endpoint(...)] requires an HTTP method (e.g. #[endpoint(GET, \"/path\", response = Out)])",
        )
    })?;
    let path = args.path.unwrap_or_default();

    let fn_ident = &input_fn.sig.ident;
    let status: u16 = args.status.unwrap_or_else(|| {
        if method == "DELETE" && args.response.is_none() {
            204
        } else {
            200
        }
    });

    let summary_owned = args.summary.or_else(|| first_doc_line(&input_fn.attrs));
    let prefix_tokens = opt_str_tokens(args.prefix.as_deref());
    let tag_tokens = opt_str_tokens(args.tag.as_deref());
    let name_tokens = opt_str_tokens(args.name.as_deref());
    let summary_tokens = opt_str_tokens(summary_owned.as_deref());

    let request_schema_tokens = match args.request {
        Some(ref ty) => quote! { Some(djangors_core::endpoint::schema_for_deserialize::<#ty>) },
        None => quote! { None },
    };
    let query_schema_tokens = match args.query {
        Some(ref ty) => quote! { Some(djangors_core::endpoint::schema_for_deserialize::<#ty>) },
        None => quote! { None },
    };
    let response_schema_tokens = match args.response {
        Some(ref ty) => quote! { Some(djangors_core::endpoint::schema_for_serialize::<#ty>) },
        None => quote! { None },
    };

    Ok(quote! {
        #input_fn

        djangors_core::inventory::submit! {
            djangors_core::EndpointRegistration {
                fn_name: stringify!(#fn_ident),
                module_path: module_path!(),
                method: #method,
                path: #path,
                prefix: #prefix_tokens,
                tag: #tag_tokens,
                operation_name: #name_tokens,
                summary: #summary_tokens,
                status: #status,
                handler_type_id: || djangors_core::endpoint::handler_type_id_of_val(&#fn_ident),
                handler_type_name: || djangors_core::endpoint::handler_type_name_of_val(&#fn_ident),
                request_schema: #request_schema_tokens,
                query_schema: #query_schema_tokens,
                response_schema: #response_schema_tokens,
            }
        }
    })
}

pub fn expand_router(attr: TokenStream2, item: TokenStream2) -> syn::Result<TokenStream2> {
    if !attr.is_empty() {
        return Err(syn::Error::new_spanned(
            attr,
            "#[router] does not take any arguments",
        ));
    }
    let input_fn = parse2::<ItemFn>(item)?;
    let fn_ident = &input_fn.sig.ident;

    Ok(quote! {
        #input_fn

        djangors_core::inventory::submit! {
            djangors_core::RouterRegistration {
                build: #fn_ident,
            }
        }
    })
}
