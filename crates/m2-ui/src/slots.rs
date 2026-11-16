//! Item slots (`Kind::Slots`): a grid of cells that hold icons, as in the
//! inventory and the quickslot bar, and the item the pointer carries
//! (`docs/specs/ui.md`, "Slots").
//!
//! An item sits in its top cell and covers `height` cells downwards in the
//! same column (a sword two cells tall). The application says what each
//! cell holds; the interface draws it and reports presses and releases on
//! cells.

use crate::geometry::Rect;

/// Most cells one slot grid may have.
pub const MAX_CELLS: usize = 1024;

/// An item in a slot, or carried by the pointer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotItem {
    /// Image of the icon: a game path, like the layouts' images.
    pub icon: String,
    /// The number drawn in its corner when it is more than 1.
    pub count: u32,
    /// Cells it covers downwards, at least 1.
    pub height: u16,
    /// Drawn darker (the item the pointer carries, left behind in its slot).
    pub dimmed: bool,
    /// Cooling down (a skill used): darkened by what is left, with the
    /// seconds left on it.
    pub cooldown: Option<Cooldown>,
}

impl SlotItem {
    /// One item with this icon, one cell tall.
    pub fn new(icon: impl Into<String>) -> Self {
        Self {
            icon: icon.into(),
            count: 1,
            height: 1,
            dimmed: false,
            cooldown: None,
        }
    }
}

/// How far a slot's cooldown has to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cooldown {
    /// What is left of it, in thousandths (1000 just begun): that much of
    /// the icon, from the top, is darkened.
    pub left: u16,
    /// The whole seconds left, shown on the icon.
    pub seconds: u32,
}

/// Which pointer button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton {
    /// The main (left) button.
    Main,
    /// The secondary (right) button.
    Secondary,
}

/// A slot grid's state.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Slots {
    /// Panel skin drawn under each cell, if any.
    pub skin: Option<String>,
    pub columns: u16,
    pub rows: u16,
    /// What each cell holds, by cell (row by row); an item is in its top
    /// cell.
    items: Vec<Option<SlotItem>>,
}

impl Slots {
    /// An empty grid. `columns` × `rows` must be checked against
    /// [`MAX_CELLS`] first.
    pub fn new(skin: Option<String>, columns: u16, rows: u16) -> Self {
        let columns = columns.max(1);
        let rows = rows.max(1);
        Self {
            skin,
            columns,
            rows,
            items: vec![None; usize::from(columns) * usize::from(rows)],
        }
    }

    pub fn cells(&self) -> usize {
        self.items.len()
    }

    /// Puts an item in a cell or empties it. False if there is no such cell.
    pub fn set(&mut self, cell: usize, item: Option<SlotItem>) -> bool {
        match self.items.get_mut(cell) {
            Some(slot) => {
                *slot = item.map(|mut item| {
                    item.height = item.height.max(1);
                    item
                });
                true
            }
            None => false,
        }
    }

    pub fn get(&self, cell: usize) -> Option<&SlotItem> {
        self.items.get(cell).and_then(Option::as_ref)
    }

    pub fn clear(&mut self) {
        self.items.iter_mut().for_each(|slot| *slot = None);
    }

    /// The items, by their top cell.
    pub fn items(&self) -> impl Iterator<Item = (usize, &SlotItem)> {
        self.items
            .iter()
            .enumerate()
            .filter_map(|(cell, item)| item.as_ref().map(|item| (cell, item)))
    }

    /// The cell whose item covers `cell`: itself if it holds one, a cell
    /// above it whose item reaches down to it, or else `cell` itself.
    pub fn owner(&self, cell: usize) -> usize {
        let columns = usize::from(self.columns);
        let mut above = cell;
        let mut reach = 1;
        loop {
            if let Some(item) = self.get(above) {
                return if usize::from(item.height) >= reach {
                    above
                } else {
                    cell
                };
            }
            match above.checked_sub(columns) {
                Some(next) => {
                    above = next;
                    reach += 1;
                }
                None => return cell,
            }
        }
    }

    fn cell_size(&self, rect: Rect) -> (f32, f32) {
        (
            rect.w / f32::from(self.columns),
            rect.h / f32::from(self.rows),
        )
    }

    /// Where `cell` is in a grid placed at `rect`, in whole pixels.
    pub fn cell_rect(&self, rect: Rect, cell: usize) -> Rect {
        self.item_rect(rect, cell, 1)
    }

    /// The rectangle an item `height` cells tall at `cell` covers, cut at
    /// the grid's bottom.
    pub fn item_rect(&self, rect: Rect, cell: usize, height: u16) -> Rect {
        let columns = usize::from(self.columns);
        let (column, row) = (cell % columns, cell / columns);
        let (w, h) = self.cell_size(rect);
        let x0 = (rect.x + column as f32 * w).round();
        let x1 = (rect.x + (column + 1) as f32 * w).round();
        let y0 = (rect.y + row as f32 * h).round();
        let y1 = (rect.y + (row + usize::from(height.max(1))) as f32 * h)
            .round()
            .min(rect.bottom());
        Rect::new(x0, y0, x1 - x0, (y1 - y0).max(0.0))
    }

    /// The cell under a point, if the grid at `rect` has one there.
    pub fn cell_at(&self, rect: Rect, x: f32, y: f32) -> Option<usize> {
        if !rect.contains(x, y) {
            return None;
        }
        let (w, h) = self.cell_size(rect);
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        let column = (((x - rect.x) / w) as usize).min(usize::from(self.columns) - 1);
        let row = (((y - rect.y) / h) as usize).min(usize::from(self.rows) - 1);
        Some(row * usize::from(self.columns) + column)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tall(height: u16) -> Option<SlotItem> {
        Some(SlotItem {
            height,
            ..SlotItem::new("icon.tga")
        })
    }

    #[test]
    fn tall_items_own_the_cells_below_them() {
        let mut slots = Slots::new(None, 5, 9);
        assert!(slots.set(1, tall(3)));
        assert!(slots.set(12, tall(1)));
        assert!(!slots.set(45, tall(1)), "no cell 45 in 5 × 9");
        // The sword in column 1 covers rows 0 to 2.
        assert_eq!(slots.owner(1), 1);
        assert_eq!(slots.owner(6), 1);
        assert_eq!(slots.owner(11), 1);
        assert_eq!(slots.owner(16), 16, "row 3 is past it");
        // A short item covers only itself; empty cells own themselves.
        assert_eq!(slots.owner(17), 17);
        assert_eq!(slots.owner(12), 12);
        assert_eq!(slots.owner(0), 0);
        assert_eq!(
            slots.items().map(|(cell, _)| cell).collect::<Vec<_>>(),
            [1, 12]
        );
    }

    #[test]
    fn cells_are_placed_in_whole_pixels() {
        let slots = Slots::new(None, 5, 9);
        let rect = Rect::new(10.0, 20.0, 160.0, 288.0);
        assert_eq!(slots.cell_rect(rect, 0), Rect::new(10.0, 20.0, 32.0, 32.0));
        assert_eq!(slots.cell_rect(rect, 7), Rect::new(74.0, 52.0, 32.0, 32.0));
        assert_eq!(
            slots.item_rect(rect, 40, 3),
            Rect::new(10.0, 276.0, 32.0, 32.0),
            "cut at the bottom"
        );
        assert_eq!(slots.cell_at(rect, 74.0, 52.0), Some(7));
        assert_eq!(slots.cell_at(rect, 169.9, 307.9), Some(44));
        assert_eq!(slots.cell_at(rect, 170.0, 30.0), None);
    }
}
