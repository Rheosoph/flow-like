use proc_macro::TokenStream;
use proc_macro2::TokenTree;
use quote::quote;
use syn::{Item, LitStr, Token, parse::Parse, parse::ParseStream, parse_macro_input};

/// Check the top-level tokens for the `struct` item keyword without building a syn AST.
///
/// Groups are deliberately opaque here: a `struct` nested in a function or macro body must not
/// make that outer item look like a struct. Rustc remains responsible for diagnosing malformed
/// token sequences; this guard only preserves the macro's validation for valid non-struct items.
fn is_struct_item(item: &proc_macro2::TokenStream) -> bool {
    item.clone()
        .into_iter()
        .any(|token| matches!(token, TokenTree::Ident(ident) if ident == "struct"))
}

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
/// ```no_run
/// use flow_like_catalog_macros::register_node;
///
/// // With explicit name (recommended):
/// #[register_node(name = "my_node_name")]
/// #[derive(Default)]
/// pub struct NamedNode {}
///
/// // Without explicit name (name must be implemented manually):
/// #[register_node]
/// #[derive(Default)]
/// pub struct MarkerNode {}
///
/// assert_eq!(NamedNode::NODE_NAME, "my_node_name");
/// let _ = MarkerNode::default();
/// ```
///
/// ```compile_fail
/// use flow_like_catalog_macros::register_node;
///
/// #[register_node]
/// enum NotANode {
///     Value,
/// }
/// ```
///
/// This will automatically register the node when the catalog is initialized.
#[proc_macro_attribute]
pub fn register_node(attr: TokenStream, item: TokenStream) -> TokenStream {
    // In-tree registrations use this as a marker. The catalog build helper performs the
    // registration, so parsing and re-emitting the complete struct here is unnecessary work on
    // the common path. Keep the named form below for API compatibility.
    if attr.is_empty() {
        let item_tokens = proc_macro2::TokenStream::from(item.clone());
        if !is_struct_item(&item_tokens) {
            return quote! {
                compile_error!("register_node can only be used on structs");
            }
            .into();
        }
        return item;
    }

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

#[cfg(test)]
mod tests {
    use super::is_struct_item;
    use quote::quote;

    #[test]
    fn empty_attribute_fast_path_accepts_struct_forms() {
        assert!(is_struct_item(&quote!(
            struct Private;
        )));
        assert!(is_struct_item(&quote!(
            pub struct Public {}
        )));
        assert!(is_struct_item(&quote!(
            #[derive(Default)]
            pub(crate) struct Generic<T>(T);
        )));
    }

    #[test]
    fn empty_attribute_fast_path_rejects_valid_non_struct_items() {
        assert!(!is_struct_item(&quote!(
            enum NotAStruct {
                Value,
            }
        )));
        assert!(!is_struct_item(&quote!(
            fn not_a_struct() {}
        )));
        assert!(!is_struct_item(&quote!(union NotAStruct { value: u32 })));
        assert!(!is_struct_item(&quote!(
            const NOT_A_STRUCT: u32 = 1;
        )));
        assert!(!is_struct_item(&quote!(
            some_macro! { struct NestedInsideMacro; }
        )));
    }
}
