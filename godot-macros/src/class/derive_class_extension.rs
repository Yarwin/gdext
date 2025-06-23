/*
 * Copyright (c) godot-rust; Bromeon and contributors.
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
use proc_macro2::{Punct, TokenStream};
use quote::quote;
use crate::{util, ParseResult};
use crate::class::data_models::fields;
use crate::class::data_models::fields::Fields;
use crate::class::data_models::group_export::{sort_fields_by_group, FieldGroup};
use crate::class::{make_class_extension_property_impl, make_class_property_impl, Field, FieldCond, FieldDefault, FieldExport, FieldVar};
use crate::util::{error, path_ends_with_complex, KvParser};

pub fn derive_class_extension(item: venial::Item) -> ParseResult<TokenStream> {
    let extension = item.as_struct().ok_or_else(|| {
        util::error_fn(
            "#[derive(ClassExtension)] is only allowed on structs",
            item.name(),
        )
    })?;
    
    let named_fields = fields::named_fields(extension, "#[derive(ClassExtension)]")?;
    let mut fields: Fields = parse_fields(named_fields)?;
    sort_fields_by_group(&mut fields);
    
    let errors = fields.errors.iter().map(|error| error.to_compile_error());
    
    let extension_name = &extension.name;

    // TODO - class extension should provide docs as well.
    // #[cfg(not(all(feature = "register-docs", since_api = "4.3")))]
    
    let prv = quote! { ::godot::private };
    let godot_exports_impl = make_class_extension_property_impl(extension_name, &fields);
    
    ParseResult::Ok(quote! {})
}

fn parse_fields(
    named_fields: Vec<(venial::NamedField, Punct)>,
) -> ParseResult<Fields> {
    let mut all_fields = vec![];
    let mut errors = vec![];
    let mut groups = vec![];
    
    for (named_field, _punct) in named_fields {
        let mut field = Field::new(&named_field);
        
        if path_ends_with_complex(&field.ty, "OnReady") || path_ends_with_complex(&field.ty, "OnEditor") {
            errors.push(error!(
                    field.ty.clone(),
                    "`OnReady<T>` is not supported for class extensions for now"
                ));
        }
        if let Some(mut parser) = KvParser::parse(&named_field.attributes, "init")? {
            // #[init(val = EXPR)]
            if let Some(default) = parser.handle_expr("val")? {
                field.default_val = Some(FieldDefault {
                    default_val: default,
                    span: parser.span(),
                });
            }
            
            // #[init(sentinel = EXPR)]
            if let Some(sentinel_value) = parser.handle_expr("sentinel")? {
                field.set_default_val_if(
                    || quote! { OnEditor::from_sentinel(#sentinel_value) },
                    FieldCond::IsOnEditor,
                    &parser,
                    &mut errors,
                );
            }
        }
        
        // #[export]
        if let Some(mut parser) = KvParser::parse(&named_field.attributes, "export")? {
            let export = FieldExport::new_from_kv(&mut parser)?;
            field.export = Some(export);
            let group = FieldGroup::new_from_kv(&mut parser, &mut groups);
            field.group = Some(group);
            parser.finish()?;
        }

        // #[var]
        if let Some(mut parser) = KvParser::parse(&named_field.attributes, "var")? {
            let var = FieldVar::new_from_kv(&mut parser)?;
            field.var = Some(var);
            parser.finish()?;
        }
        
        all_fields.push(field);
    }

    Ok(Fields {
        groups,
        all_fields,
        base_field: None,
        deprecations: vec![],
        errors,
    })
}
