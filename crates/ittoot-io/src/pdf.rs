//! Page images of a scanned PDF (one CCITT Group 4 image per page, as scanners write them).

use crate::bitmap::Bitmap;
use lopdf::{Dictionary, Document, Object};
use std::path::Path;

pub struct PageImage {
    pub page: u32,
    pub bitmap: Option<Bitmap>,
    /// Why there is no bitmap (a page with no scan, or an image type not handled).
    pub note: String,
}

fn int(d: &Dictionary, k: &[u8]) -> Option<i64> {
    d.get(k).ok().and_then(|o| o.as_i64().ok())
}

fn resolve(doc: &Document, o: &Object) -> Option<Dictionary> {
    match o {
        Object::Dictionary(d) => Some(d.clone()),
        Object::Reference(id) => doc.get_dictionary(*id).ok().cloned(),
        _ => None,
    }
}

/// All pages, in order. A page with no scanned image comes back with a note.
pub fn page_images(path: &Path) -> Result<Vec<PageImage>, String> {
    let doc = Document::load(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = Vec::new();
    for (num, id) in doc.get_pages() {
        let mut found: Option<Bitmap> = None;
        let mut note = String::from("no scanned image on this page");
        if let Ok(page) = doc.get_dictionary(id)
            && let Some(res) = page.get(b"Resources").ok().and_then(|r| resolve(&doc, r))
            && let Some(xo) = res.get(b"XObject").ok().and_then(|r| resolve(&doc, r))
        {
            for (_, v) in xo.iter() {
                let Ok(oid) = v.as_reference() else { continue };
                let Ok(Object::Stream(s)) = doc.get_object(oid) else { continue };
                if s.dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) != Some(b"Image") {
                    continue;
                }
                let (Some(w), Some(h)) = (int(&s.dict, b"Width"), int(&s.dict, b"Height")) else { continue };
                let filter = s.dict.get(b"Filter").ok().and_then(|o| match o {
                    Object::Name(n) => Some(n.clone()),
                    Object::Array(a) => a.first().and_then(|x| x.as_name().ok().map(<[u8]>::to_vec)),
                    _ => None,
                });
                if filter.as_deref() != Some(b"CCITTFaxDecode") {
                    note = format!("image filter {} is not handled", filter.map(|f| String::from_utf8_lossy(&f).into_owned()).unwrap_or_default());
                    continue;
                }
                let parms = s.dict.get(b"DecodeParms").ok().and_then(|o| resolve(&doc, o).or_else(|| o.as_array().ok().and_then(|a| a.first()).and_then(|x| resolve(&doc, x))));
                let k = parms.as_ref().and_then(|p| int(p, b"K")).unwrap_or(0);
                if k >= 0 {
                    note = "only CCITT Group 4 scans are handled".into();
                    continue;
                }
                if let (Ok(w), Ok(h)) = (usize::try_from(w), usize::try_from(h))
                    && let Some(bm) = Bitmap::from_g4(&s.content, w, h)
                {
                    found = Some(bm);
                    break;
                }
                note = "the scan could not be decoded".into();
            }
        }
        out.push(PageImage { page: num, bitmap: found, note });
    }
    Ok(out)
}
