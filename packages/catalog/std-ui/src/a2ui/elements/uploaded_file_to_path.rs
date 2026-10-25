use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::{FlowPath, UploadedFile, materialize_uploaded_files};
use flow_like_types::{Value, anyhow, async_trait, json::json};

/// Turns an uploaded file (a widget upload, a File Input entry, …) into a FlowPath.
///
/// Request-file FlowPaths only resolve inside the run's own scratch roots, so a forged
/// upload cannot reach app storage through this node.
#[crate::register_node]
#[derive(Default)]
pub struct UploadedFileToPath;

impl UploadedFileToPath {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UploadedFileToPath {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_uploaded_file_to_path",
            "Uploaded File to Path",
            "Turns an uploaded file from a widget upload or File Input into a FlowPath that file nodes can read. Downloads the file into this run when only its URL is known.",
            "UI/Elements/Files",
        );
        node.set_flowscript_name("ui", "uploadedFileToPath");
        node.add_icon("/flow/icons/a2ui.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(7)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);
        node.add_input_pin(
            "file",
            "File",
            "Uploaded file, e.g. a file field of a widget event or one entry of Get File Input Files",
            VariableType::Struct,
        )
        .set_schema::<UploadedFile>();

        node.add_output_pin(
            "exec_out",
            "▶",
            "Runs once the path is ready",
            VariableType::Execution,
        );
        node.add_output_pin(
            "path",
            "Path",
            "FlowPath of the uploaded file in this run's temporary storage",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let value: Value = context.evaluate_pin("file").await?;
        let mut file: UploadedFile = flow_like_types::json::from_value(value)
            .map_err(|error| anyhow!("Input \"file\" is not an uploaded file: {error}"))?;
        let name = file
            .name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "unnamed file".to_string());
        let scope = context.node.node_id().to_string();

        let path = materialize_uploaded_files(context, &scope, std::slice::from_mut(&mut file))
            .await
            .map_err(|error| anyhow!("Could not resolve uploaded file \"{name}\": {error}"))?
            .pop()
            .ok_or_else(|| {
                anyhow!("Uploaded file \"{name}\" has neither a FlowPath nor a URL to read it from")
            })?;

        context.set_pin_value("path", json!(path)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use ahash::AHashMap;
    use flow_like::{
        flow::{
            board::ExecutionStage,
            execution::{
                LogLevel, context::ExecutionContextCache, internal_node::InternalNode,
                internal_pin::InternalPin,
            },
        },
        profile::Profile,
        state::{FlowLikeConfig, FlowLikeState, FlowLikeStores},
        utils::http::HTTPClient,
    };
    use flow_like_storage::files::store::{FlowLikeStore, local_store::LocalObjectStore};
    use flow_like_types::{
        Cacheable,
        dispatch::REQUEST_FILES_STORE_REF,
        sync::{Mutex, RwLock},
    };
    use std::{
        path::PathBuf,
        sync::{Arc, Weak},
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "flow-like-uploaded-file-to-path-{}",
                flow_like_types::create_id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn upload_context(root: &TestDirectory) -> ExecutionContext {
        let node = UploadedFileToPath.get_node();
        let mut pins = AHashMap::new();
        let mut names = AHashMap::<String, Vec<Arc<InternalPin>>>::new();
        for pin in node.pins.values() {
            let internal = Arc::new(InternalPin::new(pin, false));
            names
                .entry(pin.name.clone())
                .or_default()
                .push(internal.clone());
            pins.insert(pin.id.clone(), internal);
        }
        let current = Arc::new(InternalNode::new(
            node,
            pins,
            Arc::new(UploadedFileToPath),
            names,
        ));
        for pin in current.pins.iter() {
            pin.init_node(Arc::downgrade(&current));
            pin.init_connected_to(Vec::new());
            pin.init_depends_on(Vec::new());
        }
        let mut context = ExecutionContext::new(
            Arc::new(AHashMap::from_iter([(
                current.node_id().to_owned(),
                current.clone(),
            )])),
            &Weak::new(),
            &Arc::new(FlowLikeState::new(
                FlowLikeConfig::new(),
                HTTPClient::new_without_refetch(),
            )),
            &current,
            &Arc::new(Mutex::new(AHashMap::new())),
            &Arc::new(RwLock::new(AHashMap::new())),
            LogLevel::Debug,
            ExecutionStage::Dev,
            Arc::new(Profile::default()),
            None,
            Arc::new(RwLock::new(Vec::new())),
            None,
            None,
            Arc::new(AHashMap::new()),
            None,
        )
        .await;
        context.execution_cache = Some(ExecutionContextCache {
            stores: FlowLikeStores::default(),
            app_id: "app".into(),
            model_usage_app_id: None,
            board_dir: flow_like_storage::Path::from("apps/app"),
            board_id: "board".into(),
            node_id: current.shared_node_id(),
            sub: "viewer".into(),
            shadow: false,
        });
        let store = LocalObjectStore::new(root.0.clone()).unwrap();
        let store: Arc<dyn Cacheable> = Arc::new(FlowLikeStore::Local(Arc::new(store)));
        context.set_cache(REQUEST_FILES_STORE_REF, store).await;
        context
    }

    fn uploaded(path: Option<&str>) -> Value {
        json!(UploadedFile {
            name: Some("report.pdf".into()),
            size: Some(9),
            mime_type: Some("application/pdf".into()),
            flow_path: path.map(|path| FlowPath::new(
                path.into(),
                REQUEST_FILES_STORE_REF.into(),
                None
            )),
            ..Default::default()
        })
    }

    async fn run_with(context: &mut ExecutionContext, file: Value) -> flow_like_types::Result<()> {
        context.set_pin_value("file", file).await.unwrap();
        context.activate_exec_pin("exec_out").await.unwrap();
        UploadedFileToPath.run(context).await
    }

    #[tokio::test]
    async fn an_upload_in_the_viewers_scratch_area_becomes_a_readable_path() {
        let root = TestDirectory::new();
        let mut context = upload_context(&root).await;
        let path = "tmp/user/viewer/apps/app/uploads/report.pdf";
        FlowPath::new(path.into(), REQUEST_FILES_STORE_REF.into(), None)
            .put(&mut context, b"%PDF-1.7".to_vec(), true)
            .await
            .unwrap();

        run_with(&mut context, uploaded(Some(path))).await.unwrap();

        let output: FlowPath = context.evaluate_pin("path").await.unwrap();
        assert_eq!(output.path, path);
        assert_eq!(output.store_ref, REQUEST_FILES_STORE_REF);
        assert_eq!(output.get(&mut context, false).await.unwrap(), b"%PDF-1.7");
        assert!(context.evaluate_pin::<bool>("exec_out").await.unwrap());
    }

    #[tokio::test]
    async fn forged_and_empty_uploads_fail_with_the_file_name_and_reason() {
        let root = TestDirectory::new();
        let mut context = upload_context(&root).await;
        let secret = root.0.join("apps/app/storage/secret.md");
        std::fs::create_dir_all(secret.parent().unwrap()).unwrap();
        std::fs::write(&secret, b"app secret").unwrap();

        for (file, reason) in [
            (
                uploaded(Some("apps/app/storage/secret.md")),
                "outside this run's request files",
            ),
            (uploaded(None), "neither a FlowPath nor a URL"),
        ] {
            let error = run_with(&mut context, file).await.unwrap_err().to_string();
            assert!(error.contains("\"report.pdf\""), "{error}");
            assert!(error.contains(reason), "{error}");
            assert!(!context.evaluate_pin::<bool>("exec_out").await.unwrap());
        }

        let error = run_with(&mut context, json!({"size":"big"}))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("not an uploaded file"), "{error}");
    }
}
