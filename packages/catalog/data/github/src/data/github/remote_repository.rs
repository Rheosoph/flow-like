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
            "Download remote branches and tags into a repository without changing checked-out files.",
            "fetchRepo",
        ),
        Operation::Pull => (
            "data_github_pull_repo",
            "Pull Repository",
            "Fetch and fast-forward the current branch of a clean repository. Divergent history produces an error.",
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
            "Clone a missing working tree, or fetch and fast-forward an existing clone of the same GitHub repository. Uncommitted changes and divergent history produce an error.",
            "syncRepo",
        ),
        Operation::Init => (
            "data_github_init_repo",
            "Init Repository",
            "Initialize a new Git working tree at the supplied directory. Refuses an existing repository.",
            "initRepo",
        ),
        Operation::AddRemote => (
            "data_github_add_repo_remote",
            "Add Repository Remote",
            "Add a named HTTPS remote to a repository without storing credentials.",
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
            "Remove a remote and its tracking references from the checkout. Does not delete the remote repository.",
            "removeRepoRemote",
        ),
    };
    let mut node = repository::node(id, title, description, script);
    if matches!(operation, Operation::Sync | Operation::Init)
        && let Some(pin) = node.pins.values_mut().find(|pin| pin.name == "repository")
    {
        pin.description = "FlowPath for the exact repository directory in a local, cloud, or memory store. The directory may be missing.".into();
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
        let provider = if matches!(
            operation,
            Operation::Fetch | Operation::Pull | Operation::Push | Operation::Sync
        ) {
            let provider: GitHubProvider = context.evaluate_pin("provider").await?;
            git::ensure_origin_allowed(context.execution_environment(), &provider).await?;
            Some(provider)
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
        let workspace = repository::Workspace::open(context, &path).await?;
        let (workspace, result) = flow_like_types::tokio::task::spawn_blocking(move || {
            let result = perform(operation, workspace.path(), provider.as_ref(), &inputs);
            (workspace, result)
        })
        .await
        .map_err(|error| flow_like_types::anyhow!("Git operation task failed: {error}"))?;
        workspace.finish(result).await
    }
    .await;
    repository::finish(context, result, &path).await
}

#[cfg(any(feature = "execute", test))]
fn branch_name(_path: &std::path::Path, name: &str) -> flow_like_types::Result<()> {
    // Node inputs always name a branch literally; @{-1} style shorthand is rejected here.
    git::validate_branch_name(name)
}

#[cfg(any(feature = "execute", test))]
fn current_branch(path: &std::path::Path) -> flow_like_types::Result<String> {
    let repo = git::open(path)?;
    let branch = git::head_branch(&repo)
        .ok_or_else(|| flow_like_types::anyhow!("HEAD is detached; switch to a branch first"))?;
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
    let config = git::open(path)?.config()?;
    let tracking_remote = config
        .get_string(&format!("branch.{current}.remote"))
        .unwrap_or_default();
    if tracking_remote.trim() == remote {
        let merge = config.get_string(&format!("branch.{current}.merge"))?;
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
    match provider {
        Some(provider) => perform_with(
            operation,
            path,
            Some(provider),
            inputs,
            Some(&git::ProviderNetwork(provider)),
        ),
        None => perform_with(operation, path, None, inputs, None),
    }
}

#[cfg(any(feature = "execute", test))]
fn perform_with(
    operation: Operation,
    path: &std::path::Path,
    provider: Option<&GitHubProvider>,
    inputs: &Inputs,
    network: Option<&dyn git::Network>,
) -> flow_like_types::Result<String> {
    let network = || network.ok_or_else(|| flow_like_types::anyhow!("Provider is required"));
    if matches!(operation, Operation::Init) {
        if inputs.branch.is_empty() {
            flow_like_types::bail!("Initial branch is required");
        }
        if std::fs::symlink_metadata(path.join(".git")).is_ok()
            || (path.exists() && git::open(path).is_ok())
        {
            flow_like_types::bail!("The target already contains a Git repository");
        }
        let parent = existing_parent(path)?;
        branch_name(parent, &inputs.branch)?;
        let repo = git2::Repository::init_opts(
            path,
            git2::RepositoryInitOptions::new()
                .initial_head(&inputs.branch)
                .mkpath(true),
        )?;
        return Ok(format!(
            "Initialized empty Git repository in {}\n",
            repo.path().display()
        ));
    }
    if matches!(operation, Operation::Sync) {
        return sync(
            path,
            provider.ok_or_else(|| flow_like_types::anyhow!("Provider is required"))?,
            inputs,
            network()?,
        );
    }
    git::ensure_repository(path)?;
    git::validate_remote(&inputs.remote)?;
    let repo = git::open(path)?;
    match operation {
        Operation::AddRemote => {
            credential_free_url(&inputs.url)?;
            repo.remote(&inputs.remote, &inputs.url)?;
            Ok(format!(
                "Added remote '{}' ({})\n",
                inputs.remote, inputs.url
            ))
        }
        Operation::SetRemoteUrl => {
            credential_free_url(&inputs.url)?;
            repo.find_remote(&inputs.remote)
                .map_err(|_| flow_like_types::anyhow!("No such remote: '{}'", inputs.remote))?;
            repo.remote_set_url(&inputs.remote, &inputs.url)?;
            Ok(format!(
                "Set the URL of remote '{}' to {}\n",
                inputs.remote, inputs.url
            ))
        }
        Operation::RemoveRemote => {
            repo.remote_delete(&inputs.remote)?;
            Ok(format!("Removed remote '{}'\n", inputs.remote))
        }
        Operation::Fetch => {
            let network = network()?;
            let depth = (inputs.unshallow && repo.is_shallow()).then_some(i32::MAX);
            network.fetch(
                &repo,
                &git::FetchRequest {
                    remote: &inputs.remote,
                    refspecs: Vec::new(),
                    prune: inputs.prune,
                    tags: inputs.tags,
                    depth,
                },
            )
        }
        Operation::Pull => {
            let network = network()?;
            git::require_clean(path)?;
            current_branch(path)?;
            let branch = remote_branch(path, &inputs.remote, &inputs.branch)?;
            let tracking = format!("refs/remotes/{}/{branch}", inputs.remote);
            let fetched = network.fetch(
                &repo,
                &git::FetchRequest {
                    remote: &inputs.remote,
                    refspecs: vec![format!("+refs/heads/{branch}:{tracking}")],
                    prune: false,
                    tags: false,
                    depth: None,
                },
            )?;
            // Like `git pull --ff-only`: never merges, rebases or autostashes.
            let merged = git::fast_forward(&repo, repo.refname_to_id(&tracking)?)?;
            Ok(format!("{fetched}{merged}"))
        }
        Operation::Push => {
            let network = network()?;
            let current = current_branch(path)?;
            let branch = remote_branch(path, &inputs.remote, &inputs.branch)?;
            let mut output = network.push(
                &repo,
                &git::PushRequest {
                    remote: &inputs.remote,
                    refspec: format!("refs/heads/{current}:refs/heads/{branch}"),
                    follow_tags: inputs.follow_tags,
                },
            )?;
            if inputs.set_upstream {
                let mut config = repo.config()?;
                config.set_str(&format!("branch.{current}.remote"), &inputs.remote)?;
                config.set_str(
                    &format!("branch.{current}.merge"),
                    &format!("refs/heads/{branch}"),
                )?;
                output.push_str(&format!(
                    "Branch '{current}' set up to track '{}/{branch}'.\n",
                    inputs.remote
                ));
            }
            Ok(output)
        }
        Operation::Sync | Operation::Init => unreachable!(),
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
    network: &dyn git::Network,
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
        let depth = (inputs.depth > 0)
            .then(|| i32::try_from(inputs.depth))
            .transpose()
            .map_err(|_| flow_like_types::anyhow!("Depth is too large"))?;
        network.clone_repo(
            &expected,
            path,
            (!inputs.branch.is_empty()).then_some(inputs.branch.as_str()),
            depth,
        )?;
        return Ok(format!("Cloned {expected}\n"));
    }
    git::ensure_repository(path)?;
    git::require_clean(path)?;
    let repo = git::open(path)?;
    let urls = git::config_values(&repo, "remote.origin.url")?;
    if urls.len() != 1 || !same_repository(&urls[0], &expected)? {
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
    let remote_ref = format!("refs/remotes/origin/{branch}");
    let fetched = network.fetch(
        &repo,
        &git::FetchRequest {
            remote: "origin",
            refspecs: vec![format!("+refs/heads/{branch}:{remote_ref}")],
            prune: inputs.prune,
            tags: false,
            depth: None,
        },
    )?;
    let remote_oid = repo.refname_to_id(&remote_ref)?;
    let local_ref = format!("refs/heads/{local_branch}");
    if let Ok(local_oid) = repo.refname_to_id(&local_ref) {
        // Refuse divergence before switching the user's current branch.
        if !git::is_ancestor(&repo, local_oid, remote_oid)?
            && !git::is_ancestor(&repo, remote_oid, local_oid)?
        {
            flow_like_types::bail!(
                "Local branch cannot be fast-forwarded to origin/{branch}; reconcile local commits first"
            );
        }
        git::checkout_commit(&repo, local_oid)?;
        repo.set_head(&local_ref)?;
    } else {
        // Shallow clones often track only the initial branch. Tracking needs a fetch mapping.
        let selected = format!("refs/heads/{branch}:refs/remotes/origin/{branch}");
        let specs: Vec<String> = repo
            .find_remote("origin")?
            .fetch_refspecs()?
            .iter()
            .flatten()
            .map(|spec| spec.trim_start_matches('+').to_owned())
            .collect();
        if !specs
            .iter()
            .any(|spec| spec == "refs/heads/*:refs/remotes/origin/*" || *spec == selected)
        {
            repo.remote_add_fetch("origin", &format!("+{selected}"))?;
        }
        let mut created = repo.branch(&local_branch, &repo.find_commit(remote_oid)?, false)?;
        created.set_upstream(Some(&format!("origin/{branch}")))?;
        git::checkout_commit(&repo, remote_oid)?;
        repo.set_head(&local_ref)?;
    }
    let merged = git::fast_forward(&repo, remote_oid)?;
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
    use super::git::testing::{LocalNetwork, TempDir, head, identity, init, write_commit};
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    struct Fixture {
        _dir: TempDir,
        seed: PathBuf,
        remote: PathBuf,
        checkout: PathBuf,
        network: LocalNetwork,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = TempDir::new("flow-like-network");
            let seed = dir.0.join("seed");
            let remote = dir.0.join("remote.git");
            let checkout = dir.0.join("checkout");
            init(&seed);
            write_commit(&seed, "file.txt", "initial", "Initial");
            git2::build::RepoBuilder::new()
                .bare(true)
                .clone(&format!("file://{}", seed.display()), &remote)
                .unwrap();
            let network = LocalNetwork {
                remote: remote.clone(),
            };
            Self {
                _dir: dir,
                seed,
                remote,
                checkout,
                network,
            }
        }
        fn run(&self, operation: Operation, inputs: &Inputs) -> flow_like_types::Result<String> {
            perform_with(
                operation,
                &self.checkout,
                Some(&provider()),
                inputs,
                Some(&self.network),
            )
        }
        /// Publishes a branch of the seed repository to the bare remote.
        fn publish(&self, branch: &str) {
            let seed = git::open(&self.seed).unwrap();
            git::push_with(
                &seed,
                &format!("file://{}", self.remote.display()),
                None,
                &[],
                &git::PushRequest {
                    remote: "origin",
                    refspec: format!("refs/heads/{branch}:refs/heads/{branch}"),
                    follow_tags: false,
                },
            )
            .unwrap();
        }
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
        let seed = git::open(&fixture.seed).unwrap();
        let tip = seed.head().unwrap().peel_to_commit().unwrap();
        seed.branch("topic", &tip, false).unwrap();
        seed.set_head("refs/heads/topic").unwrap();
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
        let checkout = git::open(&fixture.checkout).unwrap();
        let topic = checkout
            .find_branch("topic", git2::BranchType::Local)
            .unwrap();
        assert_eq!(
            topic.upstream().unwrap().name().unwrap().unwrap(),
            "origin/topic"
        );
        assert!(
            !fs::read_to_string(fixture.checkout.join(".git/config"))
                .unwrap()
                .contains("test-token")
        );
    }

    /// Records what a clone would be asked for; the local transport cannot serve shallow requests.
    #[derive(Default)]
    struct Recorder(std::cell::RefCell<Vec<(Option<String>, Option<i32>)>>);
    impl git::Network for Recorder {
        fn clone_repo(
            &self,
            _url: &str,
            _target: &Path,
            branch: Option<&str>,
            depth: Option<i32>,
        ) -> flow_like_types::Result<()> {
            self.0.borrow_mut().push((branch.map(str::to_owned), depth));
            Ok(())
        }
        fn fetch(
            &self,
            _repo: &git2::Repository,
            _request: &git::FetchRequest,
        ) -> flow_like_types::Result<String> {
            unreachable!()
        }
        fn push(
            &self,
            _repo: &git2::Repository,
            _request: &git::PushRequest,
        ) -> flow_like_types::Result<String> {
            unreachable!()
        }
    }

    #[test]
    fn sync_forwards_depth_and_branch_to_the_clone() {
        let fixture = Fixture::new();
        let recorder = Recorder::default();
        let run = |inputs: &Inputs| {
            perform_with(
                Operation::Sync,
                &fixture.checkout,
                Some(&provider()),
                inputs,
                Some(&recorder),
            )
        };
        run(&inputs()).unwrap();
        run(&Inputs {
            depth: 0,
            branch: "topic".into(),
            ..inputs()
        })
        .unwrap();
        assert!(
            run(&Inputs {
                depth: -1,
                ..inputs()
            })
            .is_err()
        );
        assert!(
            run(&Inputs {
                branch: "-bad".into(),
                ..inputs()
            })
            .is_err()
        );
        assert_eq!(
            *recorder.0.borrow(),
            vec![(None, Some(1)), (Some("topic".to_string()), None)]
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
        let fetched = fixture.run(Operation::Fetch, &inputs()).unwrap();
        assert!(fetched.contains("refs/remotes/origin/main"), "{fetched}");
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
        assert!(!git::open(&fixture.checkout).unwrap().is_shallow());
        write_commit(&fixture.checkout, "local.txt", "local", "Local");
        fixture.run(Operation::Push, &inputs()).unwrap();
        assert_eq!(head(&fixture.remote), head(&fixture.checkout));
        // The remote-tracking ref follows a successful push.
        let checkout = git::open(&fixture.checkout).unwrap();
        assert_eq!(
            checkout
                .refname_to_id("refs/remotes/origin/main")
                .unwrap()
                .to_string(),
            head(&fixture.checkout)
        );
        // The seed can fast-forward to what was pushed, then diverge from it.
        let seed = git::open(&fixture.seed).unwrap();
        git::fetch_with(
            &seed,
            &format!("file://{}", fixture.remote.display()),
            None,
            &[],
            &git::FetchRequest {
                remote: "origin",
                refspecs: vec!["+refs/heads/main:refs/remotes/origin/main".into()],
                prune: false,
                tags: false,
                depth: None,
            },
        )
        .unwrap();
        git::fast_forward(
            &seed,
            seed.refname_to_id("refs/remotes/origin/main").unwrap(),
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
    fn push_can_set_the_upstream_and_follow_annotated_tags() {
        let fixture = Fixture::new();
        fixture.run(Operation::Sync, &inputs()).unwrap();
        let checkout = git::open(&fixture.checkout).unwrap();
        identity(&checkout);
        let commit = write_commit(&fixture.checkout, "feature.txt", "x", "Feature");
        let object = checkout
            .find_object(git2::Oid::from_str(&commit).unwrap(), None)
            .unwrap();
        let signature = checkout.signature().unwrap();
        checkout
            .tag("v1", &object, &signature, "Release", false)
            .unwrap();
        checkout.tag_lightweight("light", &object, false).unwrap();
        let output = fixture
            .run(
                Operation::Push,
                &Inputs {
                    branch: "feature".into(),
                    set_upstream: true,
                    follow_tags: true,
                    ..inputs()
                },
            )
            .unwrap();
        assert!(output.contains("refs/heads/feature"), "{output}");
        let remote = git::open(&fixture.remote).unwrap();
        assert_eq!(
            remote
                .refname_to_id("refs/heads/feature")
                .unwrap()
                .to_string(),
            commit
        );
        assert!(remote.refname_to_id("refs/tags/v1").is_ok());
        assert!(remote.refname_to_id("refs/tags/light").is_err());
        let config = checkout.config().unwrap();
        assert_eq!(config.get_string("branch.main.remote").unwrap(), "origin");
        assert_eq!(
            config.get_string("branch.main.merge").unwrap(),
            "refs/heads/feature"
        );
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
        assert!(
            perform(
                Operation::Init,
                &fixture.checkout.join("other"),
                None,
                &Inputs {
                    branch: "bad branch".into(),
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
        assert!(perform(Operation::AddRemote, &fixture.checkout, None, &remote).is_err());
        let changed = Inputs {
            url: "https://github.com/owner/other.git".into(),
            ..inputs()
        };
        perform(Operation::SetRemoteUrl, &fixture.checkout, None, &changed).unwrap();
        let repo = git::open(&fixture.checkout).unwrap();
        assert_eq!(
            repo.find_remote("origin").unwrap().url().unwrap(),
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
        assert!(repo.remotes().unwrap().is_empty());
    }

    #[test]
    fn pull_and_sync_fail_when_the_remote_branch_disappeared() {
        let fixture = Fixture::new();
        fixture.run(Operation::Sync, &inputs()).unwrap();
        git::testing::switch_new(&fixture.seed, "topic");
        write_commit(&fixture.seed, "topic.txt", "topic", "Topic");
        fixture.publish("topic");
        let topic = Inputs {
            branch: "topic".into(),
            ..inputs()
        };
        fixture.run(Operation::Pull, &topic).unwrap();
        git::open(&fixture.remote)
            .unwrap()
            .find_reference("refs/heads/topic")
            .unwrap()
            .delete()
            .unwrap();
        let tracking = |fixture: &Fixture| {
            git::open(&fixture.checkout)
                .unwrap()
                .refname_to_id("refs/remotes/origin/topic")
                .ok()
        };
        let before = tracking(&fixture);
        assert!(before.is_some());
        assert!(fixture.run(Operation::Pull, &topic).is_err());
        assert!(fixture.run(Operation::Sync, &topic).is_err());
        assert_eq!(tracking(&fixture), before);
    }

    #[test]
    fn set_remote_url_requires_an_existing_remote() {
        let fixture = Fixture::new();
        fixture.run(Operation::Sync, &inputs()).unwrap();
        let ghost = Inputs {
            remote: "ghost".into(),
            url: "https://github.com/o/r2".into(),
            ..inputs()
        };
        assert!(perform(Operation::SetRemoteUrl, &fixture.checkout, None, &ghost).is_err());
        let repo = git::open(&fixture.checkout).unwrap();
        assert!(
            repo.config()
                .unwrap()
                .get_string("remote.ghost.url")
                .is_err()
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
