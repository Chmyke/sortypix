//! Cached, ordered, justified rows. Only intersecting rows are painted.
use eframe::egui::{self, Rect};
use std::ops::Range;
pub const GAP: f32 = 6.;
#[derive(Default)]
pub struct Layout {
    pub cells: Vec<Rect>,
    pub rows: Vec<Range<usize>>,
    pub height: f32,
    width: f32,
    target: f32,
    dirty: bool,
}
impl Layout {
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }
    pub fn update(&mut self, width: f32, target: f32, ratios: impl Iterator<Item = f32>) {
        if !self.dirty && self.width == width && self.target == target {
            return;
        }
        self.width = width.max(1.);
        self.target = target.max(1.);
        self.dirty = false;
        self.cells.clear();
        self.rows.clear();
        self.height = 0.;
        let ratios: Vec<_> = ratios
            .map(|r| if r.is_finite() && r > 0. { r } else { 1.5 })
            .collect();
        let mut start = 0;
        while start < ratios.len() {
            let mut end = start;
            let mut sum = 0.;
            while end < ratios.len() {
                if end > start && GAP * (end - start) as f32 >= self.width {
                    break;
                }
                let next = sum + ratios[end];
                let fit = (self.width - GAP * (end - start) as f32).max(1.) / next;
                if fit <= self.target {
                    if end > start {
                        let previous = (self.width - GAP * (end - start - 1) as f32) / sum;
                        if (previous - self.target).abs() < (fit - self.target).abs() {
                            break;
                        }
                    }
                    sum = next;
                    end += 1;
                    break;
                }
                sum = next;
                end += 1;
            }
            let fit = (self.width - GAP * (end - start - 1) as f32).max(1.) / sum;
            // A short final row stays at the requested scale, never an enormous lone photo.
            let height = if end == ratios.len() {
                fit.min(self.target)
            } else {
                fit
            };
            let mut x = 0.;
            for ratio in &ratios[start..end] {
                let width = height * ratio;
                self.cells.push(Rect::from_min_size(
                    egui::pos2(x, self.height),
                    egui::vec2(width, height),
                ));
                x += width + GAP;
            }
            self.rows.push(start..end);
            self.height += height + GAP;
            start = end;
        }
        self.height = (self.height - GAP).max(0.);
    }
    pub fn visible(&self, top: f32, bottom: f32) -> Range<usize> {
        let start = self
            .rows
            .partition_point(|row| self.cells[row.start].bottom() < top);
        let end = self
            .rows
            .partition_point(|row| self.cells[row.start].top() <= bottom);
        start..end
    }
    pub fn vertical(&self, index: usize, down: bool) -> usize {
        if self.dirty {
            return index;
        }
        let Some(cell) = self.cells.get(index) else {
            return index;
        };
        let row = self.rows.partition_point(|r| r.end <= index);
        let next = if down { row + 1 } else { row.saturating_sub(1) };
        let Some(range) = self.rows.get(next) else {
            return index;
        };
        range
            .clone()
            .min_by(|a, b| {
                (self.cells[*a].center().x - cell.center().x)
                    .abs()
                    .total_cmp(&(self.cells[*b].center().x - cell.center().x).abs())
            })
            .unwrap_or(index)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_rows_fill_width_preserve_order_ratios_and_spacing() {
        let ratios = [0.5, 1.5, 3., 1., 0.75, 2., 1.5, 0.66, 4., 1.];
        for width in [400., 800., 1240.] {
            for target in [100., 190., 360.] {
                let mut layout = Layout::default();
                layout.update(width, target, ratios.into_iter());
                assert_eq!(layout.cells.len(), ratios.len());
                let mut next = 0;
                for (n, row) in layout.rows.iter().enumerate() {
                    assert_eq!(row.start, next);
                    next = row.end;
                    for i in row.clone() {
                        let c = layout.cells[i];
                        assert!((c.width() / c.height() - ratios[i]).abs() < 0.001);
                        assert!(c.right() <= width + 0.01);
                        if i > row.start {
                            assert!((c.left() - layout.cells[i - 1].right() - GAP).abs() < 0.01);
                        }
                    }
                    if n + 1 < layout.rows.len() {
                        assert!((layout.cells[row.end - 1].right() - width).abs() < 0.01);
                        assert!(
                            (layout.cells[row.end].top() - layout.cells[row.start].bottom() - GAP)
                                .abs()
                                < 0.01
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn large_layout_only_visits_visible_rows_and_navigation_uses_geometry() {
        let mut layout = Layout::default();
        layout.update(1200., 150., (0..50000).map(|i| [0.5, 1.5, 3.][i % 3]));
        assert!(layout.visible(50000., 50800.).len() < 10);
        let next = layout.vertical(0, true);
        assert!(layout.rows[1].contains(&next));
        assert_eq!(layout.vertical(0, false), 0);
        assert_eq!(layout.cells.len(), 50000);
        let before = layout.cells.clone();
        layout.update(1200., 150., std::iter::empty());
        assert_eq!(
            layout.cells, before,
            "Unchanged layout must reuse cached geometry"
        );
    }
    #[test]
    fn empty_single_and_invalid_ratios_are_safe() {
        for ratios in [vec![], vec![0.5], vec![f32::NAN, 0., 100., 0.01]] {
            let mut layout = Layout::default();
            layout.update(800., 150., ratios.into_iter());
            assert!(layout.height.is_finite());
            for c in layout.cells {
                assert!(c.is_finite() && c.width() > 0. && c.height() > 0.);
            }
        }
    }
}
