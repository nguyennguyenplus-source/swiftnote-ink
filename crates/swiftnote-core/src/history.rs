use std::collections::VecDeque;

use crate::coords::Rect;
use crate::document::{Document, ImagePlacement, Page, PageId};
use crate::paper::PageBackground;
use crate::stroke::Stroke;

const MAX_UNDO_DEPTH: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirtyRegion {
    pub page_id: PageId,
    pub rect: Rect,
}

impl DirtyRegion {
    pub fn full_page(page: &Page) -> Self {
        Self {
            page_id: page.id,
            rect: Rect::new(0.0, 0.0, page.width_pt, page.height_pt),
        }
    }
}

#[derive(Debug, Clone)]
pub enum UndoCommand {
    AddStroke {
        page_id: PageId,
        stroke: Stroke,
    },
    EraseStrokes {
        page_id: PageId,
        removed: Vec<Stroke>,
        added: Vec<Stroke>,
    },
    AddImage {
        page_id: PageId,
        placement: ImagePlacement,
    },
    TransformImage {
        page_id: PageId,
        placement_id: u64,
        old_bounds: Rect,
        new_bounds: Rect,
    },
    DeleteImage {
        page_id: PageId,
        index: usize,
        placement: ImagePlacement,
    },
    AddPage {
        index: usize,
        page: Page,
    },
    DeletePage {
        index: usize,
        page: Page,
    },
    ChangePageBackground {
        page_id: PageId,
        old_bg: PageBackground,
        new_bg: PageBackground,
    },
}

#[derive(Debug, Clone, Default)]
pub struct UndoStack {
    undo: VecDeque<UndoCommand>,
    redo: Vec<UndoCommand>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self {
            undo: VecDeque::with_capacity(64),
            redo: Vec::with_capacity(32),
        }
    }

    pub fn push(&mut self, cmd: UndoCommand) {
        self.redo.clear();
        if self.undo.len() >= MAX_UNDO_DEPTH {
            self.undo.pop_front();
        }
        self.undo.push_back(cmd);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub fn undo(&mut self, doc: &mut Document) -> Option<DirtyRegion> {
        let cmd = self.undo.pop_back()?;
        let dirty = apply_inverse(doc, &cmd);
        self.redo.push(cmd);
        Some(dirty)
    }

    pub fn redo(&mut self, doc: &mut Document) -> Option<DirtyRegion> {
        let cmd = self.redo.pop()?;
        let dirty = apply_forward(doc, &cmd);
        self.undo.push_back(cmd);
        Some(dirty)
    }
}

fn apply_forward(doc: &mut Document, cmd: &UndoCommand) -> DirtyRegion {
    match cmd {
        UndoCommand::AddStroke { page_id, stroke } => {
            if let Some(page) = doc.page_mut(*page_id) {
                page.insert_stroke_sorted(stroke.clone());
            }
            DirtyRegion {
                page_id: *page_id,
                rect: stroke.bounds,
            }
        }
        UndoCommand::EraseStrokes {
            page_id,
            removed,
            added,
        } => {
            let mut rect = Rect::EMPTY;
            if let Some(page) = doc.page_mut(*page_id) {
                for s in removed {
                    page.remove_stroke_by_id(s.id);
                    rect = rect.union(s.bounds);
                }
                for s in added {
                    page.insert_stroke_sorted(s.clone());
                    rect = rect.union(s.bounds);
                }
            }
            DirtyRegion {
                page_id: *page_id,
                rect,
            }
        }
        UndoCommand::AddImage { page_id, placement } => {
            if let Some(page) = doc.page_mut(*page_id) {
                page.images.push(placement.clone());
            }
            DirtyRegion {
                page_id: *page_id,
                rect: placement.bounds,
            }
        }
        UndoCommand::TransformImage {
            page_id,
            placement_id,
            old_bounds,
            new_bounds,
        } => {
            if let Some(page) = doc.page_mut(*page_id) {
                if let Some(img) = page.images.iter_mut().find(|i| i.id == *placement_id) {
                    img.bounds = *new_bounds;
                }
            }
            DirtyRegion {
                page_id: *page_id,
                rect: old_bounds.union(*new_bounds),
            }
        }
        UndoCommand::DeleteImage {
            page_id,
            index,
            placement,
        } => {
            if let Some(page) = doc.page_mut(*page_id) {
                if *index < page.images.len() {
                    page.images.remove(*index);
                }
            }
            DirtyRegion {
                page_id: *page_id,
                rect: placement.bounds,
            }
        }
        UndoCommand::AddPage { index, page } => {
            let idx = (*index).min(doc.pages.len());
            let mut cloned = page.clone();
            cloned.rebuild_index();
            doc.pages.insert(idx, cloned);
            DirtyRegion::full_page(page)
        }
        UndoCommand::DeletePage { index, page } => {
            if *index < doc.pages.len() {
                doc.pages.remove(*index);
            }
            DirtyRegion::full_page(page)
        }
        UndoCommand::ChangePageBackground {
            page_id, new_bg, ..
        } => {
            if let Some(page) = doc.page_mut(*page_id) {
                page.background = new_bg.clone();
                return DirtyRegion::full_page(page);
            }
            DirtyRegion {
                page_id: *page_id,
                rect: Rect::EMPTY,
            }
        }
    }
}

fn apply_inverse(doc: &mut Document, cmd: &UndoCommand) -> DirtyRegion {
    match cmd {
        UndoCommand::AddStroke { page_id, stroke } => {
            if let Some(page) = doc.page_mut(*page_id) {
                page.remove_stroke_by_id(stroke.id);
            }
            DirtyRegion {
                page_id: *page_id,
                rect: stroke.bounds,
            }
        }
        UndoCommand::EraseStrokes {
            page_id,
            removed,
            added,
        } => {
            let mut rect = Rect::EMPTY;
            if let Some(page) = doc.page_mut(*page_id) {
                for s in added {
                    page.remove_stroke_by_id(s.id);
                    rect = rect.union(s.bounds);
                }
                for s in removed {
                    page.insert_stroke_sorted(s.clone());
                    rect = rect.union(s.bounds);
                }
            }
            DirtyRegion {
                page_id: *page_id,
                rect,
            }
        }
        UndoCommand::AddImage { page_id, placement } => {
            if let Some(page) = doc.page_mut(*page_id) {
                page.images.retain(|i| i.id != placement.id);
            }
            DirtyRegion {
                page_id: *page_id,
                rect: placement.bounds,
            }
        }
        UndoCommand::TransformImage {
            page_id,
            placement_id,
            old_bounds,
            new_bounds,
        } => {
            if let Some(page) = doc.page_mut(*page_id) {
                if let Some(img) = page.images.iter_mut().find(|i| i.id == *placement_id) {
                    img.bounds = *old_bounds;
                }
            }
            DirtyRegion {
                page_id: *page_id,
                rect: old_bounds.union(*new_bounds),
            }
        }
        UndoCommand::DeleteImage {
            page_id,
            index,
            placement,
        } => {
            if let Some(page) = doc.page_mut(*page_id) {
                let idx = (*index).min(page.images.len());
                page.images.insert(idx, placement.clone());
            }
            DirtyRegion {
                page_id: *page_id,
                rect: placement.bounds,
            }
        }
        UndoCommand::AddPage { index, page } => {
            if *index < doc.pages.len() {
                doc.pages.remove(*index);
            }
            DirtyRegion::full_page(page)
        }
        UndoCommand::DeletePage { index, page } => {
            let idx = (*index).min(doc.pages.len());
            let mut cloned = page.clone();
            cloned.rebuild_index();
            doc.pages.insert(idx, cloned);
            DirtyRegion::full_page(page)
        }
        UndoCommand::ChangePageBackground {
            page_id, old_bg, ..
        } => {
            if let Some(page) = doc.page_mut(*page_id) {
                page.background = old_bg.clone();
                return DirtyRegion::full_page(page);
            }
            DirtyRegion {
                page_id: *page_id,
                rect: Rect::EMPTY,
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::brush::default_pen_slots;
    use crate::stroke::{StrokeId, StrokePoint};

    #[test]
    fn undo_restores_erased_strokes_in_exact_stroke_id_z_order() {
        let mut doc = Document::new_untitled("n1", "Test", 1);
        let page_id = doc.pages[0].id;
        let style = default_pen_slots()[0];

        for id in [1u64, 2, 3] {
            let s = Stroke::from_points(
                StrokeId(id),
                style,
                vec![
                    StrokePoint::new(10.0, f64::from(id as u32) * 10.0, 2.0, 0.0, 0.0),
                    StrokePoint::new(50.0, f64::from(id as u32) * 10.0, 2.0, 0.0, 0.0),
                ],
            );
            doc.page_mut(page_id).unwrap().insert_stroke_sorted(s);
        }

        let removed = doc
            .page_mut(page_id)
            .unwrap()
            .remove_stroke_by_id(StrokeId(2))
            .unwrap();
        let mut history = UndoStack::new();
        history.push(UndoCommand::EraseStrokes {
            page_id,
            removed: vec![removed],
            added: Vec::new(),
        });
        assert_eq!(
            doc.page(page_id)
                .unwrap()
                .strokes
                .iter()
                .map(|s| s.id)
                .collect::<Vec<_>>(),
            vec![StrokeId(1), StrokeId(3)]
        );

        history.undo(&mut doc).unwrap();
        assert_eq!(
            doc.page(page_id)
                .unwrap()
                .strokes
                .iter()
                .map(|s| s.id)
                .collect::<Vec<_>>(),
            vec![StrokeId(1), StrokeId(2), StrokeId(3)]
        );

        history.redo(&mut doc).unwrap();
        assert_eq!(
            doc.page(page_id)
                .unwrap()
                .strokes
                .iter()
                .map(|s| s.id)
                .collect::<Vec<_>>(),
            vec![StrokeId(1), StrokeId(3)]
        );
    }
}
