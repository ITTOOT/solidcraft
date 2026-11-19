//! Table rules found in a page image, and the tables they make.
//!
//! Scans are rarely square: a rule that is level on the paper runs a few pixels downhill across the
//! page. Horizontal rules are therefore looked for at a small range of slopes, and each carries its own.

use crate::bitmap::Bitmap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HLine {
    /// Row where the rule crosses the middle of the page.
    pub y: f32,
    /// Rows gained per column to the right.
    pub slope: f32,
    pub x0: usize,
    pub x1: usize,
    pub mid: usize,
}

impl HLine {
    pub fn y_at(&self, x: usize) -> f32 {
        self.y + self.slope * (x as f32 - self.mid as f32)
    }
    pub fn len(&self) -> usize {
        self.x1.saturating_sub(self.x0)
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VLine {
    /// Column where the rule crosses the middle of the page.
    pub x: f32,
    /// Columns gained per row down.
    pub slope: f32,
    pub y0: usize,
    pub y1: usize,
    pub mid: usize,
}

impl VLine {
    pub fn x_at(&self, y: usize) -> f32 {
        self.x + self.slope * (y as f32 - self.mid as f32)
    }
    pub fn len(&self) -> usize {
        self.y1.saturating_sub(self.y0)
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Runs of ink at least `min` long, with `gap` pixels of tolerance: (start, end).
fn runs(from: usize, to: usize, gap: usize, min: usize, ink: impl Fn(usize) -> bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let (mut start, mut last, mut open) = (0usize, 0usize, false);
    for i in from..to {
        if ink(i) {
            if !open {
                open = true;
                start = i;
            }
            last = i;
        } else if open && i - last > gap {
            if last + 1 - start >= min {
                out.push((start, last + 1));
            }
            open = false;
        }
    }
    if open && last + 1 - start >= min {
        out.push((start, last + 1));
    }
    out
}

/// Horizontal rules at least `min_len` long, at any slope within about a third of a degree.
pub fn horizontal_lines(bm: &Bitmap, min_len: usize) -> Vec<HLine> {
    let mid = bm.w / 2;
    let mut found: Vec<HLine> = Vec::new();
    for k in -12i32..=12 {
        let slope = k as f32 * 0.0005;
        // rows where this slope's rules start and end, merged when they are the rows of one thick rule
        let mut raw: Vec<HLine> = Vec::new();
        for y in 12..bm.h.saturating_sub(12) {
            let ink = |x: usize| {
                let yy = y as f32 + slope * (x as f32 - mid as f32);
                yy >= 0.0 && (bm.black(x, yy as usize) || bm.black(x, yy as usize + 1))
            };
            for (a, b) in runs(0, bm.w, 3, min_len, ink) {
                raw.push(HLine { y: y as f32, slope, x0: a, x1: b, mid });
            }
        }
        let mut group: Vec<HLine> = Vec::new();
        for l in raw {
            let joins = group.last().is_some_and(|g| l.y <= g.y + 2.5 && l.x0.abs_diff(g.x0) < 40 && l.x1.abs_diff(g.x1) < 40);
            if !joins && !group.is_empty() {
                found.push(centre(&group));
                group.clear();
            }
            group.push(l);
        }
        if !group.is_empty() {
            found.push(centre(&group));
        }
    }
    // the same rule shows at neighbouring slopes: keep the longest of each
    found.sort_by_key(|l| std::cmp::Reverse(l.len()));
    let mut kept: Vec<HLine> = Vec::new();
    for l in found {
        let dup = kept.iter().any(|k| {
            let (a, b) = (l.x0.max(k.x0), l.x1.min(k.x1));
            b > a && (b - a) * 2 >= l.len() && (l.y_at((a + b) / 2) - k.y_at((a + b) / 2)).abs() < 6.0
        });
        if !dup {
            kept.push(l);
        }
    }
    kept.sort_by(|a, b| a.y.total_cmp(&b.y));
    kept
}

fn centre(g: &[HLine]) -> HLine {
    let n = g.len().max(1) as f32;
    HLine {
        y: g.iter().map(|l| l.y).sum::<f32>() / n,
        slope: g.first().map_or(0.0, |l| l.slope),
        x0: g.iter().map(|l| l.x0).min().unwrap_or(0),
        x1: g.iter().map(|l| l.x1).max().unwrap_or(0),
        mid: g.first().map_or(0, |l| l.mid),
    }
}

/// Vertical rules at least `min_len` long, at any slope within about a third of a degree.
pub fn vertical_lines(bm: &Bitmap, min_len: usize) -> Vec<VLine> {
    let mid = bm.h / 2;
    let mut found: Vec<VLine> = Vec::new();
    for k in -12i32..=12 {
        let slope = k as f32 * 0.0005;
        let mut raw: Vec<VLine> = Vec::new();
        for x in 12..bm.w.saturating_sub(12) {
            let ink = |y: usize| {
                let xx = x as f32 + slope * (y as f32 - mid as f32);
                xx >= 0.0 && (bm.black(xx as usize, y) || bm.black(xx as usize + 1, y))
            };
            for (a, b) in runs(0, bm.h, 3, min_len, ink) {
                raw.push(VLine { x: x as f32, slope, y0: a, y1: b, mid });
            }
        }
        let mut group: Vec<VLine> = Vec::new();
        for l in raw {
            let joins = group.last().is_some_and(|g| l.x <= g.x + 2.5 && l.y0.abs_diff(g.y0) < 40 && l.y1.abs_diff(g.y1) < 40);
            if !joins && !group.is_empty() {
                found.push(centre_v(&group));
                group.clear();
            }
            group.push(l);
        }
        if !group.is_empty() {
            found.push(centre_v(&group));
        }
    }
    found.sort_by_key(|l| std::cmp::Reverse(l.len()));
    let mut kept: Vec<VLine> = Vec::new();
    for l in found {
        let dup = kept.iter().any(|k| {
            let (a, b) = (l.y0.max(k.y0), l.y1.min(k.y1));
            b > a && (b - a) * 2 >= l.len() && (l.x_at((a + b) / 2) - k.x_at((a + b) / 2)).abs() < 6.0
        });
        if !dup {
            kept.push(l);
        }
    }
    kept.sort_by(|a, b| a.x.total_cmp(&b.x));
    kept
}

fn centre_v(g: &[VLine]) -> VLine {
    let n = g.len().max(1) as f32;
    VLine {
        x: g.iter().map(|l| l.x).sum::<f32>() / n,
        slope: g.first().map_or(0.0, |l| l.slope),
        y0: g.iter().map(|l| l.y0).min().unwrap_or(0),
        y1: g.iter().map(|l| l.y1).max().unwrap_or(0),
        mid: g.first().map_or(0, |l| l.mid),
    }
}

/// One table: its column rules and the row rules across it.
#[derive(Clone, Debug)]
pub struct TableGrid {
    pub xs: Vec<usize>,
    pub rules: Vec<HLine>,
}

impl TableGrid {
    /// Row boundaries down the column between `x0` and `x1`. Every rule of the table counts, each followed along its
    /// own slope even where it was broken in the scan, so a missing stretch of rule cannot shift the rows.
    pub fn edges(&self, x0: usize, x1: usize) -> Vec<usize> {
        let xc = (x0 + x1) / 2;
        let mut ys: Vec<f32> = self.rules.iter().map(|r| r.y_at(xc).max(0.0)).collect();
        ys.sort_by(f32::total_cmp);
        ys.dedup_by(|a, b| (*a - *b).abs() < 10.0);
        ys.into_iter().map(|y| y as usize).collect()
    }
}

/// Tables on a page: groups of long horizontal rules that start and end together, with the vertical rules that span them.
pub fn find_tables(h: &[HLine], v: &[VLine], min_width: usize, min_rows: usize) -> Vec<TableGrid> {
    let mut groups: Vec<Vec<HLine>> = Vec::new();
    for l in h.iter().filter(|l| l.len() >= min_width) {
        match groups.iter_mut().find(|g| g.first().is_some_and(|f| f.x1.abs_diff(l.x1) < 30)) {
            Some(g) => g.push(*l),
            None => groups.push(vec![*l]),
        }
    }
    let mut out = Vec::new();
    for mut g in groups {
        g.sort_by(|a, b| a.y.total_cmp(&b.y));
        g.dedup_by(|a, b| (a.y - b.y).abs() < 8.0);
        if g.len() < min_rows {
            continue;
        }
        let (top, bottom) = (g.first().map_or(0.0, |l| l.y), g.last().map_or(0.0, |l| l.y));
        let x0 = g.iter().map(|l| l.x0).min().unwrap_or(0);
        let x1 = g.iter().map(|l| l.x1).max().unwrap_or(0);
        let ymid = ((top + bottom) / 2.0) as usize;
        let mut xs: Vec<usize> = v
            .iter()
            .filter(|l| (l.y0 as f32) <= top + 120.0 && (l.y1 as f32) + 120.0 >= bottom)
            .map(|l| l.x_at(ymid).max(0.0) as usize)
            .filter(|x| *x + 25 >= x0 && *x <= x1 + 25)
            .collect();
        xs.sort_unstable();
        xs.dedup_by(|a, b| a.abs_diff(*b) < 12);
        if xs.len() >= 2 {
            out.push(TableGrid { xs, rules: g });
        }
    }
    out.sort_by(|a, b| a.rules.first().map_or(0.0, |l| l.y).total_cmp(&b.rules.first().map_or(0.0, |l| l.y)).then(a.xs.first().cmp(&b.xs.first())));
    out
}
