//! The IO builder's slow work: reading drawings (page images, OCR) off the window thread.

use ittoot_io::model::IoPoint;
use ittoot_io::{pdf, table};
use std::path::Path;

pub struct TableRead {
    pub points: Vec<IoPoint>,
    pub notes: Vec<String>,
}

/// Read an IO table PDF: every page with a table grid. `pages` limits to a 1-based inclusive range.
#[cfg(windows)]
pub fn read_table_pdf(path: &Path, pages: Option<(u32, u32)>, mut progress: impl FnMut(u32, u32)) -> Result<TableRead, String> {
    let ocr = crate::winocr::WinOcr::new()?;
    let imgs = pdf::page_images(path)?;
    let total = imgs.len() as u32;
    let doc = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (mut reads, mut notes) = (Vec::new(), Vec::new());
    let mut vocab = table::Vocab::default();
    for img in imgs {
        if let Some((a, b)) = pages
            && (img.page < a || img.page > b)
        {
            continue;
        }
        progress(img.page, total);
        let Some(bm) = &img.bitmap else {
            notes.push(format!("page {}: {}", img.page, img.note));
            continue;
        };
        let r = table::read_page(bm, &ocr);
        notes.extend(r.notes.iter().map(|n| format!("page {}: {n}", img.page)));
        vocab.add(&r);
        reads.push((img.page, r));
    }
    // choose each cell's reading once the whole document's vocabulary is known
    let mut points = Vec::new();
    for (page, r) in &mut reads {
        r.resolve(&vocab);
        points.extend(table::points_from(r, &doc, *page));
    }
    Ok(TableRead { points, notes })
}

#[cfg(not(windows))]
pub fn read_table_pdf(_path: &Path, _pages: Option<(u32, u32)>, _progress: impl FnMut(u32, u32)) -> Result<TableRead, String> {
    Err("reading scanned drawings needs Windows OCR, which this build does not have".into())
}

/// Everything the IO builder reads from: an IO table, a circuit diagram, or both.
#[derive(Clone, Default)]
pub struct BuildInput {
    pub table_pdf: String,
    pub drawing_pdf: String,
    /// 1-based inclusive page range of the drawing to read; `None` is every page.
    pub drawing_pages: Option<(u32, u32)>,
    /// Read every drawing page in depth, not only the ones where a quick pass saw addresses.
    pub deep: bool,
}

pub struct BuildOutput {
    pub points: Vec<IoPoint>,
    pub notes: Vec<String>,
    pub sheets_read: usize,
    pub sheets_with_addresses: usize,
}

/// Read the table, then the drawing, and check each against the other.
#[cfg(windows)]
pub fn build(input: &BuildInput, say: impl Fn(String)) -> Result<BuildOutput, String> {
    use ittoot_io::model::IoKind;
    use ittoot_io::{sheet, xref};
    use std::collections::HashSet;
    let mut notes: Vec<String> = Vec::new();
    let mut points: Vec<IoPoint> = Vec::new();
    if !input.table_pdf.trim().is_empty() {
        say("reading the IO table".into());
        let r = read_table_pdf(Path::new(input.table_pdf.trim()), None, |p, t| say(format!("IO table: page {p} of {t}")))?;
        points = r.points;
        notes.extend(r.notes);
    }
    let known: HashSet<(IoKind, u32, u8)> = points.iter().map(|p| (p.address.kind, p.address.byte, p.address.bit)).collect();
    let (mut sheets, mut read, mut with) = (Vec::new(), 0usize, 0usize);
    if !input.drawing_pdf.trim().is_empty() {
        let ocr = crate::winocr::WinOcr::new()?;
        let imgs = pdf::page_images(Path::new(input.drawing_pdf.trim()))?;
        let total = imgs.len();
        let doc = Path::new(input.drawing_pdf.trim()).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        for img in imgs {
            if let Some((a, b)) = input.drawing_pages
                && (img.page < a || img.page > b)
            {
                continue;
            }
            say(format!("drawing: page {} of {total}", img.page));
            let Some(bm) = &img.bitmap else { continue };
            read += 1;
            // a quick look first: does the sheet show IO addresses at all?
            if !input.deep {
                let quick = ocr.words_of(bm);
                if !quick.iter().any(|w| sheet::looks_like_address(&w.text)) {
                    continue;
                }
            }
            let words = sheet::page_words(bm, &ocr);
            let sh = sheet::read_sheet(img.page, &words, bm.w as i32, bm.h as i32, &known);
            if !sh.mentions.is_empty() {
                with += 1;
                sheets.push(sh);
            }
        }
        say("checking the table against the drawing".into());
        xref::against_drawing(&mut points, &sheets, &doc);
    }
    xref::within_table(&mut points);
    points.sort_by_key(|p| (p.station.clone(), !p.address.kind.is_input(), p.address.kind.is_analog(), p.address.byte, p.address.bit));
    Ok(BuildOutput { points, notes, sheets_read: read, sheets_with_addresses: with })
}

#[cfg(not(windows))]
pub fn build(_input: &BuildInput, _say: impl Fn(String)) -> Result<BuildOutput, String> {
    Err("reading scanned drawings needs Windows OCR, which this build does not have".into())
}

/// `ittoot-bridge --io-build --table T.pdf --drawing D.pdf [--pages 51-83] [--deep] [--out list.csv]`.
pub fn cli_build(args: &[String]) {
    let get = |n: &str| args.iter().position(|a| a == n).and_then(|i| args.get(i + 1)).cloned();
    let range = get("--pages").and_then(|p| {
        let (a, b) = p.split_once('-').unwrap_or((&p, &p));
        Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
    });
    let input = BuildInput { table_pdf: get("--table").unwrap_or_default(), drawing_pdf: get("--drawing").unwrap_or_default(), drawing_pages: range, deep: args.iter().any(|a| a == "--deep") };
    match build(&input, |m| eprintln!("{m}")) {
        Ok(mut r) => {
            if let Some(edits) = get("--edits").and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str::<Vec<ittoot_io::Edit>>(&t).ok()) {
                println!("{} corrections applied", ittoot_io::apply_edits(&mut r.points, &edits));
            }
            // the sources of every channel stay in the JSON; the list is sorted station, inputs first
            r.points.sort_by_key(|p| (p.station.clone(), !p.address.kind.is_input(), p.address.kind.is_analog(), p.address.byte, p.address.bit));
            ittoot_io::naming::apply(&mut r.points, &ittoot_io::naming::NameOptions::default());
            if let Some(f) = get("--json") {
                let _ = std::fs::write(f, serde_json::to_string_pretty(&r.points).unwrap_or_default());
            }
            for n in &r.notes {
                println!("note: {n}");
            }
            for p in &r.points {
                println!("{:<14} {:<8} {:<12} {:<26} | {:<30} | {}", p.station, p.address.siemens(), p.status.label(), p.final_name(), p.description, p.drawing_label);
            }
            println!("{} points; {} drawing sheets read, {} with addresses", r.points.len(), r.sheets_read, r.sheets_with_addresses);
            if let Some(out) = get("--out") {
                let e = ittoot_io::export::render(ittoot_io::export::Format::GenericCsv, &r.points, "IO");
                let _ = std::fs::write(out, e.text);
            }
        }
        Err(e) => println!("failed: {e}"),
    }
}

/// `ittoot-bridge --io-read table.pdf [--pages 2-3]`: print what the IO table reader finds.
pub fn cli(pdf: &str, pages: Option<String>) {
    let range = pages.and_then(|p| {
        let (a, b) = p.split_once('-').unwrap_or((&p, &p));
        Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
    });
    match read_table_pdf(Path::new(pdf), range, |p, t| eprintln!("page {p} of {t}")) {
        Ok(r) => {
            for n in &r.notes {
                println!("note: {n}");
            }
            for p in &r.points {
                println!("{:<14} {:<8} {:<9} t{} | {}", p.station, p.address.siemens(), p.module, p.terminal, p.description);
            }
            println!("{} points", r.points.len());
        }
        Err(e) => println!("failed: {e}"),
    }
}

/// `--io-grid table.pdf PAGE`: print the tables the rule detector finds on a page.
pub fn grid_debug(pdf_path: &str, page: u32) {
    let Ok(imgs) = pdf::page_images(Path::new(pdf_path)) else { return };
    let Some(bm) = imgs.iter().find(|i| i.page == page).and_then(|i| i.bitmap.as_ref()) else { return };
    let h = ittoot_io::grid::horizontal_lines(bm, 150);
    let v = ittoot_io::grid::vertical_lines(bm, 60);
    println!("page {}x{}: {} horizontal, {} vertical rules", bm.w, bm.h, h.len(), v.len());
    for l in h.iter().filter(|l| l.len() > 400) {
        println!("  h y={:.0} slope={:.4} x={}..{}", l.y, l.slope, l.x0, l.x1);
    }
    for l in v.iter().filter(|l| l.len() > 300) {
        println!("  v x={:.0} slope={:.4} y={}..{}", l.x, l.slope, l.y0, l.y1);
    }
    for t in ittoot_io::grid::find_tables(&h, &v, 400, 9) {
        println!("table: {} columns, {} rules, x {:?}", t.xs.len().saturating_sub(1), t.rules.len(), t.xs);
        println!("  rows down the first module column: {:?}", t.edges(t.xs.get(2).copied().unwrap_or(0), t.xs.get(3).copied().unwrap_or(0)));
    }
}

/// `ittoot-bridge --io-export points.json --out folder`: write the list in every format.
pub fn cli_export(file: &str, out: &str) {
    let Ok(points) = std::fs::read_to_string(file).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Vec<IoPoint>>(&t).map_err(|e| e.to_string())) else {
        println!("could not read {file}");
        return;
    };
    let _ = std::fs::create_dir_all(out);
    for f in ittoot_io::export::Format::ALL {
        let e = ittoot_io::export::render(f, &points, "IO_Physical");
        let name = format!("{out}/io_{}.{}", format!("{f:?}").to_ascii_lowercase(), f.extension());
        let _ = std::fs::write(&name, &e.text);
        println!("{name}: {} channels", e.count);
        for n in e.notes {
            println!("  ! {n}");
        }
    }
}
