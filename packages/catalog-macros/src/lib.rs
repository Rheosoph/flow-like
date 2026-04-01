use proc_macro::TokenStream;
use quote::quote;
use syn::{Item, LitStr, Token, parse::Parse, parse::ParseStream, parse_macro_input};

/// Attributes for the register_node macro
struct RegisterNodeAttr {
    name: Option<LitStr>,
}

impl Parse for RegisterNodeAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(RegisterNodeAttr { name: None });
        }

        let ident: syn::Ident = input.parse()?;
        if ident != "name" {
            return Err(syn::Error::new(ident.span(), "expected `name`"));
        }
        input.parse::<Token![=]>()?;
        let name: LitStr = input.parse()?;
        Ok(RegisterNodeAttr { name: Some(name) })
    }
}

/// Procedural macro to automatically register nodes in the catalog.
///
/// Usage:
/// ```
/// // With explicit name (recommended):
/// #[register_node(name = "my_node_name")]
/// #[derive(Default)]
/// pub struct MyNode {}
///
/// // Without explicit name (name must be implemented manually):
/// #[register_node]
/// #[derive(Default)]
/// pub struct MyNode {}
/// ```
///
/// This will automatically register the node when the catalog is initialized.
#[proc_macro_attribute]
pub fn register_node(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attrs = parse_macro_input!(attr as RegisterNodeAttr);
    let input = parse_macro_input!(item as Item);

    let (struct_item, struct_name) = match &input {
        Item::Struct(item_struct) => (input.clone(), item_struct.ident.clone()),
        _ => panic!("register_node can only be used on structs"),
    };

    let name_impl = if let Some(name_lit) = attrs.name {
        quote! {
            impl #struct_name {
                pub const NODE_NAME: &'static str = #name_lit;
            }
        }
    } else {
        quote! {}
    };

    let expanded = quote! {
        #struct_item

        #name_impl
    };

    TokenStream::from(expanded)
}
