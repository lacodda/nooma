//! Read the text in a picture or on a PDF's pages and print it.
//!
//!     cargo run -p lacodda-ocr --example read -- receipt.jpg
//!     cargo run -p lacodda-ocr --example read -- scan.pdf --page 1 --page 2 --lang ru
//!
//! With no `--lang`, the languages the person has chosen in Windows; with no
//! `--page`, a PDF's first page.

use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: read <picture or PDF> [--page N]... [--lang TAG]...")?;
    let (mut languages, mut pages) = (Vec::new(), Vec::new());
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--lang" => languages.push(args.next().ok_or("--lang needs a tag")?),
            "--page" => pages.push(args.next().ok_or("--page needs a number")?.parse::<u32>()?.saturating_sub(1)),
            other => return Err(format!("unexpected argument {other}").into()),
        }
    }
    println!("installed: {}", lacodda_ocr::installed()?.join(", "));
    let ocr = if languages.is_empty() {
        lacodda_ocr::Ocr::new()?
    } else {
        lacodda_ocr::Ocr::with_languages(&languages.iter().map(String::as_str).collect::<Vec<_>>())?
    };
    println!("reader: {}", ocr.reader());

    let bytes = std::fs::read(&path)?;
    let started = Instant::now();
    let texts = if bytes.starts_with(b"%PDF-") {
        if pages.is_empty() {
            pages.push(0);
        }
        ocr.read_pdf_pages(&bytes, &pages)?
    } else {
        vec![ocr.read(&bytes)?]
    };
    for text in &texts {
        println!("--- {} · angle {:?} · {} lines", text.language, text.angle, text.lines.len());
        println!("{}", text.plain());
    }
    println!("--- {} ms", started.elapsed().as_millis());
    Ok(())
}
