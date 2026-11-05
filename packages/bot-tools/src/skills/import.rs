//! Importing a zipped skill folder. The zip is untrusted: on the server this runs in the
//! executor (`skill_import` job), on a desktop in the app. It is refused for too many or too
//! large entries, a compression ratio of a zip bomb, links, encrypted entries, paths that leave
//! the folder and hidden Unicode in any text file. References are written to the workspace under
//! `bot/skills/<name>/`; creating the (pending) skill row stays with the caller.

use std::io::{Cursor, Read};

use flow_like_types::Bytes;
use flow_like_types::bot_skill::{
    ImportedSkill, MAX_COMPRESSION_RATIO, MAX_ENTRY_BYTES, MAX_UNPACKED_BYTES, MAX_ZIP_BYTES,
    MAX_ZIP_ENTRIES, ParsedSkill, SKILL_FILE, hidden_unicode, parse_skill_md, safe_relative_path,
    skill_dir,
};
use schemars::JsonSchema;
use serde::Deserialize;
use zip::ZipArchive;

use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult, code};
use crate::outcome::ToolOutput;

/// Entries below this size are never judged by their compression ratio.
const RATIO_FLOOR_BYTES: u64 = 100 * 1024;
const SKIPPED_NAMES: [&str; 2] = [".DS_Store", "Thumbs.db"];

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SkillImportArgs {
    /// Workspace path of the uploaded zip.
    pub path: String,
}

/// A validated skill folder: its `SKILL.md` and the files that come with it.
#[derive(Debug)]
pub struct Unpacked {
    pub skill: ParsedSkill,
    /// Reference files by path relative to the skill folder, `SKILL.md` included.
    pub files: Vec<(String, Bytes)>,
    pub skipped: Vec<String>,
}

fn refuse(message: impl Into<String>) -> String {
    message.into()
}

fn is_link(mode: Option<u32>) -> bool {
    mode.is_some_and(|mode| mode & 0o170000 == 0o120000)
}

fn ignorable(path: &str) -> bool {
    path.starts_with("__MACOSX/")
        || path
            .rsplit('/')
            .next()
            .is_some_and(|name| SKIPPED_NAMES.contains(&name))
}

/// The folder the single `SKILL.md` sits in: `""` for the zip root, else `"<dir>/"`.
fn skill_root(names: &[String]) -> Result<String, String> {
    let roots: Vec<&str> = names
        .iter()
        .filter(|name| !ignorable(name))
        .filter_map(|name| {
            let parent = name.strip_suffix(SKILL_FILE)?;
            let depth = parent.matches('/').count();
            (depth == 0 || (depth == 1 && parent.ends_with('/'))).then_some(parent)
        })
        .collect();
    match roots.as_slice() {
        [] => Err(refuse(
            "The zip has no SKILL.md at its top level or in its single top-level folder",
        )),
        [root] => Ok((*root).to_string()),
        _ => Err(refuse("The zip holds more than one skill; import one at a time")),
    }
}

/// Reads and validates a zip.
pub fn unpack(zip_bytes: &[u8]) -> Result<Unpacked, String> {
    if zip_bytes.len() as u64 > MAX_ZIP_BYTES {
        return Err(refuse(format!(
            "The zip has {} bytes; skills are at most {MAX_ZIP_BYTES}",
            zip_bytes.len()
        )));
    }
    let mut archive = ZipArchive::new(Cursor::new(zip_bytes))
        .map_err(|error| refuse(format!("This is not a readable zip: {error}")))?;
    if archive.len() > MAX_ZIP_ENTRIES {
        return Err(refuse(format!(
            "The zip has {} entries; skills have at most {MAX_ZIP_ENTRIES}",
            archive.len()
        )));
    }
    let mut names = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let entry = archive
            .by_index_raw(index)
            .map_err(|error| refuse(format!("Cannot read zip entry {index}: {error}")))?;
        let raw = entry
            .name()
            .map_err(|error| refuse(format!("Zip entry {index} has an unreadable name: {error}")))?
            .into_owned();
        let name = raw.replace('\\', "/");
        if name.starts_with('/') || name.split('/').any(|part| part == "..") || name.contains(':')
        {
            return Err(refuse(format!(
                "The zip entry '{raw}' leaves the skill folder"
            )));
        }
        names.push(name);
    }
    let root = skill_root(&names)?;

    let mut files: Vec<(String, Bytes)> = Vec::new();
    let mut skipped = Vec::new();
    let mut unpacked_total = 0u64;
    for index in 0..archive.len() {
        let (declared, compressed, is_dir, mode, encrypted) = {
            let entry = archive
                .by_index_raw(index)
                .map_err(|error| refuse(format!("Cannot read zip entry {index}: {error}")))?;
            (
                entry.size(),
                entry.compressed_size(),
                entry.is_dir(),
                entry.unix_mode(),
                entry.encrypted(),
            )
        };
        let name = &names[index];
        if is_dir || ignorable(name) {
            continue;
        }
        let Some(relative) = name.strip_prefix(root.as_str()) else {
            skipped.push(name.clone());
            continue;
        };
        if is_link(mode) {
            return Err(refuse(format!("The zip entry '{name}' is a link")));
        }
        if encrypted {
            return Err(refuse(format!("The zip entry '{name}' is encrypted")));
        }
        let relative = safe_relative_path(relative).map_err(refuse)?;
        if declared > MAX_ENTRY_BYTES {
            return Err(refuse(format!(
                "The zip entry '{name}' has {declared} bytes; the limit is {MAX_ENTRY_BYTES}"
            )));
        }
        if declared >= RATIO_FLOOR_BYTES && declared / compressed.max(1) > MAX_COMPRESSION_RATIO {
            return Err(refuse(format!(
                "The zip entry '{name}' unpacks {}x larger than it is stored; it looks like a zip bomb",
                declared / compressed.max(1)
            )));
        }
        let mut entry = archive
            .by_index(index)
            .map_err(|error| refuse(format!("Cannot unpack '{name}': {error}")))?;
        let mut content = Vec::new();
        (&mut entry)
            .take(MAX_ENTRY_BYTES + 1)
            .read_to_end(&mut content)
            .map_err(|error| refuse(format!("Cannot unpack '{name}': {error}")))?;
        if content.len() as u64 > MAX_ENTRY_BYTES {
            return Err(refuse(format!(
                "The zip entry '{name}' unpacks past {MAX_ENTRY_BYTES} bytes"
            )));
        }
        unpacked_total += content.len() as u64;
        if unpacked_total > MAX_UNPACKED_BYTES {
            return Err(refuse(format!(
                "The zip unpacks past {MAX_UNPACKED_BYTES} bytes in all"
            )));
        }
        match std::str::from_utf8(&content) {
            Ok(text) => {
                if let Some(c) = hidden_unicode(text) {
                    return Err(refuse(format!(
                        "'{name}' contains the hidden character U+{:04X}; skills may not hide text",
                        c as u32
                    )));
                }
            }
            Err(_) => {
                skipped.push(name.clone());
                continue;
            }
        }
        files.push((relative, Bytes::from(content)));
    }
    let skill_md = files
        .iter()
        .find(|(path, _)| path == SKILL_FILE)
        .ok_or_else(|| refuse("SKILL.md is not valid text"))?;
    let skill = parse_skill_md(std::str::from_utf8(&skill_md.1).unwrap_or_default())?;
    Ok(Unpacked {
        skill,
        files,
        skipped,
    })
}

/// Reads the zip at `args.path` in the workspace, validates it and writes the skill folder to
/// `bot/skills/<name>/`. Returns the validated fields; the host creates the pending skill.
pub async fn run_import(env: &dyn ToolEnv, args: SkillImportArgs) -> ToolResult<ToolOutput> {
    let workspace = env
        .scope()
        .workspace
        .clone()
        .ok_or_else(|| ToolError::no_workspace("import skills"))?;
    let data = env
        .read_file(&workspace, args.path.trim(), MAX_ZIP_BYTES + 1)
        .await?
        .ok_or_else(|| ToolError::not_found(format!("No file '{}' in the workspace", args.path)))?;
    if data.size > MAX_ZIP_BYTES {
        return Err(ToolError::new(
            code::TOO_LARGE,
            format!("The zip has {} bytes; skills are at most {MAX_ZIP_BYTES}", data.size),
        ));
    }
    let unpacked = unpack(&data.bytes).map_err(ToolError::invalid)?;
    let folder = skill_dir(&unpacked.skill.name);
    let mut written = Vec::with_capacity(unpacked.files.len());
    for (relative, content) in unpacked.files {
        let path = format!("{folder}/{relative}");
        env.write_workspace(&path, content, &flow_like_bot_mime(&relative))
            .await?;
        written.push(path);
    }
    let imported = ImportedSkill {
        name: unpacked.skill.name,
        description: unpacked.skill.description,
        body: unpacked.skill.body,
        files: written,
        skipped: unpacked.skipped,
    };
    Ok(ToolOutput::new(
        serde_json::to_value(&imported).map_err(|error| ToolError::new(code::FAILED, error.to_string()))?,
    ))
}

fn flow_like_bot_mime(path: &str) -> String {
    crate::files::guess_mime(path)
}

#[cfg(test)]
mod tests;
