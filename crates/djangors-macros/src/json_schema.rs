use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    spanned::Spanned, Attribute, Data, DeriveInput, Expr, Fields, GenericParam, Lit, Meta, Type,
    Variant,
};

#[derive(Default)]
struct ContainerSerde {
    rename: Option<String>,
    rename_all: Option<RenameRule>,
    tag: Option<String>,
    content: Option<String>,
    untagged: bool,
    transparent: bool,
}

#[derive(Default)]
struct FieldSerde {
    rename: Option<String>,
    skip: bool,
    skip_serializing: bool,
    skip_deserializing: bool,
    has_default: bool,
    skip_serializing_if: bool,
    flatten: bool,
}

#[derive(Clone, Copy)]
#[allow(clippy::enum_variant_names)]
enum RenameRule {
    LowerCase,
    UpperCase,
    PascalCase,
    CamelCase,
    SnakeCase,
    ScreamingSnakeCase,
    KebabCase,
    ScreamingKebabCase,
}

impl RenameRule {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "lowercase" => Some(Self::LowerCase),
            "UPPERCASE" => Some(Self::UpperCase),
            "PascalCase" => Some(Self::PascalCase),
            "camelCase" => Some(Self::CamelCase),
            "snake_case" => Some(Self::SnakeCase),
            "SCREAMING_SNAKE_CASE" => Some(Self::ScreamingSnakeCase),
            "kebab-case" => Some(Self::KebabCase),
            "SCREAMING-KEBAB-CASE" => Some(Self::ScreamingKebabCase),
            _ => None,
        }
    }

    fn apply(self, ident: &str) -> String {
        match self {
            Self::LowerCase => ident.to_ascii_lowercase(),
            Self::UpperCase => ident.to_ascii_uppercase(),
            Self::PascalCase => to_pascal_case(ident),
            Self::CamelCase => to_camel_case(ident),
            Self::SnakeCase => to_snake_case(ident),
            Self::ScreamingSnakeCase => to_snake_case(ident).to_ascii_uppercase(),
            Self::KebabCase => to_snake_case(ident).replace('_', "-"),
            Self::ScreamingKebabCase => to_snake_case(ident).to_ascii_uppercase().replace('_', "-"),
        }
    }
}

fn split_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    for ch in s.chars() {
        if ch == '_' || ch == '-' || ch.is_whitespace() {
            if !current.is_empty() {
                words.push(current.clone());
                current.clear();
            }
        } else if ch.is_uppercase()
            && !current.is_empty()
            && current.chars().last().is_some_and(|c| c.is_lowercase())
        {
            words.push(current.clone());
            current.clear();
            current.push(ch);
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn to_snake_case(s: &str) -> String {
    split_words(s)
        .into_iter()
        .map(|w| w.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("_")
}

fn to_pascal_case(s: &str) -> String {
    split_words(s)
        .into_iter()
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    let mut out = first.to_uppercase().collect::<String>();
                    out.push_str(&chars.as_str().to_ascii_lowercase());
                    out
                }
            }
        })
        .collect()
}

fn to_camel_case(s: &str) -> String {
    let words = split_words(s);
    let mut out = String::new();
    for (idx, w) in words.into_iter().enumerate() {
        if idx == 0 {
            out.push_str(&w.to_ascii_lowercase());
        } else {
            let mut chars = w.chars();
            if let Some(first) = chars.next() {
                out.extend(first.to_uppercase());
                out.push_str(&chars.as_str().to_ascii_lowercase());
            }
        }
    }
    out
}

fn extract_docs(attrs: &[Attribute]) -> Option<String> {
    let mut lines = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        if let Meta::NameValue(nv) = &attr.meta {
            if let Expr::Lit(expr_lit) = &nv.value {
                if let Lit::Str(lit) = &expr_lit.lit {
                    let text = lit.value();
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        lines.push(trimmed.to_string());
                    }
                }
            }
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn parse_container_serde(attrs: &[Attribute]) -> ContainerSerde {
    let mut out = ContainerSerde::default();
    for attr in attrs {
        if !attr.path().is_ident("serde") && !attr.path().is_ident("schemars") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                if let Ok(val) = meta.value() {
                    if let Ok(lit) = val.parse::<syn::LitStr>() {
                        out.rename = Some(lit.value());
                    }
                }
            } else if meta.path.is_ident("rename_all") {
                if let Ok(val) = meta.value() {
                    if let Ok(lit) = val.parse::<syn::LitStr>() {
                        out.rename_all = RenameRule::parse(&lit.value());
                    }
                }
            } else if meta.path.is_ident("tag") {
                if let Ok(val) = meta.value() {
                    if let Ok(lit) = val.parse::<syn::LitStr>() {
                        out.tag = Some(lit.value());
                    }
                }
            } else if meta.path.is_ident("content") {
                if let Ok(val) = meta.value() {
                    if let Ok(lit) = val.parse::<syn::LitStr>() {
                        out.content = Some(lit.value());
                    }
                }
            } else if meta.path.is_ident("untagged") {
                out.untagged = true;
            } else if meta.path.is_ident("transparent") {
                out.transparent = true;
            } else if meta.input.peek(syn::Token![=]) {
                let _ = meta.value()?.parse::<syn::Expr>();
            }
            Ok(())
        });
    }
    out
}

fn parse_field_serde(attrs: &[Attribute]) -> FieldSerde {
    let mut out = FieldSerde::default();
    for attr in attrs {
        if !attr.path().is_ident("serde") && !attr.path().is_ident("schemars") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                if let Ok(val) = meta.value() {
                    if let Ok(lit) = val.parse::<syn::LitStr>() {
                        out.rename = Some(lit.value());
                    }
                }
            } else if meta.path.is_ident("skip") {
                out.skip = true;
            } else if meta.path.is_ident("skip_serializing") {
                out.skip_serializing = true;
            } else if meta.path.is_ident("skip_deserializing") {
                out.skip_deserializing = true;
            } else if meta.path.is_ident("default") {
                out.has_default = true;
                if meta.input.peek(syn::Token![=]) {
                    let _ = meta.value()?.parse::<syn::Expr>();
                }
            } else if meta.path.is_ident("skip_serializing_if") {
                out.skip_serializing_if = true;
                if meta.input.peek(syn::Token![=]) {
                    let _ = meta.value()?.parse::<syn::Expr>();
                }
            } else if meta.path.is_ident("flatten") {
                out.flatten = true;
            } else if meta.input.peek(syn::Token![=]) {
                let _ = meta.value()?.parse::<syn::Expr>();
            }
            Ok(())
        });
    }
    out
}

fn wire_name(raw_ident: &str, explicit_rename: Option<&str>, rule: Option<RenameRule>) -> String {
    if let Some(r) = explicit_rename {
        return r.to_string();
    }
    match rule {
        Some(rule) => rule.apply(raw_ident),
        None => raw_ident.to_string(),
    }
}

pub fn expand_derive_json_schema(input: DeriveInput) -> syn::Result<TokenStream2> {
    let ident = &input.ident;
    let container_serde = parse_container_serde(&input.attrs);
    let container_doc = extract_docs(&input.attrs);

    let mut generics = input.generics.clone();
    let mut type_param_idents = Vec::new();
    for param in &mut generics.params {
        if let GenericParam::Type(tp) = param {
            type_param_idents.push(tp.ident.clone());
            tp.bounds
                .push(syn::parse_quote!(djangors_core::schemars::JsonSchema));
        }
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let base_name = container_serde
        .rename
        .clone()
        .unwrap_or_else(|| ident.to_string());

    if container_serde.transparent {
        if let Data::Struct(ref data) = input.data {
            if let Some(first_field) = data.fields.iter().next() {
                let inner_ty = &first_field.ty;
                return Ok(quote! {
                    impl #impl_generics djangors_core::schemars::JsonSchema for #ident #ty_generics #where_clause {
                        fn inline_schema() -> bool {
                            <#inner_ty as djangors_core::schemars::JsonSchema>::inline_schema()
                        }
                        fn schema_name() -> std::borrow::Cow<'static, str> {
                            <#inner_ty as djangors_core::schemars::JsonSchema>::schema_name()
                        }
                        fn schema_id() -> std::borrow::Cow<'static, str> {
                            <#inner_ty as djangors_core::schemars::JsonSchema>::schema_id()
                        }
                        fn json_schema(
                            generator: &mut djangors_core::schemars::SchemaGenerator,
                        ) -> djangors_core::schemars::Schema {
                            <#inner_ty as djangors_core::schemars::JsonSchema>::json_schema(generator)
                        }
                    }
                });
            }
        }
    }

    let schema_name_body = if type_param_idents.is_empty() {
        quote! { std::borrow::Cow::Borrowed(#base_name) }
    } else {
        quote! {
            let mut parts: Vec<String> = Vec::new();
            #(parts.push(<#type_param_idents as djangors_core::schemars::JsonSchema>::schema_name().into_owned());)*
            std::borrow::Cow::Owned(format!("{}_{}", #base_name, parts.join("_")))
        }
    };

    let schema_id_body = if type_param_idents.is_empty() {
        quote! { std::borrow::Cow::Borrowed(concat!(module_path!(), "::", #base_name)) }
    } else {
        quote! {
            let mut parts: Vec<String> = Vec::new();
            #(parts.push(<#type_param_idents as djangors_core::schemars::JsonSchema>::schema_id().into_owned());)*
            std::borrow::Cow::Owned(format!("{}::{}<{}>", module_path!(), #base_name, parts.join(", ")))
        }
    };

    let description_stmt = match container_doc {
        Some(doc) => quote! {
            map.insert(
                "description".to_owned(),
                djangors_core::serde_json::Value::String(#doc.to_owned()),
            );
        },
        None => quote! {},
    };

    let body = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => {
                let mut field_stmts = Vec::new();
                for field in &named.named {
                    let f_serde = parse_field_serde(&field.attrs);
                    if f_serde.skip {
                        continue;
                    }
                    let f_ident = field
                        .ident
                        .as_ref()
                        .ok_or_else(|| syn::Error::new(field.span(), "expected named field"))?;
                    let f_name = wire_name(
                        &f_ident.to_string(),
                        f_serde.rename.as_deref(),
                        container_serde.rename_all,
                    );
                    let f_ty = &field.ty;
                    let f_doc = extract_docs(&field.attrs);

                    let skip_ser = f_serde.skip_serializing;
                    let skip_de = f_serde.skip_deserializing;
                    let has_default = f_serde.has_default;
                    let skip_if = f_serde.skip_serializing_if;

                    let doc_insert = match f_doc {
                        Some(doc) => quote! {
                            if let djangors_core::serde_json::Value::Object(ref mut obj) = prop_val {
                                obj.entry("description".to_owned()).or_insert_with(|| {
                                    djangors_core::serde_json::Value::String(#doc.to_owned())
                                });
                            }
                        },
                        None => quote! {},
                    };

                    if f_serde.flatten {
                        field_stmts.push(quote! {
                            let should_omit = (#skip_ser && generator.contract().is_serialize())
                                || (#skip_de && generator.contract().is_deserialize());
                            if !should_omit {
                                let flat_schema = <#f_ty as djangors_core::schemars::JsonSchema>::json_schema(generator);
                                if let Some(obj) = flat_schema.as_object() {
                                    if let Some(djangors_core::serde_json::Value::Object(flat_props)) = obj.get("properties") {
                                        for (k, v) in flat_props {
                                            props.insert(k.clone(), v.clone());
                                        }
                                    }
                                    if let Some(djangors_core::serde_json::Value::Array(flat_req)) = obj.get("required") {
                                        for r in flat_req {
                                            if let Some(s) = r.as_str() {
                                                let s = s.to_owned();
                                                if !required.iter().any(|existing| existing == &s) {
                                                    required.push(s);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        });
                    } else {
                        field_stmts.push(quote! {
                            let should_omit = (#skip_ser && generator.contract().is_serialize())
                                || (#skip_de && generator.contract().is_deserialize());
                            if !should_omit {
                                let mut prop_val = generator.subschema_for::<#f_ty>().to_value();
                                #doc_insert
                                props.insert(#f_name.to_owned(), prop_val);
                                let is_opt = <#f_ty as djangors_core::schemars::JsonSchema>::_schemars_private_is_option()
                                    || #has_default
                                    || #skip_if;
                                if !is_opt {
                                    required.push(#f_name.to_owned());
                                }
                            }
                        });
                    }
                }

                quote! {
                    let mut map = djangors_core::serde_json::Map::new();
                    map.insert("type".to_owned(), djangors_core::serde_json::Value::String("object".to_owned()));
                    map.insert("title".to_owned(), djangors_core::serde_json::Value::String(Self::schema_name().into_owned()));
                    #description_stmt
                    let mut props = djangors_core::serde_json::Map::new();
                    let mut required: Vec<String> = Vec::new();
                    #(#field_stmts)*
                    map.insert("properties".to_owned(), djangors_core::serde_json::Value::Object(props));
                    if !required.is_empty() {
                        map.insert(
                            "required".to_owned(),
                            djangors_core::serde_json::Value::Array(
                                required.into_iter().map(djangors_core::serde_json::Value::String).collect(),
                            ),
                        );
                    }
                    djangors_core::schemars::Schema::from(map)
                }
            }
            Fields::Unnamed(unnamed) => {
                if unnamed.unnamed.len() == 1 {
                    let inner_ty = &unnamed.unnamed[0].ty;
                    quote! {
                        let mut val = <#inner_ty as djangors_core::schemars::JsonSchema>::json_schema(generator).to_value();
                        if let djangors_core::serde_json::Value::Object(ref mut map) = val {
                            map.entry("title".to_owned()).or_insert_with(|| {
                                djangors_core::serde_json::Value::String(Self::schema_name().into_owned())
                            });
                            #description_stmt
                        }
                        djangors_core::schemars::Schema::try_from(val).unwrap_or_else(|_| true.into())
                    }
                } else {
                    let item_schemas: Vec<TokenStream2> = unnamed
                        .unnamed
                        .iter()
                        .map(|f| {
                            let ty = &f.ty;
                            quote! { generator.subschema_for::<#ty>().to_value() }
                        })
                        .collect();
                    let len = item_schemas.len();
                    quote! {
                        let mut map = djangors_core::serde_json::Map::new();
                        map.insert("type".to_owned(), djangors_core::serde_json::Value::String("array".to_owned()));
                        map.insert("title".to_owned(), djangors_core::serde_json::Value::String(Self::schema_name().into_owned()));
                        #description_stmt
                        let items = vec![#(#item_schemas),*];
                        map.insert("prefixItems".to_owned(), djangors_core::serde_json::Value::Array(items));
                        map.insert("minItems".to_owned(), djangors_core::serde_json::Value::from(#len));
                        map.insert("maxItems".to_owned(), djangors_core::serde_json::Value::from(#len));
                        djangors_core::schemars::Schema::from(map)
                    }
                }
            }
            Fields::Unit => {
                quote! {
                    let mut map = djangors_core::serde_json::Map::new();
                    map.insert("type".to_owned(), djangors_core::serde_json::Value::String("null".to_owned()));
                    map.insert("title".to_owned(), djangors_core::serde_json::Value::String(Self::schema_name().into_owned()));
                    #description_stmt
                    djangors_core::schemars::Schema::from(map)
                }
            }
        },
        Data::Enum(data_enum) => {
            let mut active_variants: Vec<&Variant> = Vec::new();
            for variant in &data_enum.variants {
                let v_serde = parse_field_serde(&variant.attrs);
                if !v_serde.skip {
                    active_variants.push(variant);
                }
            }
            let all_unit = active_variants
                .iter()
                .all(|v| matches!(v.fields, Fields::Unit));

            if all_unit && container_serde.tag.is_none() {
                let variant_names: Vec<String> = active_variants
                    .iter()
                    .map(|v| {
                        let v_serde = parse_field_serde(&v.attrs);
                        wire_name(
                            &v.ident.to_string(),
                            v_serde.rename.as_deref(),
                            container_serde.rename_all,
                        )
                    })
                    .collect();
                quote! {
                    let mut map = djangors_core::serde_json::Map::new();
                    map.insert("type".to_owned(), djangors_core::serde_json::Value::String("string".to_owned()));
                    map.insert("title".to_owned(), djangors_core::serde_json::Value::String(Self::schema_name().into_owned()));
                    #description_stmt
                    map.insert(
                        "enum".to_owned(),
                        djangors_core::serde_json::Value::Array(vec![
                            #(djangors_core::serde_json::Value::String(#variant_names.to_owned())),*
                        ]),
                    );
                    djangors_core::schemars::Schema::from(map)
                }
            } else {
                let variant_schemas: Vec<TokenStream2> = active_variants
                    .iter()
                    .map(|v| build_enum_variant_schema(v, &container_serde))
                    .collect();
                quote! {
                    let mut map = djangors_core::serde_json::Map::new();
                    map.insert("title".to_owned(), djangors_core::serde_json::Value::String(Self::schema_name().into_owned()));
                    #description_stmt
                    map.insert(
                        "oneOf".to_owned(),
                        djangors_core::serde_json::Value::Array(vec![#(#variant_schemas),*]),
                    );
                    djangors_core::schemars::Schema::from(map)
                }
            }
        }
        Data::Union(u) => {
            return Err(syn::Error::new(
                u.union_token.span(),
                "#[derive(JsonSchema)] does not support Rust unions",
            ));
        }
    };

    Ok(quote! {
        impl #impl_generics djangors_core::schemars::JsonSchema for #ident #ty_generics #where_clause {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                #schema_name_body
            }

            fn schema_id() -> std::borrow::Cow<'static, str> {
                #schema_id_body
            }

            fn json_schema(
                generator: &mut djangors_core::schemars::SchemaGenerator,
            ) -> djangors_core::schemars::Schema {
                #body
            }
        }
    })
}

fn build_enum_variant_schema(variant: &Variant, container: &ContainerSerde) -> TokenStream2 {
    let v_serde = parse_field_serde(&variant.attrs);
    let v_name = wire_name(
        &variant.ident.to_string(),
        v_serde.rename.as_deref(),
        container.rename_all,
    );

    let payload_schema = match &variant.fields {
        Fields::Unit => None,
        Fields::Unnamed(unnamed) => {
            if unnamed.unnamed.len() == 1 {
                let ty = &unnamed.unnamed[0].ty;
                Some(quote! { generator.subschema_for::<#ty>().to_value() })
            } else {
                let items: Vec<TokenStream2> = unnamed
                    .unnamed
                    .iter()
                    .map(|f| {
                        let ty = &f.ty;
                        quote! { generator.subschema_for::<#ty>().to_value() }
                    })
                    .collect();
                let len = items.len();
                Some(quote! {
                    djangors_core::serde_json::json!({
                        "type": "array",
                        "prefixItems": [#(#items),*],
                        "minItems": #len,
                        "maxItems": #len,
                    })
                })
            }
        }
        Fields::Named(named) => {
            let mut stmts = Vec::new();
            for f in &named.named {
                let f_serde = parse_field_serde(&f.attrs);
                if f_serde.skip {
                    continue;
                }
                if let Some(f_ident) = &f.ident {
                    let f_name = wire_name(&f_ident.to_string(), f_serde.rename.as_deref(), None);
                    let ty: &Type = &f.ty;
                    let has_default = f_serde.has_default;
                    let skip_if = f_serde.skip_serializing_if;
                    stmts.push(quote! {
                        props.insert(#f_name.to_owned(), generator.subschema_for::<#ty>().to_value());
                        if !(<#ty as djangors_core::schemars::JsonSchema>::_schemars_private_is_option()
                            || #has_default
                            || #skip_if)
                        {
                            req.push(djangors_core::serde_json::Value::String(#f_name.to_owned()));
                        }
                    });
                }
            }
            Some(quote! {
                {
                    let mut props = djangors_core::serde_json::Map::new();
                    let mut req: Vec<djangors_core::serde_json::Value> = Vec::new();
                    #(#stmts)*
                    let mut obj = djangors_core::serde_json::Map::new();
                    obj.insert("type".to_owned(), djangors_core::serde_json::Value::String("object".to_owned()));
                    obj.insert("properties".to_owned(), djangors_core::serde_json::Value::Object(props));
                    if !req.is_empty() {
                        obj.insert("required".to_owned(), djangors_core::serde_json::Value::Array(req));
                    }
                    djangors_core::serde_json::Value::Object(obj)
                }
            })
        }
    };

    if container.untagged {
        return payload_schema
            .unwrap_or_else(|| quote! { djangors_core::serde_json::json!({ "type": "null" }) });
    }

    if let (Some(tag), Some(content)) = (&container.tag, &container.content) {
        return match payload_schema {
            Some(payload) => quote! {
                djangors_core::serde_json::json!({
                    "type": "object",
                    "properties": {
                        #tag: { "type": "string", "const": #v_name },
                        #content: #payload,
                    },
                    "required": [#tag, #content],
                })
            },
            None => quote! {
                djangors_core::serde_json::json!({
                    "type": "object",
                    "properties": {
                        #tag: { "type": "string", "const": #v_name },
                    },
                    "required": [#tag],
                })
            },
        };
    }

    if let Some(tag) = &container.tag {
        return match &variant.fields {
            Fields::Unit => quote! {
                djangors_core::serde_json::json!({
                    "type": "object",
                    "properties": {
                        #tag: { "type": "string", "const": #v_name },
                    },
                    "required": [#tag],
                })
            },
            _ => {
                let payload = payload_schema
                    .unwrap_or_else(|| quote! { djangors_core::serde_json::json!({}) });
                quote! {
                    {
                        let mut val = #payload;
                        if let djangors_core::serde_json::Value::Object(ref mut obj) = val {
                            if let Some(djangors_core::serde_json::Value::Object(ref mut props)) = obj.get_mut("properties") {
                                props.insert(#tag.to_owned(), djangors_core::serde_json::json!({ "type": "string", "const": #v_name }));
                            }
                            let req = obj.entry("required".to_owned()).or_insert_with(|| djangors_core::serde_json::json!([]));
                            if let djangors_core::serde_json::Value::Array(ref mut arr) = req {
                                arr.push(djangors_core::serde_json::Value::String(#tag.to_owned()));
                            }
                        }
                        val
                    }
                }
            }
        };
    }

    match payload_schema {
        None => quote! { djangors_core::serde_json::json!({ "type": "string", "const": #v_name }) },
        Some(payload) => quote! {
            djangors_core::serde_json::json!({
                "type": "object",
                "properties": {
                    #v_name: #payload,
                },
                "required": [#v_name],
            })
        },
    }
}
