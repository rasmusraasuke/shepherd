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

Milestones 1 to 10 are done: a workspace of tabs, splits and windows that survives
restarts, with file editing (undo, find and replace, syntax highlighting, saving on its own,
reloading of files changed on disk), search across the project, terminals good enough for
CLI agents, and a review of one file at a time, chosen from the tree of changed files, with
context shown on request, highlighted diffs with changes within lines, progress per
directory and moving on to the next file to review, review marks that are git's staging
area, line by line, and agents in the loop: comments on lines sent to agents in Agent tabs,
the state of the agents in the tab titles, new changes marked, and `shepherd ctl` for agents to
read what the editor shows and have it show files and reviews, and fixing in the review:
the review is the file and is edited anywhere, with the changes found again as they are
typed, reverting hunks or lines as edits that are undone with Cmd+Z, and jumping between a
change and its place in the file, and the git flow of a normal day: committing what is
reviewed with a message written in place or drafted by an agent, pushing, syncing with a
picture of what is ahead and behind, and creating and switching branches, taking the changes
along or leaving them, and agents as tabs of their own: Claude Code and Opencode found on
the PATH, started in Agent tabs without a shell, reached with a quick chat on Ctrl Ctrl with
the agent and model chosen and remembered, and sent the editor's text there and nowhere else,
and shipping as a real application: an app bundle for macOS and a tarball for Linux built by
scripts, releases made from a tag, and updates found and downloaded in the background, and
pictures for agents: a screenshot pasted or dropped into an Agent tab or a quick chat
becomes a file the agent reads, and the same fonts everywhere: IBM Plex Sans and Lilex, with
the system's fonts a switch away in the window under Cmd+/, kept in the editor's settings,
and merge conflicts in the same view: conflicts from a merge, rebase, pull or stash apply
shown as special hunks that take either side or both, are edited in place, can be handed to
an agent, and can be continued or aborted from the same view.
FEATURES.md describes the features.

Next is milestone 12: reviewing commits.

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

### 2. Files and terminals as real tabs (completed)

All three kinds of tab become usable for real work.

- File editing: the basic text editing every editor has, with undo, find and replace, and
  syntax highlighting (Kotlin first, then the other common languages)
- Search across the project
- Terminals good enough to run CLI agents, shells and build tools
- Files changed on disk by an agent reload without fuss
- The review covers one file at a time:
  - The *Changes* sidebar shows the changed files as a tree, in the same form as the project
    file tree but with only the files that changed, each with its review progress
  - A review tab shows the changes of one file. Clicking a file in the tree shows it in the
    review tab that is open (the one used last when there are several), so review tabs
    don't pile up; a new review tab is opened only on purpose
  - By default a review shows the changed parts and a few lines around them; clicking above
    or below a part shows more of the file, until the whole file is shown
- Several reviews can be open at once, each on its own file (for example in different splits)

Done when: an agent runs in a terminal tab, its changes appear in a review tab beside it,
and a file can be fixed by hand in a third.

### 3. A review that is pleasant to read (completed)

The review becomes good enough to review real agent output in every day.

- Review progress per directory in the changes tree, and moving on to the next file that
  isn't reviewed yet
- Syntax highlighting in diffs
- Changes within a line highlighted, not only whole lines
- Long lines, large diffs and large repos handled without stutter
- Mark a file or directory as reviewed in one go

Done when: reviewing an agent's change here is nicer than in IntelliJ's diff view.

### 4. Reviewed means staged (completed)

The review marks become git's staging area, and work line by line.

- Marking a change as reviewed stages it, and taking the mark away unstages it. What is
  staged shows as reviewed, also when it was staged elsewhere; the editor keeps no review
  state of its own
- A change made after it was reviewed shows up as unreviewed on top of the reviewed version,
  as git keeps the staged and the working tree versions apart
- Lines within a hunk are marked one by one: the hunk as a whole by default, or only the
  selected lines, removed and added ones independently. Marking only the removed lines of a
  replaced line stages just the removal
- The changes tree and the progress count lines that are staged, by file and directory

Done when: reviewing an agent's change leaves exactly the reviewed lines staged, ready to be
committed.

### 5. Agents in the loop (completed)

The review turns from reading into steering.

- Select any rows in the changes and tell an agent what to change there
- Review comments: notes attached to lines, collected and sent to an agent as one batch of
  feedback
- A comment disappears once the lines it is attached to change; the changed hunk shows up
  as unreviewed again
- The editor exposes its context to agents: current selection, open files, review state and
  comments, terminal and run output, git state (diagnostics follow with the language server)
- Agents can act on the editor: open a file, point at a line, ask for review
- See which terminals have an agent working, waiting for input or finished
- Changes made by an agent since the last look are easy to tell apart from already-seen ones

Done when: a full round of "agent writes, user comments on lines, agent fixes" happens
without copying anything by hand.

### 6. Editing inside the diff (completed)

- Edit the new side of any hunk directly in the review
- Revert a hunk or selected lines
- Jump from a diff line to the same place in a file tab and back

Done when: small fixes during review never require leaving the review.

### 7. Git flow: commit, push, sync (completed)

- Commit what is reviewed, as it is staged; or choose by file, hunk or line in the commit
  itself
- Commit message written in place, with an agent able to draft it
- Commit and push as one streamlined action
- Fetch and pull as one "sync" action, with a clear picture of ahead/behind
- Branches: create, switch, see the current one at all times
- Stash or carry changes when switching

Done when: a normal day of commits, pushes and pulls needs no other git tool.

### 8. Agents as tabs of their own (completed)

Until now an agent is whatever runs in a terminal, and the editor can't tell it from a shell:
text meant for an agent can land at a shell prompt and be run as a command. The editor gets
to know the agents, starts them itself and gives them a tab of their own.

- Known agents: Claude Code (`claude`) and Opencode (`opencode`), found on the `PATH`. For
  each the editor knows how to start it, how to start it with a first message and how to tell
  it about `shepherd ctl`. What is known about an agent is kept in one place, so that adding
  another one is a small change
- Agent tab, a fourth kind of tab next to file, terminal and review. It runs the agent's own
  program directly, with no shell around it, and shows its interface as a terminal does, with
  the keys, mouse, colors and clipboard handling terminals already have. It is titled with
  the agent's name and shows whether the agent works or waits. It has none of the shell
  features: when the agent ends, the tab says so and offers to start it again, rather than
  dropping to a prompt. Like any tab it is split, moved and popped out
- New agent from the + of a tab bar, next to *New terminal* and *New review*, with the
  agents that were found
- Quick chat: Ctrl pressed twice, as Shift twice opens the search, opens a small modal over
  the window it was pressed in, with a field for the message. It shows which agent and model
  are going to be used and lets another one be picked; the choice is remembered. Enter sends,
  Shift+Enter makes a new line, Esc closes. Sending starts a new session of the agent in a new
  Agent tab in that window, with the message as the first thing said to it
- Text from the editor goes to Agent tabs only: review comments, *Ask agent* and everything
  later milestones hand to an agent. Plain terminals are never offered and never receive
  text. With several Agent tabs open, the one used last comes first; with none open, a new
  one is started with the text as its first message, as a quick chat does
- *Draft* of a commit message has an agent and a model of its own, and runs in the background:
  the commit box shows that the agent is writing until the message lands
- `shepherd ctl` and the state of the agents (working, waiting) follow the Agent tabs:
  `shepherd ctl context` lists them apart from the terminals
- An Agent tab that is open when the editor quits comes back on the next start, continuing
  its conversation where the agent can do that

Done when: Ctrl Ctrl, a sentence and Enter get an agent working in a tab of its own, a draft
of a commit message needs no agent to be open first, and nothing the editor sends ever lands
in a shell.

### 9. A real application: builds, releases and updates (completed)

The editor stops being something started from the source and becomes software: installed,
versioned, published and able to replace itself.

- One script builds the app bundle from the source: `Info.plist`, the icon, an ad hoc
  signature, and a universal binary for Apple Silicon and Intel. It can install the app into
  `/Applications`, link a `shepherd` command for the terminal, and make the zip that releases
  are made of
- A Linux build from the same tag: a tarball with the binary, the desktop entry and the icon,
  which installs into `~/.local` or runs from where it was unpacked
- The version is to be had from the command line and from the keyboard shortcuts
- Started from Finder or Spotlight, the app opens the repository it opened last, and finds
  agents where Homebrew and `~/.local/bin` put them, which an app started without a shell
  otherwise does not
- A tag makes a release: GitHub Actions builds the macOS app and the Linux tarball, checks
  the tag against `Cargo.toml` and publishes them with notes
- Updates: on start the app asks for the latest release; a newer one is downloaded in the
  background and, when it is ready, the editor offers to restart into it on macOS and to show
  it on Linux

Done when: a new version is tagged, and the app installed in `/Applications` finds it,
downloads it and restarts into it.

### 10. Images for agents (completed)

An agent's work often starts from a picture: a screenshot of a bug, a design or an error in a
browser. Until now only text can be pasted into an Agent tab, so a screenshot has to be saved to
a file by hand and its path typed.

- Cmd+V in an Agent tab with an image on the clipboard writes it to a file in a temporary
  directory and pastes its path into the agent as text is pasted, with no Enter pressed, so the
  agent reads the image as any other file. When the clipboard holds both text and an image, the
  text wins, as before
- Dropping image files on an Agent tab does the same, one path per image
- A quick chat takes images the same way, pasted or dropped into its field, and sends their paths
  with the message
- Each known agent says how it is handed an image, so an agent whose interface has a way of its
  own can use it, and adding an agent stays a small change
- Temporary images are kept while the agent that was given them runs and removed when its tab is
  closed or when they are old, so screenshots do not pile up
- Plain terminals are left alone: an image is only turned into a path in Agent tabs

Done when: a screenshot pasted into an Agent tab, or attached to a quick chat, reaches the agent
as an image without saving it by hand.

### 11. Merge conflicts in the same view (completed)

- Conflicts appear in the file's review as a special kind of hunk, during merge, rebase,
  pull and stash apply
- Resolve by taking either side, both, or editing the result in place
- Hand one conflict, or all of them, to an agent and review its resolution like any other
  change
- Continue or abort the operation from the same view

Done when: a conflicting pull is resolved without leaving the review.

### 12. Reviewing commits

- History list of commits for the branch
- Open one commit, or a range, in the same review, one file at a time, with the same
  reviewed marks and comments
- See what a commit changed in a file's history; blame for a line

Done when: work an agent has already committed can be reviewed the same way as uncommitted work.

### 13. Comfortable Kotlin editing

The step that lets IntelliJ be closed for good.

- Language server support: completion, go to definition, find usages, hover, diagnostics,
  rename, formatting, quick fixes, imports
- Kotlin language server set up automatically for Gradle projects
- The same intelligence inside the review: go to definition and find usages from a diff line,
  diagnostics shown on changed lines
- Diagnostics given to agents through `shepherd ctl`
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
- Windows and Linux polish (macOS first; nothing should prevent the others)
- A settings UI (a config file is enough)

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
  reach the editor with the `shepherd ctl` command over a socket, so any CLI agent works without
  a plugin or a protocol of its own.
- **Language server first.** Code understanding comes from the Kotlin language server.
  Deeper analysis is considered only when a concrete review feature needs it.
