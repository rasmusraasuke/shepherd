use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use eframe::egui;
use egui_tiles::{Container, Tile, TileId, Tree};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Tab {
    Review,
    /// A project file, by its path relative to the repository root.
    File(String),
}

impl Tab {
    pub fn title(&self) -> &str {
        match self {
            Tab::Review => "Review",
            Tab::File(path) => path.rsplit('/').next().unwrap_or(path),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub enum Sidebar {
    #[default]
    Files,
    Changes,
}

/// One OS window with its own arrangement of tabs and splits.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub id: u64,
    pub tree: Tree<Tab>,
    /// Position of the window on screen and the size of its content, to restore it where it was.
    pub position: Option<egui::Pos2>,
    pub size: Option<egui::Vec2>,
    /// The tab used last; new tabs open next to it.
    #[serde(skip)]
    pub focused: Option<TileId>,
}

impl Window {
    fn new(id: u64, tabs: Vec<Tab>) -> Self {
        let tree = Tree::new_tabs(egui::Id::unique(("window", id)), tabs);
        Self { id, tree, position: None, size: None, focused: None }
    }

    /// The tabs of the window in the order they were opened.
    pub fn tabs(&self) -> Vec<&Tab> {
        let mut tabs: Vec<(&TileId, &Tab)> = self
            .tree
            .tiles
            .iter()
            .filter_map(|(id, tile)| match tile {
                Tile::Pane(tab) => Some((id, tab)),
                Tile::Container(_) => None,
            })
            .collect();
        tabs.sort_by_key(|(id, _)| id.0);
        tabs.into_iter().map(|(_, tab)| tab).collect()
    }

    /// Brings an already open tab to the front. Returns false when the tab isn't in this window.
    fn activate(&mut self, tab: &Tab) -> bool {
        self.tree.make_active(|_, tile| matches!(tile, Tile::Pane(t) if t == tab))
    }

    /// The group of tabs a new tab joins: the one used last, otherwise the first one.
    fn target_tabs(&self) -> Option<TileId> {
        let is_tabs = |id: &TileId| matches!(self.tree.tiles.get_container(*id), Some(Container::Tabs(_)));
        let focused = self.focused.and_then(|tile| self.tree.tiles.parent_of(tile)).filter(is_tabs);
        focused.or_else(|| self.tree.tiles.tile_ids().filter(is_tabs).min_by_key(|id| id.0))
    }

    fn add(&mut self, tab: Tab) {
        let tile = self.tree.tiles.insert_pane(tab);
        match self.target_tabs().and_then(|id| self.tree.tiles.get_mut(id)) {
            Some(Tile::Container(Container::Tabs(tabs))) => {
                tabs.add_child(tile);
                tabs.set_active(tile);
            }
            _ => {
                // an empty window, or one without a group of tabs: start a new group at the root
                let children = self.tree.root.into_iter().chain([tile]).collect();
                let root = self.tree.tiles.insert_tab_tile(children);
                if let Some(Tile::Container(Container::Tabs(tabs))) = self.tree.tiles.get_mut(root) {
                    tabs.set_active(tile);
                }
                self.tree.root = Some(root);
            }
        }
        self.focused = Some(tile);
    }

    fn remove(&mut self, tile: TileId) -> Option<Tab> {
        self.tree.remove_recursively(tile).into_iter().find_map(|tile| match tile {
            Tile::Pane(tab) => Some(tab),
            Tile::Container(_) => None,
        })
    }
}

/// Everything about the arrangement of the editor that survives a restart.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    /// The first window is the main one: it has the sidebar and is never removed.
    pub windows: Vec<Window>,
    pub sidebar: Sidebar,
    pub expanded_dirs: BTreeSet<String>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self { windows: vec![Window::new(0, vec![Tab::Review])], sidebar: Sidebar::default(), expanded_dirs: BTreeSet::new() }
    }
}

impl Workspace {
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path).ok().and_then(|json| Self::from_json(&json)).unwrap_or_default()
    }

    fn from_json(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok().filter(|workspace: &Self| !workspace.windows.is_empty())
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Shows the tab, opening it in the given window unless it is already open somewhere.
    /// Returns the index of the window the tab is in.
    pub fn open(&mut self, tab: Tab, window: usize) -> usize {
        if let Some(existing) = self.windows.iter_mut().position(|w| w.activate(&tab)) {
            return existing;
        }
        self.windows[window].add(tab);
        window
    }

    /// Moves a tab to another window, or pops it out into a new one when `to` is `None`.
    pub fn move_tab(&mut self, from: usize, tile: TileId, to: Option<usize>) {
        let Some(tab) = self.windows[from].remove(tile) else { return };
        match to {
            Some(to) => self.windows[to].add(tab),
            None => {
                let id = self.windows.iter().map(|w| w.id).max().unwrap_or(0) + 1;
                self.windows.push(Window::new(id, vec![tab]));
            }
        }
    }

    /// Closes a popped out window, bringing its tabs back to the main window.
    pub fn close_window(&mut self, index: usize) {
        if index == 0 {
            return;
        }
        let window = self.windows.remove(index);
        for tab in window.tabs() {
            self.windows[0].add(tab.clone());
        }
    }

    /// Removes popped out windows that have no tabs left.
    pub fn prune(&mut self) {
        let mut index = 0;
        self.windows.retain(|window| {
            index += 1;
            index == 1 || !window.tabs().is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> Tab {
        Tab::File(name.to_string())
    }

    fn tabs(window: &Window) -> Vec<&str> {
        let mut titles: Vec<&str> = window.tabs().into_iter().map(Tab::title).collect();
        titles.sort();
        titles
    }

    fn tile_of(window: &Window, tab: &Tab) -> TileId {
        window.tree.tiles.find_pane(tab).unwrap()
    }

    fn is_active(window: &Window, tab: &Tab) -> bool {
        let tile = tile_of(window, tab);
        window.tree.active_tiles().contains(&tile)
    }

    #[test]
    fn starts_with_the_review() {
        let workspace = Workspace::default();
        assert_eq!(tabs(&workspace.windows[0]), ["Review"]);
    }

    #[test]
    fn opening_adds_a_tab_once_and_brings_it_to_front() {
        let mut workspace = Workspace::default();
        workspace.open(file("src/a.rs"), 0);
        workspace.open(file("src/b.rs"), 0);
        assert_eq!(tabs(&workspace.windows[0]), ["Review", "a.rs", "b.rs"]);
        assert!(is_active(&workspace.windows[0], &file("src/b.rs")));

        assert_eq!(workspace.open(file("src/a.rs"), 0), 0);
        assert_eq!(workspace.windows[0].tabs().len(), 3);
        assert!(is_active(&workspace.windows[0], &file("src/a.rs")));
    }

    #[test]
    fn opens_into_an_empty_window() {
        let mut workspace = Workspace::default();
        let review = tile_of(&workspace.windows[0], &Tab::Review);
        workspace.windows[0].remove(review);
        workspace.windows[0].tree.simplify(&Default::default());
        workspace.open(file("a.rs"), 0);
        assert_eq!(tabs(&workspace.windows[0]), ["a.rs"]);
        assert!(is_active(&workspace.windows[0], &file("a.rs")));
    }

    #[test]
    fn pops_a_tab_out_and_brings_it_back() {
        let mut workspace = Workspace::default();
        workspace.open(file("a.rs"), 0);
        let tile = tile_of(&workspace.windows[0], &file("a.rs"));
        workspace.move_tab(0, tile, None);
        assert_eq!(tabs(&workspace.windows[0]), ["Review"]);
        assert_eq!(tabs(&workspace.windows[1]), ["a.rs"]);

        // a tab that is open in another window is shown there instead of being opened twice
        assert_eq!(workspace.open(file("a.rs"), 0), 1);
        assert_eq!(workspace.open(file("b.rs"), 1), 1);

        let tile = tile_of(&workspace.windows[1], &file("a.rs"));
        workspace.move_tab(1, tile, Some(0));
        assert_eq!(tabs(&workspace.windows[0]), ["Review", "a.rs"]);
        assert_eq!(tabs(&workspace.windows[1]), ["b.rs"]);
    }

    #[test]
    fn closing_a_window_brings_its_tabs_back() {
        let mut workspace = Workspace::default();
        let review = tile_of(&workspace.windows[0], &Tab::Review);
        workspace.move_tab(0, review, None);
        workspace.open(file("a.rs"), 1);
        workspace.close_window(1);
        assert_eq!(workspace.windows.len(), 1);
        assert_eq!(tabs(&workspace.windows[0]), ["Review", "a.rs"]);
    }

    #[test]
    fn prunes_empty_windows_except_the_main_one() {
        let mut workspace = Workspace::default();
        let review = tile_of(&workspace.windows[0], &Tab::Review);
        workspace.move_tab(0, review, None);
        let review = tile_of(&workspace.windows[1], &Tab::Review);
        workspace.move_tab(1, review, None);
        assert_eq!(workspace.windows.len(), 3);
        workspace.prune();
        assert_eq!(workspace.windows.len(), 2);
        assert_eq!(tabs(&workspace.windows[0]), [] as [&str; 0]);
        assert_eq!(tabs(&workspace.windows[1]), ["Review"]);
    }

    #[test]
    fn new_windows_get_unique_ids() {
        let mut workspace = Workspace::default();
        for name in ["a.rs", "b.rs"] {
            workspace.open(file(name), 0);
            let tile = tile_of(&workspace.windows[0], &file(name));
            workspace.move_tab(0, tile, None);
        }
        workspace.close_window(1);
        workspace.open(file("c.rs"), 0);
        let tile = tile_of(&workspace.windows[0], &file("c.rs"));
        workspace.move_tab(0, tile, None);
        let ids: BTreeSet<u64> = workspace.windows.iter().map(|w| w.id).collect();
        assert_eq!(ids.len(), workspace.windows.len());
    }

    #[test]
    fn survives_a_restart() {
        let mut workspace = Workspace::default();
        workspace.open(file("a.rs"), 0);
        workspace.open(file("b.rs"), 0);
        let tile = tile_of(&workspace.windows[0], &file("b.rs"));
        workspace.move_tab(0, tile, None);
        workspace.windows[1].position = Some(egui::pos2(10.0, 20.0));
        workspace.windows[1].size = Some(egui::vec2(300.0, 400.0));
        workspace.sidebar = Sidebar::Changes;
        workspace.expanded_dirs.insert("src".to_string());

        let restored = Workspace::from_json(&workspace.to_json()).unwrap();
        assert!(restored.windows.iter().zip(&workspace.windows).all(|(a, b)| a.tree == b.tree));
        assert_eq!(tabs(&restored.windows[0]), ["Review", "a.rs"]);
        assert_eq!(tabs(&restored.windows[1]), ["b.rs"]);
        assert_eq!(restored.windows[1].position, Some(egui::pos2(10.0, 20.0)));
        assert_eq!(restored.sidebar, Sidebar::Changes);
    }

    #[test]
    fn falls_back_to_the_default_layout_when_the_saved_one_is_unusable() {
        assert!(Workspace::from_json("not json").is_none());
        assert!(Workspace::from_json(r#"{"windows":[]}"#).is_none());
        let workspace = Workspace::load(Path::new("/nonexistent/layout.json"));
        assert_eq!(tabs(&workspace.windows[0]), ["Review"]);
    }
}
