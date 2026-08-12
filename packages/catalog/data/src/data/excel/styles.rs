//! Lazily-loaded cell style layer (xlsx only, parsed via umya-spreadsheet).
//!
//! calamine does not expose styling, so styles are loaded on demand — only
//! when a styling-based extraction path actually needs them. Colors are
//! classified into a small palette of names ("green", "red", …) so LLM tools
//! can reference them without raw style dumps.

use flow_like_types::{Result, anyhow};
use std::collections::HashMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CellStyle {
    pub bold: bool,
    pub italic: bool,
    /// Fill color as "RRGGBB" (uppercase), None when unfilled/theme-based
    pub fill_rgb: Option<String>,
    /// Font color as "RRGGBB" (uppercase), None when default/theme-based
    pub font_rgb: Option<String>,
}

impl CellStyle {
    pub fn is_default(&self) -> bool {
        !self.bold && !self.italic && self.fill_rgb.is_none() && self.font_rgb.is_none()
    }

    pub fn fill_color_name(&self) -> Option<&'static str> {
        self.fill_rgb.as_deref().and_then(classify_color)
    }

    pub fn font_color_name(&self) -> Option<&'static str> {
        self.font_rgb.as_deref().and_then(classify_color)
    }
}

/// Styles of one worksheet, keyed by absolute 0-based (row, col).
/// Only non-default styled cells are stored.
pub struct SheetStyles {
    map: HashMap<(usize, usize), CellStyle>,
}

/// Parses the workbook once with umya and returns per-sheet style layers.
/// Fails for non-xlsx formats.
pub fn load_workbook_styles(bytes: &[u8]) -> Result<HashMap<String, SheetStyles>> {
    let book = umya_spreadsheet::reader::xlsx::read_reader(std::io::Cursor::new(bytes), true)
        .map_err(|e| anyhow!("Styles are only available for xlsx files: {e}"))?;
    let mut out = HashMap::new();
    for ws in book.get_sheet_collection() {
        let mut map = HashMap::new();
        for cell in ws.get_cell_collection() {
            let style = cell.get_style();
            let mut cs = CellStyle::default();
            if let Some(font) = style.get_font() {
                cs.bold = *font.get_bold();
                cs.italic = *font.get_italic();
                cs.font_rgb = normalize_argb(font.get_color().get_argb());
            }
            if let Some(color) = style.get_background_color() {
                cs.fill_rgb = normalize_argb(color.get_argb());
            }
            if cs.is_default() {
                continue;
            }
            let coord = cell.get_coordinate();
            let r = (*coord.get_row_num() as usize).saturating_sub(1);
            let c = (*coord.get_col_num() as usize).saturating_sub(1);
            map.insert((r, c), cs);
        }
        out.insert(ws.get_name().to_string(), SheetStyles { map });
    }
    Ok(out)
}

impl SheetStyles {
    /// Style layer for a single sheet (parses the whole workbook — prefer
    /// [`load_workbook_styles`] when several sheets are needed).
    pub fn load(bytes: &[u8], sheet_name: &str) -> Result<Self> {
        load_workbook_styles(bytes)?
            .remove(sheet_name)
            .ok_or_else(|| anyhow!("Sheet '{sheet_name}' not found"))
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Style at absolute 0-based sheet coordinates.
    pub fn get_abs(&self, row: usize, col: usize) -> Option<&CellStyle> {
        self.map.get(&(row, col))
    }

    /// All styled cells matching a predicate, sorted row-major.
    pub fn find(&self, pred: impl Fn(&CellStyle) -> bool) -> Vec<(usize, usize, &CellStyle)> {
        let mut out: Vec<_> = self
            .map
            .iter()
            .filter(|(_, s)| pred(s))
            .map(|(&(r, c), s)| (r, c, s))
            .collect();
        out.sort_by_key(|&(r, c, _)| (r, c));
        out
    }

    /// Compact summary of the styling present on the sheet (for LLM prompts):
    /// distinct style signatures with counts and example addresses.
    pub fn summarize(&self, max_signatures: usize) -> String {
        use super::grid::col_to_letters;
        let mut groups: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
        for (&(r, c), s) in &self.map {
            let mut sig_parts: Vec<String> = Vec::new();
            if s.bold {
                sig_parts.push("bold".into());
            }
            if s.italic {
                sig_parts.push("italic".into());
            }
            if let Some(name) = s.fill_color_name() {
                sig_parts.push(format!("fill:{name}"));
            }
            if let Some(name) = s.font_color_name() {
                sig_parts.push(format!("font:{name}"));
            }
            if sig_parts.is_empty() {
                continue;
            }
            groups.entry(sig_parts.join("+")).or_default().push((r, c));
        }
        let mut entries: Vec<(String, Vec<(usize, usize)>)> = groups.into_iter().collect();
        entries.sort_by_key(|(_, cells)| std::cmp::Reverse(cells.len()));
        let mut out = Vec::new();
        for (sig, mut cells) in entries.into_iter().take(max_signatures) {
            cells.sort();
            let examples: Vec<String> = cells
                .iter()
                .take(4)
                .map(|&(r, c)| format!("{}{}", col_to_letters(c), r + 1))
                .collect();
            let suffix = if cells.len() > 4 { ", …" } else { "" };
            out.push(format!(
                "{} ({} cells, e.g. {}{})",
                sig,
                cells.len(),
                examples.join(", "),
                suffix
            ));
        }
        if out.is_empty() {
            "no notable styling".to_string()
        } else {
            out.join("; ")
        }
    }
}

fn normalize_argb(argb: &str) -> Option<String> {
    let s = argb.trim();
    let rgb = match s.len() {
        8 => &s[2..],
        6 => s,
        _ => return None,
    };
    if !rgb.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(rgb.to_ascii_uppercase())
}

/// Classifies an "RRGGBB" color into a small named palette.
pub fn classify_color(rgb: &str) -> Option<&'static str> {
    if rgb.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&rgb[0..2], 16).ok()? as i32;
    let g = u8::from_str_radix(&rgb[2..4], 16).ok()? as i32;
    let b = u8::from_str_radix(&rgb[4..6], 16).ok()? as i32;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    if max > 230 && min > 230 {
        return None; // near-white → not a meaningful highlight
    }
    if max < 45 {
        return Some("black");
    }
    if max - min < 28 {
        return Some("gray");
    }

    // Hue in degrees (0-360)
    let delta = (max - min) as f32;
    let hue = if max == r {
        60.0 * (((g - b) as f32 / delta) % 6.0)
    } else if max == g {
        60.0 * ((b - r) as f32 / delta + 2.0)
    } else {
        60.0 * ((r - g) as f32 / delta + 4.0)
    };
    let hue = if hue < 0.0 { hue + 360.0 } else { hue };

    Some(match hue as u32 {
        0..=15 | 345..=360 => "red",
        16..=40 => "orange",
        41..=70 => "yellow",
        71..=160 => "green",
        161..=200 => "cyan",
        201..=255 => "blue",
        256..=300 => "purple",
        _ => "pink",
    })
}

/// True when a user-supplied color word matches the classified color.
pub fn color_matches(query: &str, classified: Option<&'static str>) -> bool {
    let q = query.trim().to_ascii_lowercase();
    match classified {
        Some(name) => {
            q == name
                || (q == "grey" && name == "gray")
                || (q == "lime" && name == "green")
                || (q == "magenta" && name == "pink")
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_classification() {
        assert_eq!(classify_color("4CAF50"), Some("green"));
        assert_eq!(classify_color("FF0000"), Some("red"));
        assert_eq!(classify_color("FFEB3B"), Some("yellow"));
        assert_eq!(classify_color("2196F3"), Some("blue"));
        assert_eq!(classify_color("FFFFFF"), None);
        assert_eq!(classify_color("808080"), Some("gray"));
        assert_eq!(classify_color("000000"), Some("black"));
    }

    #[test]
    fn argb_normalization() {
        assert_eq!(normalize_argb("FF4CAF50"), Some("4CAF50".to_string()));
        assert_eq!(normalize_argb("4CAF50"), Some("4CAF50".to_string()));
        assert_eq!(normalize_argb(""), None);
        assert_eq!(normalize_argb("theme"), None);
    }

    #[test]
    fn color_match_aliases() {
        assert!(color_matches("green", Some("green")));
        assert!(color_matches("grey", Some("gray")));
        assert!(!color_matches("red", Some("green")));
        assert!(!color_matches("green", None));
    }
}
