#[cfg(any(feature = "execute", test))]
use super::git;
use super::{
    provider::{GITHUB_PROVIDER_ID, GitHubProvider},
    repository,
};
#[cfg(feature = "execute")]
use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[derive(Clone, Copy)]
enum Operation {
    Fetch,
    Pull,
    Push,
    Sync,
    Init,
    AddRemote,
    SetRemoteUrl,
    RemoveRemote,
}

fn string(node: &mut Node, name: &str, title: &str, description: &str, default: &str) {
    node.add_input_pin(name, title, description, VariableType::String)
        .set_default_value(Some(json!(default)));
}
fn boolean(node: &mut Node, name: &str, title: &str, description: &str, default: bool) {
    node.add_input_pin(name, title, description, VariableType::Boolean)
        .set_default_value(Some(json!(default)));
}
fn definition(operation: Operation) -> Node {
    let (id, title, description, script) = match operation {
        Operation::Fetch => (
            "data_github_fetch_repo",
            "Fetch Repository",
            "Download remote branches and tags into a local repository without changing checked-out files.",
            "fetchRepo",
        ),
        Operation::Pull => (
            "data_github_pull_repo",
            "Pull Repository",
            "Fetch and fast-forward the current branch of a clean local repository. Divergent history produces an error.",
            "pullRepo",
        ),
        Operation::Push => (
            "data_github_push_repo",
            "Push Repository",
            "Push the current local branch to GitHub. Does not force-push or delete remote branches.",
            "pushRepo",
        ),
        Operation::Sync => (
            "data_github_sync_repo",
            "Sync Repository",
            "Clone a missing local working tree, or fetch and fast-forward an existing clone of the same GitHub repository. Local changes and divergent history produce an error.",
            "syncRepo",
        ),
        Operation::Init => (
            "data_github_init_repo",
            "Init Repository",
            "Initialize a new local Git working tree at the supplied directory. Refuses an existing repository.",
            "initRepo",
        ),
        Operation::AddRemote => (
            "data_github_add_repo_remote",
            "Add Repository Remote",
            "Add a named HTTPS remote to a local repository without storing credentials.",
            "addRepoRemote",
        ),
        Operation::SetRemoteUrl => (
            "data_github_set_repo_remote_url",
            "Set Repository Remote URL",
            "Change a named remote's fetch URL. A separately configured push URL is left unchanged.",
            "setRepoRemoteUrl",
        ),
        Operation::RemoveRemote => (
            "data_github_remove_repo_remote",
            "Remove Repository Remote",
            "Remove a remote and its tracking references from the local repository. Does not delete the remote repository.",
            "removeRepoRemote",
        ),
    };
    let mut node = repository::node(id, title, description, script);
    if matches!(operation, Operation::Sync | Operation::Init)
        && let Some(pin) = node.pins.values_mut().find(|pin| pin.name == "repository")
    {
        pin.description = "Local FlowPath for the exact repository directory, not its parent. The directory may be missing. Requires Git on the runtime host.".into();
    }
    if matches!(
        operation,
        Operation::Fetch | Operation::Pull | Operation::Push | Operation::Sync
    ) {
        node.add_input_pin(
            "provider",
            "Provider",
            "GitHub authentication for this operation",
            VariableType::Struct,
        )
        .set_schema::<GitHubProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
    }
    if !matches!(operation, Operation::Sync | Operation::Init) {
        string(
            &mut node,
            "remote",
            "Remote",
            "Configured remote name",
            "origin",
        );
    }
    match operation {
        Operation::Fetch => {
            boolean(
                &mut node,
                "prune",
                "Prune",
                "Remove stale remote-tracking references",
                false,
            );
            boolean(
                &mut node,
                "tags",
                "All Tags",
                "Fetch all tags in addition to configured branches",
                false,
            );
            boolean(
                &mut node,
                "unshallow",
                "Unshallow",
                "Fetch complete history from a complete remote; fails if this clone is already complete",
                false,
            );
        }
        Operation::Pull | Operation::Push => {
            string(
                &mut node,
                "branch",
                "Remote Branch",
                "Remote branch name; empty uses the current branch's upstream, or current branch name if no upstream is configured",
                "",
            );
            if matches!(operation, Operation::Push) {
                boolean(
                    &mut node,
                    "set_upstream",
                    "Set Upstream",
                    "Record tracking information for the pushed branch",
                    true,
                );
                boolean(
                    &mut node,
                    "follow_tags",
                    "Follow Tags",
                    "Also push reachable annotated tags missing from the remote",
                    false,
                );
            }
        }
        Operation::Sync => {
            string(&mut node, "owner", "Owner", "GitHub repository owner", "");
            string(
                &mut node,
                "repo",
                "Repository Name",
                "GitHub repository name",
                "",
            );
            string(
                &mut node,
                "branch",
                "Branch",
                "Branch to clone or switch to; empty keeps the current branch or clones the default branch",
                "",
            );
            node.add_input_pin(
                "depth",
                "Depth",
                "Clone depth when creating the repository; 0 clones full history",
                VariableType::Integer,
            )
            .set_default_value(Some(json!(1)));
            boolean(
                &mut node,
                "prune",
                "Prune",
                "Remove stale origin tracking references when updating",
                false,
            );
        }
        Operation::Init => string(
            &mut node,
            "branch",
            "Initial Branch",
            "Initial branch name for the new repository",
            "main",
        ),
        Operation::AddRemote | Operation::SetRemoteUrl => string(
            &mut node,
            "url",
            "URL",
            "HTTPS repository URL without embedded credentials",
            "",
        ),
        Operation::RemoveRemote => {}
    }
    node
}

#[cfg(any(feature = "execute", test))]
#[derive(Default)]
struct Inputs {
    remote: String,
    branch: String,
    owner: String,
    repo: String,
    url: String,
    depth: i64,
    prune: bool,
    tags: bool,
    unshallow: bool,
    set_upstream: bool,
    follow_tags: bool,
}

#[cfg(feature = "execute")]
async fn execute(
    operation: Operation,
    context: &mut ExecutionContext,
) -> flow_like_types::Result<()> {
    repository::begin(context).await?;
    let path: FlowPath = context.evaluate_pin("repository").await?;
    let result = async {
        let local = repository::local_path(context, &path).await?;
        let provider = if matches!(
            operation,
            Operation::Fetch | Operation::Pull | Operation::Push | Operation::Sync
        ) {
            Some(context.evaluate_pin::<GitHubProvider>("provider").await?)
        } else {
            None
        };
        let mut inputs = Inputs::default();
        if !matches!(operation, Operation::Sync | Operation::Init) {
            inputs.remote = context.evaluate_pin("remote").await?;
        }
        if matches!(
            operation,
            Operation::Pull | Operation::Push | Operation::Sync | Operation::Init
        ) {
            inputs.branch = context.evaluate_pin("branch").await?;
        }
        match operation {
            Operation::Fetch => {
                inputs.prune = context.evaluate_pin("prune").await?;
                inputs.tags = context.evaluate_pin("tags").await?;
                inputs.unshallow = context.evaluate_pin("unshallow").await?;
            }
            Operation::Push => {
                inputs.set_upstream = context.evaluate_pin("set_upstream").await?;
                inputs.follow_tags = context.evaluate_pin("follow_tags").await?;
            }
            Operation::Sync => {
                inputs.owner = context.evaluate_pin("owner").await?;
                inputs.repo = context.evaluate_pin("repo").await?;
                inputs.depth = context.evaluate_pin("depth").await?;
                inputs.prune = context.evaluate_pin("prune").await?;
            }
            Operation::AddRemote | Operation::SetRemoteUrl => {
                inputs.url = context.evaluate_pin("url").await?
            }
            _ => {}
        }
        flow_like_types::tokio::task::spawn_blocking(move || {
            perform(operation, &local, provider.as_ref(), &inputs)
        })
        .await?
    }
    .await;
    repository::finish(context, result, &path).await
}

#[cfg(any(feature = "execute", test))]
fn branch_name(path: &std::path::Path, name: &str) -> flow_like_types::Result<()> {
    git::validate_ref(name)?;
    git::run(path, &["check-ref-format", "--branch", name])?;
    // Git expands @{-1}; node inputs always name a branch literally.
    if name.contains("@{") {
        flow_like_types::bail!("Use a literal branch name");
    }
    Ok(())
}

#[cfg(any(feature = "execute", test))]
fn current_branch(path: &std::path::Path) -> flow_like_types::Result<String> {
    let branch = git::run(path, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let branch = branch.trim().to_owned();
    branch_name(path, &branch)?;
    Ok(branch)
}

#[cfg(any(feature = "execute", test))]
fn remote_branch(
    path: &std::path::Path,
    remote: &str,
    requested: &str,
) -> flow_like_types::Result<String> {
    if !requested.is_empty() {
        branch_name(path, requested)?;
        return Ok(requested.to_owned());
    }
    let current = current_branch(path)?;
    let tracking_remote = git::run(
        path,
        &["config", "--get", &format!("branch.{current}.remote")],
    )
    .unwrap_or_default();
    if tracking_remote.trim() == remote {
        let merge = git::run(
            path,
            &["config", "--get", &format!("branch.{current}.merge")],
        )?;
        if let Some(branch) = merge.trim().strip_prefix("refs/heads/") {
            branch_name(path, branch)?;
            return Ok(branch.to_owned());
        }
    }
    Ok(current)
}

#[cfg(any(feature = "execute", test))]
fn credential_free_url(value: &str) -> flow_like_types::Result<flow_like_types::reqwest::Url> {
    if value
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        flow_like_types::bail!("Repository URLs cannot contain whitespace or control characters");
    }
    let url = flow_like_types::reqwest::Url::parse(value)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        flow_like_types::bail!(
            "Use an HTTPS repository URL without credentials, a query, or a fragment"
        );
    }
    Ok(url)
}

#[cfg(any(feature = "execute", test))]
fn same_repository(actual: &str, expected: &str) -> flow_like_types::Result<bool> {
    let mut actual = flow_like_types::reqwest::Url::parse(actual.trim())?;
    actual
        .set_username("")
        .map_err(|_| flow_like_types::anyhow!("Invalid remote URL"))?;
    actual
        .set_password(None)
        .map_err(|_| flow_like_types::anyhow!("Invalid remote URL"))?;
    let expected = credential_free_url(expected)?;
    let normalize = |url: &flow_like_types::reqwest::Url| {
        url.path()
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .to_ascii_lowercase()
    };
    Ok(actual.scheme() == expected.scheme()
        && actual.host_str() == expected.host_str()
        && actual.port_or_known_default() == expected.port_or_known_default()
        && actual.query().is_none()
        && actual.fragment().is_none()
        && normalize(&actual) == normalize(&expected))
}

#[cfg(any(feature = "execute", test))]
fn perform(
    operation: Operation,
    path: &std::path::Path,
    provider: Option<&GitHubProvider>,
    inputs: &Inputs,
) -> flow_like_types::Result<String> {
    perform_with(operation, path, provider, inputs, &git::run_network)
}

#[cfg(any(feature = "execute", test))]
fn perform_with(
    operation: Operation,
    path: &std::path::Path,
    provider: Option<&GitHubProvider>,
    inputs: &Inputs,
    network: &impl Fn(&std::path::Path, &[&str], &GitHubProvider) -> flow_like_types::Result<String>,
) -> flow_like_types::Result<String> {
    if matches!(operation, Operation::Init) {
        if inputs.branch.is_empty() {
            flow_like_types::bail!("Initial branch is required");
        }
        if std::fs::symlink_metadata(path.join(".git")).is_ok()
            || (path.exists()
                && (git::ensure_repository(path).is_ok()
                    || git::run(path, &["rev-parse", "--is-bare-repository"])
                        .is_ok_and(|value| value.trim() == "true")))
        {
            flow_like_types::bail!("The target already contains a Git repository");
        }
        let parent = existing_parent(path)?;
        branch_name(parent, &inputs.branch)?;
        let target = path
            .to_str()
            .ok_or_else(|| flow_like_types::anyhow!("Repository path must be UTF-8"))?;
        return git::run(
            parent,
            &["init", "--initial-branch", &inputs.branch, "--", target],
        );
    }
    if matches!(operation, Operation::Sync) {
        return sync(
            path,
            provider.ok_or_else(|| flow_like_types::anyhow!("Provider is required"))?,
            inputs,
            network,
        );
    }
    git::ensure_repository(path)?;
    git::validate_remote(&inputs.remote)?;
    match operation {
        Operation::AddRemote | Operation::SetRemoteUrl => {
            credential_free_url(&inputs.url)?;
            let command = if matches!(operation, Operation::AddRemote) {
                "add"
            } else {
                "set-url"
            };
            git::run(
                path,
                &["remote", command, "--", &inputs.remote, &inputs.url],
            )
        }
        Operation::RemoveRemote => git::run(path, &["remote", "remove", &inputs.remote]),
        Operation::Fetch | Operation::Pull | Operation::Push => {
            let provider =
                provider.ok_or_else(|| flow_like_types::anyhow!("Provider is required"))?;
            let mut args = Vec::new();
            let branch;
            let refspec;
            match operation {
                Operation::Fetch => {
                    args.push("fetch");
                    args.push(if inputs.prune {
                        "--prune"
                    } else {
                        "--no-prune"
                    });
                    if inputs.tags {
                        args.push("--tags");
                    }
                    if inputs.unshallow {
                        args.push("--unshallow");
                    }
                    args.extend(["--", inputs.remote.as_str()]);
                }
                Operation::Pull => {
                    git::require_clean(path)?;
                    current_branch(path)?;
                    branch = remote_branch(path, &inputs.remote, &inputs.branch)?;
                    args.extend([
                        "pull",
                        "--ff-only",
                        "--no-rebase",
                        "--no-autostash",
                        "--",
                        inputs.remote.as_str(),
                        branch.as_str(),
                    ]);
                }
                Operation::Push => {
                    current_branch(path)?;
                    branch = remote_branch(path, &inputs.remote, &inputs.branch)?;
                    refspec = format!("HEAD:refs/heads/{branch}");
                    args.push("push");
                    if inputs.set_upstream {
                        args.push("--set-upstream");
                    }
                    if inputs.follow_tags {
                        args.push("--follow-tags");
                    }
                    args.extend(["--", inputs.remote.as_str(), refspec.as_str()]);
                }
                _ => unreachable!(),
            }
            network(path, &args, provider)
        }
        _ => unreachable!(),
    }
}

#[cfg(any(feature = "execute", test))]
fn existing_parent(path: &std::path::Path) -> flow_like_types::Result<&std::path::Path> {
    let mut parent = path
        .parent()
        .ok_or_else(|| flow_like_types::anyhow!("Repository path needs a parent directory"))?;
    while !parent.exists() {
        parent = parent
            .parent()
            .ok_or_else(|| flow_like_types::anyhow!("Repository path needs an existing parent"))?;
    }
    Ok(parent)
}

#[cfg(any(feature = "execute", test))]
fn sync(
    path: &std::path::Path,
    provider: &GitHubProvider,
    inputs: &Inputs,
    network: &impl Fn(&std::path::Path, &[&str], &GitHubProvider) -> flow_like_types::Result<String>,
) -> flow_like_types::Result<String> {
    repository::validate_repo_component(&inputs.owner)?;
    repository::validate_repo_component(&inputs.repo)?;
    if inputs.depth < 0 {
        flow_like_types::bail!("Depth must be zero or positive");
    }
    let expected = provider.clone_url(&inputs.owner, &inputs.repo);
    credential_free_url(&expected)?;
    if !path.exists() {
        let parent = existing_parent(path)?;
        if !inputs.branch.is_empty() {
            branch_name(parent, &inputs.branch)?;
        }
        let depth = inputs.depth.to_string();
        let mut args = vec!["clone"];
        if inputs.depth > 0 {
            args.extend(["--depth", depth.as_str()]);
        }
        if !inputs.branch.is_empty() {
            args.extend(["--branch", inputs.branch.as_str()]);
        }
        args.extend([
            "--",
            expected.as_str(),
            path.to_str()
                .ok_or_else(|| flow_like_types::anyhow!("Repository path must be UTF-8"))?,
        ]);
        return network(parent, &args, provider);
    }
    git::ensure_repository(path)?;
    git::require_clean(path)?;
    let origin = git::run(path, &["config", "--get-all", "remote.origin.url"])?;
    let urls = origin.lines().collect::<Vec<_>>();
    if urls.len() != 1 || !same_repository(urls[0], &expected)? {
        flow_like_types::bail!(
            "Existing repository origin does not match Owner and Repository Name"
        );
    }
    let branch = if inputs.branch.is_empty() {
        remote_branch(path, "origin", "")?
    } else {
        branch_name(path, &inputs.branch)?;
        inputs.branch.clone()
    };
    let local_branch = if inputs.branch.is_empty() {
        current_branch(path)?
    } else {
        branch.clone()
    };
    // Fetch the selected branch explicitly, including branches omitted by a shallow clone's refspec.
    let refspec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
    let mut args = vec!["fetch"];
    args.push(if inputs.prune {
        "--prune"
    } else {
        "--no-prune"
    });
    args.extend(["--", "origin", refspec.as_str()]);
    let fetched = network(path, &args, provider)?;
    let remote_ref = format!("refs/remotes/origin/{branch}");
    let local_ref = format!("refs/heads/{local_branch}");
    let exists = git::run(path, &["show-ref", "--verify", "--quiet", &local_ref]).is_ok();
    if exists {
        // Refuse divergence before switching the user's current branch.
        if git::run(
            path,
            &["merge-base", "--is-ancestor", &local_ref, &remote_ref],
        )
        .is_err()
            && git::run(
                path,
                &["merge-base", "--is-ancestor", &remote_ref, &local_ref],
            )
            .is_err()
        {
            flow_like_types::bail!(
                "Local branch cannot be fast-forwarded to origin/{branch}; reconcile local commits first"
            );
        }
        git::run(path, &["switch", "--no-guess", &local_branch])?;
    } else {
        // Shallow clones often track only the initial branch. Git needs a fetch mapping to set up tracking.
        let fetch_specs =
            git::run(path, &["config", "--get-all", "remote.origin.fetch"]).unwrap_or_default();
        let selected = format!("refs/heads/{branch}:refs/remotes/origin/{branch}");
        if !fetch_specs.lines().any(|spec| {
            let spec = spec.trim_start_matches('+');
            spec == "refs/heads/*:refs/remotes/origin/*" || spec == selected
        }) {
            git::run(
                path,
                &["remote", "set-branches", "--add", "origin", &branch],
            )?;
        }
        git::run(
            path,
            &[
                "switch",
                "--create",
                &local_branch,
                "--track",
                &format!("origin/{branch}"),
            ],
        )?;
    }
    let merged = git::run(path, &["merge", "--ff-only", "--no-edit", &remote_ref])?;
    Ok(format!("{fetched}{merged}"))
}

#[crate::register_node]
#[derive(Default)]
pub struct FetchGitHubRepoNode {}

#[async_trait]
impl NodeLogic for FetchGitHubRepoNode {
    fn get_node(&self) -> Node {
        definition(Operation::Fetch)
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        execute(Operation::Fetch, context).await
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
pub struct PullGitHubRepoNode {}

#[async_trait]
impl NodeLogic for PullGitHubRepoNode {
    fn get_node(&self) -> Node {
        definition(Operation::Pull)
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        execute(Operation::Pull, context).await
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
pub struct PushGitHubRepoNode {}

#[async_trait]
impl NodeLogic for PushGitHubRepoNode {
    fn get_node(&self) -> Node {
        definition(Operation::Push)
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        execute(Operation::Push, context).await
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
pub struct SyncGitHubRepoNode {}

#[async_trait]
impl NodeLogic for SyncGitHubRepoNode {
    fn get_node(&self) -> Node {
        definition(Operation::Sync)
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        execute(Operation::Sync, context).await
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
pub struct InitGitHubRepoNode {}

#[async_trait]
impl NodeLogic for InitGitHubRepoNode {
    fn get_node(&self) -> Node {
        definition(Operation::Init)
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        execute(Operation::Init, context).await
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
pub struct AddGitHubRepoRemoteNode {}

#[async_trait]
impl NodeLogic for AddGitHubRepoRemoteNode {
    fn get_node(&self) -> Node {
        definition(Operation::AddRemote)
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        execute(Operation::AddRemote, context).await
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
pub struct SetGitHubRepoRemoteUrlNode {}

#[async_trait]
impl NodeLogic for SetGitHubRepoRemoteUrlNode {
    fn get_node(&self) -> Node {
        definition(Operation::SetRemoteUrl)
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        execute(Operation::SetRemoteUrl, context).await
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
pub struct RemoveGitHubRepoRemoteNode {}

#[async_trait]
impl NodeLogic for RemoveGitHubRepoRemoteNode {
    fn get_node(&self) -> Node {
        definition(Operation::RemoveRemote)
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        execute(Operation::RemoveRemote, context).await
    }
    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    struct Fixture {
        root: PathBuf,
        seed: PathBuf,
        remote: PathBuf,
        checkout: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("flow-like-network-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            let seed = root.join("seed");
            let remote = root.join("remote.git");
            let checkout = root.join("checkout");
            git::run(
                &root,
                &["init", "--initial-branch=main", seed.to_str().unwrap()],
            )
            .unwrap();
            identity(&seed);
            write_commit(&seed, "file.txt", "initial", "Initial");
            git::run(
                &root,
                &[
                    "clone",
                    "--bare",
                    seed.to_str().unwrap(),
                    remote.to_str().unwrap(),
                ],
            )
            .unwrap();
            Self {
                root,
                seed,
                remote,
                checkout,
            }
        }
        // Only the transport destination changes. Production command options and refspecs run against real Git repositories.
        fn network(
            &self,
            path: &Path,
            args: &[&str],
            provider: &GitHubProvider,
        ) -> flow_like_types::Result<String> {
            let mut owned: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
            let index = owned.iter().position(|arg| arg == "--").unwrap() + 1;
            owned[index] = format!("file://{}", self.remote.display());
            let output = git::run(path, &owned.iter().map(String::as_str).collect::<Vec<_>>())?;
            if args[0] == "clone" {
                let target = Path::new(args.last().unwrap());
                git::run(
                    target,
                    &[
                        "remote",
                        "set-url",
                        "origin",
                        &provider.clone_url("owner", "repo"),
                    ],
                )?;
                identity(target);
            }
            Ok(output)
        }
        fn run(&self, operation: Operation, inputs: &Inputs) -> flow_like_types::Result<String> {
            perform_with(
                operation,
                &self.checkout,
                Some(&provider()),
                inputs,
                &|path, args, provider| self.network(path, args, provider),
            )
        }
        fn publish(&self, branch: &str) {
            git::run(&self.seed, &["push", self.remote.to_str().unwrap(), branch]).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    fn identity(path: &Path) {
        git::run(path, &["config", "user.name", "Git Node Test"]).unwrap();
        git::run(path, &["config", "user.email", "git-node@example.invalid"]).unwrap();
    }
    fn write_commit(path: &Path, file: &str, content: &str, message: &str) {
        fs::write(path.join(file), content).unwrap();
        git::run(path, &["add", "--", file]).unwrap();
        git::run(path, &["commit", "-m", message]).unwrap();
    }
    fn head(path: &Path) -> String {
        git::run(path, &["rev-parse", "HEAD"]).unwrap()
    }
    fn provider() -> GitHubProvider {
        GitHubProvider {
            provider_id: "github".into(),
            access_token: "test-token".into(),
            base_url: "https://api.github.com".into(),
        }
    }
    fn inputs() -> Inputs {
        Inputs {
            remote: "origin".into(),
            owner: "owner".into(),
            repo: "repo".into(),
            depth: 1,
            ..Default::default()
        }
    }

    #[test]
    fn sync_clones_updates_and_switches_a_branch_missing_from_shallow_clone() {
        let fixture = Fixture::new();
        fixture.run(Operation::Sync, &inputs()).unwrap();
        assert_eq!(head(&fixture.checkout), head(&fixture.seed));
        fixture.run(Operation::Sync, &inputs()).unwrap();
        write_commit(&fixture.seed, "file.txt", "updated", "Update");
        fixture.publish("main");
        fixture.run(Operation::Sync, &inputs()).unwrap();
        assert_eq!(
            fs::read_to_string(fixture.checkout.join("file.txt")).unwrap(),
            "updated"
        );
        git::run(&fixture.seed, &["switch", "-c", "topic"]).unwrap();
        write_commit(&fixture.seed, "topic.txt", "topic", "Topic");
        fixture.publish("topic");
        fixture
            .run(
                Operation::Sync,
                &Inputs {
                    branch: "topic".into(),
                    ..inputs()
                },
            )
            .unwrap();
        assert_eq!(current_branch(&fixture.checkout).unwrap(), "topic");
        assert_eq!(head(&fixture.checkout), head(&fixture.seed));
        assert_eq!(
            git::run(
                &fixture.checkout,
                &["rev-parse", "--abbrev-ref", "@{upstream}"]
            )
            .unwrap()
            .trim(),
            "origin/topic"
        );
        assert!(
            !fs::read_to_string(fixture.checkout.join(".git/config"))
                .unwrap()
                .contains("test-token")
        );
    }

    #[test]
    fn sync_preserves_dirty_divergent_and_mismatched_worktrees() {
        let fixture = Fixture::new();
        fixture.run(Operation::Sync, &inputs()).unwrap();
        let original = head(&fixture.checkout);
        fs::write(fixture.checkout.join("untracked.txt"), "keep me").unwrap();
        assert!(fixture.run(Operation::Sync, &inputs()).is_err());
        assert_eq!(head(&fixture.checkout), original);
        assert_eq!(
            fs::read_to_string(fixture.checkout.join("untracked.txt")).unwrap(),
            "keep me"
        );
        fs::remove_file(fixture.checkout.join("untracked.txt")).unwrap();
        write_commit(&fixture.checkout, "local.txt", "local", "Local");
        let local = head(&fixture.checkout);
        // A branch ahead of origin is already up to date and keeps its commits.
        fixture.run(Operation::Sync, &inputs()).unwrap();
        assert_eq!(head(&fixture.checkout), local);
        write_commit(&fixture.seed, "remote.txt", "remote", "Remote");
        fixture.publish("main");
        assert!(fixture.run(Operation::Sync, &inputs()).is_err());
        assert_eq!(head(&fixture.checkout), local);
        assert_eq!(current_branch(&fixture.checkout).unwrap(), "main");
        assert!(
            fixture
                .run(
                    Operation::Sync,
                    &Inputs {
                        repo: "different".into(),
                        ..inputs()
                    }
                )
                .is_err()
        );
        assert_eq!(head(&fixture.checkout), local);
    }

    #[test]
    fn pull_fast_forwards_push_preserves_remote_commits_and_fetch_can_unshallow() {
        let fixture = Fixture::new();
        fixture.run(Operation::Sync, &inputs()).unwrap();
        write_commit(&fixture.seed, "file.txt", "updated", "Update");
        fixture.publish("main");
        fixture.run(Operation::Pull, &inputs()).unwrap();
        assert_eq!(head(&fixture.checkout), head(&fixture.seed));
        fixture
            .run(
                Operation::Fetch,
                &Inputs {
                    unshallow: true,
                    ..inputs()
                },
            )
            .unwrap();
        assert_eq!(
            git::run(&fixture.checkout, &["rev-parse", "--is-shallow-repository"])
                .unwrap()
                .trim(),
            "false"
        );
        write_commit(&fixture.checkout, "local.txt", "local", "Local");
        fixture.run(Operation::Push, &inputs()).unwrap();
        assert_eq!(head(&fixture.remote), head(&fixture.checkout));
        git::run(
            &fixture.seed,
            &[
                "pull",
                "--ff-only",
                fixture.remote.to_str().unwrap(),
                "main",
            ],
        )
        .unwrap();
        write_commit(&fixture.seed, "remote.txt", "remote", "Remote");
        fixture.publish("main");
        let remote = head(&fixture.remote);
        write_commit(&fixture.checkout, "divergent.txt", "keep", "Divergent");
        assert!(fixture.run(Operation::Push, &inputs()).is_err());
        assert_eq!(head(&fixture.remote), remote);
        assert!(fixture.run(Operation::Pull, &inputs()).is_err());
    }

    #[test]
    fn init_and_remote_lifecycle_reject_reinitialization_and_credentials() {
        let fixture = Fixture::new();
        perform(
            Operation::Init,
            &fixture.checkout,
            None,
            &Inputs {
                branch: "main".into(),
                ..inputs()
            },
        )
        .unwrap();
        assert!(
            perform(
                Operation::Init,
                &fixture.checkout,
                None,
                &Inputs {
                    branch: "main".into(),
                    ..inputs()
                }
            )
            .is_err()
        );
        assert!(
            perform(
                Operation::Init,
                &fixture.remote,
                None,
                &Inputs {
                    branch: "main".into(),
                    ..inputs()
                }
            )
            .is_err()
        );
        let remote = Inputs {
            url: "https://github.com/owner/repo.git".into(),
            ..inputs()
        };
        perform(Operation::AddRemote, &fixture.checkout, None, &remote).unwrap();
        let changed = Inputs {
            url: "https://github.com/owner/other.git".into(),
            ..inputs()
        };
        perform(Operation::SetRemoteUrl, &fixture.checkout, None, &changed).unwrap();
        assert_eq!(
            git::run(&fixture.checkout, &["remote", "get-url", "origin"])
                .unwrap()
                .trim(),
            changed.url
        );
        assert!(
            perform(
                Operation::SetRemoteUrl,
                &fixture.checkout,
                None,
                &Inputs {
                    url: "https://secret@github.com/owner/repo".into(),
                    ..inputs()
                }
            )
            .is_err()
        );
        perform(Operation::RemoveRemote, &fixture.checkout, None, &remote).unwrap();
        assert!(
            git::run(&fixture.checkout, &["remote"])
                .unwrap()
                .trim()
                .is_empty()
        );
    }

    #[test]
    fn sync_refuses_an_existing_non_repository_and_validates_repository_identity() {
        let fixture = Fixture::new();
        fs::create_dir(&fixture.checkout).unwrap();
        fs::write(fixture.checkout.join("keep.txt"), "untouched").unwrap();
        assert!(fixture.run(Operation::Sync, &inputs()).is_err());
        assert_eq!(
            fs::read_to_string(fixture.checkout.join("keep.txt")).unwrap(),
            "untouched"
        );
        assert!(
            same_repository(
                "https://legacy-token@github.com/OWNER/repo.git",
                "https://github.com/owner/repo"
            )
            .unwrap()
        );
        assert!(
            !same_repository(
                "https://github.com/owner/other",
                "https://github.com/owner/repo"
            )
            .unwrap()
        );
        assert!(
            !same_repository(
                "https://other.example/owner/repo",
                "https://github.com/owner/repo"
            )
            .unwrap()
        );
        assert!(credential_free_url("file:///tmp/repo").is_err());
        assert!(credential_free_url("https://github.com/owner/\nrepo").is_err());
    }
}
