// Copyright © 2026 Advanced Micro Devices, Inc. All rights reserved.
// SPDX-License-Identifier: MIT

use crate::change::LocalChange;
use crate::env;
use crate::util::{Extract, exec};
use anyhow::{Context, Result, bail};
use git2::message_trailers_strs;
use lazy_static::lazy_static;
use serde::Deserialize;
use std::collections::HashMap;
use std::process::Command;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pr {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub state: PrState,
    pub base_ref_name: String,
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum PrState {
    #[default]
    Open,
    Closed,
    Merged,
}

fn gh() -> Command {
    Command::new("gh")
}

lazy_static! {
    static ref REPO_ARG: String = build_repo_arg().extract();
}
fn build_repo_arg() -> Result<String> {
    let remote = env::repo()
        .find_remote(env::remote())
        .with_context(|| format!("remote not found: {}", env::remote()))?;
    let url = remote
        .url()
        .with_context(|| format!("remote has no url: {}", env::remote()))?;
    // FIXME: Making some assumptions here, just getting it working for me first.
    let repo = if url.starts_with("https://") {
        url
    } else if let Some(git_path) = url.strip_prefix("git@github.com:") {
        git_path.strip_suffix(".git").unwrap_or(git_path)
    } else {
        bail!("unhandled git remote url: {url:?}");
    };
    Ok(format!("--repo={repo}"))
}

impl Pr {
    fn args_for<I, S>(&self, subcommand: &str, opts: I) -> Vec<String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut args = vec!["pr".into()];
        args.extend([subcommand.into(), self.number.to_string(), REPO_ARG.clone()]);
        for opt in opts {
            args.push(opt.into());
        }
        args
    }

    pub fn message(&self) -> String {
        format!("{}\n\n{}", self.title, self.body)
    }

    pub fn in_state(&self, state: PrState) -> bool {
        self.state == state
    }

    pub fn set_title_and_body(&self, title: &str, body: &str) -> Result<()> {
        let mut cmd = gh();
        let args = self.args_for(
            "edit",
            [format!("--title={title}"), format!("--body={body}")],
        );
        cmd.args(args);
        exec!(dry_return = (), cmd);
        Ok(())
    }

    pub fn mark_ready(&self, ready: bool) -> Result<()> {
        let mut cmd = gh();
        let opts = if ready {
            vec![]
        } else {
            vec!["--undo".to_string()]
        };
        let args = self.args_for("ready", opts);
        cmd.args(args);
        exec!(dry_return = (), cmd);
        Ok(())
    }

    pub fn set_base(&self, base: &str) -> Result<()> {
        let mut cmd = gh();
        let args = self.args_for("edit", [format!("--base={base}")]);
        cmd.args(args);
        exec!(dry_return = (), cmd);
        Ok(())
    }

    pub fn add_reviewers(&self, reviewers: &[String]) -> Result<()> {
        if reviewers.is_empty() {
            return Ok(());
        }
        let mut cmd = gh();
        let args = self.args_for("edit", [format!("--add-reviewer={}", reviewers.join(","))]);
        cmd.args(args);
        exec!(dry_return = (), cmd);
        Ok(())
    }

    pub fn create(local_change: &LocalChange) -> Result<Pr> {
        let commit = env::repo()
            .find_commit(local_change.oid)
            .context("cannot find commit")?;
        let remote_branch_ref = local_change.remote_branch_ref();
        let title = commit.summary().context("commit has no summary")?;
        let body = commit.body().context("commit has no body")?;
        let base = env::base_branch();
        let mut cmd = gh();
        let args = vec![
            "pr".into(),
            "create".into(),
            "--draft".into(),
            format!("--base={base}"),
            format!("--title={title}"),
            format!("--body={body}"),
            format!("--head={remote_branch_ref}"),
        ];
        cmd.args(args);
        let output = exec!(dry_return = Pr::default(), cmd);
        for line in String::from_utf8_lossy(output.stdout.as_ref()).lines() {
            if line.starts_with("https://github.com") {
                let mut path_components = line.rsplitn(2, '/');
                let number = path_components
                    .next()
                    .with_context(|| format!("gh pr create printed invalid pr URL: {line}"))?;
                return Ok(Pr {
                    number: number.parse::<u64>().context("pr number is not a number")?,
                    title: title.into(),
                    body: body.into(),
                    state: PrState::Open,
                    base_ref_name: base.into(),
                });
            }
        }
        bail!("gh pr create did not produce a URL")
    }
}

fn prs<P: FnMut(&Pr) -> bool>(predicate: P) -> Result<Vec<Pr>> {
    let mut cmd = gh();
    cmd.args([
        "pr",
        "list",
        REPO_ARG.as_ref(),
        "--author=@me",
        "--state=all",
        "--json=number,title,body,state,baseRefName",
    ]);
    let output = exec!(cmd);
    let all_prs: Vec<Pr> = serde_json::from_slice(output.stdout.as_ref())?;
    Ok(all_prs.into_iter().filter(predicate).collect())
}

pub fn prs_by_change_id<P: FnMut(&Pr) -> bool>(predicate: P) -> Result<HashMap<String, Pr>> {
    let mut by_id = HashMap::new();
    for pr in prs(predicate)? {
        let trailers =
            message_trailers_strs(pr.message().as_ref()).context("message_trailers_strs failed")?;
        let mut change_ids = trailers
            .iter()
            .filter_map(|(k, v)| if k == "Change-Id" { Some(v) } else { None });
        let id = match change_ids.next() {
            Some(id) => id,
            None => continue,
        };
        if change_ids.next().is_some() {
            bail!("pr has multiple Change-Id: {pr:?}");
        }
        by_id.insert(id.to_owned(), pr);
    }
    Ok(by_id)
}
