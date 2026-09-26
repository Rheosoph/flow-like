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
        let path = repository::local_path(context, repository_path).await?;
        flow_like_types::tokio::task::spawn_blocking(move || {
            git::ensure_repository(&path)?;
            operation(&path)
        })
        .await
        .map_err(|error| flow_like_types::anyhow!("Git operation task failed: {error}"))?
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
fn branch_name(path: &Path, branch: &str) -> flow_like_types::Result<()> {
    git::validate_ref(branch)?;
    // Reject checkout shorthand such as @{-1}; branch names must be literal.
    if branch.contains('@') && branch.contains('{') {
        return Err(flow_like_types::anyhow!("Use a literal branch name"));
    }
    git::run(path, &["check-ref-format", "--branch", branch])?;
    Ok(())
}

#[cfg(feature = "execute")]
fn commit_ref(path: &Path, revision: &str) -> flow_like_types::Result<String> {
    git::validate_ref(revision)?;
    let revision = format!("{revision}^{{commit}}");
    Ok(git::run(
        path,
        &["rev-parse", "--verify", "--end-of-options", &revision],
    )?
    .trim()
    .to_string())
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

#[cfg(feature = "execute")]
fn status(path: &Path) -> flow_like_types::Result<LocalGitStatus> {
    let bytes = git::run_bytes(
        path,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let mut records = bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty());
    let mut files = Vec::new();
    while let Some(record) = records.next() {
        if record.len() < 4 || record[2] != b' ' {
            return Err(flow_like_types::anyhow!(
                "Git returned an invalid status record"
            ));
        }
        let index = record[0];
        let worktree = record[1];
        let original_path = if matches!(index, b'R' | b'C') || matches!(worktree, b'R' | b'C') {
            Some(
                String::from_utf8_lossy(records.next().ok_or_else(|| {
                    flow_like_types::anyhow!("Git returned an incomplete rename record")
                })?)
                .into_owned(),
            )
        } else {
            None
        };
        files.push(LocalGitFileStatus {
            path: String::from_utf8_lossy(&record[3..]).into_owned(),
            original_path,
            index_status: (index as char).to_string(),
            worktree_status: (worktree as char).to_string(),
            untracked: index == b'?' && worktree == b'?',
            conflicted: index == b'U'
                || worktree == b'U'
                || matches!((index, worktree), (b'A', b'A') | (b'D', b'D')),
        });
    }
    let branch = git::run(path, &["branch", "--show-current"])?
        .trim()
        .to_string();
    let commit = git::run(path, &["rev-parse", "--verify", "--quiet", "HEAD"])
        .map(|value| value.trim().to_string())
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
    let mut args = vec![
        "--literal-pathspecs",
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
    ];
    if staged {
        args.push("--cached");
    }
    let revision = if revision.is_empty() {
        None
    } else {
        Some(commit_ref(path, revision)?)
    };
    if let Some(revision) = &revision {
        args.push(revision);
    }
    args.push("--");
    args.extend(paths.iter().map(String::as_str));
    git::run(path, &args)
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
    let revision = commit_ref(path, revision)?;
    let count = max_count.to_string();
    let bytes = git::run_bytes(
        path,
        &[
            "log",
            "-z",
            "--format=%H%x00%P%x00%an%x00%ae%x00%aI%x00%s",
            "-n",
            &count,
            &revision,
            "--",
        ],
    )?;
    let fields: Vec<&[u8]> = bytes.split(|byte| *byte == 0).collect();
    let mut commits = Vec::new();
    for record in fields.chunks(6) {
        if record.len() == 1 && record[0].is_empty() {
            continue;
        }
        if record.len() != 6 {
            return Err(flow_like_types::anyhow!(
                "Git returned an invalid log record"
            ));
        }
        let text = |index: usize| String::from_utf8_lossy(record[index]).into_owned();
        commits.push(LocalGitCommit {
            sha: text(0),
            parents: text(1).split_whitespace().map(str::to_string).collect(),
            author_name: text(2),
            author_email: text(3),
            authored_at: text(4),
            subject: text(5),
        });
    }
    Ok(commits)
}

#[cfg(feature = "execute")]
fn branches(path: &Path, include_remote: bool) -> flow_like_types::Result<Vec<LocalGitBranch>> {
    let mut args = vec![
        "for-each-ref",
        "--format=%(refname)%00%(objectname)%00%(HEAD)%00%(upstream:short)",
        "refs/heads/",
    ];
    if include_remote {
        args.push("refs/remotes/");
    }
    let output = git::run(path, &args)?;
    output
        .lines()
        .map(|line| {
            let fields: Vec<&str> = line.split('\0').collect();
            if fields.len() != 4 {
                return Err(flow_like_types::anyhow!(
                    "Git returned an invalid branch record"
                ));
            }
            let remote = fields[0].starts_with("refs/remotes/");
            Ok(LocalGitBranch {
                name: fields[0]
                    .strip_prefix(if remote {
                        "refs/remotes/"
                    } else {
                        "refs/heads/"
                    })
                    .unwrap_or(fields[0])
                    .to_string(),
                commit: fields[1].to_string(),
                current: fields[2] == "*",
                remote,
                upstream: fields[3].to_string(),
            })
        })
        .collect()
}

#[cfg(feature = "execute")]
fn switch_branch(path: &Path, branch: &str, clean: bool) -> flow_like_types::Result<String> {
    branch_name(path, branch)?;
    if clean {
        git::require_clean(path)?;
    }
    git::run(path, &["switch", "--no-guess", branch])
}

#[cfg(feature = "execute")]
fn checkout_revision(path: &Path, revision: &str, clean: bool) -> flow_like_types::Result<String> {
    let revision = commit_ref(path, revision)?;
    if clean {
        git::require_clean(path)?;
    }
    git::run(path, &["switch", "--detach", &revision])
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
        git::run(
            path,
            &["switch", "--no-track", "--create", branch, &revision],
        )
    } else {
        git::run(path, &["branch", "--no-track", branch, &revision])
    }
}

#[cfg(feature = "execute")]
fn delete_branch(path: &Path, branch: &str, force: bool) -> flow_like_types::Result<String> {
    branch_name(path, branch)?;
    git::run(
        path,
        &["branch", if force { "-D" } else { "-d" }, "--", branch],
    )
}

#[cfg(feature = "execute")]
fn stage(path: &Path, paths: &[String], all: bool) -> flow_like_types::Result<String> {
    validate_paths(paths)?;
    if all && !paths.is_empty() {
        return Err(flow_like_types::anyhow!(
            "Choose All or provide Paths, not both"
        ));
    }
    if all {
        return git::run(path, &["add", "--all", "--", "."]);
    }
    if paths.is_empty() {
        return Err(flow_like_types::anyhow!("Provide Paths or enable All"));
    }
    let mut args = vec!["--literal-pathspecs", "add", "--all", "--"];
    args.extend(paths.iter().map(String::as_str));
    git::run(path, &args)
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
    if git::run(path, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_err() {
        // An unborn branch has no HEAD tree to restore into the index.
        if all {
            return git::run(path, &["read-tree", "--empty"]);
        }
        let mut args = vec![
            "--literal-pathspecs",
            "rm",
            "--cached",
            "--ignore-unmatch",
            "-r",
            "--",
        ];
        args.extend(paths.iter().map(String::as_str));
        return git::run(path, &args);
    }
    let mut args = vec!["--literal-pathspecs", "reset", "--quiet", "HEAD", "--"];
    if all {
        args.push(".");
    } else {
        args.extend(paths.iter().map(String::as_str));
    }
    git::run(path, &args)
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
    let name_config = format!("user.name={author_name}");
    let email_config = format!("user.email={author_email}");
    let mut args = Vec::new();
    if !author_name.is_empty() {
        args.extend(["-c", &name_config, "-c", &email_config]);
    }
    args.extend(["commit", "--no-gpg-sign", "--message", message]);
    if allow_empty {
        args.push("--allow-empty");
    }
    let output = git::run(path, &args)?;
    let sha = commit_ref(path, "HEAD")?;
    Ok(LocalResult::output(output).value("commit", json!(sha)))
}

#[cfg(feature = "execute")]
fn stash_save(
    path: &Path,
    message: &str,
    include_untracked: bool,
) -> flow_like_types::Result<String> {
    let mut args = vec!["stash", "push"];
    if include_untracked {
        args.push("--include-untracked");
    }
    if !message.is_empty() {
        args.extend(["--message", message]);
    }
    git::run(path, &args)
}

#[cfg(feature = "execute")]
fn stash_pop(path: &Path, index: i64, restore_index: bool) -> flow_like_types::Result<String> {
    if index < 0 {
        return Err(flow_like_types::anyhow!("Stash Index cannot be negative"));
    }
    let reference = format!("stash@{{{index}}}");
    let mut args = vec!["stash", "pop"];
    if restore_index {
        args.push("--index");
    }
    args.push(&reference);
    git::run(path, &args)
}

#[cfg(feature = "execute")]
fn stashes(path: &Path) -> flow_like_types::Result<Vec<LocalGitStash>> {
    let bytes = git::run_bytes(path, &["stash", "list", "-z", "--format=%gd%x00%H%x00%gs"])?;
    let fields: Vec<&[u8]> = bytes.split(|byte| *byte == 0).collect();
    let mut stashes = Vec::new();
    for record in fields.chunks(3) {
        if record.len() == 1 && record[0].is_empty() {
            continue;
        }
        if record.len() != 3 {
            return Err(flow_like_types::anyhow!(
                "Git returned an invalid stash record"
            ));
        }
        stashes.push(LocalGitStash {
            reference: String::from_utf8_lossy(record[0]).into_owned(),
            commit: String::from_utf8_lossy(record[1]).into_owned(),
            message: String::from_utf8_lossy(record[2]).into_owned(),
        });
    }
    Ok(stashes)
}

#[cfg(feature = "execute")]
fn reset(
    path: &Path,
    revision: &str,
    mode: &str,
    allow_destructive: bool,
) -> flow_like_types::Result<String> {
    let mode = match mode {
        "soft" => "--soft",
        "mixed" => "--mixed",
        "hard" if allow_destructive => "--hard",
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
    let revision = commit_ref(path, revision)?;
    git::run(path, &["reset", mode, &revision, "--"])
}

#[cfg(feature = "execute")]
fn merge(path: &Path, revision: &str, fast_forward_only: bool) -> flow_like_types::Result<String> {
    let revision = commit_ref(path, revision)?;
    git::require_clean(path)?;
    git::run(
        path,
        &[
            "merge",
            "--no-edit",
            "--no-gpg-sign",
            if fast_forward_only {
                "--ff-only"
            } else {
                "--ff"
            },
            &revision,
        ],
    )
}

#[cfg(feature = "execute")]
fn remotes(path: &Path) -> flow_like_types::Result<Vec<LocalGitRemote>> {
    let names = git::run(path, &["remote"])?;
    names
        .lines()
        .map(|name| {
            git::validate_remote(name)?;
            Ok(LocalGitRemote {
                name: name.to_string(),
                fetch_urls: git::run(path, &["remote", "get-url", "--all", name])?
                    .lines()
                    .map(git::redact)
                    .collect(),
                push_urls: git::run(path, &["remote", "get-url", "--push", "--all", name])?
                    .lines()
                    .map(git::redact)
                    .collect(),
            })
        })
        .collect()
}

#[cfg(feature = "execute")]
fn create_tag(
    path: &Path,
    name: &str,
    revision: &str,
    message: &str,
) -> flow_like_types::Result<String> {
    git::validate_ref(name)?;
    git::run(path, &["check-ref-format", &format!("refs/tags/{name}")])?;
    let revision = commit_ref(path, revision)?;
    if message.is_empty() {
        git::run(
            path,
            &["-c", "tag.gpgSign=false", "tag", "--", name, &revision],
        )
    } else {
        git::run(
            path,
            &[
                "tag",
                "--no-sign",
                "--annotate",
                "--message",
                message,
                "--",
                name,
                &revision,
            ],
        )
    }
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
            Ok(LocalResult::output(git::run(path, &["merge", "--abort"])?))
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
            "List tags stored in the local repository. Fetch with tags enabled to refresh tags from a remote.",
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
            let output = git::run(path, &["tag", "--list", "--sort=refname"])?;
            let tags: Vec<&str> = output.lines().collect();
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
            git::validate_ref(&name)?;
            git::run(path, &["check-ref-format", &format!("refs/tags/{name}")])?;
            Ok(LocalResult::output(git::run(
                path,
                &["tag", "--delete", "--", &name],
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
            git::run(&path, &["init", "--initial-branch=main"]).unwrap();
            git::run(&path, &["config", "user.name", "FlowLike Test"]).unwrap();
            git::run(
                &path,
                &["config", "user.email", "flow-like@example.invalid"],
            )
            .unwrap();
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
        assert_eq!(
            git::run(&repo.0, &["cat-file", "-t", "refs/tags/v1"])
                .unwrap()
                .trim(),
            "commit"
        );
        assert_eq!(
            git::run(&repo.0, &["cat-file", "-t", "refs/tags/v2"])
                .unwrap()
                .trim(),
            "tag"
        );
        assert_eq!(commit_ref(&repo.0, "v1").unwrap(), first);
        assert!(create_tag(&repo.0, "v1", "HEAD", "").is_err());
        git::run(&repo.0, &["tag", "--delete", "--", "v1"]).unwrap();
        assert_eq!(
            git::run(&repo.0, &["tag", "--list", "--sort=refname"])
                .unwrap()
                .trim(),
            "v2"
        );
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
        git::run(&repo.0, &["merge", "--abort"]).unwrap();
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
        git::run(
            &repo.0,
            &[
                "remote",
                "add",
                "origin",
                "https://user:secret@example.invalid/repository.git",
            ],
        )
        .unwrap();
        let entries = remotes(&repo.0).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "origin");
        assert!(entries[0].fetch_urls[0].contains("example.invalid/repository.git"));
        assert!(!entries[0].fetch_urls[0].contains("secret"));
        assert!(!entries[0].push_urls[0].contains("secret"));
    }
}
