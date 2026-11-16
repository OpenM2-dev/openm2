//! [`Ui`]: the windows of one or more layouts, their state (texts, values,
//! focus, chat lines), pointer and keyboard input, and the draw list.
//!
//! The application feeds input in, reads [`UiEvent`]s out, sets values by
//! node id, and draws [`Ui::draw`]'s list each frame. Nothing here reads a
//! clock or touches the GPU.

use std::collections::{BTreeSet, HashMap, VecDeque};

use crate::draw::{Colour, DrawList, Sprite, WHITE};
use crate::error::UiError;
use crate::geometry::{Anchor, Length, Rect, place};
use crate::layout::{Align, GaugeStart, Kind, Layout, Node};
use crate::skin::{Skin, Skins};
use crate::slots::{MAX_CELLS, PointerButton, SlotItem, Slots};
use crate::text::{Fonts, GlyphPage, line_height};

/// Image names starting with this are pictures the application draws (the
/// character on the select screen), not game paths: `@character`.
pub const PICTURE_PREFIX: &str = "@";

/// Space between an edit field's edge and its text, in pixels.
const EDIT_PADDING: f32 = 3.0;
/// Colour of an edit field's caret.
const CARET: Colour = [255, 255, 255, 230];
/// Laid over the slot under the pointer.
const SLOT_HOVER: Colour = [255, 255, 255, 40];
/// Darkens what is left of a cooldown over an icon.
const COOLING: Colour = [0, 0, 0, 160];
/// Tints a dimmed item.
const DIMMED: Colour = [110, 110, 110, 255];
/// Drawn for an item whose icon is missing.
const NO_ICON: Colour = [150, 120, 70, 255];
/// Behind tooltips when the skin file names no tooltip skin.
const TOOLTIP_BACK: Colour = [0, 0, 0, 200];
/// Space between a tooltip's edge and its text, in pixels.
const TOOLTIP_PADDING: f32 = 6.0;
/// Where a tooltip starts from the pointer, in pixels.
const TOOLTIP_OFFSET: (f32, f32) = (14.0, 18.0);
/// The smallest and largest scale ([`Ui::set_scale`]).
const SCALES: (f32, f32) = (0.5, 4.0);

/// A key the interface reacts to (the application maps its window's keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Submits a field.
    Enter,
    /// Leaves a field.
    Escape,
    /// Deletes the character before the caret.
    Backspace,
    /// Deletes the character after the caret.
    Delete,
    /// Caret one character left.
    Left,
    /// Caret one character right.
    Right,
    /// Caret to the start.
    Home,
    /// Caret to the end.
    End,
    /// Focus the next field of the same window.
    Tab,
}

/// Text at a point of the screen, under the windows: numbers and names
/// over what the game draws (damage over a monster's head).
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    /// The point the text is centred on, in the target's pixels.
    pub at: (f32, f32),
    /// The text.
    pub text: String,
    /// Its colour; a shadow of the same alpha is drawn a pixel (times the
    /// scale) below right.
    pub colour: Colour,
    /// Font size in layout pixels, scaled like the windows; 0 for the skin
    /// file's.
    pub size: f32,
    /// Text in a colour of its own just before the text (a level and a
    /// space before a name); the two are centred together, each with its
    /// shadow.
    pub lead: Option<(String, Colour)>,
    /// A picture in place of the text and the lead, when its look has its
    /// image; without the image, the text shows.
    pub image: Option<LabelImage>,
}

/// A picture a [`Label`] shows: centred on the label's point across,
/// standing where its line of text would, its bottom on the line's.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelImage {
    /// An image look of the skin ([`Skin::Image`]).
    pub look: String,
    /// Its size in layout pixels, scaled like the windows.
    pub size: (f32, f32),
    /// How opaque it is drawn, 0 to 255.
    pub alpha: u8,
}

/// Something the user did that the application acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiEvent {
    /// A button was clicked (pressed and released on it).
    Clicked(String),
    /// Enter in a field.
    Submitted {
        /// The field.
        id: String,
        /// Its text; empty for a secret field (read it with
        /// [`Ui::take_text`]).
        text: String,
    },
    /// Escape in a field; it lost the focus.
    Cancelled(String),
    /// A pointer button went down on a cell of a slot grid.
    SlotPressed {
        /// The grid.
        id: String,
        /// The cell: the top cell of the item covering it, if one does.
        cell: usize,
        /// Which button.
        button: PointerButton,
    },
    /// The main button came up over a cell of a slot grid (where a carried
    /// item is dropped).
    SlotReleased {
        /// The grid.
        id: String,
        /// The cell, as in `SlotPressed`.
        cell: usize,
    },
    /// The pointer moved a slider to another value (read it with
    /// [`Ui::value`]).
    Changed(String),
}

/// A text field's state.
#[derive(Clone, PartialEq)]
struct Edit {
    skin: Option<String>,
    text: String,
    /// Byte offset of the caret, on a character boundary.
    caret: usize,
    max_chars: usize,
    secret: bool,
}

/// Leaves a secret field's text out (passwords).
impl std::fmt::Debug for Edit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text: &dyn std::fmt::Debug = if self.secret {
            &"<redacted>"
        } else {
            &self.text
        };
        f.debug_struct("Edit")
            .field("skin", &self.skin)
            .field("text", text)
            .field("caret", &self.caret)
            .field("max_chars", &self.max_chars)
            .field("secret", &self.secret)
            .finish()
    }
}

impl Edit {
    /// What is shown: the text, or one `*` per character. With the caret's
    /// byte offset in it.
    fn shown(&self) -> (String, usize) {
        if self.secret {
            let caret = self.text.get(..self.caret).map_or(0, |t| t.chars().count());
            ("*".repeat(self.text.chars().count()), caret)
        } else {
            (self.text.clone(), self.caret)
        }
    }

    fn insert(&mut self, text: &str) {
        for c in text.chars().filter(|c| !c.is_control()) {
            if self.text.chars().count() >= self.max_chars {
                break;
            }
            self.text.insert(self.caret, c);
            self.caret += c.len_utf8();
        }
    }

    /// Pasted text, on one line: line breaks and tabs as spaces, the line
    /// breaks at its ends left out.
    fn paste(&mut self, text: &str) {
        let line: String = text
            .trim_matches(['\r', '\n'])
            .replace("\r\n", "\n")
            .chars()
            .map(|c| {
                if matches!(c, '\n' | '\r' | '\t') {
                    ' '
                } else {
                    c
                }
            })
            .collect();
        self.insert(&line);
    }

    fn previous(&self) -> usize {
        self.text
            .get(..self.caret)
            .and_then(|t| t.char_indices().last())
            .map_or(0, |(at, _)| at)
    }

    fn next(&self) -> usize {
        self.text
            .get(self.caret..)
            .and_then(|t| t.chars().next())
            .map_or(self.caret, |c| self.caret + c.len_utf8())
    }

    fn key(&mut self, key: Key) {
        match key {
            Key::Backspace => {
                let from = self.previous();
                self.text.replace_range(from..self.caret, "");
                self.caret = from;
            }
            Key::Delete => {
                let to = self.next();
                self.text.replace_range(self.caret..to, "");
            }
            Key::Left => self.caret = self.previous(),
            Key::Right => self.caret = self.next(),
            Key::Home => self.caret = 0,
            Key::End => self.caret = self.text.len(),
            Key::Enter | Key::Escape | Key::Tab => {}
        }
    }
}

/// A chat log's state.
#[derive(Debug, Clone, PartialEq)]
struct Log {
    lines: VecDeque<(String, Colour)>,
    max_lines: usize,
    /// Lines scrolled back from the newest.
    scroll: usize,
}

/// What a node is, with its state.
#[derive(Debug, Clone, PartialEq)]
enum Widget {
    Group,
    Panel {
        skin: String,
    },
    Image {
        image: String,
    },
    Text {
        text: String,
        align: Align,
        colour: Option<Colour>,
        size: Option<f32>,
    },
    Button {
        skin: String,
        text: String,
        /// Drawn pressed while true (the tab that is open).
        latched: bool,
    },
    Edit(Edit),
    Gauge {
        skin: String,
        value: f32,
        from: GaugeStart,
    },
    Slider {
        bar: String,
        knob: String,
        value: f32,
    },
    Log(Log),
    Slots(Slots),
}

impl Widget {
    fn from(kind: &Kind) -> Self {
        match kind.clone() {
            Kind::Group => Widget::Group,
            Kind::Panel { skin } => Widget::Panel { skin },
            Kind::Image { image } => Widget::Image { image },
            Kind::Text {
                text,
                align,
                colour,
                size,
            } => Widget::Text {
                text,
                align,
                colour,
                size,
            },
            Kind::Button { skin, text } => Widget::Button {
                skin,
                text,
                latched: false,
            },
            Kind::Edit {
                skin,
                max_chars,
                secret,
            } => Widget::Edit(Edit {
                skin,
                text: String::new(),
                caret: 0,
                max_chars,
                secret,
            }),
            Kind::Gauge { skin, value, from } => Widget::Gauge {
                skin,
                value: value.clamp(0.0, 1.0),
                from,
            },
            Kind::Slider { bar, knob, value } => Widget::Slider {
                bar,
                knob,
                value: if value.is_finite() {
                    value.clamp(0.0, 1.0)
                } else {
                    0.0
                },
            },
            Kind::Log { max_lines } => Widget::Log(Log {
                lines: VecDeque::new(),
                max_lines: max_lines.max(1),
                scroll: 0,
            }),
            Kind::Slots {
                skin,
                columns,
                rows,
            } => Widget::Slots(Slots::new(skin, columns, rows)),
        }
    }

    /// Takes the pointer: buttons, fields, sliders, logs and slots.
    fn is_interactive(&self) -> bool {
        matches!(
            self,
            Widget::Button { .. }
                | Widget::Edit(_)
                | Widget::Slider { .. }
                | Widget::Log(_)
                | Widget::Slots(_)
        )
    }
}

#[derive(Debug, Clone)]
struct NodeState {
    id: Option<String>,
    anchor: Anchor,
    pos: (f32, f32),
    size: (Length, Length),
    hidden: bool,
    click_through: bool,
    widget: Widget,
    children: Vec<usize>,
    rect: Rect,
}

/// The interface: windows, their state, input and drawing.
#[derive(Debug)]
pub struct Ui {
    skins: Skins,
    fonts: Fonts,
    nodes: Vec<NodeState>,
    /// Top-level nodes, the last one on top.
    windows: Vec<usize>,
    ids: HashMap<String, usize>,
    sprites: HashMap<String, Sprite>,
    /// Target pixels per layout pixel.
    scale: f32,
    screen: Rect,
    cursor: Option<(f32, f32)>,
    hover: Option<usize>,
    pressed: Option<usize>,
    focus: Option<usize>,
    events: Vec<UiEvent>,
    /// The item the pointer carries.
    held: Option<SlotItem>,
    /// Lines of the tooltip at the pointer; empty for none.
    tooltip: Vec<(String, Colour)>,
    /// Texts over the game's picture, under the windows.
    labels: Vec<Label>,
    list: DrawList,
}

impl Ui {
    /// An interface with these skins and fonts and no windows yet.
    pub fn new(skins: Skins, fonts: Fonts) -> Self {
        Self {
            skins,
            fonts,
            nodes: Vec::new(),
            windows: Vec::new(),
            ids: HashMap::new(),
            sprites: HashMap::new(),
            scale: 1.0,
            screen: Rect::default(),
            cursor: None,
            hover: None,
            pressed: None,
            focus: None,
            events: Vec::new(),
            held: None,
            tooltip: Vec::new(),
            labels: Vec::new(),
            list: DrawList::default(),
        }
    }

    /// Adds a layout's windows on top of the ones there. Fails, adding
    /// nothing, if a node names a skin that is missing or of the wrong kind,
    /// or reuses an id.
    pub fn add(&mut self, layout: &Layout) -> Result<(), UiError> {
        let mut ids = BTreeSet::new();
        for window in &layout.windows {
            self.check(window, &mut ids)?;
        }
        for window in &layout.windows {
            let index = self.insert(window);
            self.windows.push(index);
        }
        Ok(())
    }

    fn check(&self, node: &Node, ids: &mut BTreeSet<String>) -> Result<(), UiError> {
        let who = || match &node.id {
            Some(id) => format!("node `{id}`"),
            None => "a node without id".to_owned(),
        };
        if let Some(id) = &node.id
            && (self.ids.contains_key(id) || !ids.insert(id.clone()))
        {
            return Err(UiError::DuplicateId(id.clone()));
        }
        if let Kind::Slots { columns, rows, .. } = &node.kind {
            let cells = usize::from(*columns) * usize::from(*rows);
            if cells == 0 || cells > MAX_CELLS {
                return Err(UiError::SlotCount {
                    used_by: who(),
                    cells,
                    max: MAX_CELLS,
                });
            }
        }
        let wanted = match &node.kind {
            Kind::Panel { skin } => vec![(skin, "panel")],
            Kind::Edit {
                skin: Some(skin), ..
            }
            | Kind::Slots {
                skin: Some(skin), ..
            } => vec![(skin, "panel")],
            Kind::Button { skin, .. } => vec![(skin, "button")],
            Kind::Gauge { skin, .. } => vec![(skin, "gauge")],
            Kind::Slider { bar, knob, .. } => vec![(bar, "panel"), (knob, "panel")],
            _ => Vec::new(),
        };
        for (name, expected) in wanted {
            let skin = self
                .skins
                .skins
                .get(name)
                .ok_or_else(|| UiError::UnknownSkin {
                    skin: name.clone(),
                    used_by: who(),
                })?;
            let fits = match skin {
                Skin::Button { .. } => expected == "button",
                Skin::Gauge { .. } => expected == "gauge",
                panel => panel.is_panel() && expected == "panel",
            };
            if !fits {
                return Err(UiError::WrongSkin {
                    skin: name.clone(),
                    expected,
                    used_by: who(),
                });
            }
        }
        node.children
            .iter()
            .try_for_each(|child| self.check(child, ids))
    }

    fn insert(&mut self, node: &Node) -> usize {
        let children = node
            .children
            .iter()
            .map(|child| self.insert(child))
            .collect();
        let index = self.nodes.len();
        if let Some(id) = &node.id {
            self.ids.insert(id.clone(), index);
        }
        self.nodes.push(NodeState {
            id: node.id.clone(),
            anchor: node.anchor,
            pos: node.pos,
            size: node.size,
            hidden: node.hidden,
            click_through: node.click_through,
            widget: Widget::from(&node.kind),
            children,
            rect: Rect::default(),
        });
        index
    }

    /// The fonts (for the atlas pages to upload).
    pub fn fonts(&self) -> &Fonts {
        &self.fonts
    }

    /// Draws the layouts `scale` times their size: places, sizes, art and
    /// text, for a screen with more pixels to the inch (the window's scale
    /// factor). Text is drawn at the larger size, so it stays sharp; images
    /// are stretched. What the application hands in or reads back stays in
    /// the target's pixels: the size to draw, the pointer, node rectangles,
    /// label points. Clamped to 0.5 to 4; 1 if not finite.
    pub fn set_scale(&mut self, scale: f32) {
        self.scale = if scale.is_finite() {
            scale.clamp(SCALES.0, SCALES.1)
        } else {
            1.0
        };
    }

    /// Target pixels per layout pixel (1 unless [`Ui::set_scale`] said
    /// otherwise).
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// The skin file it draws with.
    pub fn skins(&self) -> &Skins {
        &self.skins
    }

    /// Every image the skins and layouts name, and the icons in slots and
    /// on the pointer, for the application to load and hand back with
    /// [`Ui::set_sprite`]. Names starting with [`PICTURE_PREFIX`] are left
    /// out: the application draws those itself; so are empty ones, which
    /// draw their skin's fallback colour.
    pub fn image_paths(&self) -> BTreeSet<String> {
        self.named_images().map(str::to_owned).collect()
    }

    /// The images named that have no sprite yet: new icons put in slots,
    /// for the application to load (once) and hand back.
    pub fn missing_images(&self) -> BTreeSet<String> {
        self.named_images()
            .filter(|path| !self.sprites.contains_key(*path))
            .map(str::to_owned)
            .collect()
    }

    fn named_images(&self) -> impl Iterator<Item = &str> {
        let skins = self.skins.skins.values().flat_map(Skin::images);
        let nodes = self.nodes.iter().flat_map(|node| -> Vec<&str> {
            match &node.widget {
                Widget::Image { image } => vec![image.as_str()],
                Widget::Slots(slots) => slots.items().map(|(_, item)| item.icon.as_str()).collect(),
                _ => Vec::new(),
            }
        });
        let held = self.held.iter().map(|item| item.icon.as_str());
        skins
            .chain(nodes)
            .chain(held)
            .filter(|path| !path.is_empty() && !path.starts_with(PICTURE_PREFIX))
    }

    /// Where a node was placed by the last [`Ui::layout`] or [`Ui::draw`]
    /// (the application sizes the pictures it draws by it).
    pub fn rect(&self, id: &str) -> Option<Rect> {
        self.nodes.get(*self.ids.get(id)?).map(|node| node.rect)
    }

    /// What an image path stands for. Paths without a sprite draw their
    /// skin's fallback colour.
    pub fn set_sprite(&mut self, path: &str, sprite: Sprite) {
        self.sprites.insert(path.to_owned(), sprite);
    }

    /// True if a node has this id.
    pub fn has(&self, id: &str) -> bool {
        self.ids.contains_key(id)
    }

    fn node_mut(&mut self, id: &str) -> Option<&mut NodeState> {
        let index = *self.ids.get(id)?;
        self.nodes.get_mut(index)
    }

    /// Sets a text, button label or field's text (caret at the end). False
    /// if no such node has text.
    pub fn set_text(&mut self, id: &str, value: &str) -> bool {
        match self.node_mut(id).map(|n| &mut n.widget) {
            Some(Widget::Text { text, .. } | Widget::Button { text, .. }) => {
                value.clone_into(text);
                true
            }
            Some(Widget::Edit(edit)) => {
                edit.text.clear();
                edit.caret = 0;
                edit.insert(value);
                true
            }
            _ => false,
        }
    }

    /// The text of a text, button or field.
    pub fn text(&self, id: &str) -> Option<&str> {
        let node = self.nodes.get(*self.ids.get(id)?)?;
        match &node.widget {
            Widget::Text { text, .. } | Widget::Button { text, .. } => Some(text),
            Widget::Edit(edit) => Some(&edit.text),
            _ => None,
        }
    }

    /// Moves a field's text out and empties the field: for a secret field,
    /// so the application can keep the only copy (in a type that wipes it).
    pub fn take_text(&mut self, id: &str) -> Option<String> {
        match self.node_mut(id).map(|n| &mut n.widget) {
            Some(Widget::Edit(edit)) => {
                edit.caret = 0;
                Some(std::mem::take(&mut edit.text))
            }
            _ => None,
        }
    }

    /// A gauge's or a slider's value.
    pub fn value(&self, id: &str) -> Option<f32> {
        let node = self.nodes.get(*self.ids.get(id)?)?;
        match &node.widget {
            Widget::Gauge { value, .. } | Widget::Slider { value, .. } => Some(*value),
            _ => None,
        }
    }

    /// Sets a gauge's or a slider's value, clamped to 0–1. False if it is
    /// neither.
    pub fn set_value(&mut self, id: &str, new: f32) -> bool {
        match self.node_mut(id).map(|n| &mut n.widget) {
            Some(Widget::Gauge { value, .. } | Widget::Slider { value, .. }) => {
                *value = if new.is_finite() {
                    new.clamp(0.0, 1.0)
                } else {
                    0.0
                };
                true
            }
            _ => false,
        }
    }

    /// Adds a line to a log, dropping the oldest past its limit. False if it
    /// is not a log.
    pub fn push_line(&mut self, id: &str, text: &str, colour: Colour) -> bool {
        match self.node_mut(id).map(|n| &mut n.widget) {
            Some(Widget::Log(log)) => {
                if log.lines.len() >= log.max_lines {
                    log.lines.pop_front();
                }
                log.lines.push_back((text.to_owned(), colour));
                if log.scroll > 0 {
                    // Keep showing the same lines while scrolled back.
                    log.scroll = (log.scroll + 1).min(log.lines.len().saturating_sub(1));
                }
                true
            }
            _ => false,
        }
    }

    /// Empties a log, scrolled back or not. False if it is not a log.
    pub fn clear_lines(&mut self, id: &str) -> bool {
        match self.node_mut(id).map(|n| &mut n.widget) {
            Some(Widget::Log(log)) => {
                log.lines.clear();
                log.scroll = 0;
                true
            }
            _ => false,
        }
    }

    /// The lines of a log, oldest first.
    pub fn lines(&self, id: &str) -> Vec<&str> {
        let node = self.ids.get(id).and_then(|i| self.nodes.get(*i));
        match node.map(|n| &n.widget) {
            Some(Widget::Log(log)) => log.lines.iter().map(|(t, _)| t.as_str()).collect(),
            _ => Vec::new(),
        }
    }

    fn slots_mut(&mut self, id: &str) -> Option<&mut Slots> {
        match self.node_mut(id).map(|n| &mut n.widget) {
            Some(Widget::Slots(slots)) => Some(slots),
            _ => None,
        }
    }

    /// Puts an item in a cell of a slot grid, or empties the cell. False if
    /// the node is not a slot grid or has no such cell.
    pub fn set_slot(&mut self, id: &str, cell: usize, item: Option<SlotItem>) -> bool {
        self.slots_mut(id)
            .is_some_and(|slots| slots.set(cell, item))
    }

    /// Empties every cell of a slot grid. False if it is not one.
    pub fn clear_slots(&mut self, id: &str) -> bool {
        self.slots_mut(id).map(Slots::clear).is_some()
    }

    /// What a cell of a slot grid holds (an item is in its top cell).
    pub fn slot(&self, id: &str, cell: usize) -> Option<&SlotItem> {
        match self.nodes.get(*self.ids.get(id)?).map(|n| &n.widget) {
            Some(Widget::Slots(slots)) => slots.get(cell),
            _ => None,
        }
    }

    /// The slot grid and cell under the pointer (the top cell of the item
    /// covering it, if one does).
    pub fn hovered_slot(&self) -> Option<(&str, usize)> {
        let (x, y) = self.cursor?;
        let node = self.nodes.get(self.hover?)?;
        match &node.widget {
            Widget::Slots(slots) => {
                let cell = slots.cell_at(node.rect, x, y)?;
                Some((node.id.as_deref()?, slots.owner(cell)))
            }
            _ => None,
        }
    }

    /// The slot grid's id and the cell of `node` under a point, if `node`
    /// is a slot grid with an id.
    fn slot_under(&self, node: usize, x: f32, y: f32) -> Option<(String, usize)> {
        let node = self.nodes.get(node)?;
        match &node.widget {
            Widget::Slots(slots) => {
                let cell = slots.owner(slots.cell_at(node.rect, x, y)?);
                Some((node.id.clone()?, cell))
            }
            _ => None,
        }
    }

    /// The item the pointer carries, drawn at the pointer over everything;
    /// `None` when it carries nothing.
    pub fn set_held(&mut self, item: Option<SlotItem>) {
        self.held = item;
    }

    /// The item the pointer carries.
    pub fn held(&self) -> Option<&SlotItem> {
        self.held.as_ref()
    }

    /// True if the pointer is over the node `id`, shown, in the topmost
    /// window there (click-through windows count only where they take the
    /// pointer, as for clicks): for tooltips over nodes that take no input,
    /// such as gauges. As placed by the last [`Ui::layout`] or [`Ui::draw`].
    pub fn is_hovered(&self, id: &str) -> bool {
        let (Some((x, y)), Some(&index)) = (self.cursor, self.ids.get(id)) else {
            return false;
        };
        let path = self.path_to(index);
        let shown = path
            .iter()
            .all(|at| self.nodes.get(*at).is_some_and(|node| !node.hidden));
        let over = self
            .nodes
            .get(index)
            .is_some_and(|node| node.rect.contains(x, y));
        shown && over && self.hit(x, y).map(|(window, _)| window) == path.first().copied()
    }

    /// Lines in a box at the pointer, over every window; empty for none.
    /// Not drawn while the pointer carries an item.
    pub fn set_tooltip(&mut self, lines: Vec<(String, Colour)>) {
        self.tooltip = lines;
    }

    /// The tooltip's lines.
    pub fn tooltip(&self) -> &[(String, Colour)] {
        &self.tooltip
    }

    /// Texts drawn at points of the screen under every window, replacing
    /// the last ones; empty for none.
    pub fn set_labels(&mut self, labels: Vec<Label>) {
        self.labels = labels;
    }

    /// The labels.
    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    /// Brings a window to the top. False if `id` is not a window.
    pub fn raise(&mut self, id: &str) -> bool {
        let Some(index) = self.ids.get(id).copied() else {
            return false;
        };
        if !self.windows.contains(&index) {
            return false;
        }
        self.windows.retain(|w| *w != index);
        self.windows.push(index);
        true
    }

    /// Changes a node's size, for a window that fits what it shows; its
    /// children follow at the next layout. False if there is none.
    pub fn set_size(&mut self, id: &str, size: (Length, Length)) -> bool {
        match self.node_mut(id) {
            Some(node) => {
                node.size = size;
                true
            }
            None => false,
        }
    }

    /// Draws a panel or a gauge with another look of the skin file, of its
    /// kind (the frames of an animation the application plays). False if
    /// the node is neither, or the look is missing or of another kind.
    pub fn set_look(&mut self, id: &str, look: &str) -> bool {
        let (Some(new), Some(node)) = (
            self.skins.skins.get(look),
            self.ids
                .get(id)
                .and_then(|index| self.nodes.get_mut(*index)),
        ) else {
            return false;
        };
        match &mut node.widget {
            Widget::Panel { skin } if new.is_panel() => look.clone_into(skin),
            Widget::Gauge { skin, .. } if matches!(new, Skin::Gauge { .. }) => {
                look.clone_into(skin);
            }
            _ => return false,
        }
        true
    }

    /// The look a panel or a gauge draws with.
    pub fn look(&self, id: &str) -> Option<&str> {
        match &self.nodes.get(*self.ids.get(id)?)?.widget {
            Widget::Panel { skin } | Widget::Gauge { skin, .. } => Some(skin),
            _ => None,
        }
    }

    /// Shows or hides a node and its children. False if there is none.
    pub fn set_visible(&mut self, id: &str, visible: bool) -> bool {
        let Some(index) = self.ids.get(id).copied() else {
            return false;
        };
        if let Some(node) = self.nodes.get_mut(index) {
            node.hidden = !visible;
        }
        if !visible && self.focus.is_some_and(|f| self.is_inside(f, index)) {
            self.focus = None;
        }
        if !visible && self.hover.is_some_and(|h| self.is_inside(h, index)) {
            self.hover = None;
        }
        true
    }

    /// True if the node and all its ancestors are shown.
    pub fn is_visible(&self, id: &str) -> bool {
        self.ids.get(id).is_some_and(|index| {
            self.path_to(*index)
                .iter()
                .all(|n| self.nodes.get(*n).is_some_and(|n| !n.hidden))
        })
    }

    /// Keeps a button drawn pressed (an open tab) or lets it go. False if it
    /// is not a button.
    pub fn set_latched(&mut self, id: &str, down: bool) -> bool {
        match self.node_mut(id).map(|n| &mut n.widget) {
            Some(Widget::Button { latched, .. }) => {
                *latched = down;
                true
            }
            _ => false,
        }
    }

    /// Gives a field the keyboard. False if it is not a field.
    pub fn focus(&mut self, id: &str) -> bool {
        let Some(index) = self.ids.get(id).copied() else {
            return false;
        };
        match self.nodes.get_mut(index).map(|n| &mut n.widget) {
            Some(Widget::Edit(edit)) => {
                edit.caret = edit.text.len();
                self.focus = Some(index);
                true
            }
            _ => false,
        }
    }

    /// Takes the keyboard from any field.
    pub fn blur(&mut self) {
        self.focus = None;
    }

    /// The id of the field with the keyboard.
    pub fn focused(&self) -> Option<&str> {
        self.focus
            .and_then(|f| self.nodes.get(f))
            .and_then(|n| n.id.as_deref())
    }

    /// What happened since the last call.
    pub fn take_events(&mut self) -> Vec<UiEvent> {
        std::mem::take(&mut self.events)
    }

    /// The pointer moved to (`x`, `y`) pixels.
    pub fn pointer_moved(&mut self, x: f32, y: f32) {
        self.cursor = Some((x, y));
        self.hover = self.hit(x, y).and_then(|(_, node)| node);
        // A slider follows the pointer while it is held, off it too.
        if let Some(slider) = self.pressed {
            self.slide(slider, x);
        }
    }

    /// The pointer left the window.
    pub fn pointer_left(&mut self) {
        self.cursor = None;
        self.hover = None;
    }

    /// The main button went down. True if the interface takes the click (the
    /// game should not): the pointer is over a window, or over a button or
    /// field of a click-through window.
    pub fn press(&mut self) -> bool {
        let Some((x, y)) = self.cursor else {
            return false;
        };
        let hit = self.hit(x, y);
        let node = hit.and_then(|(_, node)| node);
        // A field takes the keyboard; anything else takes it away.
        self.focus = None;
        if let Some((id, cell)) = node.and_then(|n| self.slot_under(n, x, y)) {
            let button = PointerButton::Main;
            self.events.push(UiEvent::SlotPressed { id, cell, button });
        }
        match node
            .and_then(|n| self.nodes.get_mut(n))
            .map(|n| &mut n.widget)
        {
            Some(Widget::Button { .. }) => self.pressed = node,
            Some(Widget::Edit(edit)) => {
                edit.caret = edit.text.len();
                self.focus = node;
            }
            Some(Widget::Slider { .. }) => {
                self.pressed = node;
                if let Some(slider) = node {
                    self.slide(slider, x);
                }
            }
            _ => {}
        }
        if let Some((window, _)) = hit {
            // Raise the window.
            self.windows.retain(|w| *w != window);
            self.windows.push(window);
        }
        self.takes_click(hit)
    }

    /// Moves the slider `index` (if it is one) to the pointer at `x`: the
    /// knob's middle travels between the bar's ends, less half the knob on
    /// each side. A new value is reported.
    fn slide(&mut self, index: usize, x: f32) {
        let Some(node) = self.nodes.get(index) else {
            return;
        };
        let Widget::Slider { knob, value, .. } = &node.widget else {
            return;
        };
        let rect = node.rect;
        let (width, _) = knob_size(&self.skins, &self.sprites, self.scale, knob, rect);
        let travel = rect.w - width;
        let new = if travel > 0.0 {
            ((x - rect.x - width / 2.0) / travel).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if (new - *value).abs() < f32::EPSILON {
            return;
        }
        let id = node.id.clone();
        if let Some(Widget::Slider { value, .. }) = self.nodes.get_mut(index).map(|n| &mut n.widget)
        {
            *value = new;
        }
        if let Some(id) = id {
            self.events.push(UiEvent::Changed(id));
        }
    }

    /// The secondary button went down: a press on a slot cell is reported
    /// and the window under the pointer comes to the top; fields keep the
    /// keyboard. True if the interface takes the click, as for
    /// [`Ui::press`].
    pub fn press_secondary(&mut self) -> bool {
        let Some((x, y)) = self.cursor else {
            return false;
        };
        let hit = self.hit(x, y);
        if let Some((id, cell)) = hit
            .and_then(|(_, node)| node)
            .and_then(|n| self.slot_under(n, x, y))
        {
            let button = PointerButton::Secondary;
            self.events.push(UiEvent::SlotPressed { id, cell, button });
        }
        if let Some((window, _)) = hit {
            self.windows.retain(|w| *w != window);
            self.windows.push(window);
        }
        self.takes_click(hit)
    }

    /// The main button came up. True if the pointer is over a window or a
    /// button was pressed.
    pub fn release(&mut self) -> bool {
        let pressed = self.pressed.take();
        let over = self.cursor.and_then(|(x, y)| self.hit(x, y));
        if let Some(button) = pressed
            && over.and_then(|(_, node)| node) == Some(button)
            && let Some(node) = self.nodes.get(button)
            && matches!(node.widget, Widget::Button { .. })
            && let Some(id) = node.id.clone()
        {
            self.events.push(UiEvent::Clicked(id));
        }
        if let Some((x, y)) = self.cursor
            && let Some((id, cell)) = over
                .and_then(|(_, node)| node)
                .and_then(|n| self.slot_under(n, x, y))
        {
            self.events.push(UiEvent::SlotReleased { id, cell });
        }
        self.takes_click(over) || pressed.is_some()
    }

    /// The wheel turned `lines` (positive away from the user). Scrolls the
    /// log under the pointer. True if the pointer is over a window.
    pub fn wheel(&mut self, lines: f32) -> bool {
        let Some((x, y)) = self.cursor else {
            return false;
        };
        let hit = self.hit(x, y);
        if let Some(Widget::Log(log)) = hit
            .and_then(|(_, node)| node)
            .and_then(|n| self.nodes.get_mut(n))
            .map(|n| &mut n.widget)
        {
            let steps = lines.round() as i64;
            let scroll =
                (log.scroll as i64 + steps).clamp(0, log.lines.len().saturating_sub(1) as i64);
            log.scroll = scroll as usize;
        }
        hit.is_some()
    }

    /// A key went down. True if a field took it (the game should not).
    pub fn key(&mut self, key: Key) -> bool {
        let Some(focus) = self.focus else {
            return false;
        };
        let id = self
            .nodes
            .get(focus)
            .and_then(|n| n.id.clone())
            .unwrap_or_default();
        match key {
            Key::Enter => {
                let text = match self.nodes.get(focus).map(|n| &n.widget) {
                    Some(Widget::Edit(edit)) if !edit.secret => edit.text.clone(),
                    _ => String::new(),
                };
                self.events.push(UiEvent::Submitted { id, text });
            }
            Key::Escape => {
                self.focus = None;
                self.events.push(UiEvent::Cancelled(id));
            }
            Key::Tab => self.focus_next(focus),
            _ => {
                if let Some(Widget::Edit(edit)) = self.nodes.get_mut(focus).map(|n| &mut n.widget) {
                    edit.key(key);
                }
            }
        }
        true
    }

    /// Typed text. True if a field took it.
    pub fn text_input(&mut self, text: &str) -> bool {
        match self
            .focus
            .and_then(|f| self.nodes.get_mut(f))
            .map(|n| &mut n.widget)
        {
            Some(Widget::Edit(edit)) => {
                edit.insert(text);
                true
            }
            _ => false,
        }
    }

    /// Text from the clipboard, at the caret of the field with the
    /// keyboard: on one line (line breaks and tabs as spaces, those at its
    /// ends left out), then as typed text. True if a field took it.
    pub fn paste(&mut self, text: &str) -> bool {
        match self
            .focus
            .and_then(|f| self.nodes.get_mut(f))
            .map(|n| &mut n.widget)
        {
            Some(Widget::Edit(edit)) => {
                edit.paste(text);
                true
            }
            _ => false,
        }
    }

    /// The whole text of the field with the keyboard, for the clipboard;
    /// none from a secret field, or with no field.
    pub fn copy(&self) -> Option<String> {
        match self
            .focus
            .and_then(|f| self.nodes.get(f))
            .map(|n| &n.widget)
        {
            Some(Widget::Edit(edit)) if !edit.secret => Some(edit.text.clone()),
            _ => None,
        }
    }

    /// Moves the focus to the next shown field of the same window, round.
    fn focus_next(&mut self, from: usize) {
        let Some(window) = self
            .windows
            .iter()
            .copied()
            .find(|w| self.is_inside(from, *w))
        else {
            return;
        };
        let mut fields = Vec::new();
        self.collect_fields(window, &mut fields);
        if let Some(at) = fields.iter().position(|f| *f == from)
            && let Some(next) = fields.get((at + 1) % fields.len())
        {
            self.focus = Some(*next);
        }
    }

    fn collect_fields(&self, index: usize, out: &mut Vec<usize>) {
        let Some(node) = self.nodes.get(index) else {
            return;
        };
        if node.hidden {
            return;
        }
        if matches!(node.widget, Widget::Edit(_)) {
            out.push(index);
        }
        for child in &node.children {
            self.collect_fields(*child, out);
        }
    }

    /// True if `node` is `ancestor` or inside it.
    fn is_inside(&self, node: usize, ancestor: usize) -> bool {
        self.path_to(node).contains(&ancestor)
    }

    /// The nodes from a window down to `index`.
    fn path_to(&self, index: usize) -> Vec<usize> {
        fn find(nodes: &[NodeState], at: usize, target: usize, path: &mut Vec<usize>) -> bool {
            path.push(at);
            if at == target {
                return true;
            }
            if let Some(node) = nodes.get(at) {
                for child in &node.children {
                    if find(nodes, *child, target, path) {
                        return true;
                    }
                }
            }
            path.pop();
            false
        }
        let mut path = Vec::new();
        for window in &self.windows {
            if find(&self.nodes, *window, index, &mut path) {
                break;
            }
        }
        path
    }

    /// True if a click at what [`Ui::hit`] found is the interface's: any
    /// window but a click-through one, where only buttons, fields and slots
    /// count.
    fn takes_click(&self, hit: Option<(usize, Option<usize>)>) -> bool {
        let Some((window, node)) = hit else {
            return false;
        };
        let click_through = self.nodes.get(window).is_some_and(|w| w.click_through);
        !click_through
            || matches!(
                node.and_then(|n| self.nodes.get(n)).map(|n| &n.widget),
                Some(Widget::Button { .. } | Widget::Edit(_) | Widget::Slots(_))
            )
    }

    /// The topmost shown window under the point, and the deepest interactive
    /// node under it there. Click-through windows count only where an
    /// interactive node is.
    fn hit(&self, x: f32, y: f32) -> Option<(usize, Option<usize>)> {
        fn deepest(nodes: &[NodeState], at: usize, x: f32, y: f32) -> Option<usize> {
            let node = nodes.get(at)?;
            if node.hidden || !node.rect.contains(x, y) {
                return None;
            }
            node.children
                .iter()
                .rev()
                .find_map(|child| deepest(nodes, *child, x, y))
                .or_else(|| node.widget.is_interactive().then_some(at))
        }
        self.windows.iter().rev().find_map(|window| {
            let node = self.nodes.get(*window)?;
            if node.hidden || !node.rect.contains(x, y) {
                return None;
            }
            let found = deepest(&self.nodes, *window, x, y);
            (!node.click_through || found.is_some()).then_some((*window, found))
        })
    }

    /// True if a click at the point would be the interface's (see
    /// [`Ui::press`]).
    pub fn is_over(&self, x: f32, y: f32) -> bool {
        self.takes_click(self.hit(x, y))
    }

    /// The label drawn over the point (target pixels), the last of those
    /// there, as its index in [`Ui::labels`]: over its picture, or over its
    /// line of text, lead and text together. Labels take no clicks
    /// themselves; the application asks, to let its own things be clicked
    /// by their names.
    pub fn label_at(&mut self, x: f32, y: f32) -> Option<usize> {
        let mut painter = Painter {
            list: &mut self.list,
            fonts: &mut self.fonts,
            sprites: &self.sprites,
            skins: &self.skins,
            nodes: &self.nodes,
            hover: self.hover,
            pressed: self.pressed,
            focus: self.focus,
            cursor: self.cursor,
            scale: self.scale,
        };
        self.labels
            .iter()
            .enumerate()
            .rev()
            .find(|(_, label)| {
                painter
                    .place_label(label)
                    .is_some_and(|placed| placed.rect().contains(x, y))
            })
            .map(|(index, _)| index)
    }

    /// Places every node for a screen of `width` × `height` pixels, at the
    /// scale.
    pub fn layout(&mut self, width: f32, height: f32) {
        self.screen = Rect::new(0.0, 0.0, width, height);
        let screen = self.screen;
        for window in self.windows.clone() {
            self.place(window, &screen);
        }
    }

    fn place(&mut self, index: usize, parent: &Rect) {
        let scale = self.scale;
        let Some(node) = self.nodes.get_mut(index) else {
            return;
        };
        let pos = (node.pos.0 * scale, node.pos.1 * scale);
        let size = (node.size.0.scaled(scale), node.size.1.scaled(scale));
        node.rect = place(parent, node.anchor, pos, size);
        let (rect, children) = (node.rect, node.children.clone());
        for child in children {
            self.place(child, &rect);
        }
    }

    /// Lays out and draws every shown window for a screen of `width` ×
    /// `height` pixels. Returns the quads and the glyph pages they use.
    pub fn draw(&mut self, width: f32, height: f32) -> (&DrawList, &[GlyphPage]) {
        self.layout(width, height);
        self.fonts.begin_frame();
        self.list.clear();
        let mut painter = Painter {
            list: &mut self.list,
            fonts: &mut self.fonts,
            sprites: &self.sprites,
            skins: &self.skins,
            nodes: &self.nodes,
            hover: self.hover,
            pressed: self.pressed,
            focus: self.focus,
            cursor: self.cursor,
            scale: self.scale,
        };
        for label in &self.labels {
            painter.world_label(label);
        }
        for window in &self.windows {
            painter.node(*window);
        }
        if let Some(cursor) = self.cursor {
            match &self.held {
                Some(item) => painter.held(item, cursor),
                None if !self.tooltip.is_empty() => {
                    painter.tooltip(&self.tooltip, cursor, self.screen);
                }
                None => {}
            }
        }
        (&self.list, self.fonts.pages())
    }
}

/// Where a label is drawn, in target pixels.
enum PlacedLabel {
    /// Its picture, over this box.
    Picture(Sprite, Rect),
    /// Its line of text: the font size, where the lead and the text start
    /// across, the line's top, and where the text ends.
    Text {
        size: f32,
        lead_x: f32,
        x: f32,
        y: f32,
        right: f32,
    },
}

impl PlacedLabel {
    /// The box it covers: the picture's, or the line's, lead and text
    /// together.
    fn rect(&self) -> Rect {
        match self {
            Self::Picture(_, rect) => *rect,
            Self::Text {
                size,
                lead_x,
                y,
                right,
                ..
            } => Rect::new(*lead_x, *y, right - lead_x, line_height(*size)),
        }
    }
}

/// Where a label's lead and text start, in whole pixels: the two side by
/// side, centred together on `x`.
fn label_starts(x: f32, lead_width: f32, width: f32) -> (f32, f32) {
    let start = (x - (lead_width + width) / 2.0).round();
    (start, start + lead_width.round())
}

/// A slider's knob in screen pixels: its image's size times the scale
/// when its skin is an image that has a sprite, else a square as tall as
/// the slider.
fn knob_size(
    skins: &Skins,
    sprites: &HashMap<String, Sprite>,
    scale: f32,
    knob: &str,
    rect: Rect,
) -> (f32, f32) {
    match skins.skins.get(knob) {
        Some(Skin::Image { image, .. }) => match sprites.get(image) {
            Some(sprite) => (
                (sprite.size.0 * scale).round(),
                (sprite.size.1 * scale).round(),
            ),
            None => (rect.h, rect.h),
        },
        _ => (rect.h, rect.h),
    }
}

/// Draws nodes into the list.
struct Painter<'a> {
    list: &'a mut DrawList,
    fonts: &'a mut Fonts,
    sprites: &'a HashMap<String, Sprite>,
    skins: &'a Skins,
    nodes: &'a [NodeState],
    hover: Option<usize>,
    pressed: Option<usize>,
    focus: Option<usize>,
    cursor: Option<(f32, f32)>,
    /// Target pixels per layout pixel.
    scale: f32,
}

impl Painter<'_> {
    /// Layout pixels in target pixels.
    fn px(&self, length: f32) -> f32 {
        length * self.scale
    }

    /// A line a layout pixel thick, in whole target pixels.
    fn hairline(&self) -> f32 {
        self.px(1.0).round().max(1.0)
    }

    /// The skin file's font size, in target pixels.
    fn font_size(&self) -> f32 {
        self.px(self.skins.font.size)
    }

    fn node(&mut self, index: usize) {
        let Some(node) = self.nodes.get(index) else {
            return;
        };
        if node.hidden {
            return;
        }
        let rect = node.rect;
        match &node.widget {
            Widget::Group => {}
            Widget::Panel { skin } => self.skin(rect, skin),
            Widget::Image { image } => {
                if let Some(sprite) = self.sprites.get(image) {
                    self.list.sprite(rect, sprite, WHITE, None);
                }
            }
            Widget::Text {
                text,
                align,
                colour,
                size,
            } => {
                let size = self.px(size.unwrap_or(self.skins.font.size));
                let colour = colour.unwrap_or(self.skins.text_colour);
                self.label(rect, text, *align, size, colour, None);
            }
            Widget::Button {
                skin,
                text,
                latched,
            } => self.button(index, rect, skin, text, *latched),
            Widget::Edit(edit) => self.edit(index, rect, edit),
            Widget::Gauge { skin, value, from } => self.gauge(rect, skin, *value, *from),
            Widget::Slider { bar, knob, value } => self.slider(rect, bar, knob, *value),
            Widget::Log(log) => self.log(rect, log),
            Widget::Slots(slots) => self.slots(index, rect, slots),
        }
        for child in &node.children {
            self.node(*child);
        }
    }

    /// A one-line text in `rect`, centred vertically; `size` in target
    /// pixels.
    fn label(
        &mut self,
        rect: Rect,
        text: &str,
        align: Align,
        size: f32,
        colour: Colour,
        clip: Option<Rect>,
    ) {
        if text.is_empty() {
            return;
        }
        let (width, _) = self.fonts.measure(text, size, None);
        let x = match align {
            Align::Left => rect.x,
            Align::Center => rect.x + (rect.w - width) / 2.0,
            Align::Right => rect.right() - width,
        };
        let y = rect.y + (rect.h - line_height(size)) / 2.0;
        self.fonts
            .draw_text(self.list, text, size, None, (x, y), colour, clip);
    }

    /// A label centred on its point, over its shadow; or its picture.
    fn world_label(&mut self, label: &Label) {
        match self.place_label(label) {
            Some(PlacedLabel::Picture(sprite, rect)) => {
                let alpha = label.image.as_ref().map_or(255, |image| image.alpha);
                self.list
                    .sprite(rect, &sprite, [255, 255, 255, alpha], None);
            }
            Some(PlacedLabel::Text {
                size, lead_x, x, y, ..
            }) => {
                let off = self.hairline();
                let lead = label.lead.as_ref().filter(|(text, _)| !text.is_empty());
                let pieces = lead
                    .map(|(text, colour)| (text.as_str(), *colour, lead_x))
                    .into_iter()
                    .chain([(label.text.as_str(), label.colour, x)]);
                for (text, colour, x) in pieces {
                    let shadow = [0, 0, 0, colour[3]];
                    for (at, colour) in [((x + off, y + off), shadow), ((x, y), colour)] {
                        self.fonts
                            .draw_text(self.list, text, size, None, at, colour, None);
                    }
                }
            }
            None => {}
        }
    }

    /// Where a label is drawn: its picture when its look has the image,
    /// else its lead and text; `None` for an empty one.
    fn place_label(&mut self, label: &Label) -> Option<PlacedLabel> {
        let size = if label.size > 0.0 {
            self.px(label.size)
        } else {
            self.font_size()
        };
        if let Some(image) = &label.image
            && let Some(sprite) = self.look_sprite(&image.look)
        {
            let (width, height) = (self.px(image.size.0).round(), self.px(image.size.1).round());
            let bottom = label.at.1 + line_height(size) / 2.0;
            let rect = Rect::new(
                (label.at.0 - width / 2.0).round(),
                (bottom - height).round(),
                width,
                height,
            );
            return Some(PlacedLabel::Picture(sprite, rect));
        }
        let lead = label.lead.as_ref().filter(|(text, _)| !text.is_empty());
        if label.text.is_empty() && lead.is_none() {
            return None;
        }
        let (width, _) = self.fonts.measure(&label.text, size, None);
        // The lead's advance as it stands before the text, so a space
        // ending it counts even where a line's trailing space would not.
        let lead_width = lead.map_or(0.0, |(lead, _)| {
            let whole = format!("{lead}{}", label.text);
            (self.fonts.measure(&whole, size, None).0 - width).max(0.0)
        });
        let (lead_x, x) = label_starts(label.at.0, lead_width, width);
        let y = (label.at.1 - line_height(size) / 2.0).round();
        Some(PlacedLabel::Text {
            size,
            lead_x,
            x,
            y,
            right: x + width.round(),
        })
    }

    /// The sprite of an image look's image, if it has one.
    fn look_sprite(&self, look: &str) -> Option<Sprite> {
        match self.skins.skins.get(look)? {
            Skin::Image { image, .. } => self.sprite(image),
            _ => None,
        }
    }

    /// The sprite of an image path, sized for the screen: pieces of boards
    /// and bars, tiles and icons are drawn at their size times the scale.
    fn sprite(&self, path: &str) -> Option<Sprite> {
        let sprite = self.sprites.get(path)?;
        Some(Sprite {
            size: (self.px(sprite.size.0), self.px(sprite.size.1)),
            ..*sprite
        })
    }

    /// A slider: its bar over `rect`, its knob where the value is.
    fn slider(&mut self, rect: Rect, bar: &str, knob: &str, value: f32) {
        self.skin(rect, bar);
        let (width, height) = knob_size(self.skins, self.sprites, self.scale, knob, rect);
        let x = rect.x + value * (rect.w - width).max(0.0);
        let y = rect.y + (rect.h - height) / 2.0;
        self.skin(Rect::new(x.round(), y.round(), width, height), knob);
    }

    /// A panel skin over `rect`.
    fn skin(&mut self, rect: Rect, name: &str) {
        let Some(skin) = self.skins.skins.get(name) else {
            return;
        };
        match skin {
            Skin::Colour(colour) => self.list.fill(rect, *colour, None),
            Skin::Frame {
                corners,
                edges,
                fill,
                fallback,
            } => self.frame(rect, corners, edges, fill.as_deref(), *fallback),
            Skin::Bar {
                left,
                middle,
                right,
                fallback,
            } => self.bar(rect, [left, middle, right], *fallback),
            Skin::Image { image, fallback } => match self.sprite(image) {
                Some(sprite) => self.list.sprite(rect, &sprite, WHITE, None),
                None => self.list.fill(rect, *fallback, None),
            },
            Skin::Tile { image, fallback } => match self.sprite(image) {
                Some(sprite) => self.list.tile(rect, &sprite, WHITE, Some(rect)),
                None => self.list.fill(rect, *fallback, None),
            },
            // Not panel skins; `Ui::add` and `Skins::parse` refuse them for
            // panels.
            Skin::Button { .. } | Skin::Gauge { .. } => {}
        }
    }

    fn frame(
        &mut self,
        rect: Rect,
        corners: &[String; 4],
        edges: &[String; 4],
        fill: Option<&str>,
        fallback: Colour,
    ) {
        let pieces: Option<Vec<Sprite>> = corners
            .iter()
            .chain(edges)
            .map(|path| self.sprite(path))
            .collect();
        let Some([lt, rt, lb, rb, top, bottom, left, right]) =
            pieces.and_then(|p| <[Sprite; 8]>::try_from(p).ok())
        else {
            self.list.fill(rect, fallback, None);
            return;
        };
        let inner = Rect::new(
            rect.x + left.size.0,
            rect.y + top.size.1,
            rect.w - left.size.0 - right.size.0,
            rect.h - top.size.1 - bottom.size.1,
        );
        match fill.and_then(|f| self.sprite(f)) {
            Some(sprite) => self.list.tile(inner, &sprite, WHITE, Some(inner)),
            None => self.list.fill(inner, fallback, None),
        }
        let edge_clip = Some(rect);
        self.list.tile(
            Rect::new(
                rect.x + lt.size.0,
                rect.y,
                rect.w - lt.size.0 - rt.size.0,
                top.size.1,
            ),
            &top,
            WHITE,
            edge_clip,
        );
        self.list.tile(
            Rect::new(
                rect.x + lb.size.0,
                rect.bottom() - bottom.size.1,
                rect.w - lb.size.0 - rb.size.0,
                bottom.size.1,
            ),
            &bottom,
            WHITE,
            edge_clip,
        );
        self.list.tile(
            Rect::new(
                rect.x,
                rect.y + lt.size.1,
                left.size.0,
                rect.h - lt.size.1 - lb.size.1,
            ),
            &left,
            WHITE,
            edge_clip,
        );
        self.list.tile(
            Rect::new(
                rect.right() - right.size.0,
                rect.y + rt.size.1,
                right.size.0,
                rect.h - rt.size.1 - rb.size.1,
            ),
            &right,
            WHITE,
            edge_clip,
        );
        let corner =
            |sprite: &Sprite, x: f32, y: f32| Rect::new(x, y, sprite.size.0, sprite.size.1);
        self.list
            .sprite(corner(&lt, rect.x, rect.y), &lt, WHITE, None);
        self.list.sprite(
            corner(&rt, rect.right() - rt.size.0, rect.y),
            &rt,
            WHITE,
            None,
        );
        self.list.sprite(
            corner(&lb, rect.x, rect.bottom() - lb.size.1),
            &lb,
            WHITE,
            None,
        );
        self.list.sprite(
            corner(&rb, rect.right() - rb.size.0, rect.bottom() - rb.size.1),
            &rb,
            WHITE,
            None,
        );
    }

    fn bar(&mut self, rect: Rect, [left, middle, right]: [&String; 3], fallback: Colour) {
        let (Some(l), Some(m), Some(r)) =
            (self.sprite(left), self.sprite(middle), self.sprite(right))
        else {
            self.list.fill(rect, fallback, None);
            return;
        };
        // The pieces keep their width and take the node's height.
        let stretch = |s: &Sprite| Sprite {
            size: (s.size.0, rect.h),
            ..*s
        };
        self.list
            .sprite(Rect::new(rect.x, rect.y, l.size.0, rect.h), &l, WHITE, None);
        let middle_rect = Rect::new(
            rect.x + l.size.0,
            rect.y,
            rect.w - l.size.0 - r.size.0,
            rect.h,
        );
        self.list.tile(middle_rect, &stretch(&m), WHITE, None);
        self.list.sprite(
            Rect::new(rect.right() - r.size.0, rect.y, r.size.0, rect.h),
            &r,
            WHITE,
            None,
        );
    }

    fn button(&mut self, index: usize, rect: Rect, skin: &str, text: &str, latched: bool) {
        let Some(Skin::Button {
            normal,
            hover,
            down,
            text_colour,
            fallback,
        }) = self.skins.skins.get(skin)
        else {
            return;
        };
        let (look, shade) = if latched || self.pressed == Some(index) && self.hover == Some(index) {
            (down, 0.7)
        } else if self.hover == Some(index) {
            (hover, 1.25)
        } else {
            (normal, 1.0)
        };
        match self.sprite(look) {
            Some(sprite) => self.list.sprite(rect, &sprite, WHITE, None),
            None => {
                let [r, g, b, a] = *fallback;
                let scale = |c: u8| (f32::from(c) * shade).min(255.0) as u8;
                self.list
                    .fill(rect, [scale(r), scale(g), scale(b), a], None);
            }
        }
        let size = self.font_size();
        self.label(rect, text, Align::Center, size, *text_colour, None);
    }

    fn edit(&mut self, index: usize, rect: Rect, edit: &Edit) {
        if let Some(skin) = &edit.skin {
            self.skin(rect, skin);
        }
        let size = self.font_size();
        let colour = self.skins.text_colour;
        let (shown, caret) = edit.shown();
        let caret_x = self.fonts.caret_x(&shown, size, caret);
        let padding = self.px(EDIT_PADDING);
        let room = rect.w - 2.0 * padding;
        // Scroll left so the caret stays inside.
        let scroll = (caret_x - room).max(0.0);
        let x = rect.x + padding - scroll;
        let y = rect.y + (rect.h - line_height(size)) / 2.0;
        let clip = Some(rect);
        if !shown.is_empty() {
            self.fonts
                .draw_text(self.list, &shown, size, None, (x, y), colour, clip);
        }
        if self.focus == Some(index) {
            let caret = Rect::new(
                (x + caret_x).round(),
                y.round(),
                self.hairline(),
                line_height(size),
            );
            self.list.fill(caret, CARET, clip);
        }
    }

    fn gauge(&mut self, rect: Rect, skin: &str, value: f32, from: GaugeStart) {
        let Some(Skin::Gauge {
            slot,
            fill,
            inset,
            fallback,
        }) = self.skins.skins.get(skin)
        else {
            return;
        };
        if let Some(slot) = slot {
            self.skin(rect, slot);
        }
        let (l, t, r, b) = *inset;
        // The inset scaled, the edges on whole pixels.
        let (left, top) = ((rect.x + self.px(l)).round(), (rect.y + self.px(t)).round());
        let right = (rect.right() - self.px(r)).round();
        let bottom = (rect.bottom() - self.px(b)).round();
        let inner = Rect::new(left, top, right - left, bottom - top);
        let shown = match from {
            GaugeStart::Left => Rect::new(inner.x, inner.y, (inner.w * value).round(), inner.h),
            GaugeStart::Bottom => {
                let height = (inner.h * value).round();
                Rect::new(inner.x, inner.bottom() - height, inner.w, height)
            }
        };
        match (self.sprite(fill), from) {
            (Some(sprite), GaugeStart::Left) => {
                // The pattern keeps its width and takes the gauge's height.
                let sprite = Sprite {
                    size: (sprite.size.0, inner.h),
                    ..sprite
                };
                self.list.tile(shown, &sprite, WHITE, None);
            }
            (Some(sprite), GaugeStart::Bottom) => {
                // The pattern keeps its height and takes the gauge's width,
                // repeated from the bottom up; the top one shows its lower
                // part.
                let h = sprite.size.1;
                if h < 1.0 {
                    return;
                }
                let mut bottom = shown.bottom();
                while bottom > shown.y {
                    let part = h.min(bottom - shown.y);
                    let uv_h = sprite.uv.h * part / h;
                    let uv = Rect::new(sprite.uv.x, sprite.uv.bottom() - uv_h, sprite.uv.w, uv_h);
                    let piece = Rect::new(shown.x, bottom - part, shown.w, part);
                    self.list
                        .sprite(piece, &Sprite { uv, ..sprite }, WHITE, None);
                    bottom -= h;
                }
            }
            (None, _) => self.list.fill(shown, *fallback, None),
        }
    }

    fn slots(&mut self, index: usize, rect: Rect, slots: &Slots) {
        if let Some(skin) = &slots.skin {
            for cell in 0..slots.cells() {
                self.skin(slots.cell_rect(rect, cell), skin);
            }
        }
        for (cell, item) in slots.items() {
            let area = slots.item_rect(rect, cell, item.height);
            self.item(area, item, item.dimmed, Some(rect));
        }
        // The cell (or item) under the pointer, lit.
        if self.hover == Some(index)
            && let Some(cell) = self
                .cursor
                .and_then(|(x, y)| slots.cell_at(rect, x, y))
                .map(|cell| slots.owner(cell))
        {
            let height = slots.get(cell).map_or(1, |item| item.height);
            let lit = slots.item_rect(rect, cell, height);
            self.list.fill(lit, SLOT_HOVER, Some(rect));
        }
    }

    /// An item's icon centred in `area` at its own size (smaller if it does
    /// not fit), and its count in the bottom right corner.
    fn item(&mut self, area: Rect, item: &SlotItem, dimmed: bool, clip: Option<Rect>) {
        let colour = if dimmed { DIMMED } else { WHITE };
        match self.sprite(&item.icon) {
            Some(sprite) if sprite.size.0 >= 1.0 && sprite.size.1 >= 1.0 => {
                let (w, h) = sprite.size;
                let scale = (area.w / w).min(area.h / h).min(1.0);
                let (w, h) = ((w * scale).round(), (h * scale).round());
                let at = Rect::new(
                    (area.x + (area.w - w) / 2.0).round(),
                    (area.y + (area.h - h) / 2.0).round(),
                    w,
                    h,
                );
                self.list.sprite(at, &sprite, colour, clip);
            }
            _ => {
                let gap = self.px(4.0).round();
                let mark = Rect::new(
                    area.x + gap,
                    area.y + gap,
                    area.w - 2.0 * gap,
                    area.h - 2.0 * gap,
                );
                self.list.fill(mark, NO_ICON, clip);
            }
        }
        if item.count > 1 {
            let size = self.font_size();
            let line = line_height(size);
            let right = self.px(2.0).round();
            let corner = Rect::new(area.x, area.bottom() - line, area.w - right, line);
            let text = item.count.to_string();
            let colour = self.skins.text_colour;
            self.label(corner, &text, Align::Right, size, colour, clip);
        }
        if let Some(cooldown) = item.cooldown {
            let left = f32::from(cooldown.left.min(1000)) / 1000.0;
            let shade = Rect::new(area.x, area.y, area.w, (area.h * left).round());
            self.list.fill(shade, COOLING, clip);
            let size = self.font_size();
            let line = line_height(size);
            let middle = Rect::new(area.x, area.y + (area.h - line) / 2.0, area.w, line);
            let text = cooldown.seconds.to_string();
            // Over its shadow, to read on a bright icon too.
            let off = self.hairline();
            let shadow = Rect::new(middle.x + off, middle.y + off, middle.w, middle.h);
            self.label(shadow, &text, Align::Center, size, [0, 0, 0, 255], clip);
            self.label(middle, &text, Align::Center, size, WHITE, clip);
        }
    }

    /// The carried item, centred on the pointer.
    fn held(&mut self, item: &SlotItem, (x, y): (f32, f32)) {
        let (w, h) = self.sprite(&item.icon).map_or(
            (self.px(32.0), self.px(32.0 * f32::from(item.height))),
            |s| s.size,
        );
        let area = Rect::new((x - w / 2.0).round(), (y - h / 2.0).round(), w, h);
        self.item(area, item, false, None);
    }

    /// A box with lines of text below right of the pointer, kept on the
    /// screen.
    fn tooltip(&mut self, lines: &[(String, Colour)], (x, y): (f32, f32), screen: Rect) {
        let size = self.font_size();
        let line = line_height(size);
        let padding = self.px(TOOLTIP_PADDING).round();
        let mut width: f32 = 0.0;
        for (text, _) in lines {
            width = width.max(self.fonts.measure(text, size, None).0);
        }
        let w = (width + 2.0 * padding).ceil();
        let h = (line * lines.len() as f32 + 2.0 * padding).ceil();
        let mut left = x + self.px(TOOLTIP_OFFSET.0);
        if left + w > screen.right() {
            left = x - w - padding;
        }
        let mut top = y + self.px(TOOLTIP_OFFSET.1);
        if top + h > screen.bottom() {
            top = screen.bottom() - h;
        }
        let rect = Rect::new(left.max(0.0).round(), top.max(0.0).round(), w, h);
        match &self.skins.tooltip {
            Some(skin) => self.skin(rect, skin),
            None => self.list.fill(rect, TOOLTIP_BACK, None),
        }
        for (row, (text, colour)) in lines.iter().enumerate() {
            let at = Rect::new(
                rect.x + padding,
                rect.y + padding + row as f32 * line,
                w - 2.0 * padding,
                line,
            );
            self.label(at, text, Align::Left, size, *colour, None);
        }
    }

    fn log(&mut self, rect: Rect, log: &Log) {
        let size = self.font_size();
        let wrap = Some(rect.w.max(1.0));
        let clip = Some(rect);
        let mut bottom = rect.bottom();
        for (text, colour) in log.lines.iter().rev().skip(log.scroll) {
            if bottom <= rect.y {
                break;
            }
            let (_, height) = self.fonts.measure(text, size, wrap);
            let height = height.max(line_height(size));
            bottom -= height;
            self.fonts
                .draw_text(self.list, text, size, wrap, (rect.x, bottom), *colour, clip);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::label_starts;

    #[test]
    fn a_labels_lead_and_text_are_centred_together() {
        // 30 and 50 pixels wide around 100: 60 to 90, then 90 to 140.
        assert_eq!(label_starts(100.0, 30.0, 50.0), (60.0, 90.0));
        assert_eq!(label_starts(100.0, 0.0, 50.0), (75.0, 75.0), "no lead");
        assert_eq!(
            label_starts(100.5, 10.2, 20.0),
            (85.0, 95.0),
            "whole pixels"
        );
    }
}
