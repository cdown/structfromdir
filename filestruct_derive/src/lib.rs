use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;

use quote::ToTokens;
use std::path::PathBuf;
use syn::{parse_macro_input, Data, DeriveInput, Fields, LitBool, LitStr, PathArguments, Type};

#[derive(Default)]
struct FieldAttributes {
    filename: Option<String>,
    trim: Option<bool>,
    relative_dir: Option<String>,
}

fn get_attributes(field: &syn::Field) -> Result<FieldAttributes, syn::parse::Error> {
    let mut attrs = FieldAttributes::default();

    for attr in &field.attrs {
        if attr.path().is_ident("filestruct") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("file") {
                    let value = meta.value()?;
                    let s: LitStr = value.parse()?;
                    attrs.filename = Some(s.value());
                } else if meta.path.is_ident("trim") {
                    let value = meta.value()?;
                    let b: LitBool = value.parse()?;
                    attrs.trim = Some(b.value());
                } else if meta.path.is_ident("relative_dir") {
                    let value = meta.value()?;
                    let s: LitStr = value.parse()?;
                    attrs.relative_dir = Some(s.value());
                } else {
                    return Err(meta.error("unsupported attribute"));
                }
                Ok(())
            })?;
        }
    }

    Ok(attrs)
}

fn make_trim_check(ty: impl ToTokens, explicit_trim: Option<bool>) -> TokenStream2 {
    let use_dfl_bhvr = explicit_trim.is_none();
    let trim = explicit_trim.unwrap_or(false);

    quote::quote! {
        match (#use_dfl_bhvr, TypeId::of::<#ty>() == TypeId::of::<String>(), #trim) {
            (true, false, _) | (false, _, true) => {
                raw_data.trim()
            },
            (true, true, _) | (false, _, false) => {
                &raw_data
            },
        }
    }
}

#[proc_macro_derive(FromDir, attributes(filestruct))]
pub fn from_dir(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let ident = input.ident;
    let fields = match input.data {
        Data::Struct(data) => match data.fields {
            Fields::Named(fields) => fields,
            _ => panic!("FromDir only supports named fields"),
        },
        _ => panic!("FromDir only supports structs"),
    };

    let field_parsers = fields
        .named
        .iter()
        .map(|f| {
            let field_ident = f
                .ident
                .as_ref()
                .expect("Named field should have an identifier");
            let field_ty = &f.ty;
            let attributes = get_attributes(f).expect("Invalid attributes");
            let mut file_name = attributes
                .filename
                .unwrap_or_else(|| field_ident.to_string());
            if let Some(relative_dir) = attributes.relative_dir {
                file_name = [relative_dir, file_name]
                    .iter()
                    .collect::<PathBuf>()
                    .to_str()
                    .unwrap()
                    .to_owned();
            }
            match field_ty {
                Type::Path(type_path)
                    if type_path.path.segments.last().unwrap().ident == "Option" =>
                {
                    let inner_ty = match &type_path.path.segments.last().unwrap().arguments {
                        PathArguments::AngleBracketed(inner_ty) => &inner_ty.args[0],
                        _ => panic!("Unsupported Option type"),
                    };
                    let trim_check = make_trim_check(inner_ty, attributes.trim);
                    quote::quote! {
                        let path = dir.join(#file_name);
                        let #field_ident: #field_ty = {
                            if let Ok(raw_data) = fs::read_to_string(path) {
                                let data = #trim_check;
                                #inner_ty::from_str(data).ok()
                            } else {
                                None
                            }
                        };
                    }
                }
                _ => {
                    let trim_check = make_trim_check(field_ty, attributes.trim);
                    quote::quote! {
                        let path = dir.join(#file_name);
                        let raw_data = fs::read_to_string(&path)
                            .map_err(|err| filestruct::Error::Io { file: path.clone(), err })?;
                        let data = #trim_check;
                        let #field_ident: #field_ty = #field_ty::from_str(data)
                            .map_err(|_| filestruct::Error::Parse {
                                file: path,
                                input: raw_data,
                                ty: stringify!(#field_ty).to_string()
                            })?;
                    }
                }
            }
        })
        .collect::<Vec<_>>();

    let field_idents = fields
        .named
        .iter()
        .map(|f| {
            f.ident
                .as_ref()
                .expect("Named field should have an identifier")
        })
        .collect::<Vec<_>>();

    let expanded = quote::quote! {
        #[automatically_derived]
        impl #ident {
            pub fn from_dir(dir: impl AsRef<std::path::Path>) -> Result<Self, filestruct::Error> {
                use std::fs;
                use std::str::FromStr;
                use std::any::TypeId;

                let dir = dir.as_ref();
                #(#field_parsers)*

                Ok(Self {
                    #(#field_idents),*
                })
            }
        }
    };

    TokenStream::from(expanded)
}
