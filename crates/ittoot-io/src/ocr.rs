//! Words with boxes, as an OCR engine reports them. The engine itself is the caller's (Windows OCR in the app).

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Word {
    #[serde(rename = "t")]
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Word {
    pub fn cx(&self) -> i32 {
        self.x + self.w / 2
    }
    pub fn cy(&self) -> i32 {
        self.y + self.h / 2
    }
}

/// One line of OCR output (words left to right).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Line {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub words: Vec<Word>,
}

/// Read the JSON an OCR helper writes: a list of lines (or one line), each with words.
pub fn words_from_json(text: &str) -> Vec<Word> {
    let text = text.trim_start_matches('\u{feff}');
    if let Ok(lines) = serde_json::from_str::<Vec<Line>>(text) {
        return lines.into_iter().flat_map(|l| l.words).collect();
    }
    if let Ok(line) = serde_json::from_str::<Line>(text) {
        return line.words;
    }
    Vec::new()
}

/// Anything that turns an image (a BMP) into text: lines for a small crop, words with boxes for a page.
pub trait Ocr {
    fn lines(&self, bmp: &[u8]) -> Result<Vec<String>, String>;
    fn words(&self, bmp: &[u8]) -> Result<Vec<Word>, String>;
}
