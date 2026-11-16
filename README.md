<p align="center">
  <img src="assets/icon.png" width="132" alt="Shepherd">
</p>

<h1 align="center">Shepherd</h1>

<p align="center">
  <strong>An editor for the age of agents.</strong><br>
  They write the code. You steer, review, fix and ship.
</p>

<p align="center">
  <a href="https://github.com/rasmusraasuke/shepherd/actions/workflows/ci.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/rasmusraasuke/shepherd/ci.yml?branch=main&style=flat-square&label=ci"></a>
  <a href="LICENSE"><img alt="MIT license" src="https://img.shields.io/badge/license-MIT-1B2A41?style=flat-square"></a>
  <img alt="macOS 12 or newer" src="https://img.shields.io/badge/macOS-12%2B-1B2A41?style=flat-square">
  <img alt="Apple Silicon and Intel" src="https://img.shields.io/badge/universal-arm64%20%7C%20x86__64-1B2A41?style=flat-square">
</p>

<br>

Most editors were designed for a world where you type every line. Shepherd is designed for
the world that is arriving: agents do the writing, and the work becomes steering, reading and
deciding.

It is a native macOS editor built around that loop.

<p align="center">
  <img src="assets/screenshot.png" alt="Reviewing an agent's change in Shepherd">
</p>

<p align="center">
  <sub>Reviewing an agent's change. The marks are git's staging area.</sub>
</p>

- **The review is the point.** Changed files as a tree with their progress, diffs
  highlighted down to the words that changed, comments on the lines they are about. The
  review is the file, and the file can be edited right there in the diff.
- **Reviewed means staged.** Marking a change reviewed stages it, line by line if you like.
  There is no second place where review state lives; git is the state.
- **Agents are first-class.** Claude Code and Opencode run in Agent tabs of their own, with
  an agent and model chosen per session. Review comments, quick chat and everything else the
  editor hands over go there, never to a shell.
- **Everything is a tab.** Files, terminals, reviews and agents open side by side, stack,
  drag between windows, and come back after a restart.
- **Quiet by design.** No toolbar and no status bar. One calm palette in dark and light, the
  macOS fonts, and the code taking the rest.

The full list is in [FEATURES.md](FEATURES.md). The road ahead is in [PLAN.md](PLAN.md).

## Install

Shepherd is a universal macOS app for Apple Silicon and Intel, macOS 12 or newer.

1. Download the [latest release](https://github.com/rasmusraasuke/shepherd/releases/latest).
2. Drag `Shepherd.app` into Applications.
3. The build is signed ad hoc, so the first launch is a right-click on the app and *Open*,
   or once from a terminal:
   ```
   xattr -dr com.apple.quarantine /Applications/Shepherd.app
   ```

Or build it from the source:

```sh
git clone https://github.com/rasmusraasuke/shepherd.git
cd shepherd
./scripts/bundle.sh --universal --install --link
```

That builds `Shepherd.app`, installs it and links `shepherd` into `~/.local/bin`. Add
`--zip` to also produce the release zip. The icon is `assets/icon.png`.

## Open a project

```sh
shepherd ~/code/your-project
```

Without a path it opens the current directory, or the repository it opened last, so it can
also be started from Spotlight or the Dock. The path must be inside a git repository.

A few keys to start with:

- `⌘P` opens a file by name, `⇧⇧` searches the project
- `⌃⌃` chats with an agent, `⌘T` opens a terminal
- `⌘/` shows every shortcut, and the version
## Agents

Shepherd works with **Claude Code** and **Opencode**, found on the `PATH`. Each Agent tab is a
real session that survives a restart, with the model chosen when it starts. The commit
message can be drafted by an agent of its own, in the background, so the model that writes
the message need not be the model that does the work.

Agents can see what you see. [`shepherd ctl`](FEATURES.md#agents-in-the-loop) gives them the
open file and selection, your review comments, what is staged, and the terminals; it also lets
them show you files and changes.

## Updates

When it starts, Shepherd asks GitHub for the latest release. A newer one downloads in the
background, and a bar above the editor offers to restart into it. The version is under `⌘/`,
next to a check for updates.

The repository is public, so no token is needed. A fork kept private can set
`SHEPHERD_GITHUB_TOKEN`, or put a token with read access in
`~/Library/Application Support/Shepherd/github-token`.

## Development

```sh
cargo run --release -- path/to/repository
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

CI runs the same three on every push to `main` and on pull requests.

<details>
<summary>Releasing</summary>

Change the version in `Cargo.toml`, commit, then tag and push:

```sh
git tag v0.2.0
git push origin main v0.2.0
```

The tag starts `.github/workflows/release.yml`, which builds the universal app, checks the
tag against `Cargo.toml` and publishes `Shepherd-<version>-macos-universal.zip` with the
release notes. A tag with a hyphen, such as `v0.2.0-rc.1`, is published as a pre-release and
is not offered as an update.

</details>

## License

MIT © 2026 Rasmus Raasuke. See [LICENSE](LICENSE).
