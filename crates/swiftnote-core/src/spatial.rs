use rstar::{RTree, RTreeObject, AABB};

use crate::coords::Rect;
use crate::stroke::{Stroke, StrokeId};

#[derive(Debug, Clone, Copy, PartialEq)]
struct IndexedStroke {
    id: StrokeId,
    bounds: Rect,
}

impl RTreeObject for IndexedStroke {
    type Envelope = AABB<[f64; 2]>;

    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners(
            [self.bounds.min_x, self.bounds.min_y],
            [self.bounds.max_x, self.bounds.max_y],
        )
    }
}

/// Spatial R-Tree index for $O(\log N)$ stroke hit-testing and eraser queries.
#[derive(Debug, Clone, Default)]
pub struct StrokeIndex {
    tree: RTree<IndexedStroke>,
}

impl StrokeIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, stroke: &Stroke) {
        if stroke.bounds.is_empty() {
            return;
        }
        self.tree.insert(IndexedStroke {
            id: stroke.id,
            bounds: stroke.bounds,
        });
    }

    pub fn remove(&mut self, id: StrokeId, bounds: Rect) -> bool {
        if bounds.is_empty() {
            return false;
        }
        self.tree.remove(&IndexedStroke { id, bounds }).is_some()
    }

    pub fn query_rect(&self, rect: Rect) -> Vec<StrokeId> {
        if rect.is_empty() {
            return Vec::new();
        }
        let env = AABB::from_corners([rect.min_x, rect.min_y], [rect.max_x, rect.max_y]);
        let mut ids: Vec<StrokeId> = self
            .tree
            .locate_in_envelope_intersecting(env)
            .map(|entry| entry.id)
            .collect();
        ids.sort_unstable();
        ids
    }

    pub fn rebuild(&mut self, strokes: &[Stroke]) {
        let items: Vec<IndexedStroke> = strokes
            .iter()
            .filter(|s| !s.bounds.is_empty())
            .map(|s| IndexedStroke {
                id: s.id,
                bounds: s.bounds,
            })
            .collect();
        self.tree = RTree::bulk_load(items);
    }

    pub fn len(&self) -> usize {
        self.tree.size()
    }

    pub fn is_empty(&self) -> bool {
        self.tree.size() == 0
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::brush::default_pen_slots;
    use crate::stroke::StrokePoint;
    use proptest::prelude::*;

    #[test]
    fn insert_remove_and_query_work() {
        let style = default_pen_slots()[0];
        let s1 = Stroke::from_points(
            StrokeId(1),
            style,
            vec![StrokePoint::new(10.0, 10.0, 2.0, 0.0, 0.0)],
        );
        let s2 = Stroke::from_points(
            StrokeId(2),
            style,
            vec![StrokePoint::new(100.0, 100.0, 2.0, 0.0, 0.0)],
        );
        let mut idx = StrokeIndex::new();
        idx.insert(&s1);
        idx.insert(&s2);
        assert_eq!(
            idx.query_rect(Rect::new(0.0, 0.0, 20.0, 20.0)),
            vec![StrokeId(1)]
        );
        assert!(idx.remove(s1.id, s1.bounds));
        assert!(idx.query_rect(Rect::new(0.0, 0.0, 20.0, 20.0)).is_empty());
    }

    proptest! {
        #[test]
        fn rtree_matches_bruteforce_oracle_proptest(
            boxes in prop::collection::vec((0.0f64..400.0, 0.0f64..400.0, 1.0f64..40.0, 1.0f64..40.0), 1..40),
            qx in -20.0f64..420.0,
            qy in -20.0f64..420.0,
            qw in 1.0f64..100.0,
            qh in 1.0f64..100.0,
        ) {
            let style = default_pen_slots()[0];
            let strokes: Vec<Stroke> = boxes
                .iter()
                .enumerate()
                .map(|(i, &(x, y, w, h))| {
                    Stroke {
                        id: StrokeId(i as u64 + 1),
                        style,
                        points: vec![StrokePoint::new(x, y, 2.0, 0.0, 0.0)],
                        bounds: Rect::new(x, y, x + w, y + h),
                    }
                })
                .collect();
            let mut idx = StrokeIndex::new();
            idx.rebuild(&strokes);

            let query = Rect::new(qx, qy, qx + qw, qy + qh);
            let got = idx.query_rect(query);
            let mut expected: Vec<StrokeId> = strokes
                .iter()
                .filter(|s| s.bounds.intersects(query))
                .map(|s| s.id)
                .collect();
            expected.sort_unstable();
            prop_assert_eq!(got, expected);
        }
    }
}
