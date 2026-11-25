#[cfg(feature = "execute")]
use super::git;
use super::repository;
#[cfg(feature = "execute")]
use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_types::Value;
use flow_like_types::{JsonSchema, async_trait, json::json};
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use std::path::{Component, Path};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LocalGitFileStatus {
    pub path: String,
    pub original_path: Option<String>,
    pub index_status: String,
    pub worktree_status: String,
    pub untracked: bool,
    pub conflicted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LocalGitStatus {
    pub branch: String,
    pub commit: String,
    pub clean: bool,
    pub detached: bool,
    pub files: Vec<LocalGitFileStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LocalGitCommit {
    pub sha: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: String,
    pub subject: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LocalGitBranch {
    pub name: String,
    pub commit: String,
    pub current: bool,
    pub remote: bool,
    pub upstream: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LocalGitStash {
    pub reference: String,
    pub commit: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LocalGitRemote {
    pub name: String,
    pub fetch_urls: Vec<String>,
    pub push_urls: Vec<String>,
}

fn string_input(node: &mut Node, name: &str, title: &str, description: &str, default: &str) {
    node.add_input_pin(name, title, description, VariableType::String)
        .set_default_value(Some(json!(default)));
}

fn bool_input(node: &mut Node, name: &str, title: &str, description: &str, default: bool) {
    node.add_input_pin(name, title, description, VariableType::Boolean)
        .set_default_value(Some(json!(default)));
}

fn integer_input(node: &mut Node, name: &str, title: &str, description: &str, default: i64) {
    node.add_input_pin(name, title, description, VariableType::Integer)
        .set_default_value(Some(json!(default)));
}

fn paths_input(node: &mut Node) {
    node.add_input_pin(
        "paths",
        "Paths",
        "Literal paths relative to the repository root. Wildcards are not expanded.",
        VariableType::String,
    )
    .set_value_type(ValueType::Array)
    .set_default_value(Some(json!([])));
}

fn count_output(node: &mut Node) {
    node.add_output_pin(
        "count",
        "Count",
        "Number of entries returned",
        VariableType::Integer,
    );
}

#[cfg(feature = "execute")]
struct LocalResult {
    output: String,
    values: Vec<(&'static str, Value)>,
}

#[cfg(feature = "execute")]
impl LocalResult {
    fn output(output: String) -> Self {
        Self {
            output,
            values: Vec::new(),
        }
    }

    fn value(mut self, name: &'static str, value: Value) -> Self {
        self.values.push((name, value));
        self
    }
}

#[cfg(feature = "execute")]
async fn begin_local(
    context: &mut ExecutionContext,
    defaults: &[(&str, Value)],
) -> flow_like_types::Result<()> {
    repository::begin(context).await?;
    for (name, value) in defaults {
        context.set_pin_value(name, value.clone()).await?;
    }
    Ok(())
}

#[cfg(feature = "execute")]
async fn execute_local<F>(
    context: &mut ExecutionContext,
    repository_path: &FlowPath,
    operation: F,
) -> flow_like_types::Result<()>
where
    F: FnOnce(&Path) -> flow_like_types::Result<LocalResult> + Send + 'static,
{
    let result = async {
        let workspace = repository::Workspace::open(context, repository_path).await?;
        let (workspace, result) = flow_like_types::tokio::task::spawn_blocking(move || {
            let result =
                git::ensure_repository(workspace.path()).and_then(|()| operation(workspace.path()));
            (workspace, result)
        })
        .await
        .map_err(|error| flow_like_types::anyhow!("Git operation task failed: {error}"))?;
        workspace.finish(result).await
    }
    .await;
    let result = match result {
        Ok(result) => {
            for (name, value) in result.values {
                context.set_pin_value(name, value).await?;
            }
            Ok(result.output)
        }
        Err(error) => Err(error),
    };
    repository::finish(context, result, repository_path).await
}

#[cfg(feature = "execute")]
fn branch_name(_path: &Path, branch: &str) -> flow_like_types::Result<()> {
    // Reject checkout shorthand such as @{-1}; branch names must be literal.
    if branch.contains('@') && branch.contains('{') {
        return Err(flow_like_types::anyhow!("Use a literal branch name"));
    }
    git::validate_branch_name(branch)
}

#[cfg(feature = "execute")]
fn commit_ref(path: &Path, revision: &str) -> flow_like_types::Result<String> {
    let repo = git::open(path)?;
    Ok(git::resolve_commit(&repo, revision)?.to_string())
}

#[cfg(feature = "execute")]
fn validate_paths(paths: &[String]) -> flow_like_types::Result<()> {
    for path in paths {
        if path.is_empty()
            || path.contains('\0')
            || Path::new(path).components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(flow_like_types::anyhow!(
                "File paths must be nonempty paths inside the repository: {path:?}"
            ));
        }
    }
    Ok(())
}

/// Literal repository-relative path specs; an empty spec selects the whole working tree.
#[cfg(feature = "execute")]
fn literal_specs(paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .map(|path| {
            Path::new(path)
                .components()
                .filter(|component| !matches!(component, Component::CurDir))
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect()
}

#[cfg(feature = "execute")]
fn spec_matches(entry: &[u8], spec: &str) -> bool {
    let spec = spec.as_bytes();
    spec.is_empty()
        || entry == spec
        || (entry.len() > spec.len() && entry.starts_with(spec) && entry[spec.len()] == b'/')
}

#[cfg(feature = "execute")]
fn entry_path(entry: &[u8]) -> std::path::PathBuf {
    std::path::PathBuf::from(String::from_utf8_lossy(entry).into_owned())
}

#[cfg(feature = "execute")]
fn conflict_codes(repo: &git2::Repository) -> flow_like_types::Result<Vec<(Vec<u8>, char, char)>> {
    let mut codes = Vec::new();
    for conflict in repo.index()?.conflicts()? {
        let conflict = conflict?;
        let path = [&conflict.our, &conflict.their, &conflict.ancestor]
            .into_iter()
            .flatten()
            .map(|entry| entry.path.clone())
            .next()
            .unwrap_or_default();
        // The two-letter codes `git status --porcelain` uses for unmerged paths.
        let (index, worktree) = match (
            conflict.ancestor.is_some(),
            conflict.our.is_some(),
            conflict.their.is_some(),
        ) {
            (false, true, true) => ('A', 'A'),
            (true, true, false) => ('U', 'D'),
            (true, false, true) => ('D', 'U'),
            (false, true, false) => ('A', 'U'),
            (false, false, true) => ('U', 'A'),
            (true, false, false) => ('D', 'D'),
            _ => ('U', 'U'),
        };
        codes.push((path, index, worktree));
    }
    Ok(codes)
}

#[cfg(feature = "execute")]
fn status(path: &Path) -> flow_like_types::Result<LocalGitStatus> {
    use git2::Status;
    let repo = git::open(path)?;
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .renames_head_to_index(true);
    let statuses = repo.statuses(Some(&mut options))?;
    let conflicts = conflict_codes(&repo)?;
    let mut files = Vec::new();
    for entry in statuses.iter() {
        let flags = entry.status();
        // libgit2 reports a staged rename under its old path.
        let bytes = if flags.contains(Status::INDEX_RENAMED) {
            entry
                .head_to_index()
                .and_then(|delta| delta.new_file().path_bytes())
                .unwrap_or_else(|| entry.path_bytes())
        } else {
            entry.path_bytes()
        };
        let (index, worktree) = if flags.contains(Status::CONFLICTED) {
            conflicts
                .iter()
                .find(|(path, _, _)| path == bytes)
                .map_or(('U', 'U'), |(_, index, worktree)| (*index, *worktree))
        } else if flags == Status::WT_NEW {
            ('?', '?')
        } else {
            let index = [
                (Status::INDEX_NEW, 'A'),
                (Status::INDEX_MODIFIED, 'M'),
                (Status::INDEX_DELETED, 'D'),
                (Status::INDEX_RENAMED, 'R'),
                (Status::INDEX_TYPECHANGE, 'T'),
            ]
            .into_iter()
            .find(|(flag, _)| flags.contains(*flag))
            .map_or(' ', |(_, letter)| letter);
            let worktree = [
                (Status::WT_MODIFIED, 'M'),
                (Status::WT_DELETED, 'D'),
                (Status::WT_RENAMED, 'R'),
                (Status::WT_TYPECHANGE, 'T'),
            ]
            .into_iter()
            .find(|(flag, _)| flags.contains(*flag))
            .map_or(' ', |(_, letter)| letter);
            (index, worktree)
        };
        let original_path = if flags.contains(Status::INDEX_RENAMED) {
            entry
                .head_to_index()
                .and_then(|delta| delta.old_file().path_bytes())
                .map(|old| String::from_utf8_lossy(old).into_owned())
        } else {
            None
        };
        let also_untracked = flags.contains(Status::WT_NEW)
            && flags.intersects(
                Status::INDEX_NEW
                    | Status::INDEX_MODIFIED
                    | Status::INDEX_DELETED
                    | Status::INDEX_RENAMED
                    | Status::INDEX_TYPECHANGE,
            );
        if also_untracked {
            files.push(LocalGitFileStatus {
                path: String::from_utf8_lossy(bytes).into_owned(),
                original_path: None,
                index_status: "?".into(),
                worktree_status: "?".into(),
                untracked: true,
                conflicted: false,
            });
        }
        files.push(LocalGitFileStatus {
            path: String::from_utf8_lossy(bytes).into_owned(),
            original_path,
            index_status: index.to_string(),
            worktree_status: worktree.to_string(),
            untracked: index == '?' && worktree == '?',
            conflicted: flags.contains(Status::CONFLICTED)
                || index == 'U'
                || worktree == 'U'
                || matches!((index, worktree), ('A', 'A') | ('D', 'D')),
        });
    }
    let detached = repo.head_detached().unwrap_or(false);
    let branch = if detached {
        String::new()
    } else {
        git::head_branch(&repo).unwrap_or_default()
    };
    let commit = repo
        .head()
        .ok()
        .and_then(|head| head.target())
        .map(|oid| oid.to_string())
        .unwrap_or_default();
    Ok(LocalGitStatus {
        detached: branch.is_empty() && !commit.is_empty(),
        branch,
        commit,
        clean: files.is_empty(),
        files,
    })
}

#[cfg(feature = "execute")]
fn diff(
    path: &Path,
    staged: bool,
    revision: &str,
    paths: &[String],
) -> flow_like_types::Result<String> {
    validate_paths(paths)?;
    let repo = git::open(path)?;
    if let Some(workdir) = repo.workdir() {
        reject_symlinked_parents(workdir, &literal_specs(paths))?;
    }
    let mut options = git2::DiffOptions::new();
    // Paths are literal; external diff drivers and text conversion do not exist in libgit2.
    options.disable_pathspec_match(true).context_lines(3);
    for path in literal_specs(paths) {
        options.pathspec(path);
    }
    let tree = if revision.is_empty() {
        None
    } else {
        Some(
            repo.find_commit(git::resolve_commit(&repo, revision)?)?
                .tree()?,
        )
    };
    let mut diff = if staged {
        let base = tree.or_else(|| repo.head().ok().and_then(|head| head.peel_to_tree().ok()));
        repo.diff_tree_to_index(base.as_ref(), None, Some(&mut options))?
    } else if let Some(tree) = tree {
        repo.diff_tree_to_workdir_with_index(Some(&tree), Some(&mut options))?
    } else {
        repo.diff_index_to_workdir(None, Some(&mut options))?
    };
    diff.find_similar(None)?;
    let mut patch = Vec::new();
    diff.print(git2::DiffFormat::Patch, |_, _, line| {
        if matches!(line.origin(), '+' | '-' | ' ') {
            patch.push(line.origin() as u8);
        }
        patch.extend_from_slice(line.content());
        true
    })?;
    Ok(git::redact(&String::from_utf8_lossy(&patch)))
}

/// ISO 8601 with the author's UTC offset, like `git log --format=%aI`.
#[cfg(feature = "execute")]
fn iso8601(time: git2::Time) -> String {
    let offset = i64::from(time.offset_minutes());
    let local = time.seconds() + offset * 60;
    let (days, seconds) = (local.div_euclid(86_400), local.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let sign = if offset < 0 { '-' } else { '+' };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}{sign}{:02}:{:02}",
        seconds / 3_600,
        seconds % 3_600 / 60,
        seconds % 60,
        offset.abs() / 60,
        offset.abs() % 60
    )
}

#[cfg(feature = "execute")]
fn log(
    path: &Path,
    revision: &str,
    max_count: i64,
) -> flow_like_types::Result<Vec<LocalGitCommit>> {
    if !(1..=10000).contains(&max_count) {
        return Err(flow_like_types::anyhow!(
            "Max Count must be between 1 and 10000"
        ));
    }
    let repo = git::open(path)?;
    let start = git::resolve_commit(&repo, revision)?;
    let mut walk = repo.revwalk()?;
    walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;
    walk.push(start)?;
    let mut commits = Vec::new();
    for id in walk.take(max_count as usize) {
        let commit = repo.find_commit(id?)?;
        let author = commit.author();
        commits.push(LocalGitCommit {
            sha: commit.id().to_string(),
            parents: commit.parent_ids().map(|id| id.to_string()).collect(),
            author_name: String::from_utf8_lossy(author.name_bytes()).into_owned(),
            author_email: String::from_utf8_lossy(author.email_bytes()).into_owned(),
            authored_at: iso8601(author.when()),
            subject: String::from_utf8_lossy(commit.summary_bytes().unwrap_or_default())
                .into_owned(),
        });
    }
    Ok(commits)
}

#[cfg(feature = "execute")]
fn branches(path: &Path, include_remote: bool) -> flow_like_types::Result<Vec<LocalGitBranch>> {
    let repo = git::open(path)?;
    let mut result = Vec::new();
    let kinds: &[git2::BranchType] = if include_remote {
        &[git2::BranchType::Local, git2::BranchType::Remote]
    } else {
        &[git2::BranchType::Local]
    };
    for kind in kinds {
        let remote = *kind == git2::BranchType::Remote;
        let mut part = Vec::new();
        for entry in repo.branches(Some(*kind))? {
            let (branch, _) = entry?;
            let Some(name) = branch.name()?.map(str::to_owned) else {
                continue;
            };
            // Dangling symbolic refs such as origin/HEAD of an empty remote have no commit.
            let Some(commit) = branch
                .get()
                .resolve()
                .ok()
                .and_then(|reference| reference.target())
            else {
                continue;
            };
            let upstream = if remote {
                String::new()
            } else {
                branch
                    .upstream()
                    .ok()
                    .and_then(|upstream| upstream.name().ok().flatten().map(str::to_owned))
                    .unwrap_or_default()
            };
            part.push(LocalGitBranch {
                name,
                commit: commit.to_string(),
                current: !remote && branch.is_head(),
                remote,
                upstream,
            });
        }
        part.sort_by(|a, b| a.name.cmp(&b.name));
        result.extend(part);
    }
    Ok(result)
}

#[cfg(feature = "execute")]
fn switch_branch(path: &Path, branch: &str, clean: bool) -> flow_like_types::Result<String> {
    branch_name(path, branch)?;
    if clean {
        git::require_clean(path)?;
    }
    let repo = git::open(path)?;
    let target = repo
        .find_branch(branch, git2::BranchType::Local)
        .map_err(|_| flow_like_types::anyhow!("invalid reference: {branch}"))?;
    let reference = target
        .get()
        .name()
        .ok_or_else(|| flow_like_types::anyhow!("The branch name is not UTF-8"))?
        .to_owned();
    git::checkout_commit(&repo, target.get().peel_to_commit()?.id())?;
    repo.set_head(&reference)?;
    Ok(format!("Switched to branch '{branch}'\n"))
}

#[cfg(feature = "execute")]
fn checkout_revision(path: &Path, revision: &str, clean: bool) -> flow_like_types::Result<String> {
    let revision = commit_ref(path, revision)?;
    if clean {
        git::require_clean(path)?;
    }
    let repo = git::open(path)?;
    let oid = git2::Oid::from_str(&revision)?;
    git::checkout_commit(&repo, oid)?;
    repo.set_head_detached(oid)?;
    let commit = repo.find_commit(oid)?;
    Ok(format!(
        "HEAD is now at {} {}\n",
        git::short(oid),
        String::from_utf8_lossy(commit.summary_bytes().unwrap_or_default())
    ))
}

#[cfg(feature = "execute")]
fn create_branch(
    path: &Path,
    branch: &str,
    start_point: &str,
    switch: bool,
) -> flow_like_types::Result<String> {
    branch_name(path, branch)?;
    let revision = commit_ref(path, start_point)?;
    if switch {
        git::require_clean(path)?;
    }
    let repo = git::open(path)?;
    let oid = git2::Oid::from_str(&revision)?;
    let created = repo.branch(branch, &repo.find_commit(oid)?, false)?;
    if !switch {
        return Ok(format!(
            "Created branch '{branch}' at {}\n",
            git::short(oid)
        ));
    }
    let reference = created
        .get()
        .name()
        .ok_or_else(|| flow_like_types::anyhow!("The branch name is not UTF-8"))?
        .to_owned();
    git::checkout_commit(&repo, oid)?;
    repo.set_head(&reference)?;
    Ok(format!("Switched to a new branch '{branch}'\n"))
}

#[cfg(feature = "execute")]
fn delete_branch(path: &Path, branch: &str, force: bool) -> flow_like_types::Result<String> {
    branch_name(path, branch)?;
    let repo = git::open(path)?;
    let mut target = repo
        .find_branch(branch, git2::BranchType::Local)
        .map_err(|_| flow_like_types::anyhow!("branch '{branch}' not found"))?;
    if target.is_head() {
        return Err(flow_like_types::anyhow!(
            "Cannot delete branch '{branch}' checked out in this working tree"
        ));
    }
    let oid = target.get().peel_to_commit()?.id();
    if !force {
        // Like `git branch -d`: the work must be contained in the upstream, or in HEAD without one.
        let base = match target.upstream() {
            Ok(upstream) => upstream.get().peel_to_commit()?.id(),
            Err(_) => repo.head()?.peel_to_commit()?.id(),
        };
        if !git::is_ancestor(&repo, oid, base)? {
            return Err(flow_like_types::anyhow!(
                "The branch '{branch}' is not fully merged. Use Force to delete it anyway."
            ));
        }
    }
    target.delete()?;
    Ok(format!(
        "Deleted branch {branch} (was {}).\n",
        git::short(oid)
    ))
}

/// Refuses a repository path whose parent directories include a symlink: libgit2 would read through
/// it and index files from outside the checkout (the git CLI reports "beyond a symbolic link").
#[cfg(feature = "execute")]
fn reject_symlinked_parents(workdir: &Path, specs: &[String]) -> flow_like_types::Result<()> {
    for spec in specs {
        let mut current = workdir.to_path_buf();
        let mut components = Path::new(spec).components().peekable();
        while let Some(component) = components.next() {
            if components.peek().is_none() {
                break;
            }
            current.push(component);
            if current
                .symlink_metadata()
                .is_ok_and(|meta| meta.file_type().is_symlink())
            {
                return Err(flow_like_types::anyhow!(
                    "pathspec '{spec}' is beyond a symbolic link"
                ));
            }
        }
    }
    Ok(())
}

/// Removes symlinks (never following them) that sit where the target tree has a directory, so a
/// force checkout cannot rmdir or write through them.
#[cfg(feature = "execute")]
fn clear_obstructing_symlinks(
    repo: &git2::Repository,
    tree: &git2::Tree,
) -> flow_like_types::Result<()> {
    let Some(workdir) = repo.workdir() else {
        return Ok(());
    };
    let mut dirs = Vec::new();
    tree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
        if entry.kind() == Some(git2::ObjectType::Tree)
            && let Some(name) = entry.name()
        {
            dirs.push(format!("{root}{name}"));
        }
        git2::TreeWalkResult::Ok
    })?;
    for dir in dirs {
        let full = workdir.join(&dir);
        if full
            .symlink_metadata()
            .is_ok_and(|meta| meta.file_type().is_symlink())
        {
            std::fs::remove_file(&full)?;
        }
    }
    Ok(())
}

/// Adds a file, symlink, or every non-ignored file below a directory to the index.
#[cfg(feature = "execute")]
fn add_literal(
    repo: &git2::Repository,
    index: &mut git2::Index,
    workdir: &Path,
    target: &Path,
) -> flow_like_types::Result<()> {
    let Ok(metadata) = target.symlink_metadata() else {
        return Ok(());
    };
    let relative = target.strip_prefix(workdir).unwrap_or(target);
    if metadata.is_dir() {
        if relative.file_name().is_some_and(|name| name == ".git") {
            return Ok(());
        }
        // Nested repositories are not recursed into.
        if !relative.as_os_str().is_empty() && target.join(".git").exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(target)? {
            add_literal(repo, index, workdir, &entry?.path())?;
        }
        return Ok(());
    }
    let tracked = index.get_path(relative, 0).is_some();
    if !tracked && repo.status_should_ignore(relative)? {
        return Ok(());
    }
    index.add_path(relative)?;
    Ok(())
}

#[cfg(feature = "execute")]
fn stage(path: &Path, paths: &[String], all: bool) -> flow_like_types::Result<String> {
    validate_paths(paths)?;
    if all && !paths.is_empty() {
        return Err(flow_like_types::anyhow!(
            "Choose All or provide Paths, not both"
        ));
    }
    let repo = git::open(path)?;
    let mut index = repo.index()?;
    if all {
        // An empty pathspec selects everything: additions, modifications and deletions.
        index.add_all(
            std::iter::empty::<&str>(),
            git2::IndexAddOption::DEFAULT,
            None,
        )?;
        index.update_all(std::iter::empty::<&str>(), None)?;
        index.write()?;
        return Ok("Staged all changes.\n".into());
    }
    if paths.is_empty() {
        return Err(flow_like_types::anyhow!("Provide Paths or enable All"));
    }
    let workdir = repo
        .workdir()
        .ok_or_else(|| flow_like_types::anyhow!("This operation requires a working tree"))?
        .to_path_buf();
    let specs = literal_specs(paths);
    reject_symlinked_parents(&workdir, &specs)?;
    for (spec, original) in specs.iter().zip(paths) {
        let known = workdir.join(spec).symlink_metadata().is_ok()
            || index.iter().any(|entry| spec_matches(&entry.path, spec));
        if !known {
            return Err(flow_like_types::anyhow!(
                "pathspec '{original}' did not match any files"
            ));
        }
        let literal = workdir.join(spec);
        if literal.symlink_metadata().is_ok_and(|meta| !meta.is_dir())
            && index.get_path(Path::new(spec), 0).is_none()
            && repo.status_should_ignore(Path::new(spec))?
        {
            return Err(flow_like_types::anyhow!(
                "The following paths are ignored by one of your .gitignore files: {original}"
            ));
        }
    }
    // Literal matching: `[` and `*` in a file name are not wildcards, so the files are walked here.
    for spec in &specs {
        add_literal(&repo, &mut index, &workdir, &workdir.join(spec))?;
    }
    let removed: Vec<Vec<u8>> = index
        .iter()
        .filter(|entry| {
            specs.iter().any(|spec| spec_matches(&entry.path, spec))
                && workdir
                    .join(entry_path(&entry.path))
                    .symlink_metadata()
                    .is_err()
        })
        .map(|entry| entry.path)
        .collect();
    for entry in removed {
        index.remove_path(&entry_path(&entry))?;
    }
    index.write()?;
    Ok(format!("Staged {} path(s).\n", paths.len()))
}

#[cfg(feature = "execute")]
fn unstage(path: &Path, paths: &[String], all: bool) -> flow_like_types::Result<String> {
    validate_paths(paths)?;
    if all && !paths.is_empty() {
        return Err(flow_like_types::anyhow!(
            "Choose All or provide Paths, not both"
        ));
    }
    if !all && paths.is_empty() {
        return Err(flow_like_types::anyhow!("Provide Paths or enable All"));
    }
    let repo = git::open(path)?;
    if let Some(workdir) = repo.workdir() {
        reject_symlinked_parents(workdir, &literal_specs(paths))?;
    }
    let mut index = repo.index()?;
    // An unborn branch has no HEAD tree to restore into the index.
    let head_tree = repo.head().ok().and_then(|head| head.peel_to_tree().ok());
    if all {
        match &head_tree {
            Some(tree) => index.read_tree(tree)?,
            None => index.clear()?,
        }
        index.write()?;
        return Ok("Unstaged all changes.\n".into());
    }
    let mut head_index = git2::Index::new()?;
    if let Some(tree) = &head_tree {
        head_index.read_tree(tree)?;
    }
    for spec in literal_specs(paths) {
        let current: Vec<Vec<u8>> = index
            .iter()
            .filter(|entry| spec_matches(&entry.path, &spec))
            .map(|entry| entry.path)
            .collect();
        for entry in current {
            let location = entry_path(&entry);
            let _ = index.conflict_remove(&location);
            let _ = index.remove_path(&location);
        }
        let restored: Vec<git2::IndexEntry> = head_index
            .iter()
            .filter(|entry| spec_matches(&entry.path, &spec))
            .collect();
        for entry in restored {
            index.add(&entry)?;
        }
    }
    index.write()?;
    Ok(format!("Unstaged {} path(s).\n", paths.len()))
}

#[cfg(feature = "execute")]
fn commit(
    path: &Path,
    message: &str,
    author_name: &str,
    author_email: &str,
    allow_empty: bool,
) -> flow_like_types::Result<LocalResult> {
    if message.trim().is_empty() {
        return Err(flow_like_types::anyhow!("Commit Message is required"));
    }
    if author_name.is_empty() != author_email.is_empty() {
        return Err(flow_like_types::anyhow!(
            "Provide both Author Name and Author Email, or leave both empty"
        ));
    }
    if author_name.contains(['\n', '\r', '\0']) || author_email.contains(['\n', '\r', '\0']) {
        return Err(flow_like_types::anyhow!(
            "Author identity cannot contain line breaks or NUL bytes"
        ));
    }
    let mut repo = git::open(path)?;
    let mut merge_heads = Vec::new();
    if repo.state() == git2::RepositoryState::Merge {
        repo.mergehead_foreach(|oid| {
            merge_heads.push(*oid);
            true
        })?;
    }
    let signature = if author_name.is_empty() {
        repo.signature().map_err(|_| {
            flow_like_types::anyhow!(
                "Author identity unknown: set user.name and user.email, or provide Author Name and Author Email"
            )
        })?
    } else {
        git2::Signature::now(author_name, author_email)?
    };
    let mut index = repo.index()?;
    if index.has_conflicts() {
        return Err(flow_like_types::anyhow!(
            "Committing is not possible because you have unmerged files"
        ));
    }
    let tree = repo.find_tree(index.write_tree()?)?;
    let mut parents = Vec::new();
    let head = repo
        .head()
        .ok()
        .map(|head| head.peel_to_commit())
        .transpose()?;
    let merging = repo.state() == git2::RepositoryState::Merge;
    if let Some(head) = &head {
        parents.push(head.clone());
    }
    for oid in merge_heads {
        parents.push(repo.find_commit(oid)?);
    }
    let unchanged = match &head {
        Some(head) => head.tree_id() == tree.id(),
        None => tree.is_empty(),
    };
    if unchanged && !merging && !allow_empty {
        return Err(flow_like_types::anyhow!(
            "nothing to commit: stage changes first or enable Allow Empty"
        ));
    }
    let message = git2::message_prettify(message, None)?;
    let references: Vec<&git2::Commit> = parents.iter().collect();
    let oid = repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        &message,
        &tree,
        &references,
    )?;
    if merging {
        repo.cleanup_state()?;
    }
    let branch = git::head_branch(&repo).unwrap_or_else(|| "detached HEAD".into());
    let root = if parents.is_empty() {
        " (root-commit)"
    } else {
        ""
    };
    let subject = message.lines().next().unwrap_or_default();
    Ok(
        LocalResult::output(format!("[{branch}{root} {}] {subject}\n", git::short(oid)))
            .value("commit", json!(oid.to_string())),
    )
}

#[cfg(feature = "execute")]
fn stash_save(
    path: &Path,
    message: &str,
    include_untracked: bool,
) -> flow_like_types::Result<String> {
    let mut repo = git::open(path)?;
    // Stash entries need an identity, but a missing user configuration should not block saving work.
    let signature = repo
        .signature()
        .or_else(|_| git2::Signature::now("Flow-Like", "flow-like@localhost"))?;
    let mut flags = git2::StashFlags::DEFAULT;
    if include_untracked {
        flags |= git2::StashFlags::INCLUDE_UNTRACKED;
    }
    let message = (!message.is_empty()).then_some(message);
    match repo.stash_save2(&signature, message, Some(flags)) {
        Ok(oid) => Ok(format!(
            "Saved working directory and index state {}\n",
            message.map_or_else(|| git::short(oid), str::to_owned)
        )),
        Err(error) if error.code() == git2::ErrorCode::NotFound => {
            Ok("No local changes to save\n".into())
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(feature = "execute")]
fn stash_pop(path: &Path, index: i64, restore_index: bool) -> flow_like_types::Result<String> {
    if index < 0 {
        return Err(flow_like_types::anyhow!("Stash Index cannot be negative"));
    }
    let mut repo = git::open(path)?;
    let mut options = git2::StashApplyOptions::new();
    if restore_index {
        options.reinstantiate_index();
    }
    // Like `git stash pop`: a conflicting apply reports an error, keeps the stash entry and leaves
    // the conflicts visible in the working tree. libgit2's own pop would drop the entry regardless.
    repo.stash_apply(index as usize, Some(&mut options))?;
    if repo.index()?.has_conflicts() {
        return Err(flow_like_types::anyhow!(
            "The stash conflicts with the working tree. Resolve the conflicts; stash@{{{index}}} was kept."
        ));
    }
    repo.stash_drop(index as usize)?;
    Ok(format!("Dropped stash@{{{index}}}\n"))
}

#[cfg(feature = "execute")]
fn stashes(path: &Path) -> flow_like_types::Result<Vec<LocalGitStash>> {
    let mut repo = git::open(path)?;
    let mut stashes = Vec::new();
    repo.stash_foreach(|index, message, oid| {
        stashes.push(LocalGitStash {
            reference: format!("stash@{{{index}}}"),
            commit: oid.to_string(),
            message: message.to_owned(),
        });
        true
    })?;
    Ok(stashes)
}

#[cfg(feature = "execute")]
fn reset(
    path: &Path,
    revision: &str,
    mode: &str,
    allow_destructive: bool,
) -> flow_like_types::Result<String> {
    let kind = match mode {
        "soft" => git2::ResetType::Soft,
        "mixed" => git2::ResetType::Mixed,
        "hard" if allow_destructive => git2::ResetType::Hard,
        "hard" => {
            return Err(flow_like_types::anyhow!(
                "Hard reset discards tracked changes and may remove obstructing untracked files. Enable Allow Destructive to proceed."
            ));
        }
        _ => {
            return Err(flow_like_types::anyhow!(
                "Mode must be soft, mixed, or hard"
            ));
        }
    };
    let repo = git::open(path)?;
    let oid = git::resolve_commit(&repo, revision)?;
    let commit = repo.find_commit(oid)?;
    if kind == git2::ResetType::Hard {
        clear_obstructing_symlinks(&repo, &commit.tree()?)?;
    }
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.force();
    repo.reset(commit.as_object(), kind, Some(&mut checkout))?;
    let subject = String::from_utf8_lossy(commit.summary_bytes().unwrap_or_default()).into_owned();
    Ok(match kind {
        git2::ResetType::Hard => format!("HEAD is now at {} {subject}\n", git::short(oid)),
        _ => format!("Reset ({mode}) to {} {subject}\n", git::short(oid)),
    })
}

#[cfg(feature = "execute")]
fn merge(path: &Path, revision: &str, fast_forward_only: bool) -> flow_like_types::Result<String> {
    let revision_id = commit_ref(path, revision)?;
    git::require_clean(path)?;
    let repo = git::open(path)?;
    let oid = git2::Oid::from_str(&revision_id)?;
    let theirs = repo.find_annotated_commit(oid)?;
    let head = match repo.head() {
        Ok(head) => head,
        Err(error) if error.code() == git2::ErrorCode::UnbornBranch => {
            return git::fast_forward(&repo, oid);
        }
        Err(error) => return Err(error.into()),
    };
    let (analysis, _) = repo.merge_analysis_for_ref(&head, &[&theirs])?;
    if analysis.is_up_to_date() {
        return Ok("Already up to date.\n".into());
    }
    if analysis.is_fast_forward() {
        return git::fast_forward(&repo, oid);
    }
    if fast_forward_only {
        return Err(flow_like_types::anyhow!(
            "Not possible to fast-forward, aborting."
        ));
    }
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.safe();
    repo.merge(&[&theirs], None, Some(&mut checkout))?;
    let mut index = repo.index()?;
    if index.has_conflicts() {
        let mut conflicted: Vec<String> = Vec::new();
        for conflict in index.conflicts()? {
            let conflict = conflict?;
            if let Some(entry) = conflict.our.or(conflict.their).or(conflict.ancestor) {
                conflicted.push(String::from_utf8_lossy(&entry.path).into_owned());
            }
        }
        return Err(flow_like_types::anyhow!(
            "CONFLICT in {}\nAutomatic merge failed; fix conflicts and then commit the result, or abort the merge.",
            conflicted.join(", ")
        ));
    }
    let tree = repo.find_tree(index.write_tree()?)?;
    let signature = repo.signature().map_err(|_| {
        flow_like_types::anyhow!("Author identity unknown: set user.name and user.email")
    })?;
    let kind = if repo.find_branch(revision, git2::BranchType::Local).is_ok() {
        format!("Merge branch '{revision}'")
    } else if repo
        .find_reference(&format!("refs/tags/{revision}"))
        .is_ok()
    {
        format!("Merge tag '{revision}'")
    } else {
        format!("Merge commit '{revision}'")
    };
    let ours = head.peel_to_commit()?;
    let other = repo.find_commit(oid)?;
    let merged = repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        &format!("{kind}\n"),
        &tree,
        &[&ours, &other],
    )?;
    repo.cleanup_state()?;
    Ok(format!(
        "Merge made by the 'ort' strategy.\n[{}] {kind}\n",
        git::short(merged)
    ))
}

#[cfg(feature = "execute")]
fn abort_merge(path: &Path) -> flow_like_types::Result<String> {
    let repo = git::open(path)?;
    if repo.state() != git2::RepositoryState::Merge {
        return Err(flow_like_types::anyhow!(
            "There is no merge to abort (MERGE_HEAD missing)."
        ));
    }
    let head = repo.head()?.peel_to_commit()?;
    clear_obstructing_symlinks(&repo, &head.tree()?)?;
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.force();
    repo.reset(head.as_object(), git2::ResetType::Hard, Some(&mut checkout))?;
    repo.cleanup_state()?;
    Ok("Merge aborted.\n".into())
}

#[cfg(feature = "execute")]
fn remote_urls(repo: &git2::Repository, key: &str) -> flow_like_types::Result<Vec<String>> {
    Ok(git::config_values(repo, key)?
        .iter()
        .map(|url| git::redact(url))
        .collect())
}

#[cfg(feature = "execute")]
fn remotes(path: &Path) -> flow_like_types::Result<Vec<LocalGitRemote>> {
    let repo = git::open(path)?;
    let names = repo.remotes()?;
    names
        .iter()
        .flatten()
        .map(|name| {
            git::validate_remote(name)?;
            let fetch_urls = remote_urls(&repo, &format!("remote.{name}.url"))?;
            let mut push_urls = remote_urls(&repo, &format!("remote.{name}.pushurl"))?;
            if push_urls.is_empty() {
                push_urls = fetch_urls.clone();
            }
            Ok(LocalGitRemote {
                name: name.to_string(),
                fetch_urls,
                push_urls,
            })
        })
        .collect()
}

#[cfg(feature = "execute")]
fn tag_names(path: &Path) -> flow_like_types::Result<Vec<String>> {
    let repo = git::open(path)?;
    let mut names: Vec<String> = repo
        .tag_names(None)?
        .iter()
        .flatten()
        .map(str::to_owned)
        .collect();
    names.sort();
    Ok(names)
}

#[cfg(feature = "execute")]
fn delete_tag(path: &Path, name: &str) -> flow_like_types::Result<String> {
    git::validate_tag_name(name)?;
    let repo = git::open(path)?;
    let target = repo
        .find_reference(&format!("refs/tags/{name}"))
        .map_err(|_| flow_like_types::anyhow!("tag '{name}' not found"))?
        .target();
    repo.tag_delete(name)?;
    Ok(format!(
        "Deleted tag '{name}' (was {})\n",
        target.map_or_else(String::new, git::short)
    ))
}

#[cfg(feature = "execute")]
fn create_tag(
    path: &Path,
    name: &str,
    revision: &str,
    message: &str,
) -> flow_like_types::Result<String> {
    git::validate_tag_name(name)?;
    let repo = git::open(path)?;
    let oid = git::resolve_commit(&repo, revision)?;
    let object = repo.find_object(oid, None)?;
    if message.is_empty() {
        repo.tag_lightweight(name, &object, false)?;
    } else {
        let signature = repo.signature().map_err(|_| {
            flow_like_types::anyhow!("Tagger identity unknown: set user.name and user.email")
        })?;
        repo.tag(
            name,
            &object,
            &signature,
            &git2::message_prettify(message, None)?,
            false,
        )?;
    }
    Ok(format!("Created tag '{name}' at {}\n", git::short(oid)))
}

#[crate::register_node]
#[derive(Default)]
pub struct GetLocalGitStatusNode {}

impl GetLocalGitStatusNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetLocalGitStatusNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_local_status",
            "Repository Status",
            "Inspect local changes, the current branch, and the checked out commit. Handles repositories before their first commit.",
            "repositoryStatus",
        );
        node.add_output_pin(
            "status",
            "Status",
            "Structured repository state and file changes",
            VariableType::Struct,
        )
        .set_schema::<LocalGitStatus>();
        node.add_output_pin(
            "clean",
            "Clean",
            "No tracked changes or untracked files",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "branch",
            "Branch",
            "Current branch; empty for detached HEAD",
            VariableType::String,
        );
        node.add_output_pin(
            "commit",
            "Commit",
            "Current commit SHA; empty before the first commit",
            VariableType::String,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(
            context,
            &[
                ("status", json!(null)),
                ("clean", json!(false)),
                ("branch", json!("")),
                ("commit", json!("")),
            ],
        )
        .await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        execute_local(context, &repository_path, move |path| {
            let status = status(path)?;
            Ok(LocalResult::output(String::new())
                .value("clean", json!(status.clean))
                .value("branch", json!(status.branch))
                .value("commit", json!(status.commit))
                .value("status", json!(status)))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DiffLocalGitRepositoryNode {}

impl DiffLocalGitRepositoryNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DiffLocalGitRepositoryNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_local_diff",
            "Repository Diff",
            "Read the tracked-file diff for a working tree or its staged changes. Untracked files are available from Repository Status.",
            "diffRepository",
        );
        bool_input(
            &mut node,
            "staged",
            "Staged",
            "Compare staged changes against the revision instead of working tree changes",
            false,
        );
        string_input(
            &mut node,
            "revision",
            "Revision",
            "Optional commit or revision to compare against; empty uses the index or HEAD",
            "",
        );
        paths_input(&mut node);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        let staged: bool = context.evaluate_pin("staged").await?;
        let revision: String = context.evaluate_pin("revision").await?;
        let paths: Vec<String> = context.evaluate_pin("paths").await?;
        execute_local(context, &repository_path, move |path| {
            Ok(LocalResult::output(diff(path, staged, &revision, &paths)?))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct LogLocalGitRepositoryNode {}

impl LogLocalGitRepositoryNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LogLocalGitRepositoryNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_local_log",
            "Repository Log",
            "Read recent commits from a local branch, tag, or revision.",
            "repositoryLog",
        );
        string_input(
            &mut node,
            "revision",
            "Revision",
            "Branch, tag, or revision to read",
            "HEAD",
        );
        integer_input(
            &mut node,
            "max_count",
            "Max Count",
            "Maximum commits to return, from 1 to 10000",
            20,
        );
        node.add_output_pin(
            "commits",
            "Commits",
            "Commits in newest-first order",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<LocalGitCommit>();
        count_output(&mut node);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[("commits", json!([])), ("count", json!(0))]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        let revision: String = context.evaluate_pin("revision").await?;
        let max_count: i64 = context.evaluate_pin("max_count").await?;
        execute_local(context, &repository_path, move |path| {
            let entries = log(path, &revision, max_count)?;
            Ok(LocalResult::output(String::new())
                .value("count", json!(entries.len()))
                .value("commits", json!(entries)))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ListLocalGitBranchesNode {}

impl ListLocalGitBranchesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListLocalGitBranchesNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_list_local_branches",
            "List Local Branches",
            "List local branches and optionally cached remote branches. Fetch first to refresh remote branches.",
            "listLocalBranches",
        );
        bool_input(
            &mut node,
            "include_remote",
            "Include Remote",
            "Include branches under refs/remotes",
            false,
        );
        node.add_output_pin(
            "branches",
            "Branches",
            "Branches, their commits, and upstream configuration",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<LocalGitBranch>();
        count_output(&mut node);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[("branches", json!([])), ("count", json!(0))]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        let include_remote: bool = context.evaluate_pin("include_remote").await?;
        execute_local(context, &repository_path, move |path| {
            let entries = branches(path, include_remote)?;
            Ok(LocalResult::output(String::new())
                .value("count", json!(entries.len()))
                .value("branches", json!(entries)))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct SwitchLocalGitBranchNode {}

impl SwitchLocalGitBranchNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SwitchLocalGitBranchNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_switch_local_branch",
            "Switch Branch",
            "Switch an existing local branch. Local changes are rejected by default; disabling Require Clean retains normal Git overwrite checks.",
            "switchBranch",
        );
        string_input(
            &mut node,
            "branch",
            "Branch",
            "Existing local branch name",
            "",
        );
        bool_input(
            &mut node,
            "require_clean",
            "Require Clean",
            "Reject tracked changes and untracked files before switching",
            true,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        let branch: String = context.evaluate_pin("branch").await?;
        let require_clean: bool = context.evaluate_pin("require_clean").await?;
        execute_local(context, &repository_path, move |path| {
            Ok(LocalResult::output(switch_branch(
                path,
                &branch,
                require_clean,
            )?))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CheckoutLocalGitRevisionNode {}

impl CheckoutLocalGitRevisionNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CheckoutLocalGitRevisionNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_checkout_local_revision",
            "Checkout Revision",
            "Check out a commit or tag with detached HEAD. Use Switch Branch to resume work on a local branch.",
            "checkoutRevision",
        );
        string_input(
            &mut node,
            "revision",
            "Revision",
            "Commit, tag, or revision to check out",
            "HEAD",
        );
        bool_input(
            &mut node,
            "require_clean",
            "Require Clean",
            "Reject tracked changes and untracked files before checking out",
            true,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        let revision: String = context.evaluate_pin("revision").await?;
        let require_clean: bool = context.evaluate_pin("require_clean").await?;
        execute_local(context, &repository_path, move |path| {
            Ok(LocalResult::output(checkout_revision(
                path,
                &revision,
                require_clean,
            )?))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateLocalGitBranchNode {}

impl CreateLocalGitBranchNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateLocalGitBranchNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_create_local_branch",
            "Create Local Branch",
            "Create a local branch at a commit, tag, or branch without changing a GitHub API branch directly.",
            "createLocalBranch",
        );
        string_input(&mut node, "branch", "Branch", "New local branch name", "");
        string_input(
            &mut node,
            "start_point",
            "Start Point",
            "Commit, tag, or existing branch for the new branch",
            "HEAD",
        );
        bool_input(
            &mut node,
            "switch",
            "Switch",
            "Switch to the new branch; requires a clean working tree",
            false,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        let branch: String = context.evaluate_pin("branch").await?;
        let start_point: String = context.evaluate_pin("start_point").await?;
        let switch: bool = context.evaluate_pin("switch").await?;
        execute_local(context, &repository_path, move |path| {
            Ok(LocalResult::output(create_branch(
                path,
                &branch,
                &start_point,
                switch,
            )?))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DeleteLocalGitBranchNode {}

impl DeleteLocalGitBranchNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeleteLocalGitBranchNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_delete_local_branch",
            "Delete Local Branch",
            "Delete a local branch. Git rejects unmerged branches unless Force is enabled and always protects branches checked out in a worktree.",
            "deleteLocalBranch",
        );
        string_input(&mut node, "branch", "Branch", "Local branch to delete", "");
        bool_input(
            &mut node,
            "force",
            "Force",
            "Allow deleting an unmerged branch; its commits may become unreachable",
            false,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        let branch: String = context.evaluate_pin("branch").await?;
        let force: bool = context.evaluate_pin("force").await?;
        execute_local(context, &repository_path, move |path| {
            Ok(LocalResult::output(delete_branch(path, &branch, force)?))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StageLocalGitFilesNode {}

impl StageLocalGitFilesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for StageLocalGitFilesNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_stage_local_files",
            "Stage Files",
            "Stage additions, modifications, and deletions for a commit. Provide literal paths or explicitly enable All.",
            "stageFiles",
        );
        paths_input(&mut node);
        bool_input(
            &mut node,
            "all",
            "All",
            "Stage every non-ignored change; leave Paths empty",
            false,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        let paths: Vec<String> = context.evaluate_pin("paths").await?;
        let all: bool = context.evaluate_pin("all").await?;
        execute_local(context, &repository_path, move |path| {
            Ok(LocalResult::output(stage(path, &paths, all)?))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct UnstageLocalGitFilesNode {}

impl UnstageLocalGitFilesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UnstageLocalGitFilesNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_unstage_local_files",
            "Unstage Files",
            "Remove selected changes from the staging area while preserving working files, including before the first commit.",
            "unstageFiles",
        );
        paths_input(&mut node);
        bool_input(
            &mut node,
            "all",
            "All",
            "Unstage every staged change; leave Paths empty",
            false,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        let paths: Vec<String> = context.evaluate_pin("paths").await?;
        let all: bool = context.evaluate_pin("all").await?;
        execute_local(context, &repository_path, move |path| {
            Ok(LocalResult::output(unstage(path, &paths, all)?))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CommitLocalGitRepositoryNode {}

impl CommitLocalGitRepositoryNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CommitLocalGitRepositoryNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_commit_local_repository",
            "Commit Changes",
            "Commit the staged changes. Uses the repository identity unless an author name and email are provided. Commit signing is disabled for unattended execution.",
            "commitChanges",
        );
        string_input(
            &mut node,
            "message",
            "Message",
            "Required commit message",
            "",
        );
        string_input(
            &mut node,
            "author_name",
            "Author Name",
            "Optional author and committer name; requires Author Email",
            "",
        );
        string_input(
            &mut node,
            "author_email",
            "Author Email",
            "Optional author and committer email; requires Author Name",
            "",
        );
        bool_input(
            &mut node,
            "allow_empty",
            "Allow Empty",
            "Allow creating a commit with no staged changes",
            false,
        );
        node.add_output_pin(
            "commit",
            "Commit",
            "Created commit SHA",
            VariableType::String,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[("commit", json!(""))]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        let message: String = context.evaluate_pin("message").await?;
        let author_name: String = context.evaluate_pin("author_name").await?;
        let author_email: String = context.evaluate_pin("author_email").await?;
        let allow_empty: bool = context.evaluate_pin("allow_empty").await?;
        execute_local(context, &repository_path, move |path| {
            commit(path, &message, &author_name, &author_email, allow_empty)
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct SaveLocalGitStashNode {}

impl SaveLocalGitStashNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SaveLocalGitStashNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_save_local_stash",
            "Stash Changes",
            "Save tracked worktree and staged changes in a stash. Untracked files are included only when requested; ignored files are preserved.",
            "stashChanges",
        );
        string_input(
            &mut node,
            "message",
            "Message",
            "Optional stash description",
            "",
        );
        bool_input(
            &mut node,
            "include_untracked",
            "Include Untracked",
            "Also stash and remove untracked files from the working tree",
            false,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        let message: String = context.evaluate_pin("message").await?;
        let include_untracked: bool = context.evaluate_pin("include_untracked").await?;
        execute_local(context, &repository_path, move |path| {
            Ok(LocalResult::output(stash_save(
                path,
                &message,
                include_untracked,
            )?))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct PopLocalGitStashNode {}

impl PopLocalGitStashNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for PopLocalGitStashNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_pop_local_stash",
            "Pop Stash",
            "Apply a stash and remove it after success. Conflicts route to Error and preserve the stash for recovery.",
            "popStash",
        );
        integer_input(
            &mut node,
            "index",
            "Stash Index",
            "Zero-based stash index; 0 is the newest stash",
            0,
        );
        bool_input(
            &mut node,
            "restore_index",
            "Restore Index",
            "Attempt to restore which changes were staged",
            false,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        let index: i64 = context.evaluate_pin("index").await?;
        let restore_index: bool = context.evaluate_pin("restore_index").await?;
        execute_local(context, &repository_path, move |path| {
            Ok(LocalResult::output(stash_pop(path, index, restore_index)?))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ListLocalGitStashesNode {}

impl ListLocalGitStashesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListLocalGitStashesNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_list_local_stashes",
            "List Stashes",
            "List saved local stashes in newest-first order.",
            "listStashes",
        );
        node.add_output_pin(
            "stashes",
            "Stashes",
            "Stash references, commits, and messages",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<LocalGitStash>();
        count_output(&mut node);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[("stashes", json!([])), ("count", json!(0))]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        execute_local(context, &repository_path, move |path| {
            let entries = stashes(path)?;
            Ok(LocalResult::output(String::new())
                .value("count", json!(entries.len()))
                .value("stashes", json!(entries)))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ResetLocalGitRepositoryNode {}

impl ResetLocalGitRepositoryNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ResetLocalGitRepositoryNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_reset_local_repository",
            "Reset Repository",
            "Move the current branch to a revision. Soft preserves staged changes; mixed unstages changes; hard discards tracked changes and may remove obstructing untracked files.",
            "resetRepository",
        );
        string_input(
            &mut node,
            "revision",
            "Revision",
            "Commit, branch, or tag to reset to",
            "HEAD",
        );
        node.add_input_pin("mode", "Mode", "soft, mixed, or hard", VariableType::String)
            .set_default_value(Some(json!("mixed")))
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec!["soft".into(), "mixed".into(), "hard".into()])
                    .build(),
            );
        bool_input(
            &mut node,
            "allow_destructive",
            "Allow Destructive",
            "Required for hard reset, which can discard local file contents",
            false,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        let revision: String = context.evaluate_pin("revision").await?;
        let mode: String = context.evaluate_pin("mode").await?;
        let allow_destructive: bool = context.evaluate_pin("allow_destructive").await?;
        execute_local(context, &repository_path, move |path| {
            Ok(LocalResult::output(reset(
                path,
                &revision,
                &mode,
                allow_destructive,
            )?))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MergeLocalGitRepositoryNode {}

impl MergeLocalGitRepositoryNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MergeLocalGitRepositoryNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_merge_local_repository",
            "Merge Branch",
            "Merge a branch or revision into the current branch. Requires a clean working tree. Conflicts route to Error; Abort Merge cancels an unfinished merge.",
            "mergeBranch",
        );
        string_input(
            &mut node,
            "revision",
            "Revision",
            "Branch, tag, or revision to merge",
            "",
        );
        bool_input(
            &mut node,
            "fast_forward_only",
            "Fast Forward Only",
            "Reject divergent history instead of creating a merge commit",
            true,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        let revision: String = context.evaluate_pin("revision").await?;
        let fast_forward_only: bool = context.evaluate_pin("fast_forward_only").await?;
        execute_local(context, &repository_path, move |path| {
            Ok(LocalResult::output(merge(
                path,
                &revision,
                fast_forward_only,
            )?))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct AbortLocalGitMergeNode {}

impl AbortLocalGitMergeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AbortLocalGitMergeNode {
    fn get_node(&self) -> Node {
        repository::node(
            "data_github_abort_local_merge",
            "Abort Merge",
            "Cancel an unfinished merge and restore the pre-merge state. Returns an error when no merge is in progress.",
            "abortMerge",
        )
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        execute_local(context, &repository_path, move |path| {
            Ok(LocalResult::output(abort_merge(path)?))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ListLocalGitRemotesNode {}

impl ListLocalGitRemotesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListLocalGitRemotesNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_list_local_remotes",
            "List Remotes",
            "List configured fetch and push URLs with embedded credentials removed.",
            "listRemotes",
        );
        node.add_output_pin(
            "remotes",
            "Remotes",
            "Configured remote names and URLs",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<LocalGitRemote>();
        count_output(&mut node);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[("remotes", json!([])), ("count", json!(0))]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        execute_local(context, &repository_path, move |path| {
            let entries = remotes(path)?;
            Ok(LocalResult::output(String::new())
                .value("count", json!(entries.len()))
                .value("remotes", json!(entries)))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ListLocalGitTagsNode {}

impl ListLocalGitTagsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListLocalGitTagsNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_list_local_tags",
            "List Local Tags",
            "List tags stored in the repository. Fetch with tags enabled to refresh tags from a remote.",
            "listLocalTags",
        );
        node.add_output_pin(
            "tags",
            "Tags",
            "Tag names in lexicographic order",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);
        count_output(&mut node);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[("tags", json!([])), ("count", json!(0))]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        execute_local(context, &repository_path, move |path| {
            let tags = tag_names(path)?;
            Ok(LocalResult::output(String::new())
                .value("count", json!(tags.len()))
                .value("tags", json!(tags)))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateLocalGitTagNode {}

impl CreateLocalGitTagNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateLocalGitTagNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_create_local_tag",
            "Create Local Tag",
            "Create a tag at a commit. A nonempty message creates an unsigned annotated tag; an empty message creates a lightweight tag. Existing tags are never overwritten.",
            "createLocalTag",
        );
        string_input(&mut node, "name", "Name", "New local tag name", "");
        string_input(
            &mut node,
            "revision",
            "Revision",
            "Commit, branch, or tag to tag",
            "HEAD",
        );
        string_input(
            &mut node,
            "message",
            "Message",
            "Annotation message; leave empty for a lightweight tag",
            "",
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        let name: String = context.evaluate_pin("name").await?;
        let revision: String = context.evaluate_pin("revision").await?;
        let message: String = context.evaluate_pin("message").await?;
        execute_local(context, &repository_path, move |path| {
            Ok(LocalResult::output(create_tag(
                path, &name, &revision, &message,
            )?))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DeleteLocalGitTagNode {}

impl DeleteLocalGitTagNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeleteLocalGitTagNode {
    fn get_node(&self) -> Node {
        let mut node = repository::node(
            "data_github_delete_local_tag",
            "Delete Local Tag",
            "Delete a named local tag. Remote tags are unchanged.",
            "deleteLocalTag",
        );
        string_input(&mut node, "name", "Name", "Existing local tag name", "");
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        begin_local(context, &[]).await?;
        let repository_path: FlowPath = context.evaluate_pin("repository").await?;
        let name: String = context.evaluate_pin("name").await?;
        execute_local(context, &repository_path, move |path| {
            Ok(LocalResult::output(delete_tag(path, &name)?))
        })
        .await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TestRepository(PathBuf);

    impl TestRepository {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("flow-like-local-git-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            let path = path.canonicalize().unwrap();
            git::testing::init(&path);
            Self(path)
        }

        fn write(&self, name: &str, content: &str) {
            std::fs::write(self.0.join(name), content).unwrap();
        }

        fn commit(&self, message: &str) -> String {
            stage(&self.0, &[], true).unwrap();
            commit(&self.0, message, "", "", false).unwrap();
            commit_ref(&self.0, "HEAD").unwrap()
        }
    }

    impl Drop for TestRepository {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_directories_are_not_staged_or_removed_through() {
        let repo = TestRepository::new();
        let outside =
            std::env::temp_dir().join(format!("flow-like-outside-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "TOPSECRET\n").unwrap();
        std::os::unix::fs::symlink(&outside, repo.0.join("link")).unwrap();
        let spec = vec!["link/secret.txt".to_string()];
        assert!(stage(&repo.0, &spec, false).is_err());
        assert!(unstage(&repo.0, &spec, false).is_err());
        assert!(diff(&repo.0, true, "", &spec).is_err());
        assert!(
            git::open(&repo.0)
                .unwrap()
                .index()
                .unwrap()
                .get_path(Path::new("link/secret.txt"), 0)
                .is_none()
        );
        std::fs::remove_dir_all(&outside).unwrap();

        // A force checkout must not rmdir the target of a symlink standing where HEAD has a directory.
        let repo = TestRepository::new();
        std::fs::create_dir(repo.0.join("link")).unwrap();
        repo.write("link/f.txt", "f\n");
        repo.commit("c1");
        std::fs::remove_dir_all(repo.0.join("link")).unwrap();
        let empty = std::env::temp_dir().join(format!("flow-like-empty-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&empty).unwrap();
        std::os::unix::fs::symlink(&empty, repo.0.join("link")).unwrap();
        reset(&repo.0, "HEAD", "hard", true).unwrap();
        assert!(empty.exists());
        assert!(repo.0.join("link/f.txt").is_file());
        std::fs::remove_dir_all(&empty).unwrap();
    }

    #[test]
    fn status_stage_log_merge_match_git_edge_cases() {
        let repo = TestRepository::new();
        // Merge on an unborn HEAD fast-forwards.
        repo.write("a.txt", "a\n");
        repo.commit("c1");
        let first = commit_ref(&repo.0, "HEAD").unwrap();
        let raw = git::open(&repo.0).unwrap();
        raw.branch(
            "feature",
            &raw.find_commit(git2::Oid::from_str(&first).unwrap())
                .unwrap(),
            false,
        )
        .unwrap();
        let unborn = TestRepository::new();
        let other = git::open(&unborn.0).unwrap();
        let mut remote = other
            .remote_anonymous(raw.path().to_str().unwrap())
            .unwrap();
        remote
            .fetch(&["refs/heads/feature:refs/heads/feature"], None, None)
            .unwrap();
        merge(&unborn.0, "feature", false).unwrap();
        assert_eq!(commit_ref(&unborn.0, "HEAD").unwrap(), first);

        // Deleted in the index and recreated in the worktree yields two records.
        let mut index = raw.index().unwrap();
        index.remove_path(std::path::Path::new("a.txt")).unwrap();
        index.write().unwrap();
        let current = status(&repo.0).unwrap();
        let records: Vec<_> = current.files.iter().filter(|f| f.path == "a.txt").collect();
        assert_eq!(records.len(), 2);
        assert!(records.iter().any(|f| f.index_status == "D"));
        assert!(records.iter().any(|f| f.untracked));

        // An explicitly named ignored file is an error.
        repo.write(".gitignore", "ign.txt\n");
        repo.write("ign.txt", "x\n");
        assert!(stage(&repo.0, &["ign.txt".to_string()], false).is_err());
    }

    #[test]
    fn status_and_staging_handle_unborn_branches_and_literal_paths() {
        let repo = TestRepository::new();
        let initial = status(&repo.0).unwrap();
        assert!(initial.clean);
        assert_eq!(initial.branch, "main");
        assert!(initial.commit.is_empty());

        repo.write("literal[1].txt", "selected\n");
        repo.write("literal1.txt", "other\n");
        stage(&repo.0, &["literal[1].txt".to_string()], false).unwrap();
        let current = status(&repo.0).unwrap();
        assert_eq!(current.files.len(), 2);
        assert_eq!(
            current
                .files
                .iter()
                .find(|file| file.path == "literal[1].txt")
                .unwrap()
                .index_status,
            "A"
        );
        assert!(
            current
                .files
                .iter()
                .find(|file| file.path == "literal1.txt")
                .unwrap()
                .untracked
        );
        assert!(diff(&repo.0, true, "", &[]).unwrap().contains("+selected"));
        unstage(&repo.0, &[], true).unwrap();
        assert!(
            status(&repo.0)
                .unwrap()
                .files
                .iter()
                .all(|file| file.untracked)
        );
        assert!(repo.0.join("literal[1].txt").exists());
        assert!(stage(&repo.0, &[], false).is_err());
        assert!(stage(&repo.0, &["../outside".to_string()], false).is_err());
        assert!(stage(&repo.0, &["literal1.txt".to_string()], true).is_err());

        repo.commit("Initial files");
        std::fs::rename(
            repo.0.join("literal[1].txt"),
            repo.0.join("renamed\nfile.txt"),
        )
        .unwrap();
        stage(&repo.0, &[], true).unwrap();
        let renamed = status(&repo.0).unwrap();
        assert_eq!(renamed.files.len(), 1);
        assert_eq!(renamed.files[0].path, "renamed\nfile.txt");
        assert_eq!(
            renamed.files[0].original_path.as_deref(),
            Some("literal[1].txt")
        );
        unstage(&repo.0, &[], true).unwrap();
        assert!(repo.0.join("renamed\nfile.txt").exists());
    }

    #[test]
    fn branches_preserve_local_edits_and_protect_unmerged_work() {
        let repo = TestRepository::new();
        repo.write("file.txt", "initial\n");
        let original = repo.commit("Initial");
        create_branch(&repo.0, "feature", "HEAD", false).unwrap();
        repo.write("file.txt", "local edit\n");
        assert!(switch_branch(&repo.0, "feature", true).is_err());
        assert_eq!(status(&repo.0).unwrap().branch, "main");
        switch_branch(&repo.0, "feature", false).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.0.join("file.txt")).unwrap(),
            "local edit\n"
        );
        repo.commit("Feature change");
        switch_branch(&repo.0, "main", true).unwrap();
        assert!(delete_branch(&repo.0, "feature", false).is_err());
        delete_branch(&repo.0, "feature", true).unwrap();
        assert_eq!(branches(&repo.0, false).unwrap().len(), 1);
        checkout_revision(&repo.0, &original, true).unwrap();
        assert!(status(&repo.0).unwrap().detached);
        assert!(create_branch(&repo.0, "@{-1}", "HEAD", false).is_err());
        switch_branch(&repo.0, "main", true).unwrap();
    }

    #[test]
    fn commits_logs_and_tags_keep_structured_history() {
        let repo = TestRepository::new();
        repo.write("file.txt", "first\n");
        let first = repo.commit("Initial\twith tab");
        repo.write("file.txt", "second\n");
        stage(&repo.0, &[], true).unwrap();
        commit(
            &repo.0,
            "Second commit",
            "Custom Author",
            "custom@example.invalid",
            false,
        )
        .unwrap();
        let entries = log(&repo.0, "HEAD", 10).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].parents, vec![first.clone()]);
        assert_eq!(entries[0].author_name, "Custom Author");
        assert_eq!(entries[0].author_email, "custom@example.invalid");
        assert_eq!(entries[1].subject, "Initial\twith tab");
        assert!(log(&repo.0, "HEAD", 0).is_err());
        assert_eq!(log(&repo.0, "HEAD~1", 1).unwrap()[0].sha, first);
        create_tag(&repo.0, "v1", &first, "").unwrap();
        create_tag(&repo.0, "v2", "HEAD", "Release notes").unwrap();
        let raw = git::open(&repo.0).unwrap();
        let kind = |name: &str| {
            let id = raw.refname_to_id(name).unwrap();
            raw.find_object(id, None).unwrap().kind().unwrap()
        };
        assert_eq!(kind("refs/tags/v1"), git2::ObjectType::Commit);
        assert_eq!(kind("refs/tags/v2"), git2::ObjectType::Tag);
        assert_eq!(commit_ref(&repo.0, "v1").unwrap(), first);
        assert!(create_tag(&repo.0, "v1", "HEAD", "").is_err());
        delete_tag(&repo.0, "v1").unwrap();
        assert!(delete_tag(&repo.0, "v1").is_err());
        assert_eq!(tag_names(&repo.0).unwrap(), vec!["v2".to_string()]);
    }

    #[test]
    fn stashes_restore_staged_and_untracked_changes_and_survive_conflicts() {
        let repo = TestRepository::new();
        repo.write("file.txt", "initial\n");
        repo.commit("Initial");
        repo.write("file.txt", "saved\n");
        repo.write("new.txt", "untracked\n");
        stage(&repo.0, &["file.txt".to_string()], false).unwrap();
        stash_save(&repo.0, "Saved work", true).unwrap();
        assert!(status(&repo.0).unwrap().clean);
        let entries = stashes(&repo.0).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].reference, "stash@{0}");
        assert!(entries[0].message.contains("Saved work"));
        stash_pop(&repo.0, 0, true).unwrap();
        assert!(stashes(&repo.0).unwrap().is_empty());
        assert!(repo.0.join("new.txt").exists());
        assert_eq!(
            status(&repo.0)
                .unwrap()
                .files
                .iter()
                .find(|file| file.path == "file.txt")
                .unwrap()
                .index_status,
            "M"
        );
        stash_save(&repo.0, "Conflicting work", true).unwrap();
        repo.write("file.txt", "new base\n");
        repo.commit("Change base");
        assert!(stash_pop(&repo.0, 0, false).is_err());
        assert_eq!(stashes(&repo.0).unwrap().len(), 1);
        assert!(
            status(&repo.0)
                .unwrap()
                .files
                .iter()
                .any(|file| file.conflicted)
        );
    }

    #[test]
    fn mixed_reset_preserves_files_and_hard_reset_requires_opt_in() {
        let repo = TestRepository::new();
        repo.write("file.txt", "initial\n");
        repo.commit("Initial");
        repo.write("file.txt", "modified\n");
        stage(&repo.0, &[], true).unwrap();
        reset(&repo.0, "HEAD", "mixed", false).unwrap();
        assert_eq!(status(&repo.0).unwrap().files[0].index_status, " ");
        assert_eq!(
            std::fs::read_to_string(repo.0.join("file.txt")).unwrap(),
            "modified\n"
        );
        assert!(reset(&repo.0, "HEAD", "hard", false).is_err());
        assert_eq!(
            std::fs::read_to_string(repo.0.join("file.txt")).unwrap(),
            "modified\n"
        );
        reset(&repo.0, "HEAD", "hard", true).unwrap();
        assert!(status(&repo.0).unwrap().clean);
        assert_eq!(
            std::fs::read_to_string(repo.0.join("file.txt")).unwrap(),
            "initial\n"
        );
    }

    #[test]
    fn divergent_merges_fail_by_default_and_conflicts_can_be_aborted() {
        let repo = TestRepository::new();
        repo.write("file.txt", "initial\n");
        repo.commit("Initial");
        create_branch(&repo.0, "feature", "HEAD", true).unwrap();
        repo.write("file.txt", "feature\n");
        repo.commit("Feature");
        switch_branch(&repo.0, "main", true).unwrap();
        repo.write("file.txt", "main\n");
        let before = repo.commit("Main");
        assert!(merge(&repo.0, "feature", true).is_err());
        assert!(status(&repo.0).unwrap().clean);
        assert!(merge(&repo.0, "feature", false).is_err());
        assert!(
            status(&repo.0)
                .unwrap()
                .files
                .iter()
                .any(|file| file.conflicted)
        );
        abort_merge(&repo.0).unwrap();
        assert!(abort_merge(&repo.0).is_err());
        assert!(status(&repo.0).unwrap().clean);
        assert_eq!(commit_ref(&repo.0, "HEAD").unwrap(), before);
        assert_eq!(
            std::fs::read_to_string(repo.0.join("file.txt")).unwrap(),
            "main\n"
        );
    }

    #[test]
    fn remote_lists_redact_existing_url_credentials() {
        let repo = TestRepository::new();
        git::open(&repo.0)
            .unwrap()
            .remote(
                "origin",
                "https://user:secret@example.invalid/repository.git",
            )
            .unwrap();
        let entries = remotes(&repo.0).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "origin");
        assert!(entries[0].fetch_urls[0].contains("example.invalid/repository.git"));
        assert!(!entries[0].fetch_urls[0].contains("secret"));
        assert!(!entries[0].push_urls[0].contains("secret"));
    }

    #[test]
    fn diff_is_a_unified_patch_and_log_uses_iso_timestamps() {
        let repo = TestRepository::new();
        repo.write("file.txt", "one\ntwo\n");
        let first = repo.commit("Initial");
        repo.write("file.txt", "one\nchanged\n");
        let unstaged = diff(&repo.0, false, "", &[]).unwrap();
        assert!(
            unstaged.starts_with("diff --git a/file.txt b/file.txt\n"),
            "{unstaged}"
        );
        assert!(
            unstaged.contains("@@ -1,2 +1,2 @@\n one\n-two\n+changed\n"),
            "{unstaged}"
        );
        assert!(diff(&repo.0, true, "", &[]).unwrap().is_empty());
        assert!(
            diff(&repo.0, false, &first, &["file.txt".to_string()])
                .unwrap()
                .contains("+changed")
        );
        assert!(
            diff(&repo.0, false, "", &["other.txt".to_string()])
                .unwrap()
                .is_empty()
        );
        let entries = log(&repo.0, "HEAD", 5).unwrap();
        let stamp = &entries[0].authored_at;
        assert_eq!(stamp.len(), 25, "{stamp}");
        assert!(stamp.as_bytes()[10] == b'T' && stamp.ends_with(|c: char| c.is_ascii_digit()));
        assert_eq!(iso8601(git2::Time::new(0, 0)), "1970-01-01T00:00:00+00:00");
        assert_eq!(
            iso8601(git2::Time::new(1_700_000_000, -90)),
            "2023-11-14T20:43:20-01:30"
        );
    }

    #[test]
    fn staging_directories_and_deletions_is_literal() {
        let repo = TestRepository::new();
        std::fs::create_dir(repo.0.join("dir")).unwrap();
        repo.write("dir/a.txt", "a\n");
        repo.write("dir/b.txt", "b\n");
        repo.write("other.txt", "o\n");
        stage(&repo.0, &["dir".to_string()], false).unwrap();
        let staged: Vec<_> = status(&repo.0)
            .unwrap()
            .files
            .into_iter()
            .filter(|file| file.index_status == "A")
            .map(|file| file.path)
            .collect();
        assert_eq!(staged, vec!["dir/a.txt", "dir/b.txt"]);
        repo.commit("Add dir");
        std::fs::remove_file(repo.0.join("dir/a.txt")).unwrap();
        stage(&repo.0, &["dir/a.txt".to_string()], false).unwrap();
        let current = status(&repo.0).unwrap();
        assert_eq!(current.files[0].path, "dir/a.txt");
        assert_eq!(current.files[0].index_status, "D");
        unstage(&repo.0, &["dir".to_string()], false).unwrap();
        assert_eq!(status(&repo.0).unwrap().files[0].worktree_status, "D");
        assert!(stage(&repo.0, &["missing.txt".to_string()], false).is_err());
    }

    #[test]
    fn commits_need_changes_and_conclude_merges() {
        let repo = TestRepository::new();
        repo.write("file.txt", "initial\n");
        repo.commit("Initial");
        assert!(commit(&repo.0, "Nothing", "", "", false).is_err());
        let empty = commit(&repo.0, "Empty", "", "", true).unwrap();
        assert!(empty.output.contains("Empty"));
        create_branch(&repo.0, "topic", "HEAD", true).unwrap();
        repo.write("topic.txt", "topic\n");
        repo.commit("Topic");
        switch_branch(&repo.0, "main", true).unwrap();
        repo.write("main.txt", "main\n");
        repo.commit("Main");
        // A divergent merge without conflicts creates a merge commit with both parents.
        let output = merge(&repo.0, "topic", false).unwrap();
        assert!(output.contains("Merge branch 'topic'"), "{output}");
        let entries = log(&repo.0, "HEAD", 1).unwrap();
        assert_eq!(entries[0].parents.len(), 2);
        assert!(status(&repo.0).unwrap().clean);
        assert_eq!(
            merge(&repo.0, "topic", false).unwrap(),
            "Already up to date.\n"
        );
    }
}
