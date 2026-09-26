use flow_like::flow::{
    ast::{
        declarations_by_category, declarations_by_package, node_names, node_to_signature,
        node_to_signature_in, schema_sidecar,
    },
    node::NodeLogic,
};
use flow_like_catalog_onnx::laya::LayaNode;
use std::{fs, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = Path::new("/private/tmp/flow-like-laya/metadata-new");
    fs::create_dir_all(output)?;
    let node = LayaNode {}.get_node();
    let signature = node_to_signature(&node);
    assert_eq!(node.version, Some(2));
    assert_eq!(signature.inputs[0].name, "model_dir");
    assert!(signature.outputs.iter().all(|pin| !["score", "noul"].contains(&pin.name.as_str())));
    fs::write(output.join("signature.json"), serde_json::to_string_pretty(&signature)?)?;
    fs::write(output.join("names.json"), serde_json::to_string_pretty(&node_names(&node))?)?;
    fs::write(output.join("schemas.json"), serde_json::to_string_pretty(&schema_sidecar(std::slice::from_ref(&signature)))?)?;
    fs::write(output.join("member.flow.d"), format!("{}\n\n", signature.render_namespace_member("    ")))?;
    let category = declarations_by_category(&[signature]);
    fs::write(output.join("category.flow.d"), &category[0].content)?;
    let package = declarations_by_package(&[node_to_signature_in(&node, "onnx")]);
    fs::write(output.join("package.flow.d"), &package[0].content)?;
    println!("Generated Laya metadata in {}", output.display());
    Ok(())
}
