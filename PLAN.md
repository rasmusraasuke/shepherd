# Plan

A lightweight code editor for the AI era. Agents write most of the code; the human's job is
to steer them, review what they produce, fix things by hand and ship. The editor is built
around that loop.

## Principles

- **Three first-class citizens.** File editing, agents and reviews are equals. None of them
  is a side panel of another.
- **Everything is a tab.** A file, an agent, a terminal and a review each open as a tab in
  the main space. Tabs can be placed side by side, stacked below each other, and popped out
  into their own windows.
- **Agent native.** Agents run in tabs of their own and can ask the editor for
  anything the user can see: selection, review comments, diagnostics, run output, git state.
  The editor runs the agents' own CLI programs rather than shipping its own, and adding
  another agent is a small change.
- **One editing component.** The same text component is used for files, for diffs and for
  merge conflicts, so editing works the same everywhere.
- **Stay small.** A feature is added only when it is needed for the review loop or for
  comfortable Kotlin work. See "Deliberately left out".
- **Usable every day from the first milestone.** Each milestone ends in something worth using
  next to IntelliJ, until IntelliJ is no longer needed.

## Where we are

Milestones 1 to 12 are done and FEATURES.md describes what they brought. Milestone 13 has
begun: language servers start on their own for the files opened, show their diagnostics and
hover information with its code highlighted, go to definitions, and give their diagnostics to agents through
`shepherd ctl diagnostics`; the servers are installed on their own when they are missing. What is left of it is below.

## Milestones

### 13. Comfortable Kotlin editing

The step that lets IntelliJ be closed for good.

- Language server support: completion, find usages, rename, formatting, quick fixes, imports
- Files changed on disk but not open (for example by an agent) reported to the servers
- The same intelligence inside the review: go to definition and find usages from a diff line,
  diagnostics shown on changed lines
- Structural editing helpers from the syntax tree: selection by syntax node, outline,
  folding, sensible indentation
- Other languages come for free where a language server exists

Done when: a day of Kotlin work happens here without missing IntelliJ.

### 14. Run configurations

- Run configurations stored as plain files in the repository, so they can be committed and
  shared with the team
- A run button and a picker for the current configuration, as in IntelliJ
- Output in a terminal tab like any other; rerun and stop; several running at once
- Tests: run a test or a class from the editor, failures link to the code
- Agents can list, start and read the output of run configurations

Done when: a teammate can clone the repo and press run.

## Deliberately left out (for now)

These keep the project small. Each can be reconsidered when it starts to hurt.

- Debugger
- A plugin system
- An agent or a chat interface of the editor's own (an Agent tab shows the agent's own
  program)
- Git worktrees (all agents work in one checkout)
- Kotlin analysis beyond the language server
- Pull request review for GitHub/GitLab
- Vim-style modal editing (the keymap should stay configurable so it can be added)
- Refactorings beyond what the language server offers
- Version control systems other than git
- Windows support and Linux polish (macOS first; Linux is built and released, and nothing
  should prevent Windows)
- A settings UI beyond the few switches under Cmd+/ (`settings.json` is enough)

## Decisions

- **One working tree.** All agents work in the same checkout. No git worktree support.
- **One file at a time.** A review shows the changes of one file, chosen from the tree of
  changed files, rather than all changes in one long stream. It shows the changed parts with
  a little context, which grows on request up to the whole file.
- **Review comments are short-lived.** A comment disappears once the lines it is attached
  to change; the changed hunk shows up as unreviewed again. No resolved threads.
- **Files save themselves.** As in IntelliJ, there is no unsaved state to manage: an edit is
  written to disk a moment after typing stops, and when a tab or the editor is closed.
  Agents and build tools always see what the editor shows. When an open file changes on
  disk it is reloaded; only edits typed in the last moment win over the change.
- **The review is the file.** A review edits the same text as the file's tab, anywhere, with
  the removed lines shown between its lines. The review finds the changes itself (Myers, as
  git does) on every edit, against the committed and the staged versions, so that what is
  typed shows up as a change at once and staging works on text that isn't saved yet. Reverts
  are edits of the text, so they are undone like any other edit.
- **Conflicts are hunks of the file.** Conflict markers are read from the file as it is on
  disk, so a conflict is shown and resolved as an edit of the same text: taking a side or
  both, and typing over the markers, are edits, and the resolved file is reviewed and staged
  like any other change. The file stays the source of truth, and the editor keeps no merge
  state of its own.
- **Syntax trees from tree-sitter.** Highlighting is built on tree-sitter, so that the same
  trees serve selection by syntax node, outline and folding later.
- **Terminals on alacritty_terminal.** The terminal emulation and the processes come from
  Alacritty's core, as in Zed; drawing and keyboard handling are the editor's own.
- **Reviewed means staged.** A review mark is the change being in git's index, so marks need
  no storage of their own, survive anything that keeps the index, and what is committed is
  what was reviewed. Staging done outside the editor, for example by an agent running
  `git add`, counts as reviewed.
- **A commit is what is reviewed.** There is no second way to choose what goes into a commit:
  the commit takes what is staged, and the marks of the review, by line, hunk, file,
  directory or all at once, are how it is chosen.
- **Git runs in the background and leaves no mess.** Commit, push, sync and switching run
  git itself, away from the interface. A sync always merges what it pulls, fast-forwarding
  when it can, whatever `pull.rebase` says; uncommitted changes, with their review
  marks, are set aside for the pull and brought back. A pull that runs into conflicts leaves
  them for the review, and a sync is refused when the commits to pull touch files with
  uncommitted changes.
- **Changes left on a branch live in git's stash.** Leaving the changes behind when switching
  stashes them under the name of the branch, and switching back brings them out again, review
  marks included, so they stay visible to plain git.
- **Known agents in tabs of their own.** The editor starts the agents it knows (Claude Code,
  Opencode) in Agent tabs and sends text only there, so it never has to guess whether a
  terminal holds an agent or a shell. An Agent tab is the agent's own terminal program, not
  a chat interface built by the editor. A quick chat and a draft always start a new session
  in a new tab; review comments go to a session that is already open.
- **Agents through the terminal.** Text for an agent is pasted into its terminal, and agents
  reach the editor with the `shepherd ctl` command over a socket, so an agent needs no
  protocol of its own. What the editor knows about an agent (how to start it, tell it about
  `shepherd ctl`, hand it an image and follow the session it moves to with a hook or plugin
  of its own) is kept in one place.
- **Language server first.** Code understanding comes from the Kotlin language server.
  Deeper analysis is considered only when a concrete review feature needs it.
