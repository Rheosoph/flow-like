use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{anyhow, async_trait, json::json};

use crate::data::path::FlowPath;

/// CopyWorksheetNode
/// ------------------
/// Copies an existing worksheet within the same workbook.
/// Uses `umya-spreadsheet` by cloning the source `Worksheet` and adding it back
/// with a new name. Appends at the end of the sheet list.
///
/// Impure node: modifies a file on disk/storage.
///
/// Inputs
/// - `exec_in` (Execution): trigger.
/// - `file` (Struct<FlowPath>): the XLSX file to modify.
/// - `source_sheet` (String): source worksheet identifier. Accepts name (e.g. "Data")
///   or 0-based index as string (e.g. "0").
/// - `new_name` (String): desired name for the copied sheet. Optional; if empty,
///   defaults to `"{source} (copy)"`.
/// - `if_exists` (Enum String): behavior if `new_name` already exists:
///     - `error` (default)
///     - `rename` (create a unique suffix: `name (1)`, `name (2)`, ...)
///     - `skip` (do nothing)
///
/// Outputs
/// - `exec_out` (Execution): fired when operation completes successfully.
/// - `copied` (Boolean): whether a new sheet was created.
/// - `source_name` (String): resolved source sheet name.
/// - `final_name` (String): the actual name of the new sheet.
#[crate::register_node]
#[derive(Default)]
pub struct CopyWorksheetNode {}

impl CopyWorksheetNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CopyWorksheetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "files_spreadsheet_copy_worksheet",
            "Copy Worksheet",
            "Duplicate a worksheet within the same file",
            "Data/Excel",
        );
        node.add_icon("/flow/icons/file-spreadsheet.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "file",
            "File",
            "The .xlsx file to modify",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "source_sheet",
            "Source Sheet",
            "Name or 0-based index of the source sheet",
            VariableType::String,
        )
        .set_default_value(Some(json!("0")));

        node.add_input_pin(
            "new_name",
            "New Name",
            "Name for the copied sheet (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "if_exists",
            "If Exists",
            "Behavior if the destination name exists",
            VariableType::String,
        )
        .set_default_value(Some(json!("error")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["error".into(), "rename".into(), "skip".into()])
                .build(),
        );

        node.add_output_pin(
            "exec_out",
            "Done",
            "Continue on success",
            VariableType::Execution,
        );
        node.add_output_pin(
            "copied",
            "Copied",
            "Whether a new sheet was created",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "source_name",
            "Source Name",
            "Resolved source sheet name",
            VariableType::String,
        );
        node.add_output_pin(
            "final_name",
            "Final Name",
            "Actual name of the new sheet",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use super::{CachedExcelWorkbook, flush_workbook, get_or_open_workbook};

        context.deactivate_exec_pin("exec_out").await?;

        let file: FlowPath = context.evaluate_pin("file").await?;
        let source_sheet_in: String = context.evaluate_pin("source_sheet").await?;
        let new_name_in: String = context.evaluate_pin("new_name").await.unwrap_or_default();
        let if_exists: String = context
            .evaluate_pin("if_exists")
            .await
            .unwrap_or_else(|_| "error".to_string());

        let cached = get_or_open_workbook(context, &file, false).await?;
        let wb = cached
            .as_any()
            .downcast_ref::<CachedExcelWorkbook>()
            .ok_or_else(|| anyhow!("Cache type mismatch"))?;

        let (source_name, final_name, copied) = {
            let mut book = wb.umya_book_mut()?;

            let (source_idx, source_name) = resolve_sheet_identifier(&book, &source_sheet_in)?;

            let base_name = if new_name_in.trim().is_empty() {
                format!("{} (copy)", source_name)
            } else {
                new_name_in.trim().to_string()
            };
            let mut final_name = sanitize_sheet_name(&base_name);
            if final_name.is_empty() {
                return Err(anyhow!(
                    "Destination sheet name resolves to empty after sanitization"
                ));
            }

            let exists = book.get_sheet_by_name(&final_name).is_some();
            let mut copied = false;
            if exists {
                match if_exists.as_str() {
                    "skip" => {
                        context.log_message(
                            &format!("Sheet '{}' exists. Skipping copy.", final_name),
                            LogLevel::Info,
                        );
                    }
                    "rename" => {
                        final_name = next_unique_sheet_name(&book, &final_name);
                        copy_append(&mut book, source_idx, &final_name)?;
                        copied = true;
                    }
                    _ => {
                        return Err(anyhow!(
                            "Destination sheet '{}' already exists (if_exists=error)",
                            final_name
                        ));
                    }
                }
            } else {
                copy_append(&mut book, source_idx, &final_name)?;
                copied = true;
            }

            (source_name, final_name, copied)
        };

        if copied {
            flush_workbook(wb, &file, context).await?;
        }

        context.set_pin_value("copied", json!(copied)).await?;
        context
            .set_pin_value("source_name", json!(source_name))
            .await?;
        context
            .set_pin_value("final_name", json!(final_name))
            .await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Data processing requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
fn copy_append(
    book: &mut umya_spreadsheet::Spreadsheet,
    source_idx: usize,
    new_name: &str,
) -> flow_like_types::Result<()> {
    let mut clone_sheet = book
        .get_sheet(&source_idx)
        .ok_or_else(|| anyhow!("Source sheet index {} out of range", source_idx))?
        .clone();
    clone_sheet.set_name(new_name);
    book.add_sheet(clone_sheet)
        .map_err(|e| anyhow!("Failed to add cloned sheet: {}", e))?;
    Ok(())
}

#[cfg(feature = "execute")]
fn resolve_sheet_identifier(
    book: &umya_spreadsheet::Spreadsheet,
    ident: &str,
) -> flow_like_types::Result<(usize, String)> {
    let t = ident.trim();
    if t.chars().all(|c| c.is_ascii_digit()) {
        let idx: usize = t
            .parse::<usize>()
            .map_err(|_| anyhow!("Invalid index: {}", t))?;
        let name = book
            .get_sheet(&idx)
            .ok_or_else(|| anyhow!("Sheet index {} out of range", idx))?
            .get_name()
            .to_string();
        return Ok((idx, name));
    }

    if let Some(ws) = book.get_sheet_by_name(t) {
        // Need index too; find it by iterating
        for i in 0..book.get_sheet_count() {
            if let Some(s) = book.get_sheet(&i)
                && s.get_name() == t
            {
                return Ok((i, t.to_string()));
            }
        }
        // Fallback (shouldn't happen)
        return Ok((0, ws.get_name().to_string()));
    }

    Err(anyhow!("Sheet '{}' not found", t))
}

#[cfg(feature = "execute")]
fn sanitize_sheet_name(input: &str) -> String {
    let illegal = [':', '/', '\\', '?', '*', '[', ']'];
    let mut s: String = input
        .chars()
        .map(|c| if illegal.contains(&c) { ' ' } else { c })
        .collect();
    // Remove trailing single quote
    if s.ends_with('\'') {
        s.pop();
    }
    s = s.trim().to_string();
    // Collapse whitespace
    let mut collapsed = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                collapsed.push(' ');
            }
            prev_space = true;
        } else {
            prev_space = false;
            collapsed.push(ch);
        }
    }
    if collapsed.len() > 31 {
        collapsed.truncate(31);
    }
    collapsed
}

#[cfg(feature = "execute")]
fn next_unique_sheet_name(book: &umya_spreadsheet::Spreadsheet, base: &str) -> String {
    let mut n = 1usize;
    loop {
        let candidate = format!("{} ({})", base, n);
        if book.get_sheet_by_name(&candidate).is_none() {
            return candidate;
        }
        n += 1;
        if n > 9999 {
            return format!("{} (copy)", base);
        }
    }
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::{next_unique_sheet_name, sanitize_sheet_name};

    #[test]
    fn test_sanitize() {
        assert_eq!(sanitize_sheet_name("A:B/C\\D?E*F[G]"), "A B C D E F G");
        assert_eq!(sanitize_sheet_name("   Hello   World   "), "Hello World");
        assert_eq!(
            sanitize_sheet_name("012345678901234567890123456789012345"),
            "0123456789012345678901234567890"
        );
    }

    #[test]
    fn test_unique() {
        let mut book = umya_spreadsheet::new_file();
        let _ = book.new_sheet("Sheet1");
        let _ = book.new_sheet("Hello World");
        let name = next_unique_sheet_name(&book, "Hello World");
        assert_eq!(name, "Hello World (1)");
    }
}
