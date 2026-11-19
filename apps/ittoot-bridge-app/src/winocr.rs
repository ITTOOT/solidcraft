//! Windows' own OCR engine (Windows.Media.Ocr), as the `Ocr` the IO builder reads drawings with.
//! It runs on the machine, offline, with the languages the user has installed.

use ittoot_io::ocr::{Ocr, Word};
use windows::Graphics::Imaging::BitmapDecoder;
use windows::Media::Ocr::{OcrEngine, OcrResult};
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

pub struct WinOcr {
    engine: OcrEngine,
}

impl WinOcr {
    /// Create it on the thread that uses it; Windows starts the thread's COM apartment itself.
    pub fn new() -> Result<WinOcr, String> {
        let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|e| format!("Windows OCR is not available: {e}"))?;
        Ok(WinOcr { engine })
    }

    fn recognise(&self, bmp: &[u8]) -> Result<OcrResult, String> {
        let e = |what: &str, err: windows::core::Error| format!("{what}: {err}");
        let stream = InMemoryRandomAccessStream::new().map_err(|x| e("stream", x))?;
        let writer = DataWriter::CreateDataWriter(&stream).map_err(|x| e("writer", x))?;
        writer.WriteBytes(bmp).map_err(|x| e("write", x))?;
        writer.StoreAsync().map_err(|x| e("store", x))?.join().map_err(|x| e("store", x))?;
        writer.DetachStream().map_err(|x| e("detach", x))?;
        stream.Seek(0).map_err(|x| e("seek", x))?;
        let decoder = BitmapDecoder::CreateAsync(&stream).map_err(|x| e("decoder", x))?.join().map_err(|x| e("decoder", x))?;
        let bitmap = decoder.GetSoftwareBitmapAsync().map_err(|x| e("bitmap", x))?.join().map_err(|x| e("bitmap", x))?;
        self.engine.RecognizeAsync(&bitmap).map_err(|x| e("recognise", x))?.join().map_err(|x| e("recognise", x))
    }
}

impl Ocr for WinOcr {
    fn lines(&self, bmp: &[u8]) -> Result<Vec<String>, String> {
        let r = self.recognise(bmp)?;
        let lines = r.Lines().map_err(|x| x.to_string())?;
        Ok(lines.into_iter().filter_map(|l| l.Text().ok()).map(|t| t.to_string()).collect())
    }

    fn words(&self, bmp: &[u8]) -> Result<Vec<Word>, String> {
        let r = self.recognise(bmp)?;
        let mut out = Vec::new();
        for line in r.Lines().map_err(|x| x.to_string())? {
            let Ok(words) = line.Words() else { continue };
            for w in words {
                let (Ok(t), Ok(b)) = (w.Text(), w.BoundingRect()) else { continue };
                out.push(Word { text: t.to_string(), x: b.X as i32, y: b.Y as i32, w: b.Width as i32, h: b.Height as i32 });
            }
        }
        Ok(out)
    }
}

impl WinOcr {
    /// A quick whole-page read at the page's own size: enough to see whether a sheet shows IO addresses.
    pub fn words_of(&self, bm: &ittoot_io::bitmap::Bitmap) -> Vec<Word> {
        self.words(&bm.crop_bmp(0, 0, bm.w, bm.h, 1, 0)).unwrap_or_default()
    }
}
