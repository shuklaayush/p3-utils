use quote::quote;
use syn::Fields;

pub fn generate_trait_impls(
    name: &syn::Ident,
    variants: &syn::punctuated::Punctuated<syn::Variant, syn::token::Comma>,
) -> proc_macro2::TokenStream {
    let variant_names: Vec<_> = variants.iter().map(|variant| &variant.ident).collect();
    let variant_field_types: Vec<_> = variants
        .iter()
        .map(|variant| match &variant.fields {
            Fields::Unnamed(fields) => &fields.unnamed.first().unwrap().ty,
            _ => panic!("EnumDispatch only supports enum variants with a single unnamed field"),
        })
        .collect();

    quote! {
        impl std::fmt::Display for #name {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                match self {
                    #(#name::#variant_names(_) => write!(f, stringify!(#variant_names)),)*
                }
            }
        }

        impl<F: p3_field::Field> p3_air::BaseAir<F> for #name {
            fn width(&self) -> usize {
                match self {
                    #(#name::#variant_names(chip) => <#variant_field_types as p3_air::BaseAir<F>>::width(chip),)*
                }
            }

            fn preprocessed_trace(&self) -> Option<p3_matrix::dense::RowMajorMatrix<F>> {
                match self {
                    #(#name::#variant_names(chip) => <#variant_field_types as p3_air::BaseAir<F>>::preprocessed_trace(chip),)*
                }
            }
        }

        impl<AB: p3_air::AirBuilder> p3_air::Air<AB> for #name {
            fn eval(&self, builder: &mut AB) {
                match self {
                    #(#name::#variant_names(chip) => <#variant_field_types as p3_air::Air<AB>>::eval(chip, builder),)*
                }
            }
        }

        impl<F: p3_field::Field> p3_interaction::InteractionAir<F> for #name {
            fn receives(&self) -> Vec<p3_interaction::Interaction<F>> {
                match self {
                    #(#name::#variant_names(chip) => <#variant_field_types as p3_interaction::InteractionAir<F>>::receives(chip),)*
                }
            }

            fn sends(&self) -> Vec<p3_interaction::Interaction<F>> {
                match self {
                    #(#name::#variant_names(chip) => <#variant_field_types as p3_interaction::InteractionAir<F>>::sends(chip),)*
                }
            }
        }

        impl<AB: p3_interaction::InteractionAirBuilder> p3_interaction::Rap<AB> for #name {
            fn preprocessed_width(&self) -> usize {
                match self {
                    #(#name::#variant_names(chip) => <#variant_field_types as p3_interaction::Rap<AB>>::preprocessed_width(chip),)*
                }
            }
        }

        #[cfg(feature = "trace-writer")]
        impl<F: p3_field::Field, EF: p3_field::ExtensionField<F>> p3_air_util::TraceWriter<F, EF> for #name {
            fn preprocessed_headers(&self) -> Vec<String> {
                match self {
                    #(#name::#variant_names(chip) => <#variant_field_types as p3_air_util::TraceWriter<F, EF>>::preprocessed_headers(chip),)*
                }
            }

            fn headers(&self) -> Vec<String> {
                match self {
                    #(#name::#variant_names(chip) => <#variant_field_types as p3_air_util::TraceWriter<F, EF>>::headers(chip),)*
                }
            }
        }

        impl<SC: p3_uni_stark::StarkGenericConfig> p3_machine::chip::Chip<SC> for #name {}
    }
}
