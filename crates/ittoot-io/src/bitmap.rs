//! A 1-bit page image, decoded from the scan's CCITT Group 4 data.

pub struct Bitmap {
    pub w: usize,
    pub h: usize,
    bits: Vec<u8>,
}

impl Bitmap {
    pub fn new(w: usize, h: usize) -> Self {
        Bitmap { w, h, bits: vec![0; w.saturating_mul(h)] }
    }

    /// True for ink.
    pub fn black(&self, x: usize, y: usize) -> bool {
        x < self.w && y < self.h && self.bits.get(y * self.w + x).is_some_and(|b| *b != 0)
    }

    pub fn set(&mut self, x: usize, y: usize, ink: bool) {
        if x < self.w
            && y < self.h
            && let Some(b) = self.bits.get_mut(y * self.w + x)
        {
            *b = u8::from(ink);
        }
    }

    /// Decode CCITT Group 4 data: the decoder's black is ink.
    pub fn from_g4(data: &[u8], w: usize, h: usize) -> Option<Bitmap> {
        let width = u16::try_from(w).ok()?;
        let mut bm = Bitmap::new(w, h);
        let mut y = 0usize;
        fax::decoder::decode_g4(data.iter().copied(), width, u16::try_from(h).ok(), |line| {
            if y < h {
                for (x, c) in fax::decoder::pels(line, width).enumerate() {
                    bm.set(x, y, c == fax::Color::Black);
                }
            }
            y += 1;
        })?;
        Some(bm)
    }

    /// Longest run of ink in row `y` from `x0` to `x1`, allowing `gap` white pixels inside a run: (length, start).
    pub fn longest_row_run(&self, y: usize, x0: usize, x1: usize, gap: usize) -> (usize, usize) {
        longest(x0, x1.min(self.w), gap, |x| self.black(x, y))
    }

    /// The same down a column: (length, start).
    pub fn longest_col_run(&self, x: usize, y0: usize, y1: usize, gap: usize) -> (usize, usize) {
        longest(y0, y1.min(self.h), gap, |y| self.black(x, y))
    }

    /// A grey-scale BMP of one rectangle, scaled up with smoothing and padded with white, for an OCR engine.
    pub fn crop_bmp(&self, x0: usize, y0: usize, x1: usize, y1: usize, scale: usize, pad: usize) -> Vec<u8> {
        self.crop_bmp_masked(x0, y0, x1, y1, scale, pad, 0, 0)
    }

    /// The same with the top-left corner `mask_w` by `mask_h` blanked (where a table cell prints its terminal number).
    #[allow(clippy::too_many_arguments)]
    pub fn crop_bmp_masked(&self, x0: usize, y0: usize, x1: usize, y1: usize, scale: usize, pad: usize, mask_w: usize, mask_h: usize) -> Vec<u8> {
        let (cw, ch) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
        let (w, h) = (cw * scale + 2 * pad, ch * scale + 2 * pad);
        let row = (w * 3).div_ceil(4) * 4;
        let size = 54 + row * h;
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(size as u32).to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(w as i32).to_le_bytes());
        out.extend_from_slice(&(h as i32).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        out.extend_from_slice(&[0; 24]);
        let ink = |x: i64, y: i64| -> f32 {
            if x < x0 as i64 || y < y0 as i64 || x >= x1 as i64 || y >= y1 as i64 || (x < (x0 + mask_w) as i64 && y < (y0 + mask_h) as i64) || !self.black(x as usize, y as usize) { 0.0 } else { 1.0 }
        };
        for oy in (0..h).rev() {
            for ox in 0..w {
                let v = if ox < pad || oy < pad || ox >= w - pad || oy >= h - pad {
                    255u8
                } else {
                    // bilinear between source pixel centres
                    let fx = (ox - pad) as f32 / scale as f32 - 0.5;
                    let fy = (oy - pad) as f32 / scale as f32 - 0.5;
                    let (ix, iy) = (fx.floor(), fy.floor());
                    let (tx, ty) = (fx - ix, fy - iy);
                    let (bx, by) = (x0 as i64 + ix as i64, y0 as i64 + iy as i64);
                    let a = ink(bx, by) * (1.0 - tx) + ink(bx + 1, by) * tx;
                    let b = ink(bx, by + 1) * (1.0 - tx) + ink(bx + 1, by + 1) * tx;
                    (255.0 * (1.0 - (a * (1.0 - ty) + b * ty))) as u8
                };
                out.extend_from_slice(&[v, v, v]);
            }
            out.resize(out.len() + (row - w * 3), 0);
        }
        out
    }

    /// A 24-bit BMP of the whole page, for an OCR engine that wants an image.
    pub fn to_bmp(&self) -> Vec<u8> {
        let row = (self.w * 3).div_ceil(4) * 4;
        let size = 54 + row * self.h;
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(size as u32).to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(self.w as i32).to_le_bytes());
        out.extend_from_slice(&(self.h as i32).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        out.extend_from_slice(&[0; 24]);
        for y in (0..self.h).rev() {
            for x in 0..self.w {
                let v = if self.black(x, y) { 0u8 } else { 255u8 };
                out.extend_from_slice(&[v, v, v]);
            }
            out.resize(out.len() + (row - self.w * 3), 0);
        }
        out
    }
}

fn longest(from: usize, to: usize, gap: usize, ink: impl Fn(usize) -> bool) -> (usize, usize) {
    let (mut best, mut best_at, mut start, mut last, mut open) = (0usize, 0usize, 0usize, 0usize, false);
    for i in from..to {
        if ink(i) {
            if !open {
                open = true;
                start = i;
            }
            last = i;
        } else if open && i - last > gap {
            if last + 1 - start > best {
                best = last + 1 - start;
                best_at = start;
            }
            open = false;
        }
    }
    if open && last + 1 - start > best {
        best = last + 1 - start;
        best_at = start;
    }
    (best, best_at)
}
