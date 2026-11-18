use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use eframe::egui;
use egui_tiles::{Container, Tile, TileId, Tree};
use serde::{Deserialize, Serialize};

use crate::agents::{Agent, Choice};

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Tab {
    Review(Option<String>),
    File(String),
    Terminal(u64),
    Agent {
        id: u64,
        agent: Agent,
        session: String,
        #[serde(default)]
        model: Option<String>,
    },
}

impl Tab {
    pub fn title(&self) -> String {
        let name = |path: &str| path.rsplit('/').next().unwrap_or(path).to_string();
        match self {
            Tab::Review(None) => "Review".to_string(),
            Tab::Review(Some(path)) => format!("Review: {}", name(path)),
            Tab::File(path) => name(path),
            Tab::Terminal(_) => "Terminal".to_string(),
            Tab::Agent { agent, .. } => agent.name().to_string(),
        }
    }

    pub fn path(&self) -> Option<&str> {
        match self {
            Tab::Review(path) => path.as_deref(),
            Tab::File(path) => Some(path),
            Tab::Terminal(_) | Tab::Agent { .. } => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub enum Sidebar {
    #[default]
    #[serde(alias = "Search")]
    Files,
    Changes,
}

const NEW_WINDOW_GRIP: egui::Vec2 = egui::vec2(60.0, 40.0);

const TAB_BAR_HEIGHT: f32 = 24.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(PartialEq, Debug)]
pub struct DropTarget {
    tabs: TileId,
    side: Option<Side>,
    pub rect: egui::Rect,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub id: u64,
    pub tree: Tree<Tab>,
    pub position: Option<egui::Pos2>,
    pub size: Option<egui::Vec2>,
    #[serde(skip)]
    pub screen_rect: Option<egui::Rect>,
    #[serde(skip)]
    pub focused: Option<TileId>,
}

impl Window {
    fn new(id: u64, tabs: Vec<Tab>) -> Self {
        let tree = Tree::new_tabs(egui::Id::unique(("window", id)), tabs);
        Self {
            id,
            tree,
            position: None,
            size: None,
            screen_rect: None,
            focused: None,
        }
    }

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

    fn activate(&mut self, tab: &Tab) -> bool {
        self.tree
            .make_active(|_, tile| matches!(tile, Tile::Pane(t) if t == tab))
    }

    fn target_tabs(&self) -> Option<TileId> {
        let is_tabs = |id: &TileId| self.is_tabs(id);
        let focused = self
            .focused
            .and_then(|tile| self.tree.tiles.parent_of(tile))
            .filter(is_tabs);
        focused.or_else(|| {
            self.tree
                .tiles
                .tile_ids()
                .filter(is_tabs)
                .min_by_key(|id| id.0)
        })
    }

    fn is_tabs(&self, tile: &TileId) -> bool {
        matches!(
            self.tree.tiles.get_container(*tile),
            Some(Container::Tabs(_))
        )
    }

    pub fn drop_target(&self, point: egui::Pos2) -> Option<DropTarget> {
        let point = point - self.screen_rect?.min.to_vec2();
        let tiles = &self.tree.tiles;
        let (tabs, rect) = tiles
            .tile_ids()
            .filter(|id| self.is_tabs(id))
            .find_map(|id| Some((id, tiles.rect(id).filter(|rect| rect.contains(point))?)))?;
        let (left, right) = rect.split_left_right_at_fraction(0.5);
        let (top, bottom) = rect.split_top_bottom_at_fraction(0.5);
        let (bar, body) = rect.split_top_bottom_at_y(rect.top() + TAB_BAR_HEIGHT);
        let sides = [
            (Side::Left, left),
            (Side::Right, right),
            (Side::Top, top),
            (Side::Bottom, bottom),
        ];
        let distance = |rect: &egui::Rect| rect.center().distance_sq(point);
        let nearest = sides
            .into_iter()
            .min_by(|(_, a), (_, b)| distance(a).total_cmp(&distance(b)))
            .filter(|(_, rect)| distance(rect) < distance(&body));
        Some(match nearest.filter(|_| !bar.contains(point)) {
            Some((side, rect)) => DropTarget {
                tabs,
                side: Some(side),
                rect,
            },
            None => DropTarget {
                tabs,
                side: None,
                rect: body,
            },
        })
    }

    fn add_at(&mut self, target: DropTarget, tab: Tab) {
        let Some(side) = target.side else {
            return self.add_to(Some(target.tabs), tab);
        };
        let tiles = &mut self.tree.tiles;
        let parent = tiles.parent_of(target.tabs);
        let tile = tiles.insert_pane(tab);
        let new_tabs = tiles.insert_tab_tile(vec![tile]);
        let children = if matches!(side, Side::Left | Side::Top) {
            vec![new_tabs, target.tabs]
        } else {
            vec![target.tabs, new_tabs]
        };
        let split = if matches!(side, Side::Left | Side::Right) {
            tiles.insert_horizontal_tile(children)
        } else {
            tiles.insert_vertical_tile(children)
        };
        let place = match parent.map(|parent| tiles.get_mut(parent)) {
            None => self.tree.root.as_mut(),
            Some(Some(Tile::Container(Container::Linear(linear)))) => linear
                .children
                .iter_mut()
                .find(|child| **child == target.tabs),
            Some(_) => None,
        };
        if let Some(place) = place {
            *place = split;
        }
        self.focused = Some(tile);
    }

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

    fn add_to(&mut self, tabs: Option<TileId>, tab: Tab) {
        let tile = self.tree.tiles.insert_pane(tab);
        match tabs.and_then(|id| self.tree.tiles.get_mut(id)) {
            Some(Tile::Container(Container::Tabs(tabs))) => {
                tabs.add_child(tile);
                tabs.set_active(tile);
            }
            _ => {
                let children = self.tree.root.into_iter().chain([tile]).collect();
                let root = self.tree.tiles.insert_tab_tile(children);
                if let Some(Tile::Container(Container::Tabs(tabs))) = self.tree.tiles.get_mut(root)
                {
                    tabs.set_active(tile);
                }
                self.tree.root = Some(root);
            }
        }
        self.focused = Some(tile);
    }

    fn remove(&mut self, tile: TileId) -> Option<Tab> {
        self.tree
            .remove_recursively(tile)
            .into_iter()
            .find_map(|tile| match tile {
                Tile::Pane(tab) => Some(tab),
                Tile::Container(_) => None,
            })
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub windows: Vec<Window>,
    pub sidebar: Sidebar,
    pub sidebar_open: bool,
    pub expanded_dirs: BTreeSet<String>,
    pub collapsed_changes: BTreeSet<String>,
    pub agent: Choice,
    pub draft_agent: Choice,
    #[serde(skip)]
    pub last_review: Option<Tab>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            windows: vec![Window::new(0, vec![Tab::Review(None)])],
            sidebar: Sidebar::default(),
            sidebar_open: true,
            expanded_dirs: BTreeSet::new(),
            collapsed_changes: BTreeSet::new(),
            agent: Choice::default(),
            draft_agent: Choice::default(),
            last_review: None,
        }
    }
}

impl Workspace {
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|json| Self::from_json(&json))
            .unwrap_or_default()
    }

    fn from_json(json: &str) -> Option<Self> {
        serde_json::from_str(json)
            .ok()
            .filter(|workspace: &Self| !workspace.windows.is_empty())
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn toggle_sidebar(&mut self, sidebar: Sidebar) {
        self.sidebar_open = !(self.sidebar_open && self.sidebar == sidebar);
        self.sidebar = sidebar;
    }

    pub fn open(&mut self, tab: Tab, window: usize) -> usize {
        if let Some(existing) = self.windows.iter_mut().position(|w| w.activate(&tab)) {
            return existing;
        }
        self.windows[window].add(tab);
        window
    }

    pub fn add_tab(&mut self, window: usize, tabs: Option<TileId>, tab: Tab) {
        let window = &mut self.windows[window];
        window.add_to(tabs.or_else(|| window.target_tabs()), tab);
    }

    fn next_id(&self) -> u64 {
        self.windows
            .iter()
            .flat_map(|window| window.tabs())
            .filter_map(|tab| match tab {
                Tab::Terminal(id) | Tab::Agent { id, .. } => Some(*id),
                _ => None,
            })
            .max()
            .unwrap_or(0)
            + 1
    }

    pub fn open_terminal(&mut self, window: usize, tabs: Option<TileId>) -> u64 {
        let id = self.next_id();
        self.add_tab(window, tabs, Tab::Terminal(id));
        id
    }

    pub fn open_agent(
        &mut self,
        window: usize,
        tabs: Option<TileId>,
        agent: Agent,
        session: String,
        model: Option<String>,
    ) -> u64 {
        let id = self.next_id();
        self.add_tab(
            window,
            tabs,
            Tab::Agent {
                id,
                agent,
                session,
                model,
            },
        );
        id
    }

    pub fn set_session(&mut self, agent_id: u64, new: String) -> bool {
        let session = self.windows.iter_mut().find_map(|window| {
            window.tree.tiles.tiles_mut().find_map(|tile| match tile {
                Tile::Pane(Tab::Agent { id, session, .. }) if *id == agent_id => Some(session),
                _ => None,
            })
        });
        session.map(|session| *session = new).is_some()
    }

    fn find(&self, tab: &Tab) -> Option<(usize, TileId)> {
        self.windows
            .iter()
            .enumerate()
            .find_map(|(index, window)| Some((index, window.tree.tiles.find_pane(tab)?)))
    }

    fn review_tab(&self) -> Option<(usize, TileId)> {
        let first = || {
            self.windows.iter().enumerate().find_map(|(index, window)| {
                let tiles = window
                    .tree
                    .tiles
                    .iter()
                    .filter(|(_, tile)| matches!(tile, Tile::Pane(Tab::Review(_))));
                Some((index, *tiles.map(|(id, _)| id).min_by_key(|id| id.0)?))
            })
        };
        self.last_review
            .as_ref()
            .and_then(|tab| self.find(tab))
            .or_else(first)
    }

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

    pub fn move_tab(&mut self, from: usize, tile: TileId, to: Option<usize>) {
        let Some(tab) = self.windows[from].remove(tile) else {
            return;
        };
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

    pub fn window_at(&self, point: egui::Pos2, from: usize) -> Option<usize> {
        let is_there =
            |window: &Window| window.screen_rect.is_some_and(|rect| rect.contains(point));
        (0..self.windows.len()).find(|&index| index != from && is_there(&self.windows[index]))
    }

    pub fn drop_tab(&mut self, from: usize, tile: TileId, point: egui::Pos2) -> usize {
        let to = self.window_at(point, from);
        if to.is_none() && from > 0 && self.windows[from].tabs().len() == 1 {
            return from;
        }
        let Some(tab) = self.windows[from].remove(tile) else {
            return from;
        };
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

    pub fn close_window(&mut self, index: usize) {
        if index == 0 {
            return;
        }
        let window = self.windows.remove(index);
        for tab in window.tabs() {
            self.windows[0].add(tab.clone());
        }
    }

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

        assert_eq!(workspace.open(file("a.rs"), 0), 1);
        assert_eq!(workspace.open(file("b.rs"), 1), 1);

        let tile = tile_of(&workspace.windows[1], &file("a.rs"));
        workspace.move_tab(1, tile, Some(0));
        assert_eq!(tabs(&workspace.windows[0]), ["Review", "a.rs"]);
        assert_eq!(tabs(&workspace.windows[1]), ["b.rs"]);
    }

    fn on_screen(window: &mut Window, x: f32) {
        window.screen_rect = Some(egui::Rect::from_min_size(
            egui::pos2(x, 0.0),
            egui::vec2(100.0, 100.0),
        ));
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
        assert_eq!(
            workspace.windows[1].position,
            Some(egui::pos2(500.0, 300.0) - NEW_WINDOW_GRIP)
        );

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
    fn opens_agent_tabs_that_share_their_ids_and_survive_a_restart() {
        let mut workspace = Workspace::default();
        let id = workspace.open_agent(0, None, Agent::Claude, "s1".to_string(), None);
        assert_eq!(id, 1);
        assert_eq!(tabs(&workspace.windows[0]), ["Claude Code", "Review"]);
        assert_eq!(workspace.open_terminal(0, None), 2);
        let id = workspace.open_agent(0, None, Agent::Opencode, "s2".to_string(), None);
        assert_eq!(id, 3);

        let restored = Workspace::from_json(&workspace.to_json()).unwrap();
        let restored_tabs = restored.windows[0].tabs();
        let claude = Tab::Agent {
            id: 1,
            agent: Agent::Claude,
            session: "s1".to_string(),
            model: None,
        };
        let opencode = Tab::Agent {
            id: 3,
            agent: Agent::Opencode,
            session: "s2".to_string(),
            model: None,
        };
        assert!(restored_tabs.contains(&&claude));
        assert!(restored_tabs.contains(&&opencode));
    }

    #[test]
    fn agent_tabs_follow_the_session_their_agent_moved_to() {
        let mut workspace = Workspace::default();
        let id = workspace.open_agent(0, None, Agent::Claude, "s1".to_string(), None);
        let claude = |session: &str| Tab::Agent {
            id,
            agent: Agent::Claude,
            session: session.to_string(),
            model: None,
        };
        workspace.move_tab(0, tile_of(&workspace.windows[0], &claude("s1")), None);

        assert!(workspace.set_session(id, "s2".to_string()));
        assert!(!workspace.set_session(id + 1, "s3".to_string()));
        assert_eq!(workspace.windows[1].tabs(), [&claude("s2")]);
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
        workspace.toggle_sidebar(Sidebar::Changes);
        workspace.expanded_dirs.insert("src".to_string());
        workspace.agent = Choice {
            agent: Some(Agent::Opencode),
            model: Some("deepseek/deepseek-flash".to_string()),
        };
        workspace.draft_agent = Choice {
            agent: Some(Agent::Claude),
            model: Some("haiku".to_string()),
        };

        let restored = Workspace::from_json(&workspace.to_json()).unwrap();
        assert!(
            restored
                .windows
                .iter()
                .zip(&workspace.windows)
                .all(|(a, b)| a.tree == b.tree)
        );
        assert_eq!(tabs(&restored.windows[0]), ["Review", "a.rs"]);
        assert_eq!(tabs(&restored.windows[1]), ["b.rs"]);
        assert_eq!(restored.windows[1].position, Some(egui::pos2(10.0, 20.0)));
        assert_eq!(restored.sidebar, Sidebar::Changes);
        assert!(restored.sidebar_open);
        assert_eq!(
            restored.agent,
            Choice {
                agent: Some(Agent::Opencode),
                model: Some("deepseek/deepseek-flash".to_string()),
            }
        );
        assert_eq!(
            restored.draft_agent,
            Choice {
                agent: Some(Agent::Claude),
                model: Some("haiku".to_string()),
            }
        );
    }

    #[test]
    fn opens_the_sidebar_chosen_and_closes_it_when_chosen_again() {
        let mut workspace = Workspace::default();
        assert!(workspace.sidebar_open);
        workspace.toggle_sidebar(Sidebar::Files);
        assert!(!workspace.sidebar_open);
        workspace.toggle_sidebar(Sidebar::Changes);
        assert!(workspace.sidebar_open);
        assert_eq!(workspace.sidebar, Sidebar::Changes);
        workspace.toggle_sidebar(Sidebar::Files);
        assert!(workspace.sidebar_open);
        assert_eq!(workspace.sidebar, Sidebar::Files);
        workspace.toggle_sidebar(Sidebar::Files);
        assert!(!workspace.sidebar_open);
    }

    #[test]
    fn opens_the_files_where_the_search_was_shown_before() {
        let json = Workspace::default()
            .to_json()
            .replace("\"Files\"", "\"Search\"");
        assert!(json.contains("\"Search\""));
        assert_eq!(Workspace::from_json(&json).unwrap().sidebar, Sidebar::Files);
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

        workspace.show_review("src/b.rs", true);
        assert_eq!(
            tabs(&workspace.windows[0]),
            ["Review: a.rs", "Review: b.rs"]
        );
        let first = tile_of(&workspace.windows[0], &review("src/a.rs"));
        let second = tile_of(&workspace.windows[0], &review("src/b.rs"));
        assert_eq!(
            workspace.windows[0].tree.tiles.parent_of(first),
            workspace.windows[0].tree.tiles.parent_of(second)
        );

        workspace.show_review("src/c.rs", false);
        assert_eq!(
            tabs(&workspace.windows[0]),
            ["Review: a.rs", "Review: c.rs"]
        );
        assert!(is_active(&workspace.windows[0], &review("src/c.rs")));

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
