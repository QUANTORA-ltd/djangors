use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, Ident, LitInt, LitStr, Type};

enum RelationKind {
    ForeignKey,
    OneToOne,
    ManyToMany,
}

fn classify_relation(ty: &Type) -> Option<(&Type, RelationKind)> {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                if let Some(syn::GenericArgument::Type(target_ty)) = args.args.first() {
                    if segment.ident == "ForeignKey" {
                        return Some((target_ty, RelationKind::ForeignKey));
                    }
                    if segment.ident == "OneToOne" {
                        return Some((target_ty, RelationKind::OneToOne));
                    }
                    if segment.ident == "ManyToMany" {
                        return Some((target_ty, RelationKind::ManyToMany));
                    }
                }
            }
        }
    }
    None
}

fn unwrap_option(ty: &Type) -> (&Type, bool) {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "Option" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first() {
                        return (inner_ty, true);
                    }
                }
            }
        }
    }
    (ty, false)
}

fn last_segment_ident(ty: &Type) -> Option<&Ident> {
    if let Type::Path(type_path) = ty {
        type_path.path.segments.last().map(|seg| &seg.ident)
    } else {
        None
    }
}

pub fn expand_derive_factory(input: DeriveInput) -> syn::Result<TokenStream> {
    let struct_ident = &input.ident;
    let vis = &input.vis;
    let factory_ident = format_ident!("{}Factory", struct_ident);
    let seq_ident = format_ident!(
        "__DJANGORS_FACTORY_SEQ_{}",
        struct_ident.to_string().to_uppercase()
    );

    let data_struct = match &input.data {
        Data::Struct(s) => s,
        _ => {
            return Err(syn::Error::new_spanned(
                struct_ident,
                "Factory derive macro only supports structs",
            ))
        }
    };

    let named_fields = match &data_struct.fields {
        Fields::Named(f) => f,
        _ => {
            return Err(syn::Error::new_spanned(
                struct_ident,
                "Factory derive macro only supports structs with named fields",
            ))
        }
    };

    let mut factory_fields = Vec::new();
    let mut factory_inits = Vec::new();
    let mut builder_methods = Vec::new();
    let mut build_assignments = Vec::new();

    for field in &named_fields.named {
        let field_ident = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new_spanned(field, "unnamed fields not supported"))?;
        let field_name = field_ident.to_string();
        let field_ty = &field.ty;

        let mut primary_key = false;
        let mut auto = false;
        let mut max_length: Option<usize> = None;
        let mut default_expr: Option<syn::Expr> = None;
        let mut choices: Option<Vec<String>> = None;

        for attr in &field.attrs {
            if attr.path().is_ident("djangors") {
                // Best-effort attribute parsing so unknown `#[djangors(...)]` keys do not fail Factory
                let _ = attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("primary_key") {
                        primary_key = true;
                    } else if meta.path.is_ident("auto") {
                        auto = true;
                    } else if meta.path.is_ident("max_length") {
                        let lit: LitInt = meta.value()?.parse()?;
                        max_length = Some(lit.base10_parse::<usize>()?);
                    } else if meta.path.is_ident("default") {
                        let expr: syn::Expr = meta.value()?.parse()?;
                        default_expr = Some(expr);
                    } else if meta.path.is_ident("choices") {
                        let value = meta.value()?;
                        let content;
                        syn::bracketed!(content in value);
                        let mut lits = Vec::new();
                        while !content.is_empty() {
                            let lit: LitStr = content.parse()?;
                            lits.push(lit.value());
                            if content.is_empty() {
                                break;
                            }
                            content.parse::<syn::Token![,]>()?;
                        }
                        choices = Some(lits);
                    } else if meta.input.peek(syn::Token![=]) {
                        let _: syn::Expr = meta.value()?.parse()?;
                    } else if meta.input.peek(syn::token::Paren) {
                        let content;
                        syn::parenthesized!(content in meta.input);
                        let _: proc_macro2::TokenStream = content.parse()?;
                    }
                    Ok(())
                });
            }
        }

        if let Some((target_ty, rel_kind)) = classify_relation(field_ty) {
            match rel_kind {
                RelationKind::ManyToMany => {
                    build_assignments.push(quote! {
                        #field_ident: djangors_orm::ManyToMany::<#target_ty>::new()
                    });
                    continue;
                }
                RelationKind::ForeignKey => {
                    let with_fn = format_ident!("with_{}", field_ident);
                    let with_id_fn = format_ident!("with_{}_id", field_ident);
                    factory_fields.push(quote! {
                        #field_ident: ::std::option::Option<#field_ty>
                    });
                    factory_inits.push(quote! {
                        #field_ident: ::std::option::Option::None
                    });
                    builder_methods.push(quote! {
                        /// Sets a custom [`ForeignKey`] value for this field.
                        pub fn #with_fn(mut self, val: #field_ty) -> Self {
                            self.#field_ident = ::std::option::Option::Some(val);
                            self
                        }

                        /// Sets the foreign key target ID for this field.
                        pub fn #with_id_fn(mut self, id: i64) -> Self {
                            self.#field_ident = ::std::option::Option::Some(djangors_orm::ForeignKey::<#target_ty>::new(id));
                            self
                        }
                    });
                    build_assignments.push(quote! {
                        #field_ident: self.#field_ident.clone().unwrap_or_else(|| djangors_orm::ForeignKey::<#target_ty>::new(1))
                    });
                    continue;
                }
                RelationKind::OneToOne => {
                    let with_fn = format_ident!("with_{}", field_ident);
                    let with_id_fn = format_ident!("with_{}_id", field_ident);
                    factory_fields.push(quote! {
                        #field_ident: ::std::option::Option<#field_ty>
                    });
                    factory_inits.push(quote! {
                        #field_ident: ::std::option::Option::None
                    });
                    builder_methods.push(quote! {
                        /// Sets a custom [`OneToOne`] value for this field.
                        pub fn #with_fn(mut self, val: #field_ty) -> Self {
                            self.#field_ident = ::std::option::Option::Some(val);
                            self
                        }

                        /// Sets the one-to-one target ID for this field.
                        pub fn #with_id_fn(mut self, id: i64) -> Self {
                            self.#field_ident = ::std::option::Option::Some(djangors_orm::OneToOne::<#target_ty>::new(id));
                            self
                        }
                    });
                    build_assignments.push(quote! {
                        #field_ident: self.#field_ident.clone().unwrap_or_else(|| djangors_orm::OneToOne::<#target_ty>::new(1))
                    });
                    continue;
                }
            }
        }

        let with_fn = format_ident!("with_{}", field_ident);
        factory_fields.push(quote! {
            #field_ident: ::std::option::Option<#field_ty>
        });
        factory_inits.push(quote! {
            #field_ident: ::std::option::Option::None
        });

        let (inner_ty, is_option) = unwrap_option(field_ty);
        let last_ident_str = last_segment_ident(inner_ty)
            .map(|i| i.to_string())
            .unwrap_or_default();

        if !is_option && last_ident_str == "String" {
            builder_methods.push(quote! {
                /// Overrides the default factory value for this field.
                pub fn #with_fn(mut self, val: impl ::std::convert::Into<String>) -> Self {
                    self.#field_ident = ::std::option::Option::Some(val.into());
                    self
                }
            });
        } else {
            builder_methods.push(quote! {
                /// Overrides the default factory value for this field.
                pub fn #with_fn(mut self, val: #field_ty) -> Self {
                    self.#field_ident = ::std::option::Option::Some(val);
                    self
                }
            });
        }

        let fallback_expr = if is_option {
            quote! { ::std::option::Option::None }
        } else if (primary_key || field_name == "id")
            && (auto || last_ident_str == "i64" || last_ident_str == "i32")
        {
            if last_ident_str == "i32" {
                quote! { 0i32 }
            } else {
                quote! { 0i64 }
            }
        } else if let Some(first_choice) = choices.as_ref().and_then(|c| c.first()) {
            quote! { #first_choice.to_string() }
        } else if let Some(def) = &default_expr {
            if last_ident_str == "String" {
                quote! { (#def).to_string() }
            } else if last_ident_str == "i32" {
                quote! { (#def) as i32 }
            } else if last_ident_str == "f32" {
                quote! { (#def) as f32 }
            } else {
                quote! { #def }
            }
        } else {
            match last_ident_str.as_str() {
                "String" => {
                    let is_email = field_name.contains("email");
                    let is_url = field_name.contains("url") || field_name.contains("website");
                    let base_str = if is_email {
                        quote! { format!("user{}@example.com", __seq) }
                    } else if is_url {
                        quote! { format!("https://example.com/{}/{}", #field_name, __seq) }
                    } else {
                        quote! { format!("{}_{}", #field_name, __seq) }
                    };
                    if let Some(max_len) = max_length {
                        quote! {
                            {
                                let mut __s = #base_str;
                                if __s.len() > #max_len {
                                    __s.truncate(#max_len);
                                }
                                __s
                            }
                        }
                    } else {
                        base_str
                    }
                }
                "i64" => quote! { __seq as i64 },
                "i32" => quote! { __seq as i32 },
                "u64" => quote! { __seq },
                "u32" => quote! { __seq as u32 },
                "usize" => quote! { __seq as usize },
                "f64" => quote! { __seq as f64 },
                "f32" => quote! { __seq as f32 },
                "bool" => quote! { true },
                "DateTime" => quote! { <#inner_ty>::from(::std::time::SystemTime::now()) },
                "Uuid" => quote! { <#inner_ty>::from_u128(__seq as u128) },
                "Decimal" => quote! { <#inner_ty>::from(__seq) },
                _ => quote! { ::std::default::Default::default() },
            }
        };

        build_assignments.push(quote! {
            #field_ident: self.#field_ident.clone().unwrap_or_else(|| #fallback_expr)
        });
    }

    let expanded = quote! {
        static #seq_ident: ::std::sync::atomic::AtomicU64 = ::std::sync::atomic::AtomicU64::new(1);

        /// Model factory builder for [`#struct_ident`].
        ///
        /// Generated by `#[derive(Factory)]`. Provides sensible per-field defaults
        /// with a monotonic sequence counter and fluent `.with_<field>(...)` overrides.
        #[derive(Clone, Debug, Default)]
        #vis struct #factory_ident {
            #(#factory_fields,)*
        }

        impl #factory_ident {
            /// Creates a new factory with default field generators.
            pub fn new() -> Self {
                Self {
                    #(#factory_inits,)*
                }
            }

            #(#builder_methods)*

            /// Builds an in-memory [`#struct_ident`] instance without saving it to the database.
            pub fn build(&self) -> #struct_ident {
                let __seq = #seq_ident.fetch_add(1, ::std::sync::atomic::Ordering::Relaxed);
                #struct_ident {
                    #(#build_assignments,)*
                }
            }

            /// Builds and persists a single [`#struct_ident`] instance to `db`.
            pub async fn create(
                &self,
                db: &djangors_orm::djangors_db::Database,
            ) -> ::std::result::Result<#struct_ident, djangors_orm::OrmError> {
                let instance = self.build();
                instance.save(db).await
            }

            /// Builds and persists `count` [`#struct_ident`] instances to `db`.
            pub async fn create_many(
                &self,
                db: &djangors_orm::djangors_db::Database,
                count: usize,
            ) -> ::std::result::Result<::std::vec::Vec<#struct_ident>, djangors_orm::OrmError> {
                let mut items = ::std::vec::Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.create(db).await?);
                }
                ::std::result::Result::Ok(items)
            }
        }

        impl #struct_ident {
            /// Returns a [`#factory_ident`] for building or persisting test instances of [`#struct_ident`].
            pub fn factory() -> #factory_ident {
                #factory_ident::new()
            }
        }
    };

    Ok(expanded)
}
