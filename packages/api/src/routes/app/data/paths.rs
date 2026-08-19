//! Resolves client-supplied storage prefixes to keys inside the caller's own
//! namespace.
//!
//! Client input is never taken as a key. The authorized base is always
//! prepended, and `object_store` encodes `.`, `..` and `/` inside every segment
//! it is handed, so no input can walk out of the app's namespace.
//!
//! Older clients stored the raw object-store key in widget definitions, and a
//! fork copies those values verbatim, so a key naming *another* app still has to
//! resolve against the caller's own base. Only the exact storage layout is read
//! that way — `apps/<id>/upload[/rest]` and `users/<sub>/apps/<id>[/rest]` —
//! so a folder a user happens to name `apps` stays a plain relative prefix.

use flow_like_storage::Path;

pub fn app_upload_base(app_id: &str) -> Path {
    Path::from("apps").child(app_id).child("upload")
}

pub fn user_upload_base(sub: &str, app_id: &str) -> Path {
    Path::from("users").child(sub).child("apps").child(app_id)
}

pub fn resolve_app_upload(app_id: &str, prefix: &str) -> Path {
    let relative = strip_app_layout(prefix).unwrap_or(prefix);
    append(app_upload_base(app_id), relative)
}

pub fn resolve_user_upload(sub: &str, app_id: &str, prefix: &str) -> Path {
    let relative = strip_user_layout(prefix).unwrap_or(prefix);
    append(user_upload_base(sub, app_id), relative)
}

fn append(base: Path, relative: &str) -> Path {
    relative
        .split('/')
        .filter(|segment| !segment.is_empty())
        .fold(base, |path, segment| path.child(segment))
}

fn strip_app_layout(prefix: &str) -> Option<&str> {
    let rest = prefix.strip_prefix("apps/")?;
    let (_app_id, rest) = rest.split_once('/')?;
    strip_segment(rest, "upload")
}

fn strip_user_layout(prefix: &str) -> Option<&str> {
    let rest = prefix.strip_prefix("users/")?;
    let (_sub, rest) = rest.split_once('/')?;
    let rest = rest.strip_prefix("apps/")?;
    let (_app_id, rest) = rest.split_once('/').unwrap_or((rest, ""));
    Some(rest)
}

fn strip_segment<'a>(rest: &'a str, segment: &str) -> Option<&'a str> {
    match rest.strip_prefix(segment)? {
        "" => Some(""),
        tail => tail.strip_prefix('/'),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "app-1";
    const SUB: &str = "sub-1";

    #[test]
    fn relative_prefixes_land_under_the_app_base() {
        assert_eq!(
            resolve_app_upload(APP, "media/logo.jpg").as_ref(),
            "apps/app-1/upload/media/logo.jpg"
        );
        assert_eq!(resolve_app_upload(APP, "").as_ref(), "apps/app-1/upload");
        assert_eq!(
            resolve_app_upload(APP, "//media//logo.jpg/").as_ref(),
            "apps/app-1/upload/media/logo.jpg"
        );
    }

    #[test]
    fn legacy_object_store_keys_resolve_against_the_callers_own_base() {
        assert_eq!(
            resolve_app_upload(APP, "apps/app-1/upload/media/logo.jpg").as_ref(),
            "apps/app-1/upload/media/logo.jpg"
        );
        // A fork copies widget values verbatim, so a key naming the source app
        // still has to resolve inside the fork.
        assert_eq!(
            resolve_app_upload(APP, "apps/source-app/upload/media/logo.jpg").as_ref(),
            "apps/app-1/upload/media/logo.jpg"
        );
        assert_eq!(
            resolve_app_upload(APP, "apps/app-1/upload").as_ref(),
            "apps/app-1/upload"
        );
    }

    #[test]
    fn folders_named_like_the_layout_stay_relative() {
        assert_eq!(
            resolve_app_upload(APP, "apps/logo.jpg").as_ref(),
            "apps/app-1/upload/apps/logo.jpg"
        );
        assert_eq!(
            resolve_app_upload(APP, "apps/nested/deeper/logo.jpg").as_ref(),
            "apps/app-1/upload/apps/nested/deeper/logo.jpg"
        );
        assert_eq!(
            resolve_app_upload(APP, "apps/x/uploads/logo.jpg").as_ref(),
            "apps/app-1/upload/apps/x/uploads/logo.jpg"
        );
        assert_eq!(
            resolve_user_upload(SUB, APP, "users/notes.txt").as_ref(),
            "users/sub-1/apps/app-1/users/notes.txt"
        );
    }

    #[test]
    fn traversal_attempts_cannot_escape_the_base() {
        assert_eq!(
            resolve_app_upload(APP, "../../secrets.env").as_ref(),
            "apps/app-1/upload/%2E%2E/%2E%2E/secrets.env"
        );
        assert_eq!(
            resolve_app_upload(APP, "apps/other/upload/../../../etc/passwd").as_ref(),
            "apps/app-1/upload/%2E%2E/%2E%2E/%2E%2E/etc/passwd"
        );
        // An encoded delimiter cannot inject a new path segment either.
        assert_eq!(
            resolve_app_upload(APP, "a%2Fb").as_ref(),
            "apps/app-1/upload/a%252Fb"
        );
    }

    #[test]
    fn user_keys_resolve_against_the_callers_own_sub() {
        assert_eq!(
            resolve_user_upload(SUB, APP, "notes/todo.txt").as_ref(),
            "users/sub-1/apps/app-1/notes/todo.txt"
        );
        assert_eq!(
            resolve_user_upload(SUB, APP, "users/sub-1/apps/app-1/notes/todo.txt").as_ref(),
            "users/sub-1/apps/app-1/notes/todo.txt"
        );
        // Another user's key never reaches their namespace — the caller's own
        // sub is re-imposed.
        assert_eq!(
            resolve_user_upload(SUB, APP, "users/sub-2/apps/app-1/notes/todo.txt").as_ref(),
            "users/sub-1/apps/app-1/notes/todo.txt"
        );
    }
}
