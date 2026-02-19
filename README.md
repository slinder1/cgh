GitHub stacked-PR builder for those who miss Gerrit

See `--help` for more.

# Alternatives

The most compelling alternative to `gd` is https://github.com/ejoffe/spr which differs in a few ways:

* `spr` will modify your local branches by default for logically non-destructive operations (i.e. when you try to `update` the remote)
* `spr` won't use Gerrit `Change-Id:`, and is very particular about the format of its ID; `gd` allows any string and uses the `Change-Id:` trailer
* `spr` does not seem to have a `dry-run` option, so modifications aren't forseeable
* `spr` installs itself as a git subcommand (this is really just an aesthetic quibble, but I don't think it is primarily a `git` tool, it is a GitHub tool, and exists only to patch a deficiency in GitHub as a service)
* `spr` warns you to only close/merge PRs through it, rather than just diagnosing when e.g. a PR would be created for a change which already has a merged PR
* `spr` uses YAML for configuration, `gd` uses TOML
* `spr` is noisy by default, and has no `quiet` option
* `spr` seems slightly less aggressive with parallelizing operations
* `spr` is written in Go, `gd` is written in Rust

In the end most of these are fairly aesthetic and minor, but rather than try to hack on `spr` I opted to start over and make the exact tool I wanted. YMMV
