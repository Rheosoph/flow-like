use super::{
    add_exec_out, add_folder_pin, add_scope_pin, add_text_pin,
    client::{LockedAccess, LockedAction, LockedApi},
    locked_node, normalize_folder, read_scope,
    types::{ListLockedItemsRequest, ListLockedItemsResponse, LockedFile, LockedScope},
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{anyhow, async_trait, json::json};

const PAGE_LIMIT: u32 = 500;

#[crate::register_node]
#[derive(Default)]
pub struct LockedListItemsNode;

impl LockedListItemsNode {
    pub fn new() -> Self {
        Self
    }
}

struct ListQuery {
    scope: LockedScope,
    folder: String,
    recursive: bool,
    search: String,
    limit: usize,
}

async fn list_files(
    context: &ExecutionContext,
    api: &LockedApi,
    query: &ListQuery,
) -> flow_like_types::Result<(Vec<LockedFile>, Vec<String>)> {
    let mut files = Vec::new();
    let mut folders: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let remaining = u32::try_from(query.limit - files.len()).unwrap_or(u32::MAX);
        let page: ListLockedItemsResponse = api
            .post(
                context,
                LockedAction::List,
                &ListLockedItemsRequest {
                    scope: query.scope,
                    folder: &query.folder,
                    recursive: query.recursive,
                    search: &query.search,
                    limit: remaining.min(PAGE_LIMIT),
                    cursor: cursor.as_deref(),
                },
            )
            .await?;
        let empty_page = page.items.is_empty();
        files.extend(page.items.iter().map(LockedFile::from));
        for folder in page.folders {
            if !folders.contains(&folder) {
                folders.push(folder);
            }
        }
        match page.next_cursor {
            Some(next)
                if files.len() < query.limit
                    && !empty_page
                    && cursor.as_deref() != Some(next.as_str()) =>
            {
                cursor = Some(next);
            }
            _ => break,
        }
    }
    files.truncate(query.limit);
    Ok((files, folders))
}

#[async_trait]
impl NodeLogic for LockedListItemsNode {
    fn get_node(&self) -> Node {
        let mut node = locked_node(
            "locked_list_items",
            "List Locked Files",
            "Lists the write-once, revision-safe files of this app in a folder, with their newest locked version",
            "list",
            5,
        );
        add_scope_pin(&mut node);
        add_folder_pin(
            &mut node,
            "Folder to list, such as invoices/2026; empty lists the root",
        );
        node.add_input_pin(
            "recursive",
            "Recursive",
            "Also list files in all subfolders",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        add_text_pin(
            &mut node,
            "search",
            "Search",
            "Only files whose name contains this text, ignoring case; empty lists all",
        );
        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum number of files to return",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)))
        .set_options(PinOptions::new().set_range((1.0, 10_000.0)).build());

        add_exec_out(&mut node, "The files were listed");
        node.add_output_pin(
            "files",
            "Locked Files",
            "Files in the folder, ordered by name",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<LockedFile>();
        node.add_output_pin(
            "folders",
            "Folders",
            "Names of the folder's direct subfolders; empty for recursive listings",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let limit = context.evaluate_pin::<i64>("limit").await?;
        let limit = usize::try_from(limit)
            .ok()
            .filter(|limit| *limit > 0)
            .ok_or_else(|| anyhow!("Limit must be at least 1, got {limit}"))?;
        let query = ListQuery {
            scope: read_scope(context).await?,
            folder: normalize_folder(&context.evaluate_pin::<String>("folder").await?),
            recursive: context.evaluate_pin::<bool>("recursive").await?,
            search: context
                .evaluate_pin::<String>("search")
                .await?
                .trim()
                .to_owned(),
            limit,
        };
        let api = LockedApi::bind(context, LockedAccess::Read)?;
        let (files, folders) = list_files(context, &api, &query).await?;
        context.set_pin_value("files", json!(files)).await?;
        context.set_pin_value("folders", json!(folders)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
