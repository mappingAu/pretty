//! Tiling layout tree. Pure geometry, no window or renderer dependency.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PaneId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn inset(self, gap: f32) -> Self {
        Self {
            x: self.x + gap,
            y: self.y + gap,
            w: (self.w - gap * 2.0).max(0.0),
            h: (self.h - gap * 2.0).max(0.0),
        }
    }

    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    pub fn center(self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Layout {
    Leaf(PaneId),
    Split {
        axis: Axis,
        ratio: f32,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

impl Layout {
    pub fn leaf(id: PaneId) -> Self {
        Self::Leaf(id)
    }

    pub fn split(&mut self, target: PaneId, new_pane: PaneId, axis: Axis) -> bool {
        match self {
            Layout::Leaf(id) if *id == target => {
                let old = *id;
                *self = Layout::Split {
                    axis,
                    ratio: 0.5,
                    first: Box::new(Layout::Leaf(old)),
                    second: Box::new(Layout::Leaf(new_pane)),
                };
                true
            }
            Layout::Leaf(_) => false,
            Layout::Split { first, second, .. } => {
                first.split(target, new_pane, axis) || second.split(target, new_pane, axis)
            }
        }
    }

    /// Remove a pane. Returns false when the pane was not found. Removing the
    /// final pane leaves the tree as a single other leaf; the caller decides
    /// whether to quit.
    pub fn remove(&mut self, target: PaneId) -> bool {
        match self {
            Layout::Leaf(id) => {
                if *id == target {
                    // Keep a valid tree; caller removes the last pane itself.
                    false
                } else {
                    false
                }
            }
            Layout::Split { first, second, .. } => {
                let first_leaf = matches!(**first, Layout::Leaf(id) if id == target);
                let second_leaf = matches!(**second, Layout::Leaf(id) if id == target);
                if first_leaf {
                    *self = (**second).clone();
                    true
                } else if second_leaf {
                    *self = (**first).clone();
                    true
                } else {
                    first.remove(target) || second.remove(target)
                }
            }
        }
    }

    pub fn leaves(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<PaneId>) {
        match self {
            Layout::Leaf(id) => out.push(*id),
            Layout::Split { first, second, .. } => {
                first.collect(out);
                second.collect(out);
            }
        }
    }

    pub fn contains(&self, id: PaneId) -> bool {
        match self {
            Layout::Leaf(p) => *p == id,
            Layout::Split { first, second, .. } => first.contains(id) || second.contains(id),
        }
    }

    /// Compute pixel rectangles. A fixed gap separates neighbours; the gap is
    /// drawn by the app and is not part of any pane content rectangle.
    pub fn rects(&self, area: Rect, gap: f32) -> Vec<(PaneId, Rect)> {
        let mut out = Vec::new();
        self.layout_into(area, gap, &mut out);
        out
    }

    fn layout_into(&self, area: Rect, gap: f32, out: &mut Vec<(PaneId, Rect)>) {
        match self {
            Layout::Leaf(id) => out.push((*id, area)),
            Layout::Split { axis, ratio, first, second } => {
                let ratio = ratio.clamp(0.1, 0.9);
                let half_gap = gap / 2.0;
                match axis {
                    Axis::Horizontal => {
                        let usable = (area.w - gap).max(0.0);
                        let w1 = usable * ratio;
                        let w2 = usable - w1;
                        first.layout_into(
                            Rect::new(area.x, area.y, w1, area.h),
                            gap,
                            out,
                        );
                        second.layout_into(
                            Rect::new(area.x + w1 + gap, area.y, w2, area.h),
                            gap,
                            out,
                        );
                    }
                    Axis::Vertical => {
                        let usable = (area.h - gap).max(0.0);
                        let h1 = usable * ratio;
                        let h2 = usable - h1;
                        first.layout_into(
                            Rect::new(area.x, area.y, area.w, h1),
                            gap,
                            out,
                        );
                        second.layout_into(
                            Rect::new(area.x, area.y + h1 + gap, area.w, h2),
                            gap,
                            out,
                        );
                    }
                }
                let _ = half_gap;
            }
        }
    }

    /// Change the split ratio of the split that directly contains `child` on
    /// its first side. Used by the keyboard resize command.
    pub fn adjust_ratio(&mut self, target: PaneId, delta: f32) -> bool {
        match self {
            Layout::Leaf(_) => false,
            Layout::Split { ratio, first, second, .. } => {
                if first.contains(target) || second.contains(target) {
                    if matches!(**first, Layout::Leaf(id) if id == target)
                        || matches!(**second, Layout::Leaf(id) if id == target)
                    {
                        *ratio = (*ratio + delta).clamp(0.15, 0.85);
                        true
                    } else {
                        first.adjust_ratio(target, delta) || second.adjust_ratio(target, delta)
                    }
                } else {
                    false
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_and_rects() {
        let mut l = Layout::leaf(PaneId(1));
        assert!(l.split(PaneId(1), PaneId(2), Axis::Horizontal));
        assert!(l.split(PaneId(2), PaneId(3), Axis::Vertical));
        assert_eq!(l.leaves(), vec![PaneId(1), PaneId(2), PaneId(3)]);
        let rects = l.rects(Rect::new(0.0, 0.0, 100.0, 50.0), 2.0);
        assert_eq!(rects.len(), 3);
        for (_, r) in &rects {
            assert!(r.w >= 0.0 && r.h >= 0.0);
        }
    }

    #[test]
    fn removes_leaf_and_collapses() {
        let mut l = Layout::leaf(PaneId(1));
        assert!(l.split(PaneId(1), PaneId(2), Axis::Horizontal));
        assert!(l.remove(PaneId(2)));
        assert_eq!(l.leaves(), vec![PaneId(1)]);
        assert!(l.remove(PaneId(1)) == false);
    }
}
