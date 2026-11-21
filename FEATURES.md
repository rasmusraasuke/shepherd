# Features

Everything Shepherd does today. [README.md](README.md) has the short tour.

## Workspace

- **Tabs.** Reviews, files, terminals and agents open as tabs in the main space. A tab closes
  with its × or a middle click.
- **Splits.** Drag a tab to the edge of another tab to place them side by side or below
  each other. Drag it onto another tab bar to move it there. Drag the line between splits
  to resize them. Ctrl+Tab cycles through the tabs under the pointer.
- **Windows.** Drag a tab out of its window to pop it out into a new window where it is
  dropped, or drop it on another window: on the middle or the titles of a group of tabs to
  join it, towards a side of the group to make a split there. The place it is going to
  take is highlighted, and its title follows the pointer between the windows. A window has
  its own tabs and splits. A tab can also be moved from the menu behind a right-click on
  it; closing a popped out window brings its tabs back to the main window.
- **Sidebar.** A narrow bar of icons on the left edge chooses what the sidebar shows:
  *Files* (a page), the tree of project files (everything git doesn't ignore), or *Changes*
  (two sides of a diff), the tree of changed files with their review progress, or *History*
  (a git branch), the tree of commits. Clicking the icon of
  what is already shown closes the sidebar, leaving the icons; any icon opens it again.
- **Keyboard shortcuts.** The keyboard icon at the bottom of the bar, or Cmd+/, shows all the
  keys in one place. Esc or a click outside closes it.
- **Icons in the trees.** Folders show as folders, open or closed, and files by their type,
  in its own color: Kotlin, Java, Rust, JavaScript, TypeScript, Python, Go, JSON, YAML, TOML,
  Markdown, HTML, XML, CSS and shell scripts. Any other file, plain text included, shows a
  page with lines. The names of new files are green and of changed files blue, in *Files*
  as under *Changes*.
- **Files.** Click a file in the tree to open it as a tab. A file that is already open is
  brought to the front instead, in whichever window it is.
- **Quick open.** Cmd+P (Ctrl+P outside macOS) finds a file by name: type a few characters,
  pick with the arrow keys, open with Enter. The file opens in the window the popup is in.
- **Quiet by design.** No toolbar and no status bar: the bar of icons, the sidebar and the
  tabs, with the code taking the rest. One calm palette in dark and in light, following the system, with a
  single blue accent and soft diff colors. The same fonts everywhere: IBM Plex Sans for the
  interface, Lilex for code and terminals, JuliaMono for the symbols they leave out and Noto Emoji for
  emoji, so the arrows, bullets and stars agents draw show on any system.
  Under `⌘/` the system's own fonts can be chosen instead. The tab shown in each group is the
  one in bright text. With no tab open, the main space shows the few keys to start with.
- **The branch in the title bar.** The main window draws its own title bar beside the traffic
  lights with the branch dropdown, and can be dragged and zoomed from it. Popped-out windows
  end their title with the current branch behind a
  ⎇ branch symbol, while the macOS menu bar shows just "Shepherd".
- **Projects.** Left of the branch dropdown, the name of the project folder opens a menu
  with *New Project…*, which creates a folder with a new git repository, and *Open…*, which
  picks a folder that is a git repository; both open it. Below them, *Recent Projects*
  lists the current project first, its name in blue, and then every other project ever
  opened with Shepherd, most recent first, each with its folder below the name, relative to
  the home directory. The menu grows with the list up to
  a fixed height and then scrolls. Typing anywhere in the menu replaces the actions with a
  search of the names and folders, and Enter opens the first match. A project opens in a
  new Shepherd window. Hovering a project other than the current one shows a cross that makes
  Shepherd forget it.
- **Restored on restart.** Windows with their position and size, splits, open tabs, the
  sidebar, open or closed, and the expanded directories are saved per repository in `.git/shepherd-layout.json`.
- **Closing the editor.** Closing the window or quitting saves the open files and the layout,
  ends the programs of every terminal and agent tab, and any agent working in the background,
  then leaves nothing of the editor running behind.
- **Restarting into a new build.** The program watches the binary it was started from. When a
  new build replaces it, a bar above the editor offers to restart, and the layout comes back
  as it was.

## Editing

- **Text editing.** Files are edited in their tabs: typing, selecting with Shift and the
  mouse (double click for a word, triple click for a line), copy, cut and paste, undo and
  redo. Enter keeps the indentation; Tab and Shift+Tab indent and outdent the selected lines.
- **Saved on their own.** An edit is written to disk half a second after typing stops, and when
  the tab or the editor is closed; Cmd+S saves right away. There is nothing to save by hand.
- **Uncommitted lines marked.** A file tab marks what differs from the last commit with a
  colored bar after the line numbers: green when only lines were added, red when only lines
  were deleted, and gray when lines were replaced, which counts as one change however many
  lines it covers; a run of changed lines is one continuous bar. Hovering a bar shows a
  short preview of the change as the review does, with the removed and added lines numbered,
  colored and with the words that changed marked; clicking it opens the whole change in an
  overlay that scrolls from its first line. Both stay within the file view and start at the
  first line of the change. Escape or a click outside closes the overlay. The marks follow
  commits, also ones made outside the editor, and go away once the lines are committed.
- **Reloaded on their own.** A file changed on disk, for example by an agent, shows its new
  content without asking. Undo brings the previous content back.
- **Find and replace.** Cmd+F finds text in the file, starting from what is selected. Letter
  case matters only when the query has capital letters. Enter and Shift+Enter go to the
  next and the previous place, Esc closes the bar.
- **Search in files.** Shift pressed twice, or Cmd+Shift+F, opens a search of all project
  files over the window it was pressed in, starting from the text selected in the editor; the
  query stays for the next time. As in the file, letter case matters only when the
  query has capital letters. Up and Down go through the places found and show each in its
  file behind while typing stays in the search; Enter or a click goes to the place and closes
  the search, Esc or a click outside only closes it. Results follow
  changes on disk; binary and very large files are left out, and up to 2000 places are shown.
- **Syntax highlighting.** Kotlin, Java, Rust, JavaScript, TypeScript, Python, Go, JSON,
  shell scripts, TOML, YAML, HTML and CSS are highlighted, in colors close to IntelliJ's.
  Only what changed is parsed again while typing; files over 2 MB are left plain.

| Key                           | Action                                  |
|-------------------------------|-----------------------------------------|
| Alt+Left / Alt+Right          | Previous / next word (Ctrl outside macOS) |
| Cmd+Left / Cmd+Right          | Start / end of the line                 |
| Cmd+Up / Cmd+Down             | Start / end of the file                 |
| Alt+Backspace / Cmd+Backspace | Delete the word / the line before the cursor |
| Cmd+Z / Cmd+Shift+Z           | Undo / redo                             |
| Cmd+F                         | Find and replace                        |
| Cmd+Shift+B                   | Annotate with Git Blame                 |
| Shift Shift / Cmd+Shift+F     | Search in files                         |
| Ctrl Ctrl                     | Quick chat with an agent                |
| Cmd+S                         | Save now                                |
| Cmd+/                         | All keyboard shortcuts                  |

## Terminals

- **Terminal tabs.** Cmd+T, or *New terminal* from the + at the right of any tab bar, opens a
  terminal running your shell in the project directory. Terminals are tabs like any other:
  split, moved and popped out. The tab shows the title the shell or the program gives it.
- **Made for agents.** Full screen programs, 256 and true colors, bracketed paste and the
  usual xterm keys work. Text is in Lilex, or in the platform's own monospace font when the
  system fonts are chosen, with fallbacks, so the symbols agents draw look as they do in
  iTerm2; box drawing, block and braille characters are drawn to fill their cells, so frames,
  logos and spinners join up. Shift+Enter and Option+Enter send a new line to agents such as
  Claude Code; Option+Left/Right move by words and Cmd+Left/Right to the ends of the line, as
  in other terminal emulators.
- **Selecting and scrolling.** Drag to select (double click for a word, triple click for a
  line), Cmd+C copies, Cmd+V pastes, Cmd+K clears the scrollback. The wheel scrolls back
  through the output; typing brings back the end of it.
- **Mouse.** Programs that ask for the mouse get its clicks, drags and wheel. Hold Shift to
  select text in them anyway.
- **Clipboard.** Programs can copy to the clipboard. When one wants to read it, a bar asks
  first; *Allow* holds for that terminal until its program ends.
- **Restarting and closing.** When the program ends, Enter or *Restart* starts it again in
  the same tab. What the program wrote before it ended stays on the screen. Closing the tab
  ends the program. A terminal tab that is open when the editor quits comes back with a new
  shell on the next start.
- **What the program is doing.** A blue dot in front of a terminal's title shows that its
  program is working: it keeps writing to the terminal, as agents do while they work. Once it
  stops, the dot turns orange, until the terminal gets the keyboard again: an agent is done,
  or waits for an answer.

## Agents

- **Agent tabs.** Claude Code and Opencode are found on the `PATH`. *New agent* from the + at
  the right of any tab bar starts one in a tab of its own, next to *New terminal* and *New
  review*. The agent's own program runs directly, without a shell around it, with the keys,
  mouse, colors and clipboard handling of a terminal. The tab is titled with the agent's name,
  shows with a dot whether it works or waits, and, once the agent ends, says so and offers to
  start it again rather than dropping to a prompt. Agent tabs are split, moved and popped out
  like any other tab. The model menu lists the models the installed agent offers, by name:
  Claude Code's are asked of Claude Code itself, Opencode's of its server, and both are
  loaded in the background so the editor starts without waiting. Next to it an effort menu
  lists the reasoning levels the chosen model supports, passed as `--effort` to Claude Code
  and as the `#variant` of the model to Opencode; a model without levels hides it.
- **Quick chat.** Ctrl pressed twice opens a small window over the one it was pressed in. Pick
  the agent, the model and the effort to use, type the message and press Enter; Shift+Enter
  makes a new line and Esc closes. The model and effort are remembered for every agent on
  its own, so switching to another agent and back brings them back. Sending starts a new
  session of that agent in a new Agent tab with the message as its first words.
- **Only agents get the editor's text.** Review comments, *Ask agent* and *Draft* are offered
  only to the known agents, never plain terminals, so nothing the editor sends can land at a
  shell prompt. With Agent tabs open, the one used last comes first; with none open, a new one
  is started with the text as its first message. *Draft* runs in the background instead, with
  an agent, model and effort chosen for it alone, remembered apart from the coding agent's.
- **Back on restart.** An Agent tab that is open when the editor quits comes back on the next
  start and continues its conversation where the agent can. The agent tells the editor each
  session it moves to, so after `/clear`, `/new`, `/resume` or `/sessions` the tab comes back in
  the session it ended in, not the one it was opened with. Claude Code does so from a session
  start hook, Opencode from a small terminal plugin that keeps the user's own plugins loaded.
- **`shepherd ctl`.** The `shepherd ctl` command and the state of the agents work in Agent
  tabs as they do in terminals. `shepherd ctl context` lists the Agent tabs apart from the
  terminals, and `shepherd ctl terminal <id>` shows what any of them shows.

## Images

- **Pictures for agents.** Cmd+V in an Agent tab with a picture on the clipboard writes it to
  a PNG file in a temporary directory and pastes the path into the agent as text is pasted,
  without pressing Enter, so the agent reads it as any other file. When the clipboard holds
  both text and a picture, the text is pasted, as before. Image files dropped on an Agent tab
  are pasted the same way, one path per image. Each known agent says how it is handed a
  picture, so adding an agent stays a small change.
- **In a quick chat.** Pictures pasted into the quick chat's field, or dropped into it, are
  taken the same way, and their paths are sent with the message.
- **Temporary pictures.** A picture written for an agent stays while the agent runs and is
  removed when its tab closes, and pictures left behind by an editor that ended are removed
  once they are a day old, so screenshots do not pile up. An image file you drop on a tab is
  used where it is.
- **Plain terminals.** Terminals are left alone: a picture only becomes a path in Agent tabs.

## Review

- **One file at a time.** The review covers all working-tree changes (staged, unstaged and
  untracked) against HEAD. *Changes* in the sidebar shows the changed files as a tree, like
  the project tree but with only the files that changed. Next to each file and directory is
  the number of its changed lines that are reviewed; new files are green, changed ones blue
  and deleted ones red, faded once they are fully reviewed. Directories are open until closed. A
  renamed file is one file, its old name before the new one in the review, so git can follow
  the file and merge its changes; a rename git does not know about yet shows as a deleted and
  an added file until both are marked reviewed.
- **Review tabs.** Click a file under *Changes* to see its changes in the review tab used
  last, so review tabs don't pile up. A file already shown in a review tab is brought to
  the front there. Cmd+click a file, or choose *New review* from the + of a tab bar, to open
  another review tab, for example in another split or window. Several reviews can be open
  at once, each on its own file.
- **One piece of code.** A review reads like the file itself: the changed parts with three
  lines around them, and a thin wavy line where unchanged lines are left out. Each run of
  changed lines is a hunk of its own, however close it is to the next one. A click on the
  wavy line shows those lines. The current hunk has a bar at its left edge; a click on a hunk
  makes it the current one.
- **Easy to read.** Both sides of a diff are highlighted like the file itself, and where a
  line was changed rather than replaced, the words that changed stand out. Long lines scroll
  sideways under the line numbers. Files with many hunks scroll smoothly, as only the hunks
  in sight are drawn.
- **Reviewed means staged.** Marking a change as reviewed stages it in git, and taking the
  mark away unstages it; what is staged counts as reviewed, also when it was staged
  elsewhere. The editor keeps no review state of its own, and committing what is staged
  commits what was reviewed.
- **Reviewed marks.** A hunk is marked as reviewed with the checkbox next to its first line
  or with Space; its colors fade. Under the pointer a changed line shows a smaller checkbox
  of its own, so that only some lines of a hunk are staged: for example only the removed half
  of a changed line, which stages just the removal. A hunk with only some of its lines
  reviewed shows a dash, and the checkboxes of all its lines.
  *Mark file reviewed* at the top of a review marks the whole file, and the checkbox of a
  file or directory under *Changes*, or a right click on it, marks all of it, reviewed or not. Files without lines,
  such as binary ones, are marked as a whole.
- **Rolling back.** A right click on a file or directory under *Changes* offers *Roll back*,
  which takes the file, or everything under the directory, back to how it was committed:
  changed lines go and are unstaged, added files are deleted and deleted ones come back.
  A file open in a tab or in the review is reloaded from disk.
- **Changes after the review.** When a reviewed line is changed again, for example by an
  agent, the new version shows as not reviewed, while the staged one stays as it was. A
  file is only staged when it is as the review shows it; otherwise the review asks to try
  again once it has caught up.
- **Moving on.** Once the last hunk of a file is marked, the review moves on to the next file
  in the tree that has hunks left to review; so does n when nothing is left in the file.
- **New changes.** Changes that came in since you last looked are marked with a blue bar
  between the line numbers and the code, and their files and directories with a blue dot
  under *Changes*. A hunk counts as looked at once it was the current one or was marked;
  what there is when the editor starts counts as looked at.
- The working tree is watched in the background, so changes made by an agent show up on
  their own, also when an agent adds many new files.

## Fixing in the review

- **The review is the file.** A review is edited like a file tab: click anywhere and type.
  The removed lines sit between the lines of the file, and can't be edited. Unchanged lines
  can be edited too: a line typed into shows up as a change right away, as the review finds
  the changes itself while typing rather than waiting for git. The review and a tab of the
  same file share their text, undo and find (Cmd+F); edits are saved on their own as in a tab.
  A deleted file can be typed into, which brings it back.
- **The review keys** work while the text doesn't have the keyboard: Esc or a click on the
  gutter gives them back.
- **Revert.** *Revert* in the menu of a hunk (right click) takes it back to how it was
  committed. To revert only some lines, select them with the + in the gutter and press
  *Revert* under the selection; a removed line comes back, an added one goes. A revert is an
  edit of the text like any other, so Cmd+Z undoes it, also right after the menu. What was
  staged of the reverted lines is unstaged, so the index never holds a change the files no
  longer have; after an undo those lines show as not reviewed.
- **To the file and back.** o, or *Open in file* in the menu of a line, opens the file in a
  tab with the cursor on that line. Cmd+Shift+R in a file tab goes the other way: the review
  shows the changes of the file, at the hunk nearest to the cursor.

The keys work in the review tab used last, while no text field or terminal has the keyboard.

| Key   | Action                           |
|-------|----------------------------------|
| j / k | Next / previous hunk             |
| Space | Mark the hunk reviewed (or not), staging it |
| n     | Next unreviewed hunk, in the next file once the file is done |
| o     | Open the file at the hunk        |
| r     | Refresh now                      |
| Cmd+Z / Cmd+Shift+Z | Undo / redo an edit or a revert |

## Git flow

Above and below the tree of *Changes* is everything a normal day with git needs. Git runs in
the background; what it complains about is shown above the commit message.

- **Choosing what to commit.** A commit takes what is reviewed, as that is what is staged.
  Each file and directory under *Changes* has a checkbox that marks all of it as reviewed, or
  not, and shows a dash when only a part is; *All* and *None* above the commit message do
  the same for everything. Hunks and single lines are chosen in the review itself.
- **Commit.** Write the message below the tree and press *Commit* (Cmd+Enter in the
  message). *Commit and push* (Cmd+Shift+Enter) pushes right after.
- **A message from an agent.** Above the commit message, *Draft* starts the chosen agent and
  model in the background, with the request to write the message for what is staged. It has a
  choice of agent and model of its own, so a cheaper model can write the message while a
  smarter one does the work; the ⋯ menu next to *Draft*, on the right, has the harness and the model to pick. While the agent writes, the message box says so, until the
  message lands in it with `shepherd ctl commit-message`, to be read and changed before
  committing; *Cancel* stops waiting for it.
- **Sync and push.** *Sync ↓2* shows the commits there are to pull, as far as is known from
  the last fetch, and fetches and pulls, always merging and never rebasing. Uncommitted
  changes stay as they are, reviewed marks included. A pull that runs into conflicts leaves
  them for the review, and when the commits to pull change a file with uncommitted
  changes, sync asks to commit those first. *Push ↑1* shows the commits there are to push
  and pushes them; a branch that isn't on a remote yet is pushed to `origin`.
- **Progress of git.** Right below the branch, a spinner says what git is doing
  (*Committing…*, *Pushing…*, *Syncing…*, *Switching…*), a ✓ line says what was done for a
  few seconds afterwards, and an error from git stays there until the next action.
- **Branches.** The button with the name of the current branch lists the others; a click
  switches to one. Type a name into *New branch* and press Enter to start a branch from
  where you are, with the changes.
- **Carry or leave the changes.** Switching takes uncommitted changes along, as long as they
  fit on the other branch. With *Leave the changes on …* ticked they stay behind instead, in
  git's stash, and come back, reviewed marks included, when the branch is switched to
  again; until then the list says *(changes left there)*.

## Merge conflicts

- **In the review.** When a merge, rebase, pull or stash apply stops on conflicts, each
  conflicted file is red under *Changes*, marked with "conflict", and its review shows each
  conflicting region as a special hunk holding the conflict markers and both sides. The file
  is still the file: it can be edited anywhere, including over the markers.
- **Taking a side.** Each conflict hunk has *Ours*, *Theirs* and *Both*, which replace the
  region with one side or both and remove the markers. It is an edit of the text, so Cmd+Z
  takes it back. Binary and delete conflicts, which have no markers, offer *Take ours* and
  *Take theirs* at the top of the review.
- **Handing it to an agent.** *Ask agent* in a conflict hunk sends that conflict, and *Send
  all conflicts* at the top sends every conflict in the repository, each with its file as
  `@path` and its markers quoted. The agent's resolution comes back as an ordinary change to
  review and mark.
- **Finishing.** Once every conflict is resolved and marked reviewed, *Continue* finishes
  the merge, rebase, cherry-pick or revert, and *Abort* takes the operation back; both are
  beside the sync buttons. Changes set aside for the pull come back as before, and a stash
  that didn't apply shows its conflicts the same way.

## Reviewing commits

Work that is already committed is reviewed in the same view as work in the working tree.

- **History.** *History* in the bar of icons shows the commits of every local and remote
  branch and tag, newest first, as a git tree: each branch runs in a lane of its own
  color, branching off and merging back where it did, with merge commits drawn hollow.
  Commits that are not pushed to any remote are drawn in amber. Branch and tag names sit
  beside the commits they point at, a remote branch only when no local branch of the same
  name points there too, next to the subject, the
  short hash, the author and when each was made. The tree follows new commits and switches
  of branch as they happen. There is no limit: older commits load as the list is scrolled
  towards its end. The history of a single file stays a plain list.
- **The whole message.** Hovering a commit shows its whole message, with its branches, hash,
  author, and date and time, like 21.11.26 19:35 (5 minutes ago). The commit that is open
  shows the same below the list, above its files.
- **A commit.** A click opens a commit's changes in the review tab used last, one file at a
  time. Cmd+click opens another review tab, as a file does. The files of the commit are
  listed under the commits, with the same tree and colors as *Changes*.
- **Reading committed work.** A commit's review is the same review: the same hunks,
  highlighting, comments and keys, one file at a time, and *Open in file* to jump to the
  place in the working file. It is read-only, and its marks are shown as reviewed because
  what is committed is reviewed; there is nothing to stage. A comment on a committed line is
  sent to an agent about that commit, not about the uncommitted changes.
- **A file's history.** *File history* in the menu behind a right-click on a file in *Files*,
  or in the menu of a file tab, lists the commits that changed that file. A click shows what
  the commit changed in it, as above. *All commits* goes back to the history of the branch.
- **Blame.** *Annotate with Git Blame* behind a right-click in a file, or Cmd+Shift+B while
  the file has the keyboard, shows the author of each line's last commit before the line
  numbers. The background of the name is strongest for the file's newest commit and fades
  to the oldest one, ranked among the commits of the file;
  hovering it shows the commit's hash, message and when it was made, and a click opens that
  commit in *History*, as a click on it there does. Lines that are not committed yet have no
  blame. *Hide Git Blame* turns it off again.

## Agents in the loop

- **Comments.** Under the pointer a line of a review shows a + next to the reviewed marks.
  Click it to select the line, Shift+click another one to select the lines between, or drag
  from it over the lines. Write a comment in the box below the review and *Comment*
  (Cmd+Enter) adds it under the lines; × takes it away. Comments are collected across files
  until *Send N comments* at the top of a review sends them all to an agent as one message,
  each with its lines quoted as a diff and its file mentioned as `@path/of/file`, the way
  agents such as Claude Code take a file into the conversation. Sent comments stay, faded, until their lines change.
- **Short-lived.** A comment disappears once the lines it is on change, as the changed lines
  show up as not reviewed anyway. Comments are kept while the editor runs, not across restarts.
- **Asking an agent.** *Ask agent* in the same box sends the selected lines with what was
  written to an agent right away, without keeping a comment.
- **Which agent.** Text goes to Agent tabs only, pasted as if typed and followed by Enter, so
  any CLI agent works. With several Agent tabs open, the buttons become menus listing them,
  the one used last first; with none open, the chosen agent is started in a new tab with the
  text as its first message.
- **`shepherd ctl`.** Programs in the editor's terminals and Agent tabs can ask the editor
  what the user sees and have it show things, with the `shepherd ctl` command that is on the
  PATH there. Tell your agent about it, for example in `CLAUDE.md` or `AGENTS.md`:

  | Command | What it does |
  |---------|--------------|
  | `shepherd ctl context` | All of the below in one go |
  | `shepherd ctl selection` | The place and the text selected in the file edited last |
  | `shepherd ctl comments` | The review comments on the uncommitted changes |
  | `shepherd ctl review-state` | Which changed lines are reviewed (staged), by file |
  | `shepherd ctl terminals` | The terminals and what their programs are doing |
  | `shepherd ctl terminal <id>` | What a terminal or Agent tab shows |
  | `shepherd ctl open <path>[:<line>[-<line>]]` | Opens a file, selecting the lines |
  | `shepherd ctl review [<path>]` | Shows a file's changes for review, or the next file to review |
  | `shepherd ctl commit-message <message>` | Puts a commit message into the editor, for the user to commit with |

