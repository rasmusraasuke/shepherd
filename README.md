# editor

A lightweight code editor for the AI era: agents write most of the code, the human steers,
reviews, fixes and ships. See [PLAN.md](PLAN.md) for where this is going.

## Running

```
cargo run --release -- path/to/git/repository
```

Without a path the current directory is opened. The path must be inside a git repository.

## Features

### Workspace

- **Tabs.** Reviews, files and terminals open as tabs in the main space. A tab closes with its ×
  or a middle click.
- **Splits.** Drag a tab to the edge of another tab to place them side by side or below
  each other. Drag it onto another tab bar to move it there. Drag the line between splits
  to resize them. Ctrl+Tab cycles through the tabs under the pointer.
- **Windows.** Drag a tab out of its window to pop it out into a new window where it is
  dropped, or drop it on another window: on the middle or the titles of a group of tabs to
  join it, towards a side of the group to make a split there. The place it is going to
  take is highlighted, and its title follows the pointer between the windows. A window has
  its own tabs and splits. A tab can also be moved from the menu behind a right-click on
  it; closing a popped out window brings its tabs back to the main window.
- **Sidebar.** Switches between *Files*, the tree of project files (everything git doesn't
  ignore), *Changes*, the tree of changed files with their review progress, and *Search*.
- **Files.** Click a file in the tree to open it as a tab. A file that is already open is
  brought to the front instead, in whichever window it is.
- **Quick open.** Cmd+P (Ctrl+P outside macOS) finds a file by name: type a few characters,
  pick with the arrow keys, open with Enter. The file opens in the window the popup is in.
- **Restored on restart.** Windows with their position and size, splits, open tabs, the
  sidebar and the expanded directories are saved per repository in `.git/editor-layout.json`.

### Editing

- **Text editing.** Files are edited in their tabs: typing, selecting with Shift and the
  mouse (double click for a word, triple click for a line), copy, cut and paste, undo and
  redo. Enter keeps the indentation; Tab and Shift+Tab indent and outdent the selected lines.
- **Saved on their own.** An edit is written to disk a second after typing stops, and when
  the tab or the editor is closed; Cmd+S saves right away. There is nothing to save by hand.
- **Reloaded on their own.** A file changed on disk, for example by an agent, shows its new
  content without asking. Undo brings the previous content back.
- **Find and replace.** Cmd+F finds text in the file, starting from what is selected. Letter
  case matters only when the query has capital letters. Enter and Shift+Enter go to the
  next and the previous place, Esc closes the bar.
- **Search in files.** Cmd+Shift+F searches all project files from the sidebar, starting
  from the text selected in the editor. As in the file, letter case matters only when the
  query has capital letters. Up and Down go through the places found and show each in its
  file while typing stays in the search; Enter or a click goes to the place. Results follow
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
| Cmd+Shift+F                   | Search in files                         |
| Cmd+S                         | Save now                                |

### Terminals

- **Terminal tabs.** Cmd+T, or *New terminal* from the + at the right of any tab bar, opens a
  terminal running your shell in the project directory. Terminals are tabs like any other:
  split, moved and popped out. The tab shows the title the shell or the program gives it.
- **Made for agents.** Full screen programs, 256 and true colors, bracketed paste and the
  usual xterm keys work. Text is in Menlo with the fallbacks macOS terminals use, so the
  symbols agents draw look as they do in iTerm2; box drawing, block and braille characters
  are drawn to fill their cells, so frames, logos and spinners join up. Shift+Enter and Option+Enter send a new line to agents such as
  Claude Code; Option+Left/Right move by words and Cmd+Left/Right to the ends of the line,
  as in other macOS terminals.
- **Selecting and scrolling.** Drag to select (double click for a word, triple click for a
  line), Cmd+C copies, Cmd+V pastes, Cmd+K clears the scrollback. The wheel scrolls back
  through the output; typing brings back the end of it.
- **Mouse.** Programs that ask for the mouse get its clicks, drags and wheel. Hold Shift to
  select text in them anyway.
- **Clipboard.** Programs can copy to the clipboard. When one wants to read it, a bar asks
  first; *Allow* holds for that terminal until its program ends.
- **Restarting and closing.** When the program ends, Enter or *Restart* starts it again in
  the same tab. Closing the tab ends the program. A terminal tab that is open when the
  editor quits comes back with a new shell on the next start.

### Review

- **One file at a time.** The review covers all working-tree changes (staged, unstaged and
  untracked) against HEAD. *Changes* in the sidebar shows the changed files as a tree, like
  the project tree but with only the files that changed. Next to each file is the number of
  its hunks that are reviewed; added files are green, deleted ones red, and fully reviewed
  ones dimmed. Directories are open until closed.
- **Review tabs.** Click a file under *Changes* to see its changes in the review tab used
  last, so review tabs don't pile up. A file already shown in a review tab is brought to
  the front there. Cmd+click a file, or choose *New review* from the + of a tab bar, to open
  another review tab, for example in another split or window. Several reviews can be open
  at once, each on its own file.
- **Context on request.** A review shows the changed parts with three lines around them.
  Between the parts, *Expand up* and *Expand down* show 20 more lines above or below a part,
  and *Expand all* the rest of the lines in between, until the whole file is shown.
- **Reviewed marks.** Hunks are marked as reviewed with the checkbox or Space and collapse
  to their header. A hunk becomes unreviewed again when its content changes. Reviewed marks
  are kept in `.git/editor-review`.
- The working tree is watched in the background, so changes made by an agent show up on
  their own.

The keys work in the review tab used last, while no text field or terminal has the keyboard.

| Key   | Action                           |
|-------|----------------------------------|
| j / k | Next / previous hunk             |
| Space | Mark the hunk reviewed (or not)  |
| n     | Next unreviewed hunk in the file |
| r     | Refresh now                      |

## Development

```
cargo test
```
