use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn collect(root: &Path, directory: &Path, output: &mut Vec<(String, PathBuf)>) {
    for entry in fs::read_dir(directory).expect("read standalone frontend export") {
        let entry = entry.expect("read standalone frontend asset");
        let kind = entry
            .file_type()
            .expect("inspect standalone frontend asset");
        assert!(
            !kind.is_symlink(),
            "Frontend export must not contain symlinks"
        );
        let path = entry.path();
        if kind.is_dir() {
            collect(root, &path, output);
        } else if kind.is_file() {
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .expect("UTF-8 frontend asset path")
                .replace('\\', "/");
            if !name.ends_with(".map") {
                output.push((name, path));
            }
        }
    }
}
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=frontend/out");
    if env::var_os("CARGO_FEATURE_FRONTEND").is_none() {
        return;
    }
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("frontend/out");
    assert!(
        root.join("index.html").is_file(),
        "Build the standalone frontend first: bun run --cwd apps/standalone build:frontend"
    );
    let mut assets = Vec::new();
    collect(&root, &root, &mut assets);
    assets.sort_by(|a, b| a.0.cmp(&b.0));
    let mut source = String::from("static ASSETS: &[(&str, &[u8])] = &[\n");
    for (name, path) in assets {
        source.push_str(&format!(
            "({name:?}, include_bytes!({:?})),\n",
            path.to_str().unwrap()
        ));
    }
    source.push_str("];\n");
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("standalone_frontend.rs"),
        source,
    )
    .expect("write embedded frontend table");
}
