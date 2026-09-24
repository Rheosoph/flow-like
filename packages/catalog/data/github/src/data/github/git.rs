use super::provider::GitHubProvider;
use base64::{Engine, engine::general_purpose::STANDARD};
use flow_like_types::{Result, anyhow, bail, reqwest::Url};
use std::{
    path::Path,
    process::{Command, Output, Stdio},
};

type Config = Vec<(String, String)>;

#[cfg(windows)]
const NULL_DEVICE: &str = "NUL";
#[cfg(not(windows))]
const NULL_DEVICE: &str = "/dev/null";

fn command(path: &Path, args: &[&str], config: &[(String, String)]) -> Command {
    let mut command = Command::new("git");
    command.current_dir(path).arg("--no-pager").args(args);
    command.stdin(Stdio::null());

    // Inherited Git state must not redirect an operation to a different checkout or log tokens.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .env("GIT_CONFIG_COUNT", config.len().to_string());
    for (index, (key, value)) in config.iter().enumerate() {
        command
            .env(format!("GIT_CONFIG_KEY_{index}"), key)
            .env(format!("GIT_CONFIG_VALUE_{index}"), value);
    }
    command
}

fn local_config() -> Config {
    [
        ("core.hooksPath", NULL_DEVICE),
        ("core.fsmonitor", "false"),
        ("credential.helper", ""),
        ("credential.interactive", "false"),
        ("commit.gpgSign", "false"),
        ("tag.gpgSign", "false"),
        ("push.gpgSign", "false"),
        ("submodule.recurse", "false"),
        ("fetch.recurseSubmodules", "false"),
        ("push.recurseSubmodules", "no"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect()
}

fn execute(path: &Path, args: &[&str], config: &Config) -> Result<Output> {
    command(path, args, config).output().map_err(|error| {
        anyhow!("Could not execute Git: {error}. Install Git on the execution host.")
    })
}

fn checked_output(output: Output, secrets: &[String]) -> Result<Vec<u8>> {
    if output.status.success() {
        return Ok(output.stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let details = [stdout.trim(), stderr.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let detail = if details.is_empty() {
        format!("Git exited with {}", output.status)
    } else {
        details.join("\n")
    };
    Err(anyhow!("{}", redact_secrets(&detail, secrets)))
}

pub(crate) fn run_bytes(path: &Path, args: &[&str]) -> Result<Vec<u8>> {
    checked_output(execute(path, args, &local_config())?, &[])
}

pub(crate) fn run(path: &Path, args: &[&str]) -> Result<String> {
    Ok(redact(&String::from_utf8_lossy(&run_bytes(path, args)?)))
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

pub(crate) fn ensure_repository(path: &Path) -> Result<()> {
    if !path.is_dir() || !path.join(".git").exists() {
        bail!("Select the root of a local Git working tree with its .git metadata");
    }
    if run(path, &["rev-parse", "--is-inside-work-tree"])?.trim() != "true" {
        bail!("This operation requires a Git working tree; bare repositories are not supported");
    }
    let root = run(path, &["rev-parse", "--show-toplevel"])?;
    let root = Path::new(root.trim_end_matches(['\n', '\r'])).canonicalize()?;
    if path.canonicalize()? != root {
        bail!("Select the repository root, not a directory inside another repository");
    }
    Ok(())
}

pub(crate) fn require_clean(path: &Path) -> Result<()> {
    ensure_repository(path)?;
    let metadata = run(path, &["rev-parse", "--absolute-git-dir"])?;
    let metadata = Path::new(metadata.trim_end_matches(['\n', '\r']));
    for marker in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-apply",
        "rebase-merge",
        "sequencer",
    ] {
        if metadata.join(marker).exists() {
            bail!(
                "A merge, rebase, cherry-pick, revert, or apply operation is in progress. Complete or abort it before this operation."
            );
        }
    }
    if !run_bytes(
        path,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?
    .is_empty()
    {
        bail!(
            "The working tree has staged, unstaged, or untracked changes. Commit or stash them before this operation."
        );
    }
    Ok(())
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

fn destination_index(args: &[&str]) -> Result<usize> {
    if !matches!(args.first(), Some(&"clone" | &"fetch" | &"pull" | &"push")) {
        bail!("Authenticated Git commands must be clone, fetch, pull, or push");
    }
    let mut index = 1;
    while index < args.len() {
        let argument = args[index];
        if argument == "--" {
            if index + 1 < args.len() {
                return Ok(index + 1);
            }
            break;
        }
        if !argument.starts_with('-') {
            return Ok(index);
        }
        let option = argument.split('=').next().unwrap_or(argument);
        if matches!(
            option,
            "--upload-pack" | "--receive-pack" | "--exec" | "-u" | "--config" | "-c"
        ) {
            bail!("This Git network option is not supported");
        }
        if !argument.contains('=')
            && matches!(
                option,
                "--branch"
                    | "-b"
                    | "--depth"
                    | "--deepen"
                    | "--shallow-since"
                    | "--shallow-exclude"
                    | "--filter"
                    | "--jobs"
                    | "-j"
            )
        {
            index += 1;
        }
        index += 1;
    }
    bail!("The Git network command requires an explicit repository URL or remote name");
}

fn network_config(
    path: &Path,
    args: &[&str],
    provider: &GitHubProvider,
) -> Result<(Config, Vec<String>)> {
    let origin = provider_origin(provider)?;
    let destination = destination_index(args)?;
    let authorization = STANDARD.encode(format!("x-access-token:{}", provider.access_token));
    let mut secrets = vec![provider.access_token.clone(), authorization.clone()];
    let config = local_config();
    let settings = checked_output(
        execute(path, &["config", "--null", "--list"], &config)?,
        &secrets,
    )?;
    for setting in settings.split(|byte| *byte == 0) {
        let key = setting
            .split(|byte| *byte == b'\n')
            .next()
            .unwrap_or_default();
        let key = String::from_utf8_lossy(key).to_ascii_lowercase();
        if (key.starts_with("url.")
            && (key.ends_with(".insteadof") || key.ends_with(".pushinsteadof")))
            || (key.starts_with("remote.") && key.ends_with(".vcs"))
        {
            bail!(
                "Authenticated Git operations do not support configured URL rewrites or remote helpers. Remove that configuration before retrying."
            );
        }
    }
    let urls = if args[0] == "clone" {
        vec![args[destination].to_string()]
    } else {
        validate_remote(args[destination])?;
        let query = if args[0] == "push" {
            vec!["remote", "get-url", "--push", "--all", args[destination]]
        } else {
            vec!["remote", "get-url", "--all", args[destination]]
        };
        let output = checked_output(execute(path, &query, &config)?, &secrets)?;
        String::from_utf8_lossy(&output)
            .lines()
            .map(str::to_owned)
            .collect()
    };
    if urls.is_empty() {
        bail!("The Git remote does not have a URL");
    }
    let mut config = config;
    if matches!(args[0], "fetch" | "pull") {
        config.push(("fetch.pruneTags".into(), "false".into()));
        config.push((
            format!("remote.{}.pruneTags", args[destination]),
            "false".into(),
        ));
    }
    if args[0] == "push" {
        if args.len() <= destination + 1 {
            bail!("Push requires an explicit refspec");
        }
        config.push((
            format!("remote.{}.mirror", args[destination]),
            "false".into(),
        ));
        config.push(("push.default".into(), "nothing".into()));
        config.push(("push.followTags".into(), "false".into()));
        config.push(("push.autoSetupRemote".into(), "false".into()));
    }
    config.extend(
        [
            ("protocol.allow", "never"),
            ("protocol.https.allow", "always"),
            ("http.followRedirects", "false"),
            ("http.sslVerify", "true"),
            ("http.extraHeader", ""),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string())),
    );
    for url in urls {
        let clean = validated_url(&url, &origin, &mut secrets)?;
        if args[0] == "clone" && clean != url {
            bail!("Clone requires a normalized HTTPS URL without embedded credentials");
        }
        if clean != url {
            // Keep remote tracking behavior while stripping credentials from legacy origin URLs.
            config.push((format!("url.{clean}.insteadOf"), url));
        }
        let key = format!("http.{clean}.extraHeader");
        config.push((key.clone(), String::new()));
        config.push((key, format!("Authorization: Basic {authorization}")));
        config.push((format!("http.{clean}.followRedirects"), "false".into()));
        config.push((format!("http.{clean}.sslVerify"), "true".into()));
        config.push((format!("credential.{clean}.helper"), String::new()));
    }
    Ok((config, secrets))
}

/// Credentials exist only in the child environment, scoped to validated HTTPS destinations.
pub(crate) fn run_network(path: &Path, args: &[&str], provider: &GitHubProvider) -> Result<String> {
    let (config, secrets) = network_config(path, args, provider)?;
    let mut output = execute(path, args, &config)?;
    if output.status.success() {
        output.stdout.extend_from_slice(&output.stderr);
    }
    let output = checked_output(output, &secrets)?;
    Ok(redact_secrets(&String::from_utf8_lossy(&output), &secrets))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    struct Repository(PathBuf);

    impl Repository {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("flow-like-git-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            run(&path, &["init", "--initial-branch=main"]).unwrap();
            run(&path, &["config", "user.name", "Git Node Test"]).unwrap();
            run(&path, &["config", "user.email", "git-node@example.invalid"]).unwrap();
            Self(path)
        }
    }

    impl Drop for Repository {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

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
    }

    #[test]
    fn requires_repository_root_and_accepts_linked_worktrees() {
        let repo = Repository::new();
        ensure_repository(&repo.0).unwrap();
        fs::create_dir(repo.0.join("child")).unwrap();
        assert!(ensure_repository(&repo.0.join("child")).is_err());
        run(&repo.0, &["commit", "--allow-empty", "-m", "Initial"]).unwrap();
        let linked = repo.0.join("linked");
        run(
            &repo.0,
            &["worktree", "add", "--detach", linked.to_str().unwrap()],
        )
        .unwrap();
        ensure_repository(&linked).unwrap();
        let bare = repo.0.join("bare");
        run(&repo.0, &["init", "--bare", bare.to_str().unwrap()]).unwrap();
        assert!(ensure_repository(&bare).is_err());
    }

    #[test]
    fn clean_check_covers_untracked_staged_and_unstaged_changes() {
        let repo = Repository::new();
        require_clean(&repo.0).unwrap();
        fs::write(repo.0.join("file.txt"), "initial\n").unwrap();
        assert!(require_clean(&repo.0).is_err());
        run(&repo.0, &["add", "--", "file.txt"]).unwrap();
        assert!(require_clean(&repo.0).is_err());
        run(&repo.0, &["commit", "-m", "Initial"]).unwrap();
        require_clean(&repo.0).unwrap();
        fs::write(repo.0.join("file.txt"), "changed\n").unwrap();
        assert!(require_clean(&repo.0).is_err());
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
        let repo = Repository::new();
        let error = run(
            &repo.0,
            &["rev-parse", "--verify", "https://secret@github.com/x"],
        )
        .unwrap_err()
        .to_string();
        assert!(!error.contains("secret"));
    }

    #[test]
    fn authentication_is_ephemeral_and_legacy_credentials_are_removed() {
        let repo = Repository::new();
        run(
            &repo.0,
            &[
                "remote",
                "add",
                "origin",
                "https://old-token@github.com/org/repo.git",
            ],
        )
        .unwrap();
        let before = fs::read(repo.0.join(".git/config")).unwrap();
        let provider = provider();
        let args = ["fetch", "--prune", "--", "origin"];
        let (config, secrets) = network_config(&repo.0, &args, &provider).unwrap();
        let output = checked_output(
            execute(&repo.0, &["remote", "get-url", "origin"], &config).unwrap(),
            &secrets,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap().trim(),
            "https://github.com/org/repo.git"
        );
        let command = command(&repo.0, &args, &config);
        let argv: Vec<_> = command.get_args().collect();
        assert!(
            argv.iter()
                .all(|arg| !arg.to_string_lossy().contains(&provider.access_token))
        );
        assert!(config.iter().any(|(key, value)| key
            == "http.https://github.com/org/repo.git.extraHeader"
            && value.starts_with("Authorization: Basic ")));
        assert!(
            config
                .iter()
                .any(|(key, value)| key == "http.followRedirects" && value == "false")
        );
        assert_eq!(fs::read(repo.0.join(".git/config")).unwrap(), before);
    }

    #[test]
    fn rejects_foreign_hosts_rewrites_and_multiple_push_destinations() {
        let repo = Repository::new();
        let provider = provider();
        for url in [
            "https://attacker.invalid/org/repo.git",
            "http://github.com/org/repo.git",
            "https://github.com:444/org/repo.git",
            "https://github.com/org/repo.git?token=secret",
        ] {
            assert!(network_config(&repo.0, &["clone", "--", url, "target"], &provider).is_err());
        }
        run(
            &repo.0,
            &["remote", "add", "origin", "https://github.com/org/repo.git"],
        )
        .unwrap();
        run(
            &repo.0,
            &[
                "config",
                "--add",
                "remote.origin.pushurl",
                "https://github.com/org/repo.git",
            ],
        )
        .unwrap();
        run(
            &repo.0,
            &[
                "config",
                "--add",
                "remote.origin.pushurl",
                "https://attacker.invalid/stolen.git",
            ],
        )
        .unwrap();
        assert!(network_config(&repo.0, &["push", "--", "origin", "main"], &provider).is_err());
        run(
            &repo.0,
            &[
                "config",
                "url.https://attacker.invalid/.insteadOf",
                "https://github.com/",
            ],
        )
        .unwrap();
        assert!(network_config(&repo.0, &["fetch", "--", "origin"], &provider).is_err());
    }

    #[test]
    fn clone_options_and_enterprise_hosts_are_supported() {
        let repo = Repository::new();
        let provider = GitHubProvider {
            base_url: "https://github.example.com/api/v3".into(),
            ..provider()
        };
        let args = [
            "clone",
            "--branch",
            "main",
            "--depth",
            "1",
            "--",
            "https://github.example.com/org/repo.git",
            "target",
        ];
        assert_eq!(destination_index(&args).unwrap(), 6);
        network_config(&repo.0, &args, &provider).unwrap();
        assert!(destination_index(&["fetch", "--all"]).is_err());
        assert!(destination_index(&["fetch", "--upload-pack=sh", "origin"]).is_err());
    }

    #[test]
    fn authentication_overrides_url_specific_redirect_and_tls_configuration() {
        let repo = Repository::new();
        let url = "https://github.com/org/repo.git";
        run(&repo.0, &["remote", "add", "origin", url]).unwrap();
        run(
            &repo.0,
            &["config", &format!("http.{url}.followRedirects"), "true"],
        )
        .unwrap();
        run(
            &repo.0,
            &["config", &format!("http.{url}.sslVerify"), "false"],
        )
        .unwrap();
        run(
            &repo.0,
            &[
                "config",
                &format!("http.{url}.extraHeader"),
                "Authorization: stale",
            ],
        )
        .unwrap();
        let (config, secrets) =
            network_config(&repo.0, &["fetch", "--", "origin"], &provider()).unwrap();
        for (key, expected) in [
            ("http.followRedirects", "false"),
            ("http.sslVerify", "true"),
        ] {
            let output = checked_output(
                execute(&repo.0, &["config", "--get-urlmatch", key, url], &config).unwrap(),
                &secrets,
            )
            .unwrap();
            assert_eq!(String::from_utf8(output).unwrap().trim(), expected);
        }
        let output = checked_output(
            execute(
                &repo.0,
                &["config", "--get-urlmatch", "http.extraHeader", url],
                &config,
            )
            .unwrap(),
            &secrets,
        )
        .unwrap();
        assert!(!String::from_utf8(output).unwrap().contains("stale"));
    }

    #[test]
    fn byte_output_preserves_git_objects() {
        let repo = Repository::new();
        let data = b"blob\0\xff\nhttps://literal@example.invalid/\n";
        fs::write(repo.0.join("binary"), data).unwrap();
        let hash = run(&repo.0, &["hash-object", "-w", "--", "binary"]).unwrap();
        let output = run_bytes(&repo.0, &["cat-file", "blob", hash.trim()]).unwrap();
        assert_eq!(output, data);
    }

    #[test]
    fn clean_check_rejects_an_unfinished_merge_with_no_file_changes() {
        let repo = Repository::new();
        run(&repo.0, &["commit", "--allow-empty", "-m", "Initial"]).unwrap();
        run(&repo.0, &["switch", "--create", "topic"]).unwrap();
        run(&repo.0, &["commit", "--allow-empty", "-m", "Topic"]).unwrap();
        run(&repo.0, &["switch", "main"]).unwrap();
        run(&repo.0, &["merge", "--no-ff", "--no-commit", "topic"]).unwrap();
        assert!(
            run_bytes(&repo.0, &["status", "--porcelain=v1", "-z"])
                .unwrap()
                .is_empty()
        );
        assert!(
            require_clean(&repo.0)
                .unwrap_err()
                .to_string()
                .contains("in progress")
        );
        run(&repo.0, &["merge", "--abort"]).unwrap();
        require_clean(&repo.0).unwrap();
    }

    #[test]
    fn explicit_push_cannot_inherit_mirror_or_tag_pushes() {
        let repo = Repository::new();
        run(
            &repo.0,
            &["remote", "add", "origin", "https://github.com/org/repo.git"],
        )
        .unwrap();
        run(&repo.0, &["config", "remote.origin.mirror", "true"]).unwrap();
        run(&repo.0, &["config", "push.followTags", "true"]).unwrap();
        let (config, secrets) = network_config(
            &repo.0,
            &["push", "--", "origin", "HEAD:refs/heads/main"],
            &provider(),
        )
        .unwrap();
        for key in ["remote.origin.mirror", "push.followTags"] {
            let output = checked_output(
                execute(&repo.0, &["config", "--get", key], &config).unwrap(),
                &secrets,
            )
            .unwrap();
            assert_eq!(String::from_utf8(output).unwrap().trim(), "false");
        }
        assert!(network_config(&repo.0, &["push", "--", "origin"], &provider()).is_err());
    }

    #[test]
    fn pruning_remote_branches_preserves_local_tags() {
        let repo = Repository::new();
        run(
            &repo.0,
            &["remote", "add", "origin", "https://github.com/org/repo.git"],
        )
        .unwrap();
        for key in ["fetch.pruneTags", "remote.origin.pruneTags"] {
            run(&repo.0, &["config", key, "true"]).unwrap();
        }
        let (config, secrets) =
            network_config(&repo.0, &["fetch", "--prune", "--", "origin"], &provider()).unwrap();
        for key in ["fetch.pruneTags", "remote.origin.pruneTags"] {
            let output = checked_output(
                execute(&repo.0, &["config", "--get", key], &config).unwrap(),
                &secrets,
            )
            .unwrap();
            assert_eq!(String::from_utf8(output).unwrap().trim(), "false");
        }
    }
}
