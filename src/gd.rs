// Copyright © 2026 Advanced Micro Devices, Inc. All rights reserved.
// SPDX-License-Identifier: MIT

use crate::change::{self, AnyChange, Change, LocalChange};
use crate::cli;
use crate::env;
use crate::gh::{self, Pr, PrState};
use crate::util::{Extract, RepoExt};
use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use std::collections::HashSet;
use std::ffi::{OsString, OsStr};
use std::fs::File;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process;

pub fn gd() -> Result<()> {
    let cli = env::cli();
    if cli.globals.serial {
        rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build_global()
            .context("could not install serial thread pool")
            .extract();
    }
    if let cli::Command::Config(ref cfg) = cli.command {
        config(cfg)
    } else {
        env::validate().context("invalid configuration (Hint: try the `config` subcommand)")?;
        match cli.command {
            cli::Command::Push(ref cfg) => push(cfg),
            cli::Command::InstallHook(ref cfg) => install_hook(cfg),
            cli::Command::Config(_) => unreachable!("config should have already been handled"),
        }
    }
}

fn push(cfg: &cli::Push) -> Result<()> {
    let repo = env::repo();
    let branch = repo.head_branch().context("HEAD must be a branch")?;
    let branch_desc = repo.branch_desc(branch).ok();
    let mut reviewers = vec![];
    for group_key in cfg.reviewer_groups.iter() {
        let group = env::reviewer_groups()
            .context("used a review group, but none were found in the config file")?
            .get(group_key)
            .with_context(|| format!("reviewer group {group_key:?} not found in config file"))?;
        reviewers.extend_from_slice(group);
    }
    let local_changes =
        change::get_local_changes().context("could not enumerate current local branch")?;
    let mut prs_by_change_id = gh::prs_by_change_id(|pr| !pr.in_state(PrState::Closed))
        .context("could not enumerate remote prs")?;
    let mut any_changes = vec![];
    for local_change in local_changes {
        any_changes.push(match prs_by_change_id.remove(&local_change.id) {
            None => AnyChange::LocalChange(local_change),
            Some(pr) => {
                if pr.in_state(PrState::Merged) && local_change.is_nonempty() {
                    bail!(
                        "pr {} with Change-Id {} already merged",
                        pr.number,
                        local_change.id
                    );
                }
                AnyChange::Change(Change { local_change, pr })
            }
        });
    }
    let has_cycles = detect_cycles(&any_changes);
    if has_cycles || cfg.draft {
        any_changes
            .par_iter()
            .filter_map(|ac| {
                if let AnyChange::Change(c) = ac
                    && c.is_nonempty()
                {
                    Some(c)
                } else {
                    None
                }
            })
            .map(|c| {
                c.pr.mark_ready(false)
                    .with_context(|| format!("could not mark pr as draft: {:?}", c.pr))?;
                // FIXME: This is pretty coarse-grained, could find the minimal set.
                if has_cycles {
                    c.pr.set_base(env::base_branch()).with_context(|| {
                        format!(
                            "could not retarget pr {} to base branch: {:?}",
                            c.pr.number,
                            env::base_branch(),
                        )
                    })?;
                }
                Ok(())
            })
            .collect::<Result<Vec<_>>>()?;
    }
    LocalChange::fetch_all(any_changes.iter().filter_map(|ac| match ac {
        AnyChange::Change(c) if c.is_nonempty() => Some(&c.local_change),
        _ => None,
    }))
    .context("could not fetch base branches for all existing prs")?;
    let diffs = any_changes
        .par_iter()
        .map(|ac| ac.diff())
        .collect::<Result<Vec<_>>>()
        .context("could not build diffs")?;
    // FIXME: Should we push a slightly modified version of the branch with empty commits stripped
    // out? It would clean up the "Commits" tab on the PRs, and make the final merge message
    // correct without edits.
    LocalChange::push_all(any_changes.iter().map(|ac| ac.local_change()))
        .context("could not push all local changes")?;
    // FIXME: Should try to restore the original branch contents if we fail from this point on. It
    // would be at least an attempt at being "atomic" about the push, and it would mean we don't
    // lose the interdiff in a future re-run.
    let changes = any_changes
        .into_par_iter()
        .map(|any_change| {
            let change = match any_change {
                AnyChange::LocalChange(local_change) => {
                    let pr = Pr::create(&local_change)
                        .with_context(|| format!("could not create new pr for {local_change:?}"))?;
                    Change { local_change, pr }
                }
                AnyChange::Change(change) => change,
            };
            Ok(change)
        })
        .collect::<Result<Vec<_>>>()
        .context("could not create new prs")?;
    changes
        .par_iter()
        .enumerate()
        .map(|(i, c)| {
            let parents = &changes[i + 1..];
            let base = parents
                .iter()
                .find(|c| c.is_nonempty())
                .map(|p| p.local_change.remote_branch())
                .unwrap_or_else(|| env::base_branch().to_owned());
            if c.is_nonempty() {
                c.pr.set_base(base.as_ref()).with_context(|| {
                    format!(
                        "could not retarget pr {} to branch: {:?}",
                        c.pr.number, base,
                    )
                })?;
            }
            c.render_pr_ui(&changes, branch_desc.as_deref())
                .context("could not render pseudo-ui in pr title/body")
        })
        .collect::<Result<Vec<_>>>()
        .context("could not set pr bases and bodies")?;
    changes
        .par_iter()
        .zip(diffs)
        .map(|(c, diff)| c.pr.add_details_comment(&diff))
        .collect::<Result<Vec<_>>>()
        .context("could not add interdiff comments")?;
    changes
        .par_iter()
        .filter(|c| c.is_nonempty())
        .map(|c| c.pr.add_reviewers(reviewers.as_ref()))
        .collect::<Result<Vec<_>>>()
        .context("could not add pr reviewers")?;
    if !cfg.draft {
        changes
            .par_iter()
            .filter(|c| c.is_nonempty())
            .map(|c| c.pr.mark_ready(true))
            .collect::<Result<Vec<_>>>()
            .context("could not mark prs as ready")?;
    }
    Ok(())
}

fn detect_cycles(any_changes: &[AnyChange]) -> bool {
    let mut parent_refs_seen: HashSet<String> = HashSet::new();
    for any_change in any_changes.iter() {
        if let AnyChange::Change(change) = any_change
            && change.local_change.is_nonempty()
        {
            if !parent_refs_seen.is_empty()
                && !parent_refs_seen.contains(&change.local_change.remote_branch())
            {
                return true;
            }
            parent_refs_seen.insert(change.pr.base_ref_name.clone());
        }
    }
    false
}

static COMMIT_MSG_HOOK_SRC: &str = include_str!("commit-msg");
static EXECUTABLE_MODE_BITS: u32 = 0o111;

fn install_hook(cfg: &cli::InstallHook) -> Result<()> {
    let mut hook_path = PathBuf::from(env::repo().commondir());
    hook_path.extend(["hooks", "commit-msg"]);
    if env::dry_run() {
        let verb = if cfg.force { "overwrite" } else { "write" };
        eprintln!("would {verb} {hook_path:?}");
        return Ok(());
    }
    let mut hook_file: File = if cfg.force {
        File::create(&hook_path)
            .with_context(|| format!("could not create hook file: {hook_path:?}"))
    } else {
        File::create_new(&hook_path)
            .with_context(|| format!("could not create hook file (try --force): {hook_path:?}"))
    }?;
    hook_file
        .write_all(COMMIT_MSG_HOOK_SRC.as_bytes())
        .with_context(|| format!("could not write to hook file: {hook_path:?}"))?;
    hook_file
        .flush()
        .with_context(|| format!("could not flush hook file: {hook_path:?}"))?;
    let mut perms = hook_file
        .metadata()
        .with_context(|| format!("could not get metadata for hook file: {hook_path:?}"))?
        .permissions();
    perms.set_mode(perms.mode() | EXECUTABLE_MODE_BITS);
    hook_file
        .set_permissions(perms)
        .with_context(|| format!("could not set permissions for hook file: {hook_path:?}"))?;
    Ok(())
}

fn config(_cfg: &cli::Config) -> Result<()> {
    let repo = env::repo();
    let workdir = repo.workdir().context("git repo has no workdir")?;
    let path = workdir.join("gd.toml");
    let exists = std::fs::exists(&path)
        .with_context(|| format!("could not determine if config exists: {path:?}"))?;
    if env::dry_run() {
        let verb = if exists { "edit" } else { "create and edit" };
        eprintln!("would {verb} {path:?}");
        return Ok(());
    }
    if !exists {
        let mut file = File::create_new(&path)
            .with_context(|| format!("could not create config file: {path:?}"))?;
        let mut config_text = vec![];
        config_text.extend_from_slice(
            b"remote = \"origin\"\nbase_branch = \"main\"\nuser_branch_prefix = \"users/",
        );
        config_text.extend(
            std::env::var_os("USER")
                .unwrap_or(OsString::from("USER"))
                .into_encoded_bytes(),
        );
        config_text.extend_from_slice(b"/\"\n");
        file.write_all(&config_text)
            .with_context(|| format!("could not write to config file: {path:?}. Warning: partial template may have been written"))?;
    }
    let edit_result = open_editor(&path);
    if !exists
        && let Err(e) =
            edit_result.with_context(|| format!("config template was still saved to {path:?}"))
    {
        eprint!("Warning: {e:?}");
    }
    Ok(())
}

fn open_editor<S>(path: S) -> Result<()>
where
    S: AsRef<OsStr>,
{
    let editor = std::env::var_os("VISUAL")
        .or(std::env::var_os("EDITOR"))
        .context("neither VISUAL nor EDITOR set")?;
    let editor = match editor.into_string() {
        Ok(e) => e,
        Err(_) => bail!("VISUAL/EDITOR is not valid utf-8"),
    };
    let mut cmd_and_args = editor.split_whitespace();
    let cmd = cmd_and_args.next().context("VISUAL/EDITOR is blank")?;
    let args: Vec<&str> = cmd_and_args.collect();
    let status = process::Command::new(cmd)
        .args(args)
        .arg(&path)
        .status()
        .context("VISUAL/EDITOR failed to exec")?;
    if !status.success() {
        bail!("VISUAL/EDITOR exited with non-zero status: {status:?}");
    }
    Ok(())
}
