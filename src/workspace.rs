use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use eframe::egui;
use egui_tiles::{Container, Tile, TileId, Tree};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Tab {
    /// The review of the changes of one file, by its path; none until a file is chosen.
    Review(Option<String>),
    /// A project file, by its path relative to the repository root.
    File(String),
    /// A terminal, by an id of its own.
    Terminal(u64),
}

impl Tab {
    pub fn title(&self) -> String {
        let name = |path: &str| path.rsplit('/').next().unwrap_or(path).to_string();
        match self {
            Tab::Review(None) => "Review".to_string(),
            Tab::Review(Some(path)) => format!("Review: {}", name(path)),
            Tab::File(path) => name(path),
            Tab::Terminal(_) => "Terminal".to_string(),
        }
    }

    /// The project file the tab is about.
    pub fn path(&self) -> Option<&str> {
        match self {
            Tab::Review(path) => path.as_deref(),
            Tab::File(path) => Some(path),
            Tab::Terminal(_) => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub enum Sidebar {
    #[default]
    Files,
    Changes,
    Search,
}

/// Where the pointer holds a window that was created by dragging a tab out, from its top left corner.
const NEW_WINDOW_GRIP: egui::Vec2 = egui::vec2(60.0, 40.0);

/// Height of the bar with the titles of a group of tabs, as egui_tiles draws it.
const TAB_BAR_HEIGHT: f32 = 24.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// Where a tab dropped on a window lands.
#[derive(PartialEq, Debug)]
pub struct DropTarget {
    /// The group of tabs under the pointer.
    tabs: TileId,
    /// The side of the group the tab is placed on as a new split; none when it joins the group.
    side: Option<Side>,
    /// The part of the window the tab is going to take up.
    pub rect: egui::Rect,
}

/// One OS window with its own arrangement of tabs and splits.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub id: u64,
    pub tree: Tree<Tab>,
    /// Position of the window on screen and the size of its content, to restore it where it was.
    pub position: Option<egui::Pos2>,
    pub size: Option<egui::Vec2>,
    /// Where the content of the window is on the screen at the moment.
    #[serde(skip)]
    pub screen_rect: Option<egui::Rect>,
    /// The tab used last; new tabs open next to it.
    #[serde(skip)]
    pub focused: Option<TileId>,
}

impl Window {
    fn new(id: u64, tabs: Vec<Tab>) -> Self {
        let tree = Tree::new_tabs(egui::Id::unique(("window", id)), tabs);
        Self { id, tree, position: None, size: None, screen_rect: None, focused: None }
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
        let is_tabs = |id: &TileId| self.is_tabs(id);
        let focused = self.focused.and_then(|tile| self.tree.tiles.parent_of(tile)).filter(is_tabs);
        focused.or_else(|| self.tree.tiles.tile_ids().filter(is_tabs).min_by_key(|id| id.0))
    }

    fn is_tabs(&self, tile: &TileId) -> bool {
        matches!(self.tree.tiles.get_container(*tile), Some(Container::Tabs(_)))
    }

    /// Where a tab dropped at the given point of the screen lands: in the group of tabs that
    /// is there when dropped on its middle or its titles, next to the group when dropped
    /// towards one of its sides.
    pub fn drop_target(&self, point: egui::Pos2) -> Option<DropTarget> {
        let point = point - self.screen_rect?.min.to_vec2();
        let tiles = &self.tree.tiles;
        let (tabs, rect) = tiles.tile_ids().filter(|id| self.is_tabs(id)).find_map(|id| Some((id, tiles.rect(id).filter(|rect| rect.contains(point))?)))?;
        let (left, right) = rect.split_left_right_at_fraction(0.5);
        let (top, bottom) = rect.split_top_bottom_at_fraction(0.5);
        let (bar, body) = rect.split_top_bottom_at_y(rect.top() + TAB_BAR_HEIGHT);
        let sides = [(Side::Left, left), (Side::Right, right), (Side::Top, top), (Side::Bottom, bottom)];
        let distance = |rect: &egui::Rect| rect.center().distance_sq(point);
        let nearest = sides.into_iter().min_by(|(_, a), (_, b)| distance(a).total_cmp(&distance(b))).filter(|(_, rect)| distance(rect) < distance(&body));
        Some(match nearest.filter(|_| !bar.contains(point)) {
            Some((side, rect)) => DropTarget { tabs, side: Some(side), rect },
            None => DropTarget { tabs, side: None, rect: body },
        })
    }

    /// Adds the tab where the target says.
    fn add_at(&mut self, target: DropTarget, tab: Tab) {
        let Some(side) = target.side else { return self.add_to(Some(target.tabs), tab) };
        let tiles = &mut self.tree.tiles;
        let parent = tiles.parent_of(target.tabs);
        let tile = tiles.insert_pane(tab);
        let new_tabs = tiles.insert_tab_tile(vec![tile]);
        let children = if matches!(side, Side::Left | Side::Top) { vec![new_tabs, target.tabs] } else { vec![target.tabs, new_tabs] };
        let split = if matches!(side, Side::Left | Side::Right) { tiles.insert_horizontal_tile(children) } else { tiles.insert_vertical_tile(children) };
        // the split takes the place of the group
        let place = match parent.map(|parent| tiles.get_mut(parent)) {
            None => self.tree.root.as_mut(),
            Some(Some(Tile::Container(Container::Linear(linear)))) => linear.children.iter_mut().find(|child| **child == target.tabs),
            Some(_) => None,
        };
        if let Some(place) = place {
            *place = split;
        }
        self.focused = Some(tile);
    }

    /// Shows another tab in place of the one in the given tile.
    fn replace(&mut self, tile: TileId, tab: Tab) {
        if let Some(Tile::Pane(pane)) = self.tree.tiles.get_mut(tile) {
            *pane = tab;
        }
        self.tree.make_active(|id, _| id == tile);
        self.focused = Some(tile);
    }

    fn add(&mut self, tab: Tab) {
        self.add_to(self.target_tabs(), tab);
    }

    /// Adds the tab to the given group of tabs.
    fn add_to(&mut self, tabs: Option<TileId>, tab: Tab) {
        let tile = self.tree.tiles.insert_pane(tab);
        match tabs.and_then(|id| self.tree.tiles.get_mut(id)) {
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
    /// The directories closed in the tree of changed files, where they are open by default.
    pub collapsed_changes: BTreeSet<String>,
    /// The review tab used last, where the changes of a file chosen in the tree are shown.
    #[serde(skip)]
    pub last_review: Option<Tab>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            windows: vec![Window::new(0, vec![Tab::Review(None)])],
            sidebar: Sidebar::default(),
            expanded_dirs: BTreeSet::new(),
            collapsed_changes: BTreeSet::new(),
            last_review: None,
        }
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

    /// Adds a tab to the given group of tabs of a window, or next to the tab used last.
    pub fn add_tab(&mut self, window: usize, tabs: Option<TileId>, tab: Tab) {
        let window = &mut self.windows[window];
        window.add_to(tabs.or_else(|| window.target_tabs()), tab);
    }

    /// Opens a new terminal in the given group of tabs of a window, or next to the tab used
    /// last. Returns its id.
    pub fn open_terminal(&mut self, window: usize, tabs: Option<TileId>) -> u64 {
        let id = self.windows.iter().flat_map(|window| window.tabs()).filter_map(|tab| if let Tab::Terminal(id) = tab { Some(*id) } else { None }).max().unwrap_or(0) + 1;
        self.add_tab(window, tabs, Tab::Terminal(id));
        id
    }

    /// The window and the tile a tab is in.
    fn find(&self, tab: &Tab) -> Option<(usize, TileId)> {
        self.windows.iter().enumerate().find_map(|(index, window)| Some((index, window.tree.tiles.find_pane(tab)?)))
    }

    /// The review tab used last, otherwise the review tab opened first.
    fn review_tab(&self) -> Option<(usize, TileId)> {
        let first = || {
            self.windows.iter().enumerate().find_map(|(index, window)| {
                let tiles = window.tree.tiles.iter().filter(|(_, tile)| matches!(tile, Tile::Pane(Tab::Review(_))));
                Some((index, *tiles.map(|(id, _)| id).min_by_key(|id| id.0)?))
            })
        };
        self.last_review.as_ref().and_then(|tab| self.find(tab)).or_else(first)
    }

    /// Shows the changes of a file: in the review tab that shows them already, otherwise in
    /// the review tab used last, or in a new one when `new_tab` is set or there is none.
    /// Returns the index of the window the review is in.
    pub fn show_review(&mut self, path: &str, new_tab: bool) -> usize {
        let tab = Tab::Review(Some(path.to_string()));
        let window = match self.find(&tab) {
            Some((window, _)) => self.open(tab.clone(), window),
            None => match self.review_tab() {
                Some((window, tile)) if !new_tab => {
                    self.windows[window].replace(tile, tab.clone());
                    window
                }
                Some((window, tile)) => {
                    let tabs = self.windows[window].tree.tiles.parent_of(tile);
                    self.add_tab(window, tabs, tab.clone());
                    window
                }
                None => self.open(tab.clone(), 0),
            },
        };
        self.last_review = Some(tab);
        window
    }

    /// Moves a tab to another window, or pops it out into a new one when `to` is `None`.
    pub fn move_tab(&mut self, from: usize, tile: TileId, to: Option<usize>) {
        let Some(tab) = self.windows[from].remove(tile) else { return };
        match to {
            Some(to) => self.windows[to].add(tab),
            None => {
                self.new_window(tab);
            }
        }
    }

    fn new_window(&mut self, tab: Tab) -> &mut Window {
        let id = self.windows.iter().map(|w| w.id).max().unwrap_or(0) + 1;
        self.windows.push(Window::new(id, vec![tab]));
        self.windows.last_mut().unwrap()
    }

    /// The window shown at the given point of the screen, other than the one the tab is dragged from.
    pub fn window_at(&self, point: egui::Pos2, from: usize) -> Option<usize> {
        let is_there = |window: &Window| window.screen_rect.is_some_and(|rect| rect.contains(point));
        (0..self.windows.len()).find(|&index| index != from && is_there(&self.windows[index]))
    }

    /// Drops a tab that was dragged out of its window at the given point of the screen: into
    /// the window that is there, otherwise into a new window. Returns the index of the window
    /// the tab is in now.
    pub fn drop_tab(&mut self, from: usize, tile: TileId, point: egui::Pos2) -> usize {
        let to = self.window_at(point, from);
        if to.is_none() && from > 0 && self.windows[from].tabs().len() == 1 {
            return from; // the tab already has a window of its own
        }
        let Some(tab) = self.windows[from].remove(tile) else { return from };
        match to {
            Some(to) => {
                let window = &mut self.windows[to];
                match window.drop_target(point) {
                    Some(target) => window.add_at(target, tab),
                    None => window.add(tab),
                }
                to
            }
            None => {
                self.new_window(tab).position = Some(point - NEW_WINDOW_GRIP);
                self.windows.len() - 1
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

    fn tabs(window: &Window) -> Vec<String> {
        let mut titles: Vec<String> = window.tabs().into_iter().map(Tab::title).collect();
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
        let review = tile_of(&workspace.windows[0], &Tab::Review(None));
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

    fn on_screen(window: &mut Window, x: f32) {
        window.screen_rect = Some(egui::Rect::from_min_size(egui::pos2(x, 0.0), egui::vec2(100.0, 100.0)));
    }

    #[test]
    fn drops_a_dragged_tab_into_the_window_under_the_pointer() {
        let mut workspace = Workspace::default();
        workspace.open(file("a.rs"), 0);
        let tile = tile_of(&workspace.windows[0], &file("a.rs"));
        workspace.move_tab(0, tile, None);
        workspace.open(file("b.rs"), 1);
        on_screen(&mut workspace.windows[0], 0.0);
        on_screen(&mut workspace.windows[1], 200.0);

        let tile = tile_of(&workspace.windows[1], &file("a.rs"));
        workspace.drop_tab(1, tile, egui::pos2(50.0, 50.0));
        assert_eq!(tabs(&workspace.windows[0]), ["Review", "a.rs"]);
        assert!(is_active(&workspace.windows[0], &file("a.rs")));
        assert_eq!(tabs(&workspace.windows[1]), ["b.rs"]);
    }

    #[test]
    fn drops_a_dragged_tab_into_a_new_window_where_there_is_none() {
        let mut workspace = Workspace::default();
        workspace.open(file("a.rs"), 0);
        on_screen(&mut workspace.windows[0], 0.0);

        let tile = tile_of(&workspace.windows[0], &file("a.rs"));
        workspace.drop_tab(0, tile, egui::pos2(500.0, 300.0));
        assert_eq!(tabs(&workspace.windows[0]), ["Review"]);
        assert_eq!(tabs(&workspace.windows[1]), ["a.rs"]);
        assert_eq!(workspace.windows[1].position, Some(egui::pos2(500.0, 300.0) - NEW_WINDOW_GRIP));

        // the only tab of a popped out window stays where it is
        let tile = tile_of(&workspace.windows[1], &file("a.rs"));
        workspace.drop_tab(1, tile, egui::pos2(900.0, 300.0));
        assert_eq!(workspace.windows.len(), 2);
        assert_eq!(tabs(&workspace.windows[1]), ["a.rs"]);
    }

    #[test]
    fn opens_terminals_with_ids_of_their_own() {
        let mut workspace = Workspace::default();
        assert_eq!(workspace.open_terminal(0, None), 1);
        let tile = tile_of(&workspace.windows[0], &Tab::Terminal(1));
        workspace.move_tab(0, tile, None);
        assert_eq!(workspace.open_terminal(0, None), 2);
        assert_eq!(tabs(&workspace.windows[0]), ["Review", "Terminal"]);
        assert!(is_active(&workspace.windows[0], &Tab::Terminal(2)));
    }

    #[test]
    fn closing_a_window_brings_its_tabs_back() {
        let mut workspace = Workspace::default();
        let review = tile_of(&workspace.windows[0], &Tab::Review(None));
        workspace.move_tab(0, review, None);
        workspace.open(file("a.rs"), 1);
        workspace.close_window(1);
        assert_eq!(workspace.windows.len(), 1);
        assert_eq!(tabs(&workspace.windows[0]), ["Review", "a.rs"]);
    }

    #[test]
    fn prunes_empty_windows_except_the_main_one() {
        let mut workspace = Workspace::default();
        let review = tile_of(&workspace.windows[0], &Tab::Review(None));
        workspace.move_tab(0, review, None);
        let review = tile_of(&workspace.windows[1], &Tab::Review(None));
        workspace.move_tab(1, review, None);
        assert_eq!(workspace.windows.len(), 3);
        workspace.prune();
        assert_eq!(workspace.windows.len(), 2);
        assert!(tabs(&workspace.windows[0]).is_empty());
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

    fn review(path: &str) -> Tab {
        Tab::Review(Some(path.to_string()))
    }

    #[test]
    fn shows_the_changes_of_a_file_in_the_review_tab_used_last() {
        let mut workspace = Workspace::default();
        workspace.show_review("src/a.rs", false);
        assert_eq!(tabs(&workspace.windows[0]), ["Review: a.rs"]);

        // a second review tab, on purpose, next to the first one
        workspace.show_review("src/b.rs", true);
        assert_eq!(tabs(&workspace.windows[0]), ["Review: a.rs", "Review: b.rs"]);
        let first = tile_of(&workspace.windows[0], &review("src/a.rs"));
        let second = tile_of(&workspace.windows[0], &review("src/b.rs"));
        assert_eq!(workspace.windows[0].tree.tiles.parent_of(first), workspace.windows[0].tree.tiles.parent_of(second));

        // the file goes to the tab used last
        workspace.show_review("src/c.rs", false);
        assert_eq!(tabs(&workspace.windows[0]), ["Review: a.rs", "Review: c.rs"]);
        assert!(is_active(&workspace.windows[0], &review("src/c.rs")));

        // a file shown in a review tab already is shown there, wherever the tab is
        workspace.move_tab(0, first, None);
        assert_eq!(workspace.show_review("src/a.rs", false), 1);
        assert_eq!(workspace.last_review, Some(review("src/a.rs")));
        assert_eq!(tabs(&workspace.windows[0]), ["Review: c.rs"]);
    }

    #[test]
    fn shows_the_changes_of_a_file_in_a_new_review_tab_when_there_is_none() {
        let mut workspace = Workspace::default();
        let tile = tile_of(&workspace.windows[0], &Tab::Review(None));
        workspace.windows[0].remove(tile);
        workspace.open(file("a.rs"), 0);
        workspace.last_review = Some(review("gone.rs"));
        workspace.show_review("b.rs", false);
        assert_eq!(tabs(&workspace.windows[0]), ["Review: b.rs", "a.rs"]);
    }
}
