//! Git backend built on libgit2 (the `git2` crate).
//!
//! The cloud executors have no `git` program, so no code in this module spawns a process. libgit2
//! never runs hooks, credential helpers, `fsmonitor`, GPG programs or submodule updates, which keeps
//! the isolation guarantees the CLI backend had to configure explicitly.
use super::provider::GitHubProvider;
use base64::{Engine, engine::general_purpose::STANDARD};
use flow_like::flow::execution::{ExecutionEnvironment, egress};
use flow_like_types::{Result, anyhow, bail, reqwest::Url};
use git2::{
    AutotagOption, FetchOptions, FetchPrune, ObjectType, Oid, ProxyOptions, PushOptions,
    RemoteCallbacks, RemoteRedirect, Repository, RepositoryOpenFlags,
    build::{CheckoutBuilder, RepoBuilder},
};
use std::path::Path;

/// Opens the repository whose working tree is exactly `path`. Parent directories are never searched
/// and no `GIT_*` environment variable can redirect the operation to a different checkout.
pub(crate) fn open(path: &Path) -> Result<Repository> {
    Ok(Repository::open_ext(
        path,
        RepositoryOpenFlags::NO_SEARCH,
        std::iter::empty::<&std::ffi::OsStr>(),
    )?)
}

pub(crate) fn validate_ref(value: &str) -> Result<()> {
    if value.is_empty()
        || value.starts_with('-')
        || value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        bail!(
            "A Git reference must be nonempty and cannot start with '-' or contain whitespace or control characters"
        );
    }
    Ok(())
}

pub(crate) fn validate_remote(name: &str) -> Result<()> {
    if name.is_empty()
        || !name.as_bytes()[0].is_ascii_alphanumeric()
        || name
            .bytes()
            .any(|ch| !ch.is_ascii_alphanumeric() && !b"._-".contains(&ch))
        || name.contains("..")
    {
        bail!(
            "Use a remote name such as 'origin', containing only letters, numbers, '.', '_' or '-'"
        );
    }
    Ok(())
}

/// Validates a literal branch name the way `git check-ref-format --branch` does.
pub(crate) fn validate_branch_name(name: &str) -> Result<()> {
    validate_ref(name)?;
    if name.contains("@{") || !git2::Branch::name_is_valid(name)? {
        bail!("'{name}' is not a valid branch name");
    }
    Ok(())
}

/// Validates a tag name the way `git check-ref-format refs/tags/<name>` does.
pub(crate) fn validate_tag_name(name: &str) -> Result<()> {
    validate_ref(name)?;
    if !git2::Reference::is_valid_name(&format!("refs/tags/{name}")) {
        bail!("'{name}' is not a valid tag name");
    }
    Ok(())
}

pub(crate) fn short(oid: Oid) -> String {
    oid.to_string().chars().take(7).collect()
}

/// Resolves a branch, tag, or revision expression to a commit id.
pub(crate) fn resolve_commit(repo: &Repository, revision: &str) -> Result<Oid> {
    validate_ref(revision)?;
    let object = repo
        .revparse_single(revision)
        .map_err(|_| anyhow!("Unknown revision or path not in the working tree: {revision}"))?;
    Ok(object.peel_to_commit()?.id())
}

pub(crate) fn ensure_repository(path: &Path) -> Result<()> {
    if !path.is_dir() || !path.join(".git").exists() {
        bail!(
            "Select a Git working tree with its .git metadata. For a stored file snapshot, clone again with Include .git enabled."
        );
    }
    let repo = open(path)?;
    if repo.is_bare() {
        bail!("This operation requires a Git working tree; bare repositories are not supported");
    }
    let root = repo
        .workdir()
        .ok_or_else(|| anyhow!("This operation requires a Git working tree"))?
        .canonicalize()?;
    if path.canonicalize()? != root {
        bail!("Select the repository root, not a directory inside another repository");
    }
    Ok(())
}

/// Fails while a merge, rebase, cherry-pick, revert or apply operation is unfinished or the working
/// tree has staged, unstaged, or untracked changes.
pub(crate) fn require_clean(path: &Path) -> Result<()> {
    ensure_repository(path)?;
    let repo = open(path)?;
    for marker in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-apply",
        "rebase-merge",
        "sequencer",
    ] {
        if repo.path().join(marker).exists() {
            bail!(
                "A merge, rebase, cherry-pick, revert, or apply operation is in progress. Complete or abort it before this operation."
            );
        }
    }
    if repo.state() != git2::RepositoryState::Clean {
        bail!(
            "A merge, rebase, cherry-pick, revert, or apply operation is in progress. Complete or abort it before this operation."
        );
    }
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);
    if !repo.statuses(Some(&mut options))?.is_empty() {
        bail!(
            "The working tree has staged, unstaged, or untracked changes. Commit or stash them before this operation."
        );
    }
    Ok(())
}

/// True when `ancestor` equals `descendant` or is reachable from it.
pub(crate) fn is_ancestor(repo: &Repository, ancestor: Oid, descendant: Oid) -> Result<bool> {
    Ok(ancestor == descendant || repo.graph_descendant_of(descendant, ancestor)?)
}

/// Checks out `commit` without overwriting local modifications or untracked files.
pub(crate) fn checkout_commit(repo: &Repository, commit: Oid) -> Result<()> {
    let commit = repo.find_commit(commit)?;
    repo.checkout_tree(commit.as_object(), Some(CheckoutBuilder::new().safe()))?;
    Ok(())
}

/// Name of the branch HEAD points at, also when that branch has no commit yet.
pub(crate) fn head_branch(repo: &Repository) -> Option<String> {
    let head = repo.find_reference("HEAD").ok()?;
    head.symbolic_target()?
        .strip_prefix("refs/heads/")
        .map(str::to_owned)
}

/// Moves the current branch (or a detached HEAD) forward to `target`; refuses divergent history.
pub(crate) fn fast_forward(repo: &Repository, target: Oid) -> Result<String> {
    let current = match repo.head() {
        Ok(head) => head,
        Err(error) if error.code() == git2::ErrorCode::UnbornBranch => {
            let branch = head_branch(repo).ok_or_else(|| anyhow!("HEAD is not a branch"))?;
            checkout_commit(repo, target)?;
            repo.reference(
                &format!("refs/heads/{branch}"),
                target,
                false,
                "fast-forward",
            )?;
            return Ok(format!("Fast-forward to {}\n", short(target)));
        }
        Err(error) => return Err(error.into()),
    };
    let from = current.peel_to_commit()?.id();
    if is_ancestor(repo, target, from)? {
        return Ok("Already up to date.\n".into());
    }
    if !repo.graph_descendant_of(target, from)? {
        bail!("Not possible to fast-forward, aborting.");
    }
    checkout_commit(repo, target)?;
    if current.is_branch() {
        let name = current
            .name()
            .ok_or_else(|| anyhow!("The current branch name is not UTF-8"))?;
        repo.reference(name, target, true, "fast-forward")?;
    } else {
        repo.set_head_detached(target)?;
    }
    Ok(format!(
        "Updating {}..{}\nFast-forward\n",
        short(from),
        short(target)
    ))
}

/// Removes credentials left in remote URLs by older clones.
pub(crate) fn redact(value: &str) -> String {
    let mut result = value.to_string();
    for scheme in ["https://", "http://"] {
        let mut offset = 0;
        while let Some(start) = result[offset..].find(scheme) {
            let authority = offset + start + scheme.len();
            let end = result[authority..]
                .find(|ch: char| ch == '/' || ch == '?' || ch == '#' || ch.is_whitespace())
                .map(|index| authority + index)
                .unwrap_or(result.len());
            if let Some(at) = result[authority..end].rfind('@') {
                result.replace_range(authority..authority + at, "***");
                offset = authority + 4;
            } else {
                offset = end;
            }
        }
    }
    result
}

fn redact_secrets(value: &str, secrets: &[String]) -> String {
    let mut result = value.to_string();
    for secret in secrets {
        if !secret.is_empty() {
            result = result.replace(secret, "***");
        }
    }
    redact(&result)
}

fn provider_origin(provider: &GitHubProvider) -> Result<Url> {
    let mut url = Url::parse(&provider.base_url)
        .map_err(|_| anyhow!("The GitHub provider API base URL is invalid"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!(
            "Authenticated Git operations require an HTTPS provider URL without credentials, query parameters, or fragments"
        );
    }
    if url.host_str() == Some("api.github.com") {
        url.set_host(Some("github.com"))
            .map_err(|_| anyhow!("Invalid GitHub host"))?;
    }
    url.set_path("/");
    Ok(url)
}

/// libgit2 opens its own connections, which the egress resolver cannot see, so
/// the provider origin every authenticated network operation is pinned to is
/// vetted once up front.
pub(crate) async fn ensure_origin_allowed(
    environment: ExecutionEnvironment,
    provider: &GitHubProvider,
) -> Result<()> {
    egress::ensure_url_resolves_allowed(environment, &provider_origin(provider)?).await
}

fn validated_url(value: &str, origin: &Url, secrets: &mut Vec<String>) -> Result<String> {
    if value
        .chars()
        .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        bail!("The Git remote URL cannot contain whitespace or control characters");
    }
    let mut url = Url::parse(value).map_err(|_| anyhow!("The Git remote URL is invalid"))?;
    if url.scheme() != "https"
        || url.host_str() != origin.host_str()
        || url.port_or_known_default() != origin.port_or_known_default()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!(
            "The Git remote must use HTTPS on the GitHub provider's host and cannot contain query parameters or fragments"
        );
    }
    for part in [Some(url.username()), url.password()].into_iter().flatten() {
        if !part.is_empty() {
            secrets.push(part.to_string());
            if let Ok(decoded) = urlencoding::decode(part) {
                secrets.push(decoded.into_owned());
            }
        }
    }
    url.set_username("")
        .map_err(|_| anyhow!("Cannot remove Git URL credentials"))?;
    url.set_password(None)
        .map_err(|_| anyhow!("Cannot remove Git URL credentials"))?;
    Ok(url.to_string())
}

/// Everything a network operation needs besides the repository: the single vetted destination and
/// the secrets that must never appear in an error message or an output.
pub(crate) struct Prepared {
    pub url: String,
    header: String,
    secrets: Vec<String>,
}

pub(crate) struct FetchRequest<'a> {
    pub remote: &'a str,
    /// Empty uses the refspecs configured for the remote.
    pub refspecs: Vec<String>,
    pub prune: bool,
    pub tags: bool,
    pub depth: Option<i32>,
}

pub(crate) struct PushRequest<'a> {
    pub remote: &'a str,
    /// Always an explicit `refs/heads/<local>:refs/heads/<remote>` mapping.
    pub refspec: String,
    pub follow_tags: bool,
}

/// Transport used by the repository nodes. The production implementation pins every request to the
/// GitHub provider's origin; tests substitute a local bare repository.
pub(crate) trait Network {
    fn clone_repo(
        &self,
        url: &str,
        target: &Path,
        branch: Option<&str>,
        depth: Option<i32>,
    ) -> Result<()>;
    fn fetch(&self, repo: &Repository, request: &FetchRequest) -> Result<String>;
    fn push(&self, repo: &Repository, request: &PushRequest) -> Result<String>;
}

/// Authenticated transport restricted to the provider origin.
pub(crate) struct ProviderNetwork<'a>(pub &'a GitHubProvider);

fn auth_secrets(provider: &GitHubProvider) -> (String, Vec<String>) {
    let authorization = STANDARD.encode(format!("x-access-token:{}", provider.access_token));
    let secrets = vec![provider.access_token.clone(), authorization.clone()];
    (authorization, secrets)
}

pub(crate) fn config_values(repo: &Repository, key: &str) -> Result<Vec<String>> {
    let config = repo.config()?;
    let mut values = Vec::new();
    config.multivar(key, None)?.for_each(|entry| {
        if let Some(value) = entry.value() {
            values.push(value.to_owned());
        }
    })?;
    Ok(values)
}

/// Rejects configuration that could silently redirect an authenticated request.
fn reject_url_rewrites(repo: &Repository) -> Result<()> {
    let config = repo.config()?;
    let mut rewritten = false;
    config.entries(None)?.for_each(|entry| {
        let key = entry.name().unwrap_or_default().to_ascii_lowercase();
        if (key.starts_with("url.")
            && (key.ends_with(".insteadof") || key.ends_with(".pushinsteadof")))
            || (key.starts_with("remote.") && key.ends_with(".vcs"))
        {
            rewritten = true;
        }
    })?;
    if rewritten {
        bail!(
            "Authenticated Git operations do not support configured URL rewrites or remote helpers. Remove that configuration before retrying."
        );
    }
    Ok(())
}

/// Repository-level proxy settings would send the request to an address the repository controls
/// instead of the vetted provider host; user and system level proxies stay honored.
fn reject_repository_proxy(repo: &Repository) -> Result<()> {
    let config = repo.config()?;
    let Ok(local) = config.open_level(git2::ConfigLevel::Local) else {
        return Ok(());
    };
    let mut proxied = false;
    local.entries(None)?.for_each(|entry| {
        let key = entry.name().unwrap_or_default().to_ascii_lowercase();
        if (key.starts_with("http.") && key.ends_with(".proxy") || key == "http.proxy")
            || (key.starts_with("remote.") && key.ends_with(".proxy"))
        {
            proxied = true;
        }
    })?;
    if proxied {
        bail!(
            "Authenticated Git operations do not support repository-level proxy settings. Remove http.proxy and remote.*.proxy from the repository configuration."
        );
    }
    Ok(())
}

/// Validates a clone destination and prepares the credentials for it.
pub(crate) fn prepare_clone(url: &str, provider: &GitHubProvider) -> Result<Prepared> {
    let origin = provider_origin(provider)?;
    let (authorization, mut secrets) = auth_secrets(provider);
    let clean = validated_url(url, &origin, &mut secrets)?;
    if clean != url {
        bail!("Clone requires a normalized HTTPS URL without embedded credentials");
    }
    Ok(Prepared {
        url: clean,
        header: format!("Authorization: Basic {authorization}"),
        secrets,
    })
}

/// Validates every URL of a remote and prepares the credentials for it. Credentials left in legacy
/// remote URLs are never sent: the request uses the stripped URL only.
pub(crate) fn prepare_remote(
    repo: &Repository,
    remote: &str,
    push: bool,
    provider: &GitHubProvider,
) -> Result<Prepared> {
    let origin = provider_origin(provider)?;
    validate_remote(remote)?;
    let (authorization, mut secrets) = auth_secrets(provider);
    reject_url_rewrites(repo)?;
    reject_repository_proxy(repo)?;
    let mut urls = if push {
        config_values(repo, &format!("remote.{remote}.pushurl"))?
    } else {
        Vec::new()
    };
    if urls.is_empty() {
        urls = config_values(repo, &format!("remote.{remote}.url"))?;
    }
    if urls.is_empty() {
        bail!("The Git remote does not have a URL");
    }
    let mut clean_urls = Vec::new();
    for url in &urls {
        clean_urls.push(validated_url(url, &origin, &mut secrets)?);
    }
    Ok(Prepared {
        url: clean_urls.remove(0),
        header: format!("Authorization: Basic {authorization}"),
        secrets,
    })
}

fn git_error(error: git2::Error, secrets: &[String]) -> flow_like_types::Error {
    anyhow!("{}", redact_secrets(error.message(), secrets))
}

fn proxy_options<'a>() -> ProxyOptions<'a> {
    let mut proxy = ProxyOptions::new();
    proxy.auto();
    proxy
}

// The token is sent only to the vetted URL: libgit2 refuses redirects to another host (a same-host
// redirect, including to another port, can still be followed; there is no hook to forbid it) and no credential prompt or
// helper can supply a different identity.
fn fetch_options<'a>(header: Option<&str>, callbacks: RemoteCallbacks<'a>) -> FetchOptions<'a> {
    let mut options = FetchOptions::new();
    options
        .follow_redirects(RemoteRedirect::None)
        .proxy_options(proxy_options())
        .remote_callbacks(callbacks);
    if let Some(header) = header {
        options.custom_headers(&[header]);
    }
    options
}

fn push_options<'a>(header: Option<&str>, callbacks: RemoteCallbacks<'a>) -> PushOptions<'a> {
    let mut options = PushOptions::new();
    options
        .follow_redirects(RemoteRedirect::None)
        .proxy_options(proxy_options())
        .remote_callbacks(callbacks);
    if let Some(header) = header {
        options.custom_headers(&[header]);
    }
    options
}

fn base_callbacks<'a>() -> RemoteCallbacks<'a> {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_, _, _| {
        Err(git2::Error::from_str(
            "Authentication failed: the access token was not accepted",
        ))
    });
    callbacks
}

pub(crate) fn clone_with(
    url: &str,
    header: Option<&str>,
    secrets: &[String],
    target: &Path,
    branch: Option<&str>,
    depth: Option<i32>,
) -> Result<()> {
    let mut fetch = fetch_options(header, base_callbacks());
    if let Some(depth) = depth {
        fetch.depth(depth);
    }
    let mut builder = RepoBuilder::new();
    builder.fetch_options(fetch);
    if let Some(branch) = branch {
        validate_ref(branch)?;
        builder.branch(branch);
    }
    builder
        .clone(url, target)
        .map_err(|error| git_error(error, secrets))?;
    Ok(())
}

pub(crate) fn fetch_with(
    repo: &Repository,
    url: &str,
    header: Option<&str>,
    secrets: &[String],
    request: &FetchRequest,
) -> Result<String> {
    let refspecs = if request.refspecs.is_empty() {
        let configured = repo.find_remote(request.remote)?;
        configured
            .fetch_refspecs()?
            .iter()
            .flatten()
            .map(str::to_owned)
            .collect()
    } else {
        request.refspecs.clone()
    };
    // libgit2 ignores explicit refspecs whose source is missing on the remote; git fails with
    // "couldn't find remote ref". Detach the stale destination refs so a missing source is detectable.
    let mut detached: Vec<(String, Oid)> = Vec::new();
    if !request.refspecs.is_empty() {
        for spec in &request.refspecs {
            let spec = spec.trim_start_matches('+');
            if let Some((src, dst)) = spec.split_once(':')
                && !src.contains('*')
                && dst.starts_with("refs/")
                && let Ok(mut reference) = repo.find_reference(dst)
                && let Some(oid) = reference.target()
                && reference.delete().is_ok()
            {
                detached.push((dst.to_owned(), oid));
            }
        }
    }
    let restore = |detached: &[(String, Oid)]| {
        for (name, oid) in detached {
            if repo.find_reference(name).is_err() {
                let _ = repo.reference(name, *oid, true, "restore stale tracking ref");
            }
        }
    };
    let mut lines = Vec::new();
    {
        let mut callbacks = base_callbacks();
        callbacks.update_tips(|name, old, new| {
            if old.is_zero() {
                lines.push(format!(" * [new]     {name} {}", short(new)));
            } else if !new.is_zero() {
                lines.push(format!(
                    "   [updated] {name} {}..{}",
                    short(old),
                    short(new)
                ));
            } else {
                lines.push(format!(" - [deleted] {name}"));
            }
            true
        });
        let mut options = fetch_options(header, callbacks);
        options
            .prune(if request.prune {
                FetchPrune::On
            } else {
                FetchPrune::Off
            })
            .download_tags(if request.tags {
                AutotagOption::All
            } else {
                AutotagOption::Auto
            })
            .update_fetchhead(true);
        if let Some(depth) = request.depth {
            options.depth(depth);
        }
        let mut remote = repo.remote_anonymous(url)?;
        if let Err(error) = remote.fetch(&refspecs, Some(&mut options), None) {
            restore(&detached);
            return Err(git_error(error, secrets));
        }
    }
    if let Some((name, _)) = detached
        .iter()
        .find(|(name, _)| repo.find_reference(name).is_err())
    {
        restore(&detached);
        bail!("Couldn't find remote ref for {name}");
    }
    Ok(if lines.is_empty() {
        format!("From {url}\nAlready up to date.\n")
    } else {
        format!("From {url}\n{}\n", lines.join("\n"))
    })
}

pub(crate) fn push_with(
    repo: &Repository,
    url: &str,
    header: Option<&str>,
    secrets: &[String],
    request: &PushRequest,
) -> Result<String> {
    let (source, destination) = request
        .refspec
        .trim_start_matches('+')
        .split_once(':')
        .ok_or_else(|| anyhow!("Push requires an explicit refspec"))?;
    if request.refspec.starts_with('+') {
        bail!("Forced pushes are not supported");
    }
    let pushed = repo.refname_to_id(source)?;
    let mut summary = format!("To {url}\n");
    push_refspecs(
        repo,
        url,
        header,
        secrets,
        std::slice::from_ref(&request.refspec),
        &mut summary,
        true,
    )?;
    if destination.starts_with("refs/heads/") {
        update_tracking_ref(repo, request.remote, destination, pushed)?;
    }
    if request.follow_tags {
        let tags = followed_tags(repo, pushed)?;
        if !tags.is_empty() {
            push_refspecs(repo, url, header, secrets, &tags, &mut summary, false)?;
        }
    }
    Ok(summary)
}

fn push_refspecs(
    repo: &Repository,
    url: &str,
    header: Option<&str>,
    secrets: &[String],
    refspecs: &[String],
    summary: &mut String,
    strict: bool,
) -> Result<()> {
    let mut rejected = Vec::new();
    let mut accepted = Vec::new();
    {
        let mut callbacks = base_callbacks();
        callbacks.push_update_reference(|name, status| {
            match status {
                Some(reason) => rejected.push(format!("{name}: {reason}")),
                None => accepted.push(name.to_owned()),
            }
            Ok(())
        });
        let mut options = push_options(header, callbacks);
        let mut remote = repo.remote_anonymous(url)?;
        remote
            .push(refspecs, Some(&mut options))
            .map_err(|error| git_error(error, secrets))?;
    }
    for name in &accepted {
        summary.push_str(&format!(" * {name}\n"));
    }
    if !rejected.is_empty() {
        let detail = redact_secrets(&rejected.join("; "), secrets);
        if strict {
            bail!("The remote rejected the push: {detail}");
        }
        summary.push_str(&format!("Not pushed: {detail}\n"));
    }
    Ok(())
}

/// Mirrors the remote-tracking update `git push` performs after a successful push.
fn update_tracking_ref(repo: &Repository, remote: &str, destination: &str, oid: Oid) -> Result<()> {
    let Ok(configured) = repo.find_remote(remote) else {
        return Ok(());
    };
    for spec in configured.refspecs() {
        if spec.direction() == git2::Direction::Fetch && spec.src_matches(destination) {
            let tracking = spec.transform(destination)?;
            if let Some(name) = tracking.as_str() {
                repo.reference(name, oid, true, "update by push")?;
            }
            break;
        }
    }
    Ok(())
}

/// Annotated tags whose commit is contained in the pushed history (`push --follow-tags`).
fn followed_tags(repo: &Repository, pushed: Oid) -> Result<Vec<String>> {
    let mut tags = Vec::new();
    for reference in repo.references_glob("refs/tags/*")? {
        let reference = reference?;
        let (Some(name), Some(target)) = (reference.name(), reference.target()) else {
            continue;
        };
        let object = repo.find_object(target, None)?;
        if object.kind() != Some(ObjectType::Tag) {
            continue;
        }
        if let Ok(commit) = object.peel_to_commit()
            && is_ancestor(repo, commit.id(), pushed)?
        {
            tags.push(format!("{name}:{name}"));
        }
    }
    tags.sort();
    Ok(tags)
}

impl Network for ProviderNetwork<'_> {
    fn clone_repo(
        &self,
        url: &str,
        target: &Path,
        branch: Option<&str>,
        depth: Option<i32>,
    ) -> Result<()> {
        let prepared = prepare_clone(url, self.0)?;
        clone_with(
            &prepared.url,
            Some(&prepared.header),
            &prepared.secrets,
            target,
            branch,
            depth,
        )
    }

    fn fetch(&self, repo: &Repository, request: &FetchRequest) -> Result<String> {
        let prepared = prepare_remote(repo, request.remote, false, self.0)?;
        let output = fetch_with(
            repo,
            &prepared.url,
            Some(&prepared.header),
            &prepared.secrets,
            request,
        )?;
        Ok(redact_secrets(&output, &prepared.secrets))
    }

    fn push(&self, repo: &Repository, request: &PushRequest) -> Result<String> {
        let prepared = prepare_remote(repo, request.remote, true, self.0)?;
        let output = push_with(
            repo,
            &prepared.url,
            Some(&prepared.header),
            &prepared.secrets,
            request,
        )?;
        Ok(redact_secrets(&output, &prepared.secrets))
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use git2::RepositoryInitOptions;
    use std::path::PathBuf;

    /// Scratch directory removed on drop.
    pub(crate) struct TempDir(pub PathBuf);

    impl TempDir {
        pub(crate) fn new(prefix: &str) -> Self {
            let path = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub(crate) fn identity(repo: &Repository) {
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Git Node Test").unwrap();
        config
            .set_str("user.email", "git-node@example.invalid")
            .unwrap();
    }

    /// Initializes a repository on `main` with a test identity.
    pub(crate) fn init(path: &Path) -> Repository {
        let repo = Repository::init_opts(
            path,
            RepositoryInitOptions::new()
                .initial_head("main")
                .mkpath(true),
        )
        .unwrap();
        identity(&repo);
        repo
    }

    /// Writes one file and commits it on the current branch.
    pub(crate) fn write_commit(path: &Path, file: &str, content: &str, message: &str) -> String {
        std::fs::write(path.join(file), content).unwrap();
        let repo = open(path).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(file)).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = repo.signature().unwrap();
        let parents: Vec<_> = repo
            .head()
            .ok()
            .map(|head| head.peel_to_commit().unwrap())
            .into_iter()
            .collect();
        let parents: Vec<_> = parents.iter().collect();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )
        .unwrap()
        .to_string()
    }

    pub(crate) fn head(path: &Path) -> String {
        open(path)
            .unwrap()
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .id()
            .to_string()
    }

    /// `git status --porcelain=v1`-style lines for the working tree.
    pub(crate) fn porcelain(path: &Path) -> Vec<String> {
        use git2::Status;
        let repo = open(path).unwrap();
        let mut options = git2::StatusOptions::new();
        options.include_untracked(true).recurse_untracked_dirs(true);
        let statuses = repo.statuses(Some(&mut options)).unwrap();
        statuses
            .iter()
            .map(|entry| {
                let flags = entry.status();
                let code = if flags.contains(Status::CONFLICTED) {
                    "UU".to_string()
                } else if flags == Status::WT_NEW {
                    "??".to_string()
                } else {
                    let pick = |table: &[(Status, char)]| {
                        table
                            .iter()
                            .find(|(flag, _)| flags.contains(*flag))
                            .map_or(' ', |(_, letter)| *letter)
                    };
                    format!(
                        "{}{}",
                        pick(&[
                            (Status::INDEX_NEW, 'A'),
                            (Status::INDEX_MODIFIED, 'M'),
                            (Status::INDEX_DELETED, 'D'),
                        ]),
                        pick(&[(Status::WT_MODIFIED, 'M'), (Status::WT_DELETED, 'D')])
                    )
                };
                format!("{code} {}", entry.path().unwrap())
            })
            .collect()
    }

    /// Content of a blob addressed like `git show`: `HEAD:file` or `:file` for the index.
    pub(crate) fn show(path: &Path, spec: &str) -> String {
        let repo = open(path).unwrap();
        let blob = match spec.strip_prefix(':') {
            // libgit2 does not implement the `:path` index syntax of revparse.
            Some(file) => {
                let entry = repo.index().unwrap().get_path(Path::new(file), 0).unwrap();
                repo.find_blob(entry.id).unwrap()
            }
            None => repo.revparse_single(spec).unwrap().peel_to_blob().unwrap(),
        };
        String::from_utf8(blob.content().to_vec()).unwrap()
    }

    pub(crate) fn current_branch_name(path: &Path) -> String {
        head_branch(&open(path).unwrap()).unwrap_or_default()
    }

    pub(crate) fn stage_path(path: &Path, file: &str) {
        let repo = open(path).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(file)).unwrap();
        index.write().unwrap();
    }

    /// Commits whatever is staged.
    pub(crate) fn commit_index(path: &Path, message: &str) {
        let repo = open(path).unwrap();
        let mut index = repo.index().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = repo.signature().unwrap();
        let parent = repo.head().ok().map(|head| head.peel_to_commit().unwrap());
        let parents: Vec<_> = parent.iter().collect();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )
        .unwrap();
    }

    /// Commit subjects from HEAD, one per line.
    pub(crate) fn log_subjects(path: &Path, count: usize) -> String {
        let repo = open(path).unwrap();
        let mut walk = repo.revwalk().unwrap();
        walk.push_head().unwrap();
        walk.take(count)
            .map(|id| {
                let commit = repo.find_commit(id.unwrap()).unwrap();
                format!("{}\n", commit.summary().unwrap())
            })
            .collect()
    }

    /// Creates `name` at HEAD and checks it out.
    pub(crate) fn switch_new(path: &Path, name: &str) {
        let repo = open(path).unwrap();
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch(name, &commit, false).unwrap();
        repo.set_head(&format!("refs/heads/{name}")).unwrap();
    }

    pub(crate) fn switch(path: &Path, name: &str) {
        let repo = open(path).unwrap();
        repo.set_head(&format!("refs/heads/{name}")).unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
    }

    /// Merges `name` into HEAD; conflicts leave the merge unfinished and return an error.
    pub(crate) fn merge(path: &Path, name: &str) -> Result<()> {
        let repo = open(path)?;
        let theirs = repo.find_annotated_commit(resolve_commit(&repo, name)?)?;
        repo.merge(&[&theirs], None, Some(CheckoutBuilder::new().safe()))?;
        if repo.index()?.has_conflicts() {
            bail!("merge conflict");
        }
        Ok(())
    }

    pub(crate) fn abort_merge(path: &Path) -> Result<()> {
        let repo = open(path)?;
        if repo.state() != git2::RepositoryState::Merge {
            bail!("no merge to abort");
        }
        let head = repo.head()?.peel_to_commit()?;
        repo.reset(
            head.as_object(),
            git2::ResetType::Hard,
            Some(CheckoutBuilder::new().force()),
        )?;
        repo.cleanup_state()?;
        Ok(())
    }

    /// Transport that talks to a local bare repository instead of the GitHub origin. Only the
    /// destination changes: every production code path below the URL checks runs unchanged.
    pub(crate) struct LocalNetwork {
        pub remote: PathBuf,
    }

    impl LocalNetwork {
        fn url(&self) -> String {
            format!("file://{}", self.remote.display())
        }
    }

    impl Network for LocalNetwork {
        fn clone_repo(
            &self,
            url: &str,
            target: &Path,
            branch: Option<&str>,
            depth: Option<i32>,
        ) -> Result<()> {
            // The local transport cannot serve shallow requests; shallow behavior is covered by the
            // ignored HTTPS test below.
            let _ = depth;
            clone_with(&self.url(), None, &[], target, branch, None)?;
            let repo = open(target)?;
            repo.remote_set_url("origin", url)?;
            identity(&repo);
            Ok(())
        }

        fn fetch(&self, repo: &Repository, request: &FetchRequest) -> Result<String> {
            let request = FetchRequest {
                remote: request.remote,
                refspecs: request.refspecs.clone(),
                prune: request.prune,
                tags: request.tags,
                depth: None,
            };
            fetch_with(repo, &self.url(), None, &[], &request)
        }

        fn push(&self, repo: &Repository, request: &PushRequest) -> Result<String> {
            push_with(repo, &self.url(), None, &[], request)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use std::fs;

    fn provider() -> GitHubProvider {
        GitHubProvider {
            provider_id: "github".into(),
            access_token: "test-token-that-must-not-leak".into(),
            base_url: "https://api.github.com".into(),
        }
    }

    #[test]
    fn validates_references_and_remote_names() {
        for reference in ["HEAD", "HEAD~1", "release/v1", "v1.0^{commit}"] {
            validate_ref(reference).unwrap();
        }
        for reference in ["", "-main", "main\n", "main branch", "main\0"] {
            assert!(validate_ref(reference).is_err());
        }
        for remote in ["origin", "upstream", "origin-2", "company.github"] {
            validate_remote(remote).unwrap();
        }
        for remote in [
            "",
            "--all",
            "/tmp/repo",
            "../repo",
            "a/b",
            "a:b",
            "https://github.com/repo",
            "a\n",
        ] {
            assert!(validate_remote(remote).is_err());
        }
        validate_branch_name("feature/x").unwrap();
        for branch in ["a..b", "a b", "@{-1}", "x.lock", "-x", "a~1", "a:b"] {
            assert!(validate_branch_name(branch).is_err(), "{branch}");
        }
        validate_tag_name("v1.0").unwrap();
        assert!(validate_tag_name("v1..0").is_err());
    }

    #[test]
    fn requires_repository_root_and_accepts_linked_worktrees() {
        let dir = TempDir::new("flow-like-git");
        let repo = init(&dir.0);
        ensure_repository(&dir.0).unwrap();
        fs::create_dir(dir.0.join("child")).unwrap();
        assert!(ensure_repository(&dir.0.join("child")).is_err());
        write_commit(&dir.0, "file.txt", "x", "Initial");
        let linked = dir.0.join("linked");
        repo.worktree("linked", &linked, None).unwrap();
        ensure_repository(&linked).unwrap();
        let bare = dir.0.join("bare");
        Repository::init_bare(&bare).unwrap();
        assert!(ensure_repository(&bare).is_err());
    }

    #[test]
    fn clean_check_covers_untracked_staged_and_unstaged_changes() {
        let dir = TempDir::new("flow-like-git");
        init(&dir.0);
        require_clean(&dir.0).unwrap();
        fs::write(dir.0.join("file.txt"), "initial\n").unwrap();
        assert!(require_clean(&dir.0).is_err());
        write_commit(&dir.0, "file.txt", "initial\n", "Initial");
        require_clean(&dir.0).unwrap();
        fs::write(dir.0.join("file.txt"), "changed\n").unwrap();
        assert!(require_clean(&dir.0).is_err());
    }

    #[test]
    fn redacts_url_credentials_and_known_secrets() {
        let message =
            "fatal: https://old-token@github.com/a/b.git and http://name:password@example.com/x";
        assert_eq!(
            redact(message),
            "fatal: https://***@github.com/a/b.git and http://***@example.com/x"
        );
        assert_eq!(
            redact_secrets("token secret", &["secret".into()]),
            "token ***"
        );
        assert_eq!(redact_secrets("unchanged", &[String::new()]), "unchanged");
        let dir = TempDir::new("flow-like-git");
        let repo = init(&dir.0);
        let error = resolve_commit(&repo, "https://secret@github.com/x")
            .unwrap_err()
            .to_string();
        assert!(!redact(&error).contains("secret"));
    }

    #[test]
    fn authentication_is_ephemeral_and_legacy_credentials_are_removed() {
        let dir = TempDir::new("flow-like-git");
        let repo = init(&dir.0);
        repo.remote("origin", "https://old-token@github.com/org/repo.git")
            .unwrap();
        let before = fs::read(dir.0.join(".git/config")).unwrap();
        let provider = provider();
        let prepared = prepare_remote(&repo, "origin", false, &provider).unwrap();
        assert_eq!(prepared.url, "https://github.com/org/repo.git");
        assert!(prepared.header.starts_with("Authorization: Basic "));
        assert!(prepared.secrets.contains(&"old-token".to_string()));
        assert!(prepared.secrets.contains(&provider.access_token));
        assert!(!prepared.url.contains(&provider.access_token));
        assert_eq!(fs::read(dir.0.join(".git/config")).unwrap(), before);
    }

    #[test]
    fn repository_proxy_settings_are_rejected_for_authenticated_operations() {
        let dir = TempDir::new("flow-like-git");
        let repo = init(&dir.0);
        let provider = provider();
        repo.remote("origin", "https://github.com/org/repo.git")
            .unwrap();
        prepare_remote(&repo, "origin", false, &provider).unwrap();
        repo.config()
            .unwrap()
            .set_str("http.proxy", "http://127.0.0.1:9")
            .unwrap();
        assert!(prepare_remote(&repo, "origin", false, &provider).is_err());
    }

    #[test]
    fn rejects_foreign_hosts_rewrites_and_multiple_push_destinations() {
        let dir = TempDir::new("flow-like-git");
        let repo = init(&dir.0);
        let provider = provider();
        for url in [
            "https://attacker.invalid/org/repo.git",
            "http://github.com/org/repo.git",
            "https://github.com:444/org/repo.git",
            "https://github.com/org/repo.git?token=secret",
            "https://github.com/org/repo.git\n--upload-pack=sh",
            "file:///tmp/repo",
        ] {
            assert!(prepare_clone(url, &provider).is_err(), "{url}");
        }
        repo.remote("origin", "https://github.com/org/repo.git")
            .unwrap();
        repo.remote_set_pushurl("origin", Some("https://github.com/org/repo.git"))
            .unwrap();
        prepare_remote(&repo, "origin", true, &provider).unwrap();
        repo.config()
            .unwrap()
            .set_multivar(
                "remote.origin.pushurl",
                "^$",
                "https://attacker.invalid/stolen.git",
            )
            .unwrap();
        assert!(prepare_remote(&repo, "origin", true, &provider).is_err());
        // Fetching is unaffected by the extra push destination.
        prepare_remote(&repo, "origin", false, &provider).unwrap();
        repo.config()
            .unwrap()
            .set_str(
                "url.https://attacker.invalid/.insteadOf",
                "https://github.com/",
            )
            .unwrap();
        assert!(prepare_remote(&repo, "origin", false, &provider).is_err());
        assert!(prepare_remote(&repo, "--all", false, &provider).is_err());
    }

    #[test]
    fn clone_urls_must_be_normalized_and_enterprise_hosts_are_supported() {
        let provider = GitHubProvider {
            base_url: "https://github.example.com/api/v3".into(),
            ..provider()
        };
        let url = "https://github.example.com/org/repo.git";
        assert_eq!(prepare_clone(url, &provider).unwrap().url, url);
        assert!(
            prepare_clone("https://user:pw@github.example.com/org/repo.git", &provider).is_err()
        );
        assert!(prepare_clone("https://github.com/org/repo.git", &provider).is_err());
    }

    #[test]
    fn provider_origins_on_the_host_plane_are_refused_server_side() {
        let runtime = flow_like_types::tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let with_base = |base_url: &str| GitHubProvider {
            base_url: base_url.into(),
            ..provider()
        };
        runtime.block_on(async {
            for base_url in [
                "https://169.254.169.254/api/v3",
                "https://127.0.0.1:8443/api/v3",
                "https://localhost/api/v3",
            ] {
                let provider = with_base(base_url);
                assert!(
                    ensure_origin_allowed(ExecutionEnvironment::Server, &provider)
                        .await
                        .is_err(),
                    "{base_url} must be refused server-side"
                );
                assert!(
                    ensure_origin_allowed(ExecutionEnvironment::Local, &provider)
                        .await
                        .is_ok()
                );
            }
            assert!(
                ensure_origin_allowed(
                    ExecutionEnvironment::Server,
                    &with_base("https://203.0.113.7/api/v3")
                )
                .await
                .is_ok()
            );
        });
    }

    #[test]
    fn clean_check_rejects_an_unfinished_merge_with_no_file_changes() {
        let dir = TempDir::new("flow-like-git");
        let repo = init(&dir.0);
        write_commit(&dir.0, "file.txt", "x", "Initial");
        let initial = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("topic", &initial, false).unwrap();
        repo.set_head("refs/heads/topic").unwrap();
        write_commit(&dir.0, "other.txt", "y", "Topic");
        let topic = repo.head().unwrap().peel_to_commit().unwrap().id();
        repo.set_head("refs/heads/main").unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
        // `merge --no-ff --no-commit`: MERGE_HEAD is written and the commit is left to the user.
        let annotated = repo.find_annotated_commit(topic).unwrap();
        repo.merge(&[&annotated], None, Some(CheckoutBuilder::new().safe()))
            .unwrap();
        assert!(
            require_clean(&dir.0)
                .unwrap_err()
                .to_string()
                .contains("in progress")
        );
        repo.cleanup_state().unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
        require_clean(&dir.0).unwrap();
    }

    #[test]
    fn explicit_push_refspecs_are_required_and_forced_pushes_are_refused() {
        let dir = TempDir::new("flow-like-git");
        let repo = init(&dir.0);
        write_commit(&dir.0, "file.txt", "x", "Initial");
        let request = |refspec: &str| PushRequest {
            remote: "origin",
            refspec: refspec.into(),
            follow_tags: false,
        };
        assert!(push_with(&repo, "file:///nowhere", None, &[], &request("main")).is_err());
        assert!(
            push_with(
                &repo,
                "file:///nowhere",
                None,
                &[],
                &request("+refs/heads/main:refs/heads/main")
            )
            .unwrap_err()
            .to_string()
            .contains("Forced")
        );
    }

    /// Needs network access: `cargo test -p flow-like-catalog-data-github --features execute -- --ignored`.
    #[test]
    #[ignore = "requires access to github.com"]
    fn shallow_clone_and_unshallow_over_https() {
        let dir = TempDir::new("flow-like-https");
        let target = dir.0.join("hello");
        let url = "https://github.com/octocat/Hello-World.git";
        clone_with(url, None, &[], &target, None, Some(1)).unwrap();
        let repo = open(&target).unwrap();
        assert!(repo.is_shallow());
        let request = FetchRequest {
            remote: "origin",
            refspecs: Vec::new(),
            prune: false,
            tags: false,
            depth: Some(i32::MAX),
        };
        fetch_with(&repo, url, None, &[], &request).unwrap();
        assert!(!repo.is_shallow());
    }
}
