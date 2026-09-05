#[cfg(feature = "execute")]
use crate::data::path::FlowPath;
#[cfg(feature = "execute")]
use std::sync::Arc;

pub mod copy_worksheet;
pub mod get_row;
pub mod get_sheet_names;
#[cfg(feature = "execute")]
pub mod grid;
pub mod insert_column;
pub mod insert_row;
pub mod loop_rows;
pub mod new_worksheet;
pub mod read_cell;
pub mod remove_column;
pub mod remove_row;
#[cfg(feature = "execute")]
pub mod sheet_compressor;
#[cfg(feature = "execute")]
pub mod styles;
pub mod table_detect;
pub mod try_extract_tables;
pub mod try_extract_tables_ai;
pub mod write_cell;
pub mod write_cell_html;

pub use flow_like_catalog_data_support::data::excel::{CSVTable, Cell};

/// Parses a 1-based row index from a string.
pub fn parse_row_1_based(s: &str) -> flow_like_types::Result<u32> {
    let trimmed = s.trim();
    let n: u32 = trimmed
        .parse()
        .map_err(|e| flow_like_types::anyhow!("Invalid row '{}': {}", s, e))?;
    if n == 0 {
        return Err(flow_like_types::anyhow!("Row must be 1-based (>=1): {}", s));
    }
    Ok(n)
}

/// Parses a 1-based column index from a string. Accepts letters (A, AA, XFD) or a number.
pub fn parse_col_1_based(s: &str) -> flow_like_types::Result<u32> {
    let trimmed = s.trim();

    // If it's a number, use it directly (1-based)
    if let Ok(n) = trimmed.parse::<u32>() {
        if n == 0 {
            return Err(flow_like_types::anyhow!(
                "Column number must be 1-based (>=1): {}",
                s
            ));
        }
        return Ok(n);
    }

    // Otherwise treat as letters
    let mut acc: u32 = 0;
    for ch in trimmed.to_ascii_uppercase().chars() {
        if !ch.is_ascii_uppercase() {
            return Err(flow_like_types::anyhow!(
                "Invalid column '{}': only letters A-Z or a positive number are allowed",
                s
            ));
        }
        let v = (ch as u32) - ('A' as u32) + 1; // A=1
        acc = acc
            .checked_mul(26)
            .and_then(|x| x.checked_add(v))
            .ok_or_else(|| {
                flow_like_types::anyhow!("Column index overflow for '{}': too large", s)
            })?;
    }

    if acc == 0 {
        return Err(flow_like_types::anyhow!("Column must not be empty"));
    }

    Ok(acc)
}

// -------- Excel Workbook Caching (execute only) --------

/// Cached workbook stored in the execution context.
/// Supports both umya-spreadsheet (read/write, xlsx only) and calamine
/// (read-only fallback for xls, xlsb, ods, etc.).
#[allow(clippy::large_enum_variant)]
// every construction site is Arc::new(..); boxing would only add a second hop on the per-cell read path
#[cfg(feature = "execute")]
pub enum CachedExcelWorkbook {
    Umya {
        book: std::sync::RwLock<umya_spreadsheet::Spreadsheet>,
    },
    Calamine {
        sheets: std::collections::HashMap<String, calamine::Range<calamine::Data>>,
    },
}

#[cfg(feature = "execute")]
impl CachedExcelWorkbook {
    pub fn umya_book(
        &self,
    ) -> flow_like_types::Result<std::sync::RwLockReadGuard<'_, umya_spreadsheet::Spreadsheet>>
    {
        match self {
            CachedExcelWorkbook::Umya { book } => book
                .read()
                .map_err(|e| flow_like_types::anyhow!("Lock poisoned: {}", e)),
            CachedExcelWorkbook::Calamine { .. } => Err(flow_like_types::anyhow!(
                "Write operations require xlsx format"
            )),
        }
    }

    pub fn umya_book_mut(
        &self,
    ) -> flow_like_types::Result<std::sync::RwLockWriteGuard<'_, umya_spreadsheet::Spreadsheet>>
    {
        match self {
            CachedExcelWorkbook::Umya { book } => book
                .write()
                .map_err(|e| flow_like_types::anyhow!("Lock poisoned: {}", e)),
            CachedExcelWorkbook::Calamine { .. } => Err(flow_like_types::anyhow!(
                "Write operations require xlsx format"
            )),
        }
    }
}

#[cfg(feature = "execute")]
impl flow_like_types::Cacheable for CachedExcelWorkbook {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(feature = "execute")]
pub fn excel_cache_key(file: &FlowPath) -> String {
    format!("excel_{}_{}", file.store_ref, file.path)
}

/// Get a cached workbook or open one from storage.
/// Tries umya-spreadsheet first (xlsx, read/write). On failure, falls back to
/// calamine (xls, xlsb, ods — read-only). Both variants are cached.
/// When `create_if_missing` is true an empty xlsx workbook is created if the
/// file does not exist or is empty.
#[cfg(feature = "execute")]
pub async fn get_or_open_workbook(
    ctx: &mut flow_like::flow::execution::context::ExecutionContext,
    file: &FlowPath,
    create_if_missing: bool,
) -> flow_like_types::Result<Arc<dyn flow_like_types::Cacheable>> {
    use calamine::Reader;

    let key = excel_cache_key(file);
    if let Some(cached) = ctx.get_cache(&key).await
        && cached
            .as_any()
            .downcast_ref::<CachedExcelWorkbook>()
            .is_some()
    {
        return Ok(cached);
    }

    let bytes_result = file.get(ctx, false).await;
    let bytes = match bytes_result {
        Ok(ref b) if !b.is_empty() => b.clone(),
        Ok(_) if create_if_missing => {
            let cached: Arc<CachedExcelWorkbook> = Arc::new(CachedExcelWorkbook::Umya {
                book: std::sync::RwLock::new(umya_spreadsheet::new_file()),
            });
            ctx.set_cache(&key, cached.clone()).await;
            return Ok(cached);
        }
        Ok(_) => {
            return Err(flow_like_types::anyhow!(
                "Excel file is empty or could not be read"
            ));
        }
        Err(_) if create_if_missing => {
            let cached: Arc<CachedExcelWorkbook> = Arc::new(CachedExcelWorkbook::Umya {
                book: std::sync::RwLock::new(umya_spreadsheet::new_file()),
            });
            ctx.set_cache(&key, cached.clone()).await;
            return Ok(cached);
        }
        Err(e) => return Err(e),
    };

    // Try umya-spreadsheet first (xlsx, read/write)
    if let Ok(book) =
        umya_spreadsheet::reader::xlsx::read_reader(std::io::Cursor::new(&bytes), true)
    {
        let cached: Arc<CachedExcelWorkbook> = Arc::new(CachedExcelWorkbook::Umya {
            book: std::sync::RwLock::new(book),
        });
        ctx.set_cache(&key, cached.clone()).await;
        return Ok(cached);
    }

    // Calamine fallback (xls, xlsb, ods — read-only)
    let mut wb = calamine::open_workbook_auto_from_rs(std::io::Cursor::new(&bytes))
        .map_err(|e| flow_like_types::anyhow!("Failed to open workbook: {}", e))?;
    let sheet_names = wb.sheet_names().to_vec();
    let mut sheets = std::collections::HashMap::with_capacity(sheet_names.len());
    for name in sheet_names {
        if let Ok(range) = wb.worksheet_range(&name) {
            sheets.insert(name, range);
        }
    }
    let cached: Arc<CachedExcelWorkbook> = Arc::new(CachedExcelWorkbook::Calamine { sheets });
    ctx.set_cache(&key, cached.clone()).await;
    Ok(cached)
}

/// Serialize the cached workbook and write it back to storage.
/// Only works for the Umya (xlsx) variant.
#[cfg(feature = "execute")]
fn serialize_workbook(wb: &CachedExcelWorkbook) -> flow_like_types::Result<Vec<u8>> {
    let CachedExcelWorkbook::Umya { book } = wb else {
        return Err(flow_like_types::anyhow!(
            "Cannot write back non-xlsx (calamine) workbooks"
        ));
    };
    let guard = book
        .read()
        .map_err(|e| flow_like_types::anyhow!("Lock poisoned: {}", e))?;
    let mut out = Vec::new();
    umya_spreadsheet::writer::xlsx::write_writer(&guard, &mut out)
        .map_err(|e| flow_like_types::anyhow!("Failed to serialize workbook: {}", e))?;
    Ok(out)
}

#[cfg(feature = "execute")]
pub async fn flush_workbook(
    wb: &CachedExcelWorkbook,
    file: &FlowPath,
    ctx: &mut flow_like::flow::execution::context::ExecutionContext,
) -> flow_like_types::Result<()> {
    let out = serialize_workbook(wb)?;
    file.put(ctx, out, false).await?;
    Ok(())
}
