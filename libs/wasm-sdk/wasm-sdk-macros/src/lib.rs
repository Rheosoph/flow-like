use proc_macro::TokenStream;
use quote::quote;
use syn::{Item, parse_macro_input};

/// Register a WASM node struct for automatic discovery.
///
/// Place this on a struct that implements `Default` and `WasmNode`.
/// The macro generates an `inventory::submit!` call that registers the node
/// so `wasm_main!()` can auto-discover it at startup.
///
/// # Example
///
/// ```rust,ignore
/// use flow_like_wasm_sdk::prelude::*;
///
/// #[register_node]
/// #[derive(Default)]
/// pub struct MyNode;
///
/// impl WasmNode for MyNode {
///     fn get_node(&self) -> NodeDefinition {
///         let mut node = NodeDefinition::new("my_node", "My Node", "Desc", "Cat");
///         node.add_pin(PinDefinition::input("exec", "Exec", "Trigger", "Exec"));
///         node.add_pin(PinDefinition::output("exec_out", "Done", "Done", "Exec"));
///         node
///     }
///
///     fn run(&self, mut ctx: Context) -> ExecutionResult {
///         ctx.activate_exec("exec_out");
///         ctx.success()
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn register_node(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as Item);

    let (struct_item, struct_name) = match &input {
        Item::Struct(item_struct) => (input.clone(), item_struct.ident.clone()),
        _ => {
            return syn::Error::new_spanned(
                &input,
                "register_node can only be applied to structs",
            )
            .to_compile_error()
            .into();
        }
    };

    let expanded = quote! {
        #struct_item

        ::flow_like_wasm_sdk::inventory::submit! {
            ::flow_like_wasm_sdk::WasmNodeEntry::new(
                || <#struct_name as ::flow_like_wasm_sdk::WasmNode>::get_node(&<#struct_name>::default()),
                |ctx| <#struct_name as ::flow_like_wasm_sdk::WasmNode>::run(&<#struct_name>::default(), ctx),
            )
        }
    };

    TokenStream::from(expanded)
}
