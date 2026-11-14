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

- **Tabs.** The review and files open as tabs in the main space. A tab closes with its ×
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
  ignore), and *Changes*, the list of changed files with their review progress.
- **Files.** Click a file in the tree to open it as a read-only tab. A file that is already
  open is brought to the front instead, in whichever window it is. Files changed on disk
  are reloaded.
- **Quick open.** Cmd+P (Ctrl+P outside macOS) finds a file by name: type a few characters,
  pick with the arrow keys, open with Enter. The file opens in the window the popup is in.
- **Restored on restart.** Windows with their position and size, splits, open tabs, the
  sidebar and the expanded directories are saved per repository in `.git/editor-layout.json`.

### Review

- All working-tree changes (staged, unstaged and untracked) against HEAD in one stream.
- Hunks are marked as reviewed with the checkbox or Space and collapse to their header.
  A hunk becomes unreviewed again when its content changes. Reviewed marks are kept in
  `.git/editor-review`.
- Click a file under *Changes* to jump to its first hunk; this also reopens the review tab
  if it was closed.
- The working tree is watched in the background, so changes made by an agent show up on
  their own.

| Key   | Action                          |
|-------|---------------------------------|
| j / k | Next / previous hunk            |
| Space | Mark the hunk reviewed (or not) |
| n     | Next unreviewed hunk            |
| r     | Refresh now                     |

## Development

```
cargo test
```
