# Plan

A lightweight code editor for the AI era. Agents write most of the code; the human's job is
to steer them, review what they produce, fix things by hand and ship. The editor is built
around that loop.

## Principles

- **Three first-class citizens.** File editing, terminals (where agents live) and reviews are
  equals. None of them is a side panel of another.
- **Everything is a tab.** A file, a terminal and a review each open as a tab in the main
  space. Tabs can be placed side by side, stacked below each other, and popped out into
  their own windows.
- **Agent native.** Agents run in the editor's own terminals and can ask the editor for
  anything the user can see: selection, review comments, diagnostics, run output, git state.
  The editor works with any CLI agent rather than shipping its own.
- **One editing component.** The same text component is used for files, for diffs and for
  merge conflicts, so editing works the same everywhere.
- **Stay small.** A feature is added only when it is needed for the review loop or for
  comfortable Kotlin work. See "Deliberately left out".
- **Usable every day from the first milestone.** Each milestone ends in something worth using
  next to IntelliJ, until IntelliJ is no longer needed.

## Where we are

Milestone 1 is done: the review lives in a workspace of tabs, splits and windows next to
read-only file tabs, with a file tree, quick open and a layout that survives restarts.
README.md describes the features.

Milestone 2 is in progress. Done so far: file editing with undo, find and replace, saving on
its own, reloading of files changed on disk, and syntax highlighting. Still to do, in this
order: search across the project, terminals, several reviews at once.

The review itself is still the prototype: all working-tree changes in one stream, hunks
marked as reviewed (and unreviewed again when their content changes), a flat list of changed
files, keyboard navigation, automatic refresh.

## Milestones

### 1. Workspace: tabs, splits and windows (completed)

The foundation everything else lives in.

- The main space holds tabs; the existing review becomes one of them
- Tabs can be split side by side and below each other, moved between splits by dragging,
  and popped out into separate windows (and brought back)
- Sidebar on the left with the project file tree, switchable with the changes tree
- Files open as tabs from the tree, read-only for now
- Quick open by file name
- Layout, windows and open tabs are restored on restart

Done when: a review and a few files can be arranged next to each other across two windows.

### 2. Files and terminals as real tabs

All three kinds of tab become usable for real work.

- File editing: the basic text editing every editor has, with undo, find and replace, and
  syntax highlighting (Kotlin first, then the other common languages)
- Search across the project
- Terminals good enough to run CLI agents, shells and build tools
- Files changed on disk by an agent reload without fuss
- Several reviews can be open at once (for example in different splits)

Done when: an agent runs in a terminal tab, its changes appear in a review tab beside it,
and a file can be fixed by hand in a third.

### 3. A review that is pleasant to read

The stream becomes good enough to review real agent output in every day.

- Changed files shown as a tree, so it is clear which directories the changes are in, with
  review progress per file and per directory
- Syntax highlighting in diffs
- Changes within a line highlighted, not only whole lines
- Long lines, large diffs and large repos handled without stutter
- Expand context around a hunk; show a whole file when needed
- Mark a file or directory as reviewed in one go

Done when: reviewing an agent's change here is nicer than in IntelliJ's diff view.

### 4. Agents in the loop

The review turns from reading into steering.

- Select any rows in the changes and tell an agent what to change there
- Review comments: notes attached to lines, collected and sent to an agent as one batch of
  feedback
- A comment disappears once the lines it is attached to change; the changed hunk shows up
  as unreviewed again
- The editor exposes its context to agents: current selection, open files, review state and
  comments, diagnostics, terminal and run output, git state
- Agents can act on the editor: open a file, point at a line, ask for review
- See which terminals have an agent working, waiting for input or finished
- Changes made by an agent since the last look are easy to tell apart from already-seen ones

Done when: a full round of "agent writes, user comments on lines, agent fixes" happens
without copying anything by hand.

### 5. Editing inside the diff

- Edit the new side of any hunk directly in the stream
- Revert a hunk or selected lines
- Jump from a diff line to the same place in a file tab and back

Done when: small fixes during review never require leaving the review.

### 6. Git flow: commit, push, sync

- Choose what goes into a commit by file, hunk or line; reviewed state helps here
  ("commit what I have reviewed")
- Commit message written in place, with an agent able to draft it
- Commit and push as one streamlined action
- Fetch and pull as one "sync" action, with a clear picture of ahead/behind
- Branches: create, switch, see the current one at all times
- Stash or carry changes when switching

Done when: a normal day of commits, pushes and pulls needs no other git tool.

### 7. Merge conflicts in the same view

- Conflicts appear in the review stream as a special kind of hunk, during merge, rebase,
  pull and stash apply
- Resolve by taking either side, both, or editing the result in place
- Hand one conflict, or all of them, to an agent and review its resolution like any other
  change
- Continue or abort the operation from the same view

Done when: a conflicting pull is resolved without leaving the review.

### 8. Reviewing commits

- History list of commits for the branch
- Open one commit, or a range, in the same review stream with the same reviewed marks and
  comments
- See what a commit changed in a file's history; blame for a line

Done when: work an agent has already committed can be reviewed the same way as uncommitted work.

### 9. Comfortable Kotlin editing

The step that lets IntelliJ be closed for good.

- Language server support: completion, go to definition, find usages, hover, diagnostics,
  rename, formatting, quick fixes, imports
- Kotlin language server set up automatically for Gradle projects
- The same intelligence inside the review: go to definition and find usages from a diff line,
  diagnostics shown on changed lines
- Structural editing helpers from the syntax tree: selection by syntax node, outline,
  folding, sensible indentation
- Other languages come for free where a language server exists

Done when: a day of Kotlin work happens here without missing IntelliJ.

### 10. Run configurations

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
- A built-in agent chat panel (agents live in terminal tabs)
- Git worktrees (all agents work in one checkout)
- Kotlin analysis beyond the language server
- Pull request review for GitHub/GitLab
- Vim-style modal editing (the keymap should stay configurable so it can be added)
- Refactorings beyond what the language server offers
- Version control systems other than git
- Windows and Linux polish (macOS first; nothing should prevent the others)
- A settings UI (a config file is enough)

## Decisions

- **One working tree.** All agents work in the same checkout. No git worktree support.
- **Review comments are short-lived.** A comment disappears once the lines it is attached
  to change; the changed hunk shows up as unreviewed again. No resolved threads.
- **Files save themselves.** As in IntelliJ, there is no unsaved state to manage: an edit is
  written to disk a moment after typing stops, and when a tab or the editor is closed.
  Agents and build tools always see what the editor shows. When an open file changes on
  disk it is reloaded; only edits typed in the last moment win over the change.
- **Syntax trees from tree-sitter.** Highlighting is built on tree-sitter, so that the same
  trees serve selection by syntax node, outline and folding later.
- **Terminals on alacritty_terminal.** The terminal emulation comes from Alacritty's core
  with portable-pty for the processes; drawing is the editor's own.
- **Language server first.** Code understanding comes from the Kotlin language server.
  Deeper analysis is considered only when a concrete review feature needs it.
