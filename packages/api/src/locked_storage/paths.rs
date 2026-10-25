use crate::entity::sea_orm_active_enums::LockedItemScope;
use crate::error::ApiError;

const MAX_SEGMENT_CHARS: usize = 255;
const MAX_FOLDER_CHARS: usize = 1024;
const MAX_FOLDER_DEPTH: usize = 32;

/// `a/b`: no leading, trailing or repeated slashes, no `.`/`..`, no control characters.
pub fn normalize_folder(raw: &str) -> Result<String, ApiError> {
    let segments = raw
        .split('/')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(|segment| validate_segment(segment, "folder"))
        .collect::<Result<Vec<_>, _>>()?;
    if segments.len() > MAX_FOLDER_DEPTH {
        return Err(ApiError::unprocessable(format!(
            "Folders can be at most {MAX_FOLDER_DEPTH} levels deep."
        )));
    }
    let folder = segments.join("/");
    if folder.chars().count() > MAX_FOLDER_CHARS {
        return Err(ApiError::unprocessable(format!(
            "Folder paths can be at most {MAX_FOLDER_CHARS} characters long."
        )));
    }
    Ok(folder)
}

pub fn normalize_name(raw: &str) -> Result<String, ApiError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(ApiError::unprocessable("A locked file needs a name."));
    }
    if name.contains('/') {
        return Err(ApiError::unprocessable(
            "File names cannot contain '/'; put the file into a folder instead.",
        ));
    }
    validate_segment(name, "file name").map(str::to_owned)
}

fn validate_segment<'a>(segment: &'a str, what: &str) -> Result<&'a str, ApiError> {
    if segment == "." || segment == ".." {
        return Err(ApiError::unprocessable(format!(
            "'{segment}' is not a valid {what}."
        )));
    }
    if segment.chars().any(|c| c.is_control() || c == '\\') {
        return Err(ApiError::unprocessable(format!(
            "The {what} '{segment}' contains a control character or a backslash."
        )));
    }
    if segment.chars().count() > MAX_SEGMENT_CHARS {
        return Err(ApiError::unprocessable(format!(
            "A {what} can be at most {MAX_SEGMENT_CHARS} characters long."
        )));
    }
    Ok(segment)
}

/// Unique per app. Case-insensitive so `Invoice.pdf` and `invoice.pdf` cannot
/// become two revision-safe records that look like one file.
pub fn path_key(scope: LockedItemScope, owner: Option<&str>, folder: &str, name: &str) -> String {
    let scope = match scope {
        LockedItemScope::Project => "project",
        LockedItemScope::User => "user",
    };
    let path = if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    };
    format!(
        "{scope}:{}:{}",
        owner.unwrap_or_default(),
        path.to_lowercase()
    )
}

/// Every version gets its own key, so nothing is ever overwritten.
pub fn object_key(
    app_id: &str,
    scope: LockedItemScope,
    owner: Option<&str>,
    item_id: &str,
    version_id: &str,
) -> Result<String, ApiError> {
    crate::credentials::validate_path_component(app_id, "app id")
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok(match (scope, owner) {
        (LockedItemScope::User, Some(owner)) => {
            crate::credentials::validate_path_component(owner, "owner")
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            format!("apps/{app_id}/users/{owner}/{item_id}/{version_id}")
        }
        (LockedItemScope::User, None) => {
            return Err(ApiError::internal("user-scoped locked item without owner"));
        }
        (LockedItemScope::Project, _) => format!("apps/{app_id}/project/{item_id}/{version_id}"),
    })
}

/// Immediate children of `folder` among the stored folder paths.
pub fn child_folders<'a>(folder: &str, folders: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut children: Vec<String> = folders
        .into_iter()
        .filter_map(|candidate| {
            let rest = if folder.is_empty() {
                candidate
            } else {
                candidate.strip_prefix(folder)?.strip_prefix('/')?
            };
            rest.split('/')
                .next()
                .filter(|child| !child.is_empty())
                .map(str::to_owned)
        })
        .collect();
    children.sort();
    children.dedup();
    children
}

pub fn like_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_are_normalized_and_traversal_is_refused() {
        assert_eq!(normalize_folder("").unwrap(), "");
        assert_eq!(
            normalize_folder("/Übersicht//2026/ ").unwrap(),
            "Übersicht/2026"
        );
        assert!(normalize_folder("a/../b").is_err());
        assert!(normalize_folder("a/./b").is_err());
        assert!(normalize_folder("a\\b").is_err());
        assert!(normalize_folder(&"a/".repeat(33)).is_err());
    }

    #[test]
    fn names_keep_unicode_and_reject_separators() {
        assert_eq!(
            normalize_name(" Übersicht (2)#1.pdf ").unwrap(),
            "Übersicht (2)#1.pdf"
        );
        assert!(normalize_name("a/b.pdf").is_err());
        assert!(normalize_name("..").is_err());
        assert!(normalize_name("   ").is_err());
        assert!(normalize_name("a\u{0007}.pdf").is_err());
    }

    #[test]
    fn path_keys_are_case_insensitive_and_scoped() {
        assert_eq!(
            path_key(LockedItemScope::Project, None, "Invoices", "A.pdf"),
            "project::invoices/a.pdf"
        );
        assert_eq!(
            path_key(LockedItemScope::User, Some("u1"), "", "A.pdf"),
            "user:u1:a.pdf"
        );
    }

    #[test]
    fn object_keys_separate_project_and_user_scopes() {
        assert_eq!(
            object_key("app", LockedItemScope::Project, None, "item", "ver").unwrap(),
            "apps/app/project/item/ver"
        );
        assert_eq!(
            object_key("app", LockedItemScope::User, Some("sub|1"), "item", "ver").unwrap(),
            "apps/app/users/sub|1/item/ver"
        );
        assert!(object_key("app", LockedItemScope::User, Some("../x"), "item", "ver").is_err());
        assert!(object_key("app", LockedItemScope::User, None, "item", "ver").is_err());
    }

    #[test]
    fn child_folders_lists_only_the_next_level() {
        let stored = ["a", "a/b", "a/b/c", "a/d", "x", "ab/c"];
        assert_eq!(child_folders("", stored), vec!["a", "ab", "x"]);
        assert_eq!(child_folders("a", stored), vec!["b", "d"]);
        assert_eq!(child_folders("a/b", stored), vec!["c"]);
    }

    #[test]
    fn like_patterns_escape_wildcards() {
        assert_eq!(like_escape("100%_a\\b"), "100\\%\\_a\\\\b");
    }
}
