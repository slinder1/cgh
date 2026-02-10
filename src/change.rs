// Copyright © 2026 Advanced Micro Devices, Inc. All rights reserved.
// SPDX-License-Identifier: MIT

use crate::env;
use crate::gh::Pr;
use crate::util::exec;
use anyhow::{Context, Result, bail};
use git2::Oid;
use git2::message_trailers_bytes;
use std::process::Command;

#[derive(Debug)]
pub enum AnyChange {
    LocalChange(LocalChange),
    Change(Change),
}

impl AnyChange {
    pub fn local_change(&self) -> &LocalChange {
        match self {
            AnyChange::LocalChange(local_change) => local_change,
            AnyChange::Change(change) => &change.local_change,
        }
    }
}

#[derive(Debug)]
pub struct LocalChange {
    pub id: String,
    pub oid: Oid,
}

impl LocalChange {
    pub fn remote_branch(&self) -> String {
        let branch_prefix = env::user_branch_prefix();
        let change_id = &self.id;
        format!("{branch_prefix}{change_id}")
    }
    pub fn remote_branch_ref(&self) -> String {
        format!("refs/heads/{}", self.remote_branch())
    }
    pub fn push_refspec(&self) -> String {
        let oid = self.oid;
        let remote_branch_ref = self.remote_branch_ref();
        format!("{oid}:{remote_branch_ref}")
    }
    pub fn push_all<'a, I: Iterator<Item = &'a Self>>(iterator: I) -> Result<()> {
        let mut cmd = Command::new("git");
        let mut args = vec!["push".to_string(), env::remote().into(), "--force".into()];
        args.extend(iterator.map(|lc| lc.push_refspec()));
        cmd.args(args);
        exec!(dry_return = (), cmd);
        Ok(())
    }
}

#[derive(Debug)]
pub struct Change {
    pub local_change: LocalChange,
    pub pr: Pr,
}

impl Change {
    pub fn render_pr_ui(&self, changes: &[Self]) -> Result<()> {
        let commit = env::repo().find_commit(self.local_change.oid)?;
        let mut index = None;
        let title = String::from(commit.summary().context("commit has no summary")?);
        let mut body = String::from(commit.body().context("commit has no body")?);
        body.push_str("\n\n---\n\n");
        body.push_str("**Stack**:\n");
        for (i, c) in changes.iter().enumerate() {
            body.push_str(&format!("- #{}", c.pr.number));
            if self.pr.number == c.pr.number {
                index = Some(i);
                body.push('⬅');
            }
            body.push('\n');
        }
        let count = changes.len();
        let position = count
            - index.expect(
                "render_pr_ui asked to render into a stack of changes which does not contain self?",
            );
        self.pr
            .set_title_and_body(&format!("[{position}/{count}]: {title}"), &body)
    }
}

pub fn get_local_changes() -> Result<Vec<LocalChange>> {
    let repo = env::repo();
    let mut local_changes = vec![];
    let mut revwalk = repo.revwalk()?;
    revwalk.push_head()?;
    revwalk.hide_ref(env::base_branch_ref())?;
    for oid in revwalk {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;
        let trailers = message_trailers_bytes(
            commit
                .message_raw()
                .with_context(|| format!("commit lacks message: {commit:?}"))?,
        )
        .context("message_trailer_bytes failed")?;
        let mut change_ids = trailers.iter().filter_map(|(k, v)| {
            if k == b"Change-Id" {
                Some(String::from_utf8(v.into()))
            } else {
                None
            }
        });
        let id = change_ids
            .next()
            .with_context(|| format!("commit lacks Change-Id: {commit:?}"))?
            .with_context(|| format!("commit Change-Id is not valid utf8: {commit:?}"))?;
        if change_ids.next().is_some() {
            bail!("commit has multiple Change-Id: {commit:?}");
        }
        local_changes.push(LocalChange { id, oid });
    }
    Ok(local_changes)
}
