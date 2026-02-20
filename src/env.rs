// Copyright © 2026 Advanced Micro Devices, Inc. All rights reserved.
// SPDX-License-Identifier: MIT

use crate::util::Extract;
use anyhow::{Context, Result, bail};
use atomic_counter::{AtomicCounter, RelaxedCounter};
use clap::{ArgAction, Args, Parser, Subcommand};
use git2::Repository;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, read_to_string};
use std::path::PathBuf;
use tlrepo::ThreadLocalRepo;

#[derive(Default, Serialize, Deserialize)]
pub struct Config {
    remote: String,
    base_branch: String,
    user_branch_prefix: String,
    reviewer_groups: Option<HashMap<String, Vec<String>>>,
}

fn repo_config_path(filename: &str) -> Option<PathBuf> {
    REPO.get()
        .ok()
        .and_then(|r| r.workdir())
        .map(|wd| wd.join(filename))
        .filter(|p| fs::exists(p).unwrap_or(false))
}

fn user_config_path(filename: &str) -> Option<PathBuf> {
    dirs::config_dir()
        .map(|cd| cd.join(filename))
        .filter(|p| fs::exists(p).unwrap_or(false))
}

fn read_config() -> Result<Config> {
    let path = std::env::var_os("GD_CONFIG_PATH")
        .map(PathBuf::from)
        .or_else(|| repo_config_path(".gd.toml"))
        .or_else(|| repo_config_path("gd.toml"))
        .or_else(|| user_config_path("gd.toml"));
    let path = match path {
        Some(p) => p,
        None => {
            // We will catch this later when the config is validated, but we cannot
            // fail here or -h/--help will not be reached.
            return Ok(Default::default());
        }
    };
    let contents = read_to_string(path.clone())
        .with_context(|| format!("could not read config file: {path:?}"))?;
    let config: Config = toml::from_str(contents.as_ref())
        .with_context(|| format!("invalid config file: {path:?}"))?;
    Ok(config)
}

/// GitHub stacked-PR builder for those who miss Gerrit
///
/// Main features:
///
/// * Never touches your local branches. The tool only reads from your local branch and attempts to
///   mirror it to GitHub by: fetching remote tracking branches, force-pushing namespaced refs,
///   creating PRs, and maintaining PR bodies and comments to present a pseudo-UI for the stack.
/// * Treats one branch as one patch-stack, where each commit maps 1:1 to a PR.
/// * Uses the same "Change-Id" trailer used by Gerrit. You can install the commit-msg hook from
///   a Gerrit instance or use the install-hook subcommand to install an embedded copy.
/// * Generates "interdiff"-esque diffs for updates to changes and posts them as a comment on your
///   behalf when you push. This is a bit of a workaround to mitigate the fallout from having to
///   force-push.
/// * Quiet by default. No news is good news, but you can also get verbose output or a dry-run.
/// * Uses the official `gh` tool to interface with the GitHub API, so you don't have
///   to go through authenticating another app.
/// * Painfully slow, but at least tries to claw back performance where possible, primarily by
///   parallelizing steps across all patches in the branch.
///
/// And currently its greatest shortcomings are:
///
/// * Does not even try to avoid force pushes. Review comments will regularly end up marked as
///   stale with no relation to the latest patch contents. This seems to happen frequently anyway,
///   and avoiding it in the general case requires never rebasing which is not viable for anything
///   but an extremely short-lived review process. Ideas about how to potentially resolve this is
///   documented at https://github.com/slinder1/gd/blob/main/DESIGN.md and contributions are
///   welcome!
/// * Loses track of merged/closed PRs. This may be mildly confusing, but is more-or-less by
///   design: the change commit which corresponds to a merged PR will naturally disappear from the
///   branch on rebase. As a sort of workaround, if you rebase using `--reapply-cherry-picks
///   --empty=keep` you can maintain the stack at the cost of having empty commits in your local
///   stack.
/// * Currently lacks a lot of polish and documentation.
///
/// It reads configuration from the first of the following:
///
/// * The file identified by the environment variable `CM_CONFIG_PATH`, if that variable is set.
/// * The file `.gd.toml` in the git repo's workdir, if it exists.
/// * The file `gd.toml` in the git repo's workdir, if it exists.
/// * The file `gd.toml` in platform-dependant user config dir, otherwise.
///
/// An example config file is:
///
///     remote = "origin"
///     base_branch = "main"
///     user_branch_prefix = "users/$USER/"
#[derive(Parser)]
#[command(version, verbatim_doc_comment, args_override_self = true)]
pub struct Cli {
    #[clap(flatten)]
    pub globals: Globals,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Args)]
pub struct Globals {
    /// The name of the git remote corresponding to the GitHub repo to operate on.
    #[arg(long, global = true, default_value_t = CONFIG.remote.clone())]
    pub remote: String,
    /// The branch on `remote` which acts as the "base" branch, which all PRs are ultimately
    /// relative to.
    #[arg(long, global = true, default_value_t = CONFIG.base_branch.clone())]
    pub base_branch: String,
    /// The prefix for all remote branches created by the tool. Can be empty.
    #[arg(long, global = true, default_value_t = CONFIG.user_branch_prefix.clone())]
    pub user_branch_prefix: String,
    /// Limit the global thread pool used by `gd` to have only one thread.
    #[arg(long, global = true)]
    pub serial: bool,
    /// Give a verbose summary of what would happen if executed.
    ///
    /// Note: This still makes read-only queries to the git repo and GitHub APIs. Only operations
    /// which have the potential to mutate remote state are skipped and printed.
    #[arg(short = '#', long, global = true)]
    pub dry_run: bool,
    /// Output all commands executed, and their stdout/stderr.
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand)]
pub enum Command {
    /// Push the current branch as a stack of GitHub PRs.
    ///
    /// The commits `${base}..HEAD` must each have a `Change-Id:` trailer. Each commit will be
    /// force-pushed to a corresponding branch named `${user_branch_prefix}${change_id}` on
    /// `${remote}`. Each commit will be matched to its existing PR or else a new PR will be
    /// created for it. The PRs will be "stacked" such that they reproduce the local branch
    /// sequence, with additional trailers in the PR message body to help reviewers navigate the
    /// stack.
    ///
    /// Note: This command will never modify the local repo. No local branches are created or
    /// destroyed, and no commits are touched. All mutation occurs exclusively on the `$remote`.
    #[command(visible_alias = "p")]
    Push(Push),
    /// Install a commit-msg hook in the current git repo to create `Change-Id:` trailers.
    InstallHook(InstallHook),
}

#[derive(Args)]
pub struct Push {
    /// A comma-separated list of reviewer groups to apply from the config file.
    ///
    /// An example config snippet defining two groups `internal` and `public`:
    ///
    ///     [reviewer_groups]
    ///     internal = [ "dev1", "dev2" ]
    ///     public = [ "dev1", "dev3", "dev4" ]
    #[arg(short, long, action = ArgAction::Set, value_delimiter = ',')]
    pub reviewer_groups: Vec<String>,
}

#[derive(Args)]
pub struct InstallHook {
    /// Install the hook over any existing commit-msg hook.
    #[arg(short, long)]
    pub force: bool,
}

lazy_static! {
    static ref REPO: ThreadLocalRepo = ThreadLocalRepo::new(".".into());
    static ref CONFIG: Config = read_config().extract();
    static ref CLI: Cli = Cli::parse();
    static ref BASE_BRANCH_REF: String = format!("refs/heads/{}", CLI.globals.base_branch);
    static ref EXEC_IDS: RelaxedCounter = RelaxedCounter::new(0);
}

pub fn validate() -> Result<()> {
    if remote().is_empty() {
        bail!("field `remote` cannot be empty");
    }
    if base_branch().is_empty() {
        bail!("field `base_branch` cannot be empty");
    }
    if !user_branch_prefix().is_empty() && !user_branch_prefix().ends_with('/') {
        bail!("if field `user_branch_prefix` is non-empty it must end with `/`");
    }
    Ok(())
}

pub fn cli() -> &'static Cli {
    &CLI
}

pub fn remote() -> &'static str {
    CLI.globals.remote.as_ref()
}

pub fn base_branch() -> &'static str {
    CLI.globals.base_branch.as_ref()
}

pub fn base_branch_ref() -> &'static str {
    BASE_BRANCH_REF.as_ref()
}

pub fn user_branch_prefix() -> &'static str {
    CLI.globals.user_branch_prefix.as_ref()
}

pub fn reviewer_groups() -> Option<&'static HashMap<String, Vec<String>>> {
    CONFIG.reviewer_groups.as_ref()
}

pub fn dry_run() -> bool {
    CLI.globals.dry_run
}

pub fn verbose() -> bool {
    CLI.globals.verbose
}

pub fn always_echo() -> bool {
    dry_run() || verbose()
}

pub fn repo() -> &'static Repository {
    REPO.get().context("not in a git repo").extract()
}

pub fn next_exec_id() -> usize {
    EXEC_IDS.inc()
}
