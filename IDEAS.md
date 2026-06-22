At the core of the "stacked PR" is a hack to work around a deficiency in
GitHub's PR model. This document is intended to catalog the issues that fall
out of that deficiency, and outline possible workarounds.

# Issues

## Supply-chain security

I think the most critical failing of the "stacked PR" model is normalizing the
PR author's control of the diff under review.

With some subtle manipulation of the base branch of intermediate PRs a
malicious author can effectively "launder" their changes to appear as-if they
were just part of the codebase already. Unless reviewers are careful to
meticulously review the stacking of the PRs looking for rogue commits, and
always do full re-review of the final diff before actually approving, they are
left open to someone sneaking in changes which were never actually reviewed or
approved. Put another way: the "stacked PR" crutch makes socially engineering
approval of code which was never actually reviewed far easier.

## Redundant approval

It seems like GitHub recognized the perils of allowing the stacked PR author to
carry forward approval when the PR base changes, and so patched this by
requiring another approval when the base changes.

I don't think this actually moves the needle much, though. Considering only my
own capacity for code review, I imagine that this extra approval traffic more
often leads to reviewers just hitting approve again when they are asked,
because "GitHub stacked PRs just require these extra approvals."

So, while GitHub has nominally patched this issue, I don't feel like they have
really resolved it. The issue is actually non-technical, and a technical patch
can't fix that, it can only push around the blame. At the root there is a
technical fix: supporting a patch series workflow in PRs without the "stacked
PR" hack.

## Loss of comment context

Any force-push means PR feedback context is lost. This leads to the following
guidance:

* Avoid rebasing your PRs for as long as you can, as a true rebase requires a
force push.
* When addressing feedback, add fixup commits instead of amending.

These constraints make maintaining a large patch series very difficult in an
active project like e.g. LLVM:

* The longer one avoids rebasing the more painful it eventually is, and the
higher the risk that the patch fundamentally changes in a way that will require
duplicated effort in reviewing later.
* Littering a series with many fixups makes it difficult to manage, and forces
the author to maintain the logical patchset in their mind rather than record it
to the branch.

Some of this can be mitigated with tools like `rerere` and frequent uncommited
rebasing/merging. However, it is fundamentally a chore, and one that is clearly
not fundamental to the code review process, as evidenced by tools like Gerrit
Just Working while requiring no such guidance.

# Ideas

Below I will refer to a "change" in the same way that it is understood in this
codebase: a unique change, identified by a string which is currently encoded
as a "Commit-Id:" footer in commit messages and PR bodies. A change "has"
local commit(s) and PR(s).

# Just use even more branches

(For simplicity I will assume all of the local changes already have associated PRs.)

Each change has a unique remote branch for both the PR base and head (rather
than chaining them such that the head of one PR is the base of another).

When syncing the local series of changes to GitHub, for each change:

* Add a new commit to the PR with the same tree as the new "logical base" of
the change. The diff for this commit basically "undoes" the effects of the
change itself while also carrying forward any diff from a rebase or
rearrangement of commits. Call this commit A.
* Add another new commit on top of commit A, with the same tree as the local
commit for the change. The diff for this commit basically "redoes" the full
effects of the change relative to commit A, as if the author was creating a new
PR for the change. Call this commit B.
* Fast-forward the unique base branch to commit A, and the unique head branch
to commit B.

This avoids any force-pushes while still reflecting fixups and rebases on the
local patch series branch. I haven’t done extensive testing, but from some
quick checks it seems like GitHub doesn’t lose track of the comments in this
system like it does with a force-push.

This does add a new issue, in that not even the first commit in the stack is
based on main. You can’t just hit merge. The tool will have to manage the
merge, and it likely can’t even do that directly because GitHub clears
approvals when the base branch changes on a PR.

The best idea I have for managing this would be to require a new PR be created
after "real review" results in an approval. The new PR has a single commit with
the same final tree as the approved stacked PR. A bot could even create the new
PR or just verify that they are identical. The bot could also approve the new
PR (because its tree was approved by a human), and close the stacked version
once the proxy is merged.

This all seems a bit convoluted, and requires the community to host a bot which
it trusts to approve reviews, but it would have the nice side-effect of leaving
the original stacked PRs alone, so all discussions on them are correctly
maintained against the code as it was when they were made in perpetuity.

## had a new idea

computers were a mistake
