use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Fields, Ident, LitInt, LitStr, Type};

struct ModelField {
    ident: Ident,
    column_name: String,
    is_auto: bool,
    is_primary_key: bool,
    is_relation: bool,
    last_ident: Option<String>,
    is_nullable: bool,
    null_bind_tok: proc_macro2::TokenStream,
    auto_now_add: bool,
    auto_now: bool,
}

fn field_value_expr(f: &ModelField) -> TokenStream {
    let ident = &f.ident;
    if f.is_relation {
        quote! { djangors_orm::expr::Value::from(self.#ident.id) }
    } else {
        let last_ident_str = f.last_ident.as_deref().unwrap_or("");
        if f.is_nullable {
            match last_ident_str {
                "String" => quote! {
                    match &self.#ident {
                        Some(v) => djangors_orm::expr::Value::from(v.clone()),
                        None => djangors_orm::expr::Value::Null,
                    }
                },
                "i32" => quote! {
                    match self.#ident {
                        Some(v) => djangors_orm::expr::Value::from(v as i64),
                        None => djangors_orm::expr::Value::Null,
                    }
                },
                "f32" => quote! {
                    match self.#ident {
                        Some(v) => djangors_orm::expr::Value::from(v as f64),
                        None => djangors_orm::expr::Value::Null,
                    }
                },
                "NaiveDate" => quote! {
                    match &self.#ident {
                        Some(v) => djangors_orm::expr::Value::Text(format!("{}", v)),
                        None => djangors_orm::expr::Value::Null,
                    }
                },
                "NaiveTime" => quote! {
                    match &self.#ident {
                        Some(v) => djangors_orm::expr::Value::Text(format!("{}", v)),
                        None => djangors_orm::expr::Value::Null,
                    }
                },
                "Duration" => quote! {
                    match &self.#ident {
                        Some(v) => djangors_orm::expr::Value::Text(format!("{:?}", v)),
                        None => djangors_orm::expr::Value::Null,
                    }
                },
                "Uuid" => quote! {
                    match &self.#ident {
                        Some(v) => djangors_orm::expr::Value::Text(v.to_string()),
                        None => djangors_orm::expr::Value::Null,
                    }
                },
                "Decimal" => quote! {
                    match &self.#ident {
                        Some(v) => djangors_orm::expr::Value::Text(v.to_string()),
                        None => djangors_orm::expr::Value::Null,
                    }
                },
                _ => quote! {
                    match self.#ident {
                        Some(v) => djangors_orm::expr::Value::from(v),
                        None => djangors_orm::expr::Value::Null,
                    }
                },
            }
        } else {
            match last_ident_str {
                "String" => quote! { djangors_orm::expr::Value::from(self.#ident.clone()) },
                "i32" => quote! { djangors_orm::expr::Value::from(self.#ident as i64) },
                "f32" => quote! { djangors_orm::expr::Value::from(self.#ident as f64) },
                "NaiveDate" => {
                    quote! { djangors_orm::expr::Value::Text(format!("{}", self.#ident)) }
                }
                "NaiveTime" => {
                    quote! { djangors_orm::expr::Value::Text(format!("{}", self.#ident)) }
                }
                "Duration" => {
                    quote! { djangors_orm::expr::Value::Text(format!("{:?}", self.#ident)) }
                }
                "Uuid" => quote! { djangors_orm::expr::Value::Text(self.#ident.to_string()) },
                "Decimal" => quote! { djangors_orm::expr::Value::Text(self.#ident.to_string()) },
                _ => quote! { djangors_orm::expr::Value::from(self.#ident) },
            }
        }
    }
}

fn field_write_json_expr(f: &ModelField) -> TokenStream {
    let ident = &f.ident;
    if f.is_relation {
        quote! { djangors_orm::json::write_json_i64(buf, self.#ident.id) }
    } else {
        let last_ident_str = f.last_ident.as_deref().unwrap_or("");
        if f.is_nullable {
            match last_ident_str {
                "String" => quote! {
                    match &self.#ident {
                        Some(v) => djangors_orm::json::write_json_str(buf, v),
                        None => djangors_orm::json::write_json_null(buf),
                    }
                },
                "i64" => quote! {
                    match self.#ident {
                        Some(v) => djangors_orm::json::write_json_i64(buf, v),
                        None => djangors_orm::json::write_json_null(buf),
                    }
                },
                "i32" => quote! {
                    match self.#ident {
                        Some(v) => djangors_orm::json::write_json_i64(buf, v as i64),
                        None => djangors_orm::json::write_json_null(buf),
                    }
                },
                "f64" => quote! {
                    match self.#ident {
                        Some(v) => djangors_orm::json::write_json_f64(buf, v),
                        None => djangors_orm::json::write_json_null(buf),
                    }
                },
                "f32" => quote! {
                    match self.#ident {
                        Some(v) => djangors_orm::json::write_json_f64(buf, v as f64),
                        None => djangors_orm::json::write_json_null(buf),
                    }
                },
                "bool" => quote! {
                    match self.#ident {
                        Some(v) => djangors_orm::json::write_json_bool(buf, v),
                        None => djangors_orm::json::write_json_null(buf),
                    }
                },
                "NaiveDate" | "NaiveTime" | "Uuid" | "Decimal" => quote! {
                    match &self.#ident {
                        Some(v) => djangors_orm::json::write_json_display(buf, v),
                        None => djangors_orm::json::write_json_null(buf),
                    }
                },
                "Duration" => quote! {
                    match &self.#ident {
                        Some(v) => djangors_orm::json::write_json_debug(buf, v),
                        None => djangors_orm::json::write_json_null(buf),
                    }
                },
                _ => quote! {
                    match &self.#ident {
                        Some(v) => djangors_orm::json::write_json_datetime(buf, v),
                        None => djangors_orm::json::write_json_null(buf),
                    }
                },
            }
        } else {
            match last_ident_str {
                "String" => quote! { djangors_orm::json::write_json_str(buf, &self.#ident) },
                "i64" => quote! { djangors_orm::json::write_json_i64(buf, self.#ident) },
                "i32" => quote! { djangors_orm::json::write_json_i64(buf, self.#ident as i64) },
                "f64" => quote! { djangors_orm::json::write_json_f64(buf, self.#ident) },
                "f32" => quote! { djangors_orm::json::write_json_f64(buf, self.#ident as f64) },
                "bool" => quote! { djangors_orm::json::write_json_bool(buf, self.#ident) },
                "NaiveDate" | "NaiveTime" | "Uuid" | "Decimal" => {
                    quote! { djangors_orm::json::write_json_display(buf, &self.#ident) }
                }
                "Duration" => {
                    quote! { djangors_orm::json::write_json_debug(buf, &self.#ident) }
                }
                _ => quote! { djangors_orm::json::write_json_datetime(buf, &self.#ident) },
            }
        }
    }
}

pub fn expand_derive_model(input: DeriveInput) -> syn::Result<TokenStream> {
    // 1. Parse struct-level attributes
    let mut app = None;
    let mut ordering = None;
    let mut table_name = None;
    let mut unique_together = None;
    let mut indexes = None;
    let mut manager = None;

    let struct_name_ident = &input.ident;
    let struct_name = struct_name_ident.to_string();

    for attr in &input.attrs {
        if attr.path().is_ident("djangors") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("app") {
                    let lit: LitStr = meta.value()?.parse()?;
                    app = Some(lit.value());
                } else if meta.path.is_ident("ordering") {
                    let expr: syn::Expr = meta.value()?.parse()?;
                    match expr {
                        syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(lit_str), .. }) => {
                            let fields: Vec<String> = lit_str.value()
                                .split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect();
                            ordering = Some(fields);
                        }
                        syn::Expr::Array(expr_array) => {
                            let mut fields = Vec::new();
                            for el in expr_array.elems {
                                if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(lit_str), .. }) = el {
                                    fields.push(lit_str.value());
                                } else {
                                    return Err(syn::Error::new_spanned(el, "ordering elements must be string literals"));
                                }
                            }
                            ordering = Some(fields);
                        }
                        _ => {
                            return Err(syn::Error::new_spanned(expr, "ordering must be a string literal or an array of string literals"));
                        }
                    }
                } else if meta.path.is_ident("table_name") {
                    let lit: LitStr = meta.value()?.parse()?;
                    table_name = Some(lit.value());
                } else if meta.path.is_ident("unique_together") {
                    let expr: syn::Expr = meta.value()?.parse()?;
                    let mut ut_outer = Vec::new();
                    match expr {
                        syn::Expr::Array(expr_array) => {
                            for el in expr_array.elems {
                                let mut ut_inner = Vec::new();
                                match el {
                                    syn::Expr::Array(inner_array) => {
                                        for inner_el in inner_array.elems {
                                            if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(lit_str), .. }) = inner_el {
                                                ut_inner.push(lit_str.value());
                                            } else {
                                                return Err(syn::Error::new_spanned(inner_el, "unique_together elements must be string literals"));
                                            }
                                        }
                                    }
                                    syn::Expr::Tuple(inner_tuple) => {
                                        for inner_el in inner_tuple.elems {
                                            if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(lit_str), .. }) = inner_el {
                                                ut_inner.push(lit_str.value());
                                            } else {
                                                return Err(syn::Error::new_spanned(inner_el, "unique_together elements must be string literals"));
                                            }
                                        }
                                    }
                                    _ => {
                                        return Err(syn::Error::new_spanned(el, "unique_together must contain arrays or tuples of string literals"));
                                    }
                                }
                                ut_outer.push(ut_inner);
                            }
                        }
                        _ => {
                            return Err(syn::Error::new_spanned(expr, "unique_together must be an array of arrays/tuples of string literals"));
                        }
                    }
                    unique_together = Some(ut_outer);
                } else if meta.path.is_ident("indexes") {
                    let expr: syn::Expr = meta.value()?.parse()?;
                    let mut idx_outer = Vec::new();
                    match expr {
                        syn::Expr::Array(expr_array) => {
                            for el in expr_array.elems {
                                match el {
                                    syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(lit_str), .. }) => {
                                        idx_outer.push((lit_str.value(), Vec::new()));
                                    }
                                    syn::Expr::Tuple(inner_tuple) => {
                                        let mut name = None;
                                        let mut fields = Vec::new();
                                        for inner_el in inner_tuple.elems {
                                            match inner_el {
                                                syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(lit_str), .. }) => {
                                                    if name.is_none() {
                                                        name = Some(lit_str.value());
                                                    } else {
                                                        fields.push(lit_str.value());
                                                    }
                                                }
                                                syn::Expr::Array(arr) => {
                                                    for item in arr.elems {
                                                        if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(lit_str), .. }) = item {
                                                            fields.push(lit_str.value());
                                                        } else {
                                                            return Err(syn::Error::new_spanned(item, "index field names must be string literals"));
                                                        }
                                                    }
                                                }
                                                _ => {
                                                    return Err(syn::Error::new_spanned(inner_el, "index must be a string (name) or a tuple (name, [fields])"));
                                                }
                                            }
                                        }
                                        match name {
                                            Some(n) => idx_outer.push((n, fields)),
                                            None => {
                                                return Err(syn::Error::new(
                                                    proc_macro2::Span::call_site(),
                                                    "index tuple must start with a name string literal",
                                                ))
                                            }
                                        }
                                    }
                                    _ => {
                                        return Err(syn::Error::new_spanned(el, "indexes must contain a string (name) or tuple (name, [fields])"));
                                    }
                                }
                            }
                        }
                        _ => {
                            return Err(syn::Error::new_spanned(expr, "indexes must be an array of strings or (name, [fields]) tuples"));
                        }
                    }
                    indexes = Some(idx_outer);
                } else if meta.path.is_ident("manager") {
                    let expr: syn::Expr = meta.value()?.parse()?;
                    match expr {
                        syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(lit_str), .. }) => {
                            manager = Some(syn::parse_str::<syn::Path>(&lit_str.value())?);
                        }
                        _ => {
                            return Err(syn::Error::new_spanned(
                                expr,
                                "manager must be a string literal naming a type implementing djangors_orm::Manager",
                            ));
                        }
                    }
                }
                Ok(())
            })?;
        }
    }

    let app_label = match app {
        Some(a) => a,
        None => {
            return Err(syn::Error::new_spanned(
                struct_name_ident,
                "models must specify #[djangors(app = \"...\")]",
            ));
        }
    };

    let table_name =
        table_name.unwrap_or_else(|| format!("{}_{}", app_label, snake_case(&struct_name)));

    // 2. Parse struct fields
    let data_struct = match input.data {
        Data::Struct(s) => s,
        _ => {
            return Err(syn::Error::new_spanned(
                struct_name_ident,
                "Model derive macro only supports structs",
            ))
        }
    };

    let named_fields = match data_struct.fields {
        Fields::Named(f) => f,
        _ => {
            return Err(syn::Error::new_spanned(
                struct_name_ident,
                "Model derive macro only supports structs with named fields",
            ))
        }
    };

    struct ParsedField {
        ident: Ident,
        primary_key: bool,
        field_meta_tokens: TokenStream,
        // Parsed and baked into `field_meta_tokens` above rather than read back out here, but
        // kept as named fields so the parse stays self-documenting and so a future codegen step
        // that needs them doesn't have to re-parse the attribute.
        #[allow(dead_code)]
        choices: Option<Vec<String>>,
        #[allow(dead_code)]
        auto_now_add: bool,
        #[allow(dead_code)]
        auto_now: bool,
    }

    struct ParsedRelation {
        relation_meta_tokens: TokenStream,
    }

    let mut parsed_fields = Vec::new();
    let mut parsed_relations = Vec::new();
    let mut column_names = std::collections::HashMap::new();
    let mut from_row_assignments = Vec::new();
    let mut model_fields = Vec::new();
    let mut m2m_construct = Vec::new();
    let mut form_names = Vec::new();
    let mut form_types = Vec::new();
    let mut form_validators = Vec::new();
    let mut form_assignments = Vec::new();
    let mut form_idents = Vec::new();

    for field in named_fields.named {
        let field_ident = field
            .ident
            .clone()
            .ok_or_else(|| syn::Error::new_spanned(&field, "unnamed fields not supported"))?;
        let field_name_str = field_ident.to_string();

        // 2a. Parse field attributes
        let mut primary_key = false;
        let mut auto = false;
        let mut max_length = None;
        let mut file_field = false;
        let mut default = None;
        let mut unique = false;
        let mut db_index = false;
        let mut verbose_name = None;
        let mut help_text = None;
        let mut column = None;
        let mut max_digits = None;
        let mut decimal_places = None;
        let mut on_delete = None;
        let mut related_name = None;
        let mut choices: Option<Vec<String>> = None;
        let mut auto_now_add = false;
        let mut auto_now = false;

        for attr in &field.attrs {
            if attr.path().is_ident("djangors") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("primary_key") {
                        primary_key = true;
                    } else if meta.path.is_ident("auto") {
                        auto = true;
                    } else if meta.path.is_ident("max_length") {
                        let lit: LitInt = meta.value()?.parse()?;
                        max_length = Some(lit.base10_parse::<u32>()?);
                    } else if meta.path.is_ident("file_field") {
                        file_field = true;
                    } else if meta.path.is_ident("default") {
                        let expr: syn::Expr = meta.value()?.parse()?;
                        default = Some(expr);
                    } else if meta.path.is_ident("unique") {
                        unique = true;
                    } else if meta.path.is_ident("db_index") {
                        db_index = true;
                    } else if meta.path.is_ident("verbose_name") {
                        let lit: LitStr = meta.value()?.parse()?;
                        verbose_name = Some(lit.value());
                    } else if meta.path.is_ident("help_text") {
                        let lit: LitStr = meta.value()?.parse()?;
                        help_text = Some(lit.value());
                    } else if meta.path.is_ident("column") {
                        let lit: LitStr = meta.value()?.parse()?;
                        column = Some(lit.value());
                    } else if meta.path.is_ident("max_digits") {
                        let lit: LitInt = meta.value()?.parse()?;
                        max_digits = Some(lit.base10_parse::<u32>()?);
                    } else if meta.path.is_ident("decimal_places") {
                        let lit: LitInt = meta.value()?.parse()?;
                        decimal_places = Some(lit.base10_parse::<u32>()?);
                    } else if meta.path.is_ident("foreign_key") {
                        meta.parse_nested_meta(|nested| {
                            if nested.path.is_ident("on_delete") {
                                let lit: LitStr = nested.value()?.parse()?;
                                on_delete = Some(lit.value());
                            } else if nested.path.is_ident("related_name") {
                                let lit: LitStr = nested.value()?.parse()?;
                                related_name = Some(lit.value());
                            } else if nested.path.is_ident("to") {
                                // ignore/consume
                                let _: syn::Expr = nested.value()?.parse()?;
                            } else {
                                return Err(nested.error("unrecognized foreign_key option"));
                            }
                            Ok(())
                        })?;
                    } else if meta.path.is_ident("many_to_many") {
                        meta.parse_nested_meta(|nested| {
                            if nested.path.is_ident("related_name") {
                                let lit: LitStr = nested.value()?.parse()?;
                                related_name = Some(lit.value());
                            } else if nested.path.is_ident("to") {
                                let _: syn::Expr = nested.value()?.parse()?;
                            } else {
                                return Err(nested.error("unrecognized many_to_many option"));
                            }
                            Ok(())
                        })?;
                    } else if meta.path.is_ident("auto_now_add") {
                        if meta.input.peek(syn::Token![=]) {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            if !lit.value {
                                return Err(meta.error("auto_now_add must be `true`"));
                            }
                        }
                        auto_now_add = true;
                    } else if meta.path.is_ident("auto_now") {
                        if meta.input.peek(syn::Token![=]) {
                            let value = meta.value()?;
                            let lit: syn::LitBool = value.parse()?;
                            if !lit.value {
                                return Err(meta.error("auto_now must be `true`"));
                            }
                        }
                        auto_now = true;
                    } else if meta.path.is_ident("choices") {
                        let value = meta.value()?;
                        let content;
                        syn::bracketed!(content in value);
                        let mut lits = Vec::new();
                        while !content.is_empty() {
                            let lit: syn::LitStr = content.parse()?;
                            lits.push(lit.value());
                            if content.is_empty() {
                                break;
                            }
                            content.parse::<syn::Token![,]>()?;
                        }
                        choices = Some(lits);
                    }
                    Ok(())
                })?;
            }
        }

        let final_column = column.clone().unwrap_or_else(|| field_name_str.clone());
        if column_names
            .insert(final_column.clone(), field.span())
            .is_some()
        {
            return Err(syn::Error::new_spanned(
                &field_ident,
                format!("duplicate column name '{}'", final_column),
            ));
        }

        // 2b. Check if ForeignKey/OneToOne relation
        if let Some(relation_field) = get_relation_field(&field.ty) {
            let target_ty = relation_field.target_ty;

            // ManyToMany is a relation but NOT a column: it lives in a join table.
            // Record the relation metadata and construct the marker on from_row,
            // but expose no column (no save/filter/field_values/form entry).
            if relation_field.kind == RelationFieldKind::ManyToMany {
                let related_name_tok = match &related_name {
                    Some(rn) => quote! { Some(#rn) },
                    None => quote! { None },
                };
                let relation_meta_tokens = quote! {
                    djangors_orm::RelationMeta {
                        field_name: #field_name_str,
                        kind: djangors_orm::RelationKind::ManyToMany,
                        target: <#target_ty as djangors_orm::Model>::meta,
                        on_delete: djangors_orm::OnDelete::Cascade,
                        related_name: #related_name_tok,
                    }
                };
                parsed_relations.push(ParsedRelation {
                    relation_meta_tokens,
                });
                from_row_assignments.push(quote! {
                    #field_ident: djangors_orm::ManyToMany::<#target_ty>::new()
                });
                m2m_construct.push(quote! {
                    #field_ident: djangors_orm::ManyToMany::<#target_ty>::new()
                });
                continue;
            }

            let on_delete_enum = match on_delete.as_deref().map(|s| s.to_lowercase()).as_deref() {
                Some("cascade") => quote! { djangors_orm::OnDelete::Cascade },
                Some("protect") => quote! { djangors_orm::OnDelete::Protect },
                Some("set_null") => quote! { djangors_orm::OnDelete::SetNull },
                Some("restrict") => quote! { djangors_orm::OnDelete::Restrict },
                Some("do_nothing") => quote! { djangors_orm::OnDelete::DoNothing },
                Some(other) => {
                    return Err(syn::Error::new_spanned(
                        &field_ident,
                        format!("invalid on_delete value '{}'. Valid options are: cascade, protect, set_null, restrict, do_nothing", other)
                    ));
                }
                None => quote! { djangors_orm::OnDelete::Cascade },
            };

            let related_name_tok = match related_name {
                Some(rn) => quote! { Some(#rn) },
                None => quote! { None },
            };

            let target_ty = relation_field.target_ty;
            let (kind_tok, wrapper_tok) = match relation_field.kind {
                RelationFieldKind::ForeignKey => (
                    quote! { djangors_orm::RelationKind::ForeignKey },
                    quote! { djangors_orm::ForeignKey },
                ),
                RelationFieldKind::OneToOne => (
                    quote! { djangors_orm::RelationKind::OneToOne },
                    quote! { djangors_orm::OneToOne },
                ),
                RelationFieldKind::ManyToMany => {
                    unreachable!("ManyToMany handled before the column path")
                }
            };

            // target resolves to `TargetType::meta`
            let relation_meta_tokens = quote! {
                djangors_orm::RelationMeta {
                    field_name: #field_name_str,
                    kind: #kind_tok,
                    target: <#target_ty as djangors_orm::Model>::meta,
                    on_delete: #on_delete_enum,
                    related_name: #related_name_tok,
                }
            };
            parsed_relations.push(ParsedRelation {
                relation_meta_tokens,
            });
            from_row_assignments.push(quote! {
                #field_ident: #wrapper_tok::new(
                    row.try_i64_by_name(#field_name_str)
                        .map_err(djangors_orm::OrmError::from)?
                        .ok_or_else(|| djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound))?
                )
            });
            model_fields.push(ModelField {
                ident: field_ident.clone(),
                column_name: field_name_str.clone(),
                is_auto: false,
                is_primary_key: false,
                is_relation: true,
                last_ident: None,
                is_nullable: false,
                null_bind_tok: quote! { i64 },
                auto_now_add: false,
                auto_now: false,
            });
            form_names.push(field_name_str.clone());
            form_idents.push(field_ident.clone());
            form_types.push(quote! { Option<i64> });
            form_validators.push(quote! { djangors_orm::djangors_forms::IntegerField { min: None, max: None, required: true } });
            form_assignments.push(quote! { #wrapper_tok::new(cleaned.unwrap()) });
        } else {
            // It is a regular field!
            let (inner_ty, nullable) = resolve_option_type(&field.ty);
            let last_ident = get_last_path_segment_ident(inner_ty);
            let last_ident_str = last_ident.map(|id| id.to_string());
            let is_string = last_ident.map(|id| id == "String").unwrap_or(false);

            if max_length.is_some() && !is_string {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "max_length is only valid on String fields",
                ));
            }
            if file_field && !is_string {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "file_field is only valid on String fields",
                ));
            }

            if last_ident.map(|id| id == "Decimal").unwrap_or(false) {
                if max_digits.is_none() && decimal_places.is_none() {
                    return Err(syn::Error::new_spanned(
                        &field_ident,
                        "Decimal field is missing max_digits and decimal_places",
                    ));
                } else if max_digits.is_none() {
                    return Err(syn::Error::new_spanned(
                        &field_ident,
                        "Decimal field is missing max_digits",
                    ));
                } else if decimal_places.is_none() {
                    return Err(syn::Error::new_spanned(
                        &field_ident,
                        "Decimal field is missing decimal_places",
                    ));
                }
            }

            // Check for unsupported types for save/update/delete codegen
            if let Some(ref type_name) = last_ident_str {
                if type_name == "NaiveDate"
                    || type_name == "NaiveTime"
                    || type_name == "Duration"
                    || type_name == "Uuid"
                    || type_name == "Decimal"
                {
                    // Previously rejected; now supported via Text serialization.
                }
            }

            let kind_token = match last_ident.map(|id| id.to_string()).as_deref() {
                Some("String") => {
                    if file_field {
                        quote! { djangors_orm::FieldKind::FileField }
                    } else if max_length.is_some() {
                        quote! { djangors_orm::FieldKind::Char }
                    } else {
                        quote! { djangors_orm::FieldKind::Text }
                    }
                }
                Some("i32") => quote! { djangors_orm::FieldKind::Integer },
                Some("i64") => quote! { djangors_orm::FieldKind::BigInt },
                Some("f32") | Some("f64") => quote! { djangors_orm::FieldKind::Float },
                Some("bool") => quote! { djangors_orm::FieldKind::Boolean },
                Some("DateTime") => quote! { djangors_orm::FieldKind::DateTime },
                Some("NaiveDate") => quote! { djangors_orm::FieldKind::Date },
                Some("NaiveTime") => quote! { djangors_orm::FieldKind::Time },
                Some("Duration") => quote! { djangors_orm::FieldKind::Duration },
                Some("Uuid") => quote! { djangors_orm::FieldKind::Uuid },
                Some("Decimal") => {
                    let md = max_digits.unwrap_or(0);
                    let dp = decimal_places.unwrap_or(0);
                    quote! { djangors_orm::FieldKind::Decimal { max_digits: #md, decimal_places: #dp } }
                }
                Some(other) => {
                    return Err(syn::Error::new_spanned(
                        inner_ty,
                        format!("unsupported model field type '{}'", other),
                    ));
                }
                None => {
                    return Err(syn::Error::new_spanned(
                        inner_ty,
                        "unsupported model field type",
                    ));
                }
            };

            let default_tok = match default {
                Some(expr) => parse_default_value(&expr)?,
                None => quote! { djangors_orm::DefaultValue::None },
            };

            let max_length_tok = match max_length {
                Some(ml) => quote! { Some(#ml) },
                None => quote! { None },
            };

            let verbose_name_tok = match verbose_name {
                Some(vn) => quote! { Some(#vn) },
                None => quote! { None },
            };

            let help_text_tok = match help_text {
                Some(ht) => quote! { Some(#ht) },
                None => quote! { None },
            };

            // primary_key implies unique and db_index
            let unique_val = unique || primary_key;
            let db_index_val = db_index || primary_key;

            let choices_tok = match &choices {
                Some(choice_vec) => {
                    let pairs = choice_vec.iter().map(|s| {
                        let s_str = s.as_str();
                        quote! { (#s_str, #s_str) }
                    });
                    quote! { &[ #(#pairs),* ] }
                }
                None => quote! { &[] },
            };

            let field_meta_tokens = quote! {
                djangors_orm::FieldMeta {
                    name: #field_name_str,
                    column_name: #final_column,
                    kind: #kind_token,
                    nullable: #nullable,
                    primary_key: #primary_key,
                    auto: #auto,
                    unique: #unique_val,
                    db_index: #db_index_val,
                    default: #default_tok,
                    max_length: #max_length_tok,
                    verbose_name: #verbose_name_tok,
                    help_text: #help_text_tok,
                    choices: #choices_tok,
                }
            };

            parsed_fields.push(ParsedField {
                ident: field_ident.clone(),
                primary_key,
                field_meta_tokens,
                choices: choices.clone(),
                auto_now_add,
                auto_now,
            });
            let from_row_code = match (last_ident_str.as_deref(), nullable) {
                (Some("String"), true) => {
                    quote! { row.try_string_by_name(#final_column).map_err(djangors_orm::OrmError::from)? }
                }
                (Some("String"), false) => {
                    quote! { row.try_string_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.ok_or_else(|| djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound))? }
                }
                (Some("i64"), true) => {
                    quote! { row.try_i64_by_name(#final_column).map_err(djangors_orm::OrmError::from)? }
                }
                (Some("i64"), false) => {
                    quote! { row.try_i64_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.ok_or_else(|| djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound))? }
                }
                (Some("i32"), true) => {
                    quote! { row.try_i64_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.map(|v| v as i32) }
                }
                (Some("i32"), false) => {
                    quote! { row.try_i64_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.ok_or_else(|| djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound))? as i32 }
                }
                (Some("f64"), true) => {
                    quote! { row.try_f64_by_name(#final_column).map_err(djangors_orm::OrmError::from)? }
                }
                (Some("f64"), false) => {
                    quote! { row.try_f64_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.ok_or_else(|| djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound))? }
                }
                (Some("f32"), true) => {
                    quote! { row.try_f64_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.map(|v| v as f32) }
                }
                (Some("f32"), false) => {
                    quote! { row.try_f64_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.ok_or_else(|| djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound))? as f32 }
                }
                (Some("bool"), true) => {
                    quote! { row.try_bool_by_name(#final_column).map_err(djangors_orm::OrmError::from)? }
                }
                (Some("bool"), false) => {
                    quote! { row.try_bool_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.ok_or_else(|| djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound))? }
                }
                (Some("DateTime"), true) => {
                    quote! { row.try_datetime_by_name(#final_column).map_err(djangors_orm::OrmError::from)? }
                }
                (Some("DateTime"), false) => {
                    quote! { row.try_datetime_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.ok_or_else(|| djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound))? }
                }
                (Some("NaiveDate"), true) => {
                    quote! { row.try_string_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.and_then(|s| s.parse().ok()) }
                }
                (Some("NaiveDate"), false) => {
                    quote! { row.try_string_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.ok_or_else(|| djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound))?.parse().map_err(|e| djangors_orm::OrmError::InvalidQuery(format!("failed to parse NaiveDate '{}'", #final_column)))? }
                }
                (Some("NaiveTime"), true) => {
                    quote! { row.try_string_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.and_then(|s| s.parse().ok()) }
                }
                (Some("NaiveTime"), false) => {
                    quote! { row.try_string_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.ok_or_else(|| djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound))?.parse().map_err(|e| djangors_orm::OrmError::InvalidQuery(format!("failed to parse NaiveTime '{}'", #final_column)))? }
                }
                (Some("Duration"), true) => {
                    quote! { row.try_string_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.and_then(|s| s.parse().ok()) }
                }
                (Some("Duration"), false) => {
                    quote! { row.try_string_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.ok_or_else(|| djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound))?.parse().map_err(|e| djangors_orm::OrmError::InvalidQuery(format!("failed to parse {} field: {}", #final_column, e)))? }
                }
                (Some("Uuid"), true) => {
                    quote! { row.try_string_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.and_then(|s| uuid::Uuid::parse_str(&s).ok()) }
                }
                (Some("Uuid"), false) => {
                    quote! { uuid::Uuid::parse_str(&row.try_string_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.ok_or_else(|| djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound))?).map_err(|e| djangors_orm::OrmError::InvalidQuery(format!("failed to parse Uuid '{}'", #final_column)))? }
                }
                (Some("Decimal"), true) => {
                    quote! { row.try_string_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.and_then(|s| s.parse().ok()) }
                }
                (Some("Decimal"), false) => {
                    quote! { row.try_string_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.ok_or_else(|| djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound))?.parse().map_err(|e| djangors_orm::OrmError::InvalidQuery(format!("failed to parse {} field: {}", #final_column, e)))? }
                }
                (_, true) => {
                    quote! { row.try_string_by_name(#final_column).map_err(djangors_orm::OrmError::from)? }
                }
                (_, false) => {
                    quote! { row.try_string_by_name(#final_column).map_err(djangors_orm::OrmError::from)?.ok_or_else(|| djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound))? }
                }
            };
            from_row_assignments.push(quote! {
                #field_ident: #from_row_code
            });
            let null_bind_ty = inner_ty.clone();
            model_fields.push(ModelField {
                ident: field_ident.clone(),
                column_name: final_column.clone(),
                is_auto: auto,
                is_primary_key: primary_key,
                is_relation: false,
                last_ident: last_ident_str.clone(),
                is_nullable: nullable,
                null_bind_tok: quote! { #null_bind_ty },
                auto_now_add,
                auto_now,
            });
            if !auto
                && !primary_key
                && !file_field
                && matches!(
                    last_ident_str.as_deref(),
                    Some("String" | "i32" | "i64" | "bool")
                )
            {
                let required = !nullable;
                let form_field = match last_ident_str.as_deref() {
                    Some("String") => {
                        let max_len = match max_length {
                            Some(v) => quote! { Some(#v as usize) },
                            None => quote! { None },
                        };
                        if let Some(ref choice_vec) = choices {
                            let choice_strs: Vec<&str> =
                                choice_vec.iter().map(|s| s.as_str()).collect();
                            quote! { djangors_orm::djangors_forms::ChoiceField { choices: &[ #(#choice_strs),* ], required: #required } }
                        } else {
                            quote! { djangors_orm::djangors_forms::CharField { max_length: #max_len, required: #required } }
                        }
                    }
                    Some("i32") | Some("i64") => quote! {
                        djangors_orm::djangors_forms::IntegerField { min: None, max: None, required: #required }
                    },
                    Some("bool") => quote! {
                        // A required BooleanField's `clean()` treats `false` as "missing"
                        // (it mirrors Django's real ModelForm behavior: an HTML checkbox
                        // that's unchecked submits no value at all, so `required` means
                        // "must be checked/true", not "must be present"). A plain non-
                        // nullable `bool` model column has no such "absent" state and can
                        // legitimately be `false`, so the generated form field must always
                        // accept both `true` and `false` here regardless of the model
                        // field's own nullability.
                        djangors_orm::djangors_forms::BooleanField { required: false }
                    },
                    _ => {
                        quote! { djangors_orm::djangors_forms::CharField { max_length: None, required: #required } }
                    }
                };
                let cleaned_ty = match last_ident_str.as_deref() {
                    Some("i32") | Some("i64") => quote! { Option<i64> },
                    Some("bool") => quote! { bool },
                    _ => quote! { String },
                };
                let assignment = match (last_ident_str.as_deref(), nullable) {
                    (Some("i32"), true) => quote! { cleaned.map(|v| v as i32) },
                    (Some("i64"), true) => quote! { cleaned },
                    (Some("i32"), false) => quote! { cleaned.unwrap() as i32 },
                    (Some("i64"), false) => quote! { cleaned.unwrap() },
                    (Some("bool"), true) => quote! { Some(cleaned) },
                    (Some("bool"), false) => quote! { cleaned },
                    (Some("String"), true) => quote! { Some(cleaned) },
                    _ => quote! { cleaned },
                };
                form_names.push(field_name_str.clone());
                form_idents.push(field_ident.clone());
                form_types.push(cleaned_ty);
                form_validators.push(form_field);
                form_assignments.push(assignment);
            }
        }
    }

    // 3. Validation: exactly one primary key field
    let pk_fields: Vec<&Ident> = parsed_fields
        .iter()
        .filter(|f| f.primary_key)
        .map(|f| &f.ident)
        .collect();
    if pk_fields.is_empty() {
        return Err(syn::Error::new_spanned(
            struct_name_ident,
            "model must have exactly one #[djangors(primary_key)] field",
        ));
    } else if pk_fields.len() > 1 {
        let names: Vec<String> = pk_fields.iter().map(|id| id.to_string()).collect();
        return Err(syn::Error::new_spanned(
            struct_name_ident,
            format!(
                "model has multiple primary key fields: {}",
                names.join(", ")
            ),
        ));
    }

    // 4. Generate metadata lists
    let fields_expanded = parsed_fields.iter().map(|f| &f.field_meta_tokens);
    let relations_expanded = parsed_relations.iter().map(|r| &r.relation_meta_tokens);

    let ordering_tok = match ordering {
        Some(ord) => {
            let items = ord.iter().map(|o| quote! { #o });
            quote! { &[ #(#items),* ] }
        }
        None => quote! { &[] },
    };

    let unique_together_tok = match unique_together {
        Some(ut) => {
            let inner_lists = ut.iter().map(|list| {
                let items = list.iter().map(|item| quote! { #item });
                quote! { &[ #(#items),* ] }
            });
            quote! { &[ #(#inner_lists),* ] }
        }
        None => quote! { &[] },
    };

    let indexes_tok = match indexes {
        Some(idx) => {
            let structs = idx.iter().map(|(name, fields)| {
                let field_items = fields.iter().map(|f| quote! { #f });
                quote! {
                    djangors_orm::IndexMeta {
                        name: #name,
                        fields: &[ #(#field_items),* ],
                    }
                }
            });
            quote! { &[ #(#structs),* ] }
        }
        None => quote! { &[] },
    };

    let objects_override = match &manager {
        Some(manager_path) => {
            quote! {
                fn objects() -> djangors_orm::queryset::QuerySet<Self>
                where
                    Self: Sized + djangors_orm::FromRow,
                {
                    <#manager_path as djangors_orm::Manager<Self>>::objects()
                }
            }
        }
        None => quote! {},
    };

    // 5. Generate save, update, delete SQL building & binds
    let save_fields: Vec<&ModelField> = model_fields.iter().filter(|f| !f.is_auto).collect();
    let save_cols_vec: Vec<String> = save_fields.iter().map(|f| f.column_name.clone()).collect();

    let save_bind_stmts = save_fields.iter().map(|f| {
        let val_expr = if f.auto_now_add || f.auto_now {
            quote! { djangors_orm::expr::Value::DateTime(chrono::Utc::now()) }
        } else {
            field_value_expr(f)
        };
        let null_kind_tok = match f.null_bind_tok.to_string().as_str() {
            "String" => quote! { djangors_orm::NullKind::Text },
            "f64" | "f32" => quote! { djangors_orm::NullKind::F64 },
            "bool" => quote! { djangors_orm::NullKind::Bool },
            "chrono :: DateTime < chrono :: Utc >" | "DateTime < Utc >" | "DateTime" => {
                quote! { djangors_orm::NullKind::DateTime }
            }
            _ => quote! { djangors_orm::NullKind::I64 },
        };
        quote! {
            let val = #val_expr;
            let bind_val = match val {
                djangors_orm::expr::Value::Null => djangors_orm::BindValue::Null(#null_kind_tok),
                other => djangors_orm::BindValue::from(other),
            };
            bind_values.push(bind_val);
        }
    });

    let pk_field = model_fields.iter().find(|f| f.is_primary_key).unwrap();
    let pk_col_str = pk_field.column_name.clone();
    let update_fields: Vec<&ModelField> =
        model_fields.iter().filter(|f| !f.is_primary_key).collect();
    let update_cols_vec: Vec<String> = update_fields
        .iter()
        .map(|f| f.column_name.clone())
        .collect();

    let update_bind_stmts = update_fields
        .iter()
        .chain(std::iter::once(&pk_field))
        .map(|f| {
            let val_expr = if f.auto_now {
                quote! { djangors_orm::expr::Value::DateTime(chrono::Utc::now()) }
            } else {
                field_value_expr(f)
            };
            let null_kind_tok = match f.null_bind_tok.to_string().as_str() {
                "String" => quote! { djangors_orm::NullKind::Text },
                "f64" | "f32" => quote! { djangors_orm::NullKind::F64 },
                "bool" => quote! { djangors_orm::NullKind::Bool },
                "chrono :: DateTime < chrono :: Utc >" | "DateTime < Utc >" | "DateTime" => quote! { djangors_orm::NullKind::DateTime },
                _ => quote! { djangors_orm::NullKind::I64 },
            };
            quote! {
                let val = #val_expr;
                let bind_val = match val {
                    djangors_orm::expr::Value::Null => djangors_orm::BindValue::Null(#null_kind_tok),
                    other => djangors_orm::BindValue::from(other),
                };
                bind_values.push(bind_val);
            }
        });

    let delete_bind = field_value_expr(pk_field);

    let field_value_pairs = model_fields.iter().map(|f| {
        let name_str = f.ident.to_string();
        let val_expr = field_value_expr(f);
        quote! { (#name_str, #val_expr) }
    });

    let mut json_sorted_fields: Vec<&ModelField> = model_fields.iter().collect();
    json_sorted_fields.sort_by_key(|a| a.ident.to_string());

    let write_json_stmts = json_sorted_fields.iter().enumerate().map(|(idx, f)| {
        let name_str = f.ident.to_string();
        let prefix = if idx == 0 {
            format!("{{\"{}\":", name_str)
        } else {
            format!(",\"{}\":", name_str)
        };
        let val_write = field_write_json_expr(f);
        quote! {
            buf.extend_from_slice(#prefix.as_bytes());
            #val_write;
        }
    });

    let write_json_filtered_stmts = json_sorted_fields.iter().map(|f| {
        let name_str = f.ident.to_string();
        let key_colon = format!("\"{}\":", name_str);
        let val_write = field_write_json_expr(f);
        quote! {
            if include(#name_str) {
                if !first {
                    buf.push(b',');
                }
                first = false;
                buf.extend_from_slice(#key_colon.as_bytes());
                #val_write;
            }
        }
    });

    let field_names_list: Vec<String> = model_fields.iter().map(|f| f.ident.to_string()).collect();

    let form_cleaned_name = Ident::new(
        &format!("{}FormCleaned", struct_name),
        struct_name_ident.span(),
    );
    let form_val_idents: Vec<Ident> = form_idents
        .iter()
        .map(|i| Ident::new(&format!("{}_cleaned", i), i.span()))
        .collect();
    let form_apply = form_idents
        .iter()
        .zip(form_assignments.iter())
        .map(|(id, assign)| quote! { self.#id = { let cleaned = cleaned.#id; #assign }; });
    let form_construct = model_fields.iter().map(|f| {
        if let Some(pos) = form_idents.iter().position(|id| id == &f.ident) {
            let id = &f.ident;
            let assign = &form_assignments[pos];
            quote! { #id: { let cleaned = cleaned.#id; #assign } }
        } else {
            let id = &f.ident;
            quote! { #id: Default::default() }
        }
    });

    Ok(quote! {
        #[allow(missing_docs)]
        #[derive(Debug)]
        pub struct #form_cleaned_name { #(pub #form_idents: #form_types),* }

        #[allow(missing_docs)]
        impl #struct_name_ident {
            pub fn validate_form(data: &std::collections::HashMap<String, String>) -> Result<#form_cleaned_name, djangors_orm::djangors_forms::FormErrors> {
                use djangors_orm::djangors_forms::FormField;
                let mut errors = djangors_orm::djangors_forms::FormErrors::new();
                #(let #form_val_idents = match (#form_validators).clean(data.get(#form_names).map(String::as_str)) { Ok(v) => Some(v), Err(e) => { for m in e.0 { errors.add_field_error(#form_names, m); } None } };)*
                if !errors.is_empty() { return Err(errors); }
                Ok(#form_cleaned_name { #(#form_idents: #form_val_idents.unwrap()),* })
            }

            pub fn from_cleaned_form(cleaned: #form_cleaned_name) -> Self {
                Self { #(#form_construct),* #(,#m2m_construct)* }
            }

            pub fn apply_cleaned_form(&mut self, cleaned: #form_cleaned_name) {
                #(#form_apply)*
            }

            pub fn meta() -> &'static djangors_orm::ModelMeta {
                static META: std::sync::OnceLock<djangors_orm::ModelMeta> = std::sync::OnceLock::new();
                META.get_or_init(|| djangors_orm::ModelMeta {
                    struct_name: #struct_name,
                    app_label: #app_label,
                    table_name: #table_name,
                    fields: &[
                        #(#fields_expanded),*
                    ],
                    relations: &[
                        #(#relations_expanded),*
                    ],
                    indexes: #indexes_tok,
                    unique_together: #unique_together_tok,
                    ordering: #ordering_tok,
                })
            }

            pub fn pre_save_signal() -> &'static djangors_orm::signals::ModelSignal<djangors_orm::signals::ModelSignalPayload> {
                static SIGNAL: std::sync::OnceLock<djangors_orm::signals::ModelSignal<djangors_orm::signals::ModelSignalPayload>> = std::sync::OnceLock::new();
                SIGNAL.get_or_init(djangors_orm::signals::ModelSignal::new)
            }
            pub fn post_save_signal() -> &'static djangors_orm::signals::ModelSignal<djangors_orm::signals::ModelSignalPayload> {
                static SIGNAL: std::sync::OnceLock<djangors_orm::signals::ModelSignal<djangors_orm::signals::ModelSignalPayload>> = std::sync::OnceLock::new();
                SIGNAL.get_or_init(djangors_orm::signals::ModelSignal::new)
            }
            pub fn pre_delete_signal() -> &'static djangors_orm::signals::ModelSignal<djangors_orm::signals::ModelSignalPayload> {
                static SIGNAL: std::sync::OnceLock<djangors_orm::signals::ModelSignal<djangors_orm::signals::ModelSignalPayload>> = std::sync::OnceLock::new();
                SIGNAL.get_or_init(djangors_orm::signals::ModelSignal::new)
            }
            pub fn post_delete_signal() -> &'static djangors_orm::signals::ModelSignal<djangors_orm::signals::ModelSignalPayload> {
                static SIGNAL: std::sync::OnceLock<djangors_orm::signals::ModelSignal<djangors_orm::signals::ModelSignalPayload>> = std::sync::OnceLock::new();
                SIGNAL.get_or_init(djangors_orm::signals::ModelSignal::new)
            }

            /// Construct Self from a database row, reading each field by its column name.
            pub fn from_row(row: &djangors_orm::DbRow) -> Result<Self, djangors_orm::OrmError> {
                Ok(Self {
                    #(#from_row_assignments),*
                })
            }

            /// Save a new row to the database.
            ///
            /// This is INSERT-only (i.e. always creates a new row). Fields with `auto`
            /// set to true are ignored during insertion and populated by the database.
            /// Returns a new instance populated from the inserted database row.
            pub async fn save(&self, db: &djangors_orm::djangors_db::Database) -> Result<Self, djangors_orm::OrmError> {
                use djangors_orm::djangors_db::DbExecutor;
                Self::pre_save_signal().send(djangors_orm::Model::field_values(self)).await;
                let mut db_ref = db;
                let dialect = db_ref.dialect();
                let save_cols: Vec<&str> = vec![#(#save_cols_vec),*];
                let placeholders: Vec<String> = (1..=save_cols.len()).map(|i| dialect.placeholder(i)).collect();
                let save_cols_quoted: Vec<String> = save_cols.iter().map(|c| format!("\"{}\"", c)).collect();
                let sql = format!(
                    "INSERT INTO {} ({}) VALUES ({}) RETURNING *",
                    #table_name,
                    save_cols_quoted.join(", "),
                    placeholders.join(", ")
                );
                let mut bind_values = Vec::new();
                #(#save_bind_stmts)*
                let row = db_ref.conn().fetch_one(&sql, &bind_values).await?;
                let saved = Self::from_row(&row)?;
                djangors_orm::query_cache::invalidate_table(#table_name);
                Self::post_save_signal().send(djangors_orm::Model::field_values(&saved)).await;
                Ok(saved)
            }

            /// Update an existing row in the database.
            ///
            /// Every non-primary-key column is set to the instance's current field values,
            /// matching on the primary key. Returns `OrmError::NotFound` if no row was updated.
            pub async fn update(&self, db: &djangors_orm::djangors_db::Database) -> Result<(), djangors_orm::OrmError> {
                use djangors_orm::djangors_db::DbExecutor;
                Self::pre_save_signal().send(djangors_orm::Model::field_values(self)).await;
                let mut db_ref = db;
                let dialect = db_ref.dialect();
                let update_cols: Vec<&str> = vec![#(#update_cols_vec),*];
                let set_clauses: Vec<String> = update_cols
                    .iter()
                    .enumerate()
                    .map(|(i, col)| format!("\"{}\" = {}", col, dialect.placeholder(i + 1)))
                    .collect();
                let pk_placeholder = dialect.placeholder(update_cols.len() + 1);
                let sql = format!(
                    "UPDATE {} SET {} WHERE \"{}\" = {}",
                    #table_name,
                    set_clauses.join(", "),
                    #pk_col_str,
                    pk_placeholder
                );
                let mut bind_values = Vec::new();
                #(#update_bind_stmts)*
                let rows_affected = db_ref.conn().execute(&sql, &bind_values).await?;
                if rows_affected == 0 {
                    Err(djangors_orm::OrmError::NotFound {
                        model: Self::meta().struct_name,
                    })
                } else {
                    djangors_orm::query_cache::invalidate_table(#table_name);
                    Self::post_save_signal().send(djangors_orm::Model::field_values(self)).await;
                    Ok(())
                }
            }

            /// Delete an existing row from the database.
            ///
            /// Deletes the row matching the primary key. Returns `OrmError::NotFound` if no row was deleted.
            pub async fn delete(&self, db: &djangors_orm::djangors_db::Database) -> Result<(), djangors_orm::OrmError> {
                use djangors_orm::djangors_db::DbExecutor;
                let payload = djangors_orm::Model::field_values(self);
                Self::pre_delete_signal().send(payload.clone()).await;
                let mut db_ref = db;
                let dialect = db_ref.dialect();
                let sql = format!(
                    "DELETE FROM {} WHERE \"{}\" = {}",
                    #table_name,
                    #pk_col_str,
                    dialect.placeholder(1)
                );
                let val = #delete_bind;
                let bind_val = djangors_orm::BindValue::from(val);
                let rows_affected = db_ref.conn().execute(&sql, &[bind_val]).await?;
                if rows_affected == 0 {
                    Err(djangors_orm::OrmError::NotFound {
                        model: Self::meta().struct_name,
                    })
                } else {
                    djangors_orm::query_cache::invalidate_table(#table_name);
                    Self::post_delete_signal().send(payload).await;
                    Ok(())
                }
            }
        }

        #[allow(missing_docs)]
        impl djangors_orm::ModelForm for #struct_name_ident {
            type FormCleaned = #form_cleaned_name;
            fn validate_form(data: &std::collections::HashMap<String, String>) -> Result<Self::FormCleaned, djangors_orm::djangors_forms::FormErrors> { Self::validate_form(data) }
            fn from_cleaned_form(cleaned: Self::FormCleaned) -> Self { Self::from_cleaned_form(cleaned) }
            fn apply_cleaned_form(&mut self, cleaned: Self::FormCleaned) { Self::apply_cleaned_form(self, cleaned) }
        }

        #[allow(missing_docs)]
        impl djangors_orm::Model for #struct_name_ident {
            fn meta() -> &'static djangors_orm::ModelMeta {
                #struct_name_ident::meta()
            }

            fn field_values(&self) -> Vec<(&'static str, djangors_orm::expr::Value)> {
                vec![
                    #(#field_value_pairs),*
                ]
            }

            fn field_names() -> Vec<&'static str> {
                vec![
                    #(#field_names_list),*
                ]
            }

            fn write_json(&self, buf: &mut Vec<u8>) {
                #(#write_json_stmts)*
                buf.push(b'}');
            }

            fn write_json_filtered(&self, buf: &mut Vec<u8>, include: &mut dyn FnMut(&'static str) -> bool) {
                buf.push(b'{');
                let mut first = true;
                #(#write_json_filtered_stmts)*
                buf.push(b'}');
            }

            #objects_override
        }

        #[allow(missing_docs)]
        impl djangors_orm::FromRow for #struct_name_ident {
            fn from_row(row: &djangors_orm::DbRow) -> Result<Self, djangors_orm::OrmError> {
                #struct_name_ident::from_row(row)
            }
        }

        djangors_orm::inventory::submit! {
            djangors_orm::ModelRegistration {
                meta_fn: #struct_name_ident::meta,
            }
        }
    })
}

// The kind of scalar-to-relation wrapper a model field uses.
#[derive(Clone, Copy, PartialEq)]
enum RelationFieldKind {
    ForeignKey,
    OneToOne,
    ManyToMany,
}

struct ParsedRelationField<'a> {
    target_ty: &'a syn::Type,
    kind: RelationFieldKind,
}

// Extracts the target model type and wrapper kind from a field wrapped in
// `ForeignKey<TargetModel>`, `OneToOne<TargetModel>`, or `ManyToMany<TargetModel>`.
fn get_relation_field(ty: &Type) -> Option<ParsedRelationField<'_>> {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                if let Some(syn::GenericArgument::Type(target_ty)) = args.args.first() {
                    if segment.ident == "ForeignKey" {
                        return Some(ParsedRelationField {
                            target_ty,
                            kind: RelationFieldKind::ForeignKey,
                        });
                    }
                    if segment.ident == "OneToOne" {
                        return Some(ParsedRelationField {
                            target_ty,
                            kind: RelationFieldKind::OneToOne,
                        });
                    }
                    if segment.ident == "ManyToMany" {
                        return Some(ParsedRelationField {
                            target_ty,
                            kind: RelationFieldKind::ManyToMany,
                        });
                    }
                }
            }
        }
    }
    None
}

// Determines if `ty` is wrapped in `Option<T>`, returning the inner type `T`
// and a boolean indicating whether it is optional.
fn resolve_option_type(ty: &Type) -> (&Type, bool) {
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

// Safely extracts the last identifier segment of a path type (e.g. the `T` from `std::path::T`).
fn get_last_path_segment_ident(ty: &Type) -> Option<&Ident> {
    if let Type::Path(type_path) = ty {
        type_path.path.segments.last().map(|seg| &seg.ident)
    } else {
        None
    }
}

// Parses default value expressions (string, integer, float, bool literals, including negative numbers)
// into token streams corresponding to the `djangors_orm::DefaultValue` enum.
fn parse_default_value(expr: &syn::Expr) -> syn::Result<proc_macro2::TokenStream> {
    match expr {
        syn::Expr::Lit(syn::ExprLit { lit, .. }) => match lit {
            syn::Lit::Str(lit_str) => Ok(quote! { djangors_orm::DefaultValue::Text(#lit_str) }),
            syn::Lit::Bool(lit_bool) => Ok(quote! { djangors_orm::DefaultValue::Bool(#lit_bool) }),
            syn::Lit::Int(lit_int) => {
                let val: i64 = lit_int.base10_parse()?;
                Ok(quote! { djangors_orm::DefaultValue::I64(#val) })
            }
            syn::Lit::Float(lit_float) => {
                let val: f64 = lit_float.base10_parse()?;
                Ok(quote! { djangors_orm::DefaultValue::F64(#val) })
            }
            _ => Err(syn::Error::new_spanned(
                lit,
                "unsupported default value literal",
            )),
        },
        syn::Expr::Unary(syn::ExprUnary {
            op: syn::UnOp::Neg(_),
            expr: inner_expr,
            ..
        }) => {
            if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Int(lit_int),
                ..
            }) = &**inner_expr
            {
                let val: i64 = lit_int.base10_parse()?;
                let neg_val = -val;
                Ok(quote! { djangors_orm::DefaultValue::I64(#neg_val) })
            } else if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Float(lit_float),
                ..
            }) = &**inner_expr
            {
                let val: f64 = lit_float.base10_parse()?;
                let neg_val = -val;
                Ok(quote! { djangors_orm::DefaultValue::F64(#neg_val) })
            } else {
                Err(syn::Error::new_spanned(
                    expr,
                    "unsupported default value expression",
                ))
            }
        }
        _ => Err(syn::Error::new_spanned(
            expr,
            "unsupported default value expression",
        )),
    }
}

// Converts a CamelCase string (such as a struct name) into snake_case.
fn snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}
