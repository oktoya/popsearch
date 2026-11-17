//! Local text extraction (Markdown) from docx / xlsx / pptx / hwpx files.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

type Zip<'a> = zip::ZipArchive<Cursor<&'a [u8]>>;

const MAX_ENTRY_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ROWS: usize = 1000;

enum Ev {
    /// (local name, attributes with local keys)
    Start(String, Vec<(String, String)>),
    End(String),
    Text(String),
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn start_name(e: &BytesStart) -> String {
    local(&String::from_utf8_lossy(e.name().as_ref())).to_string()
}

fn start_attrs(e: &BytesStart) -> Vec<(String, String)> {
    e.attributes()
        .filter_map(|a| a.ok())
        .filter_map(|a| {
            let key = String::from_utf8_lossy(a.key.as_ref()).to_string();
            let val = a.unescape_value().ok()?.into_owned();
            Some((local(&key).to_string(), val))
        })
        .collect()
}

fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn parse_xml(xml: &str) -> Option<Vec<Ev>> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) => out.push(Ev::Start(start_name(&e), start_attrs(&e))),
            Event::Empty(e) => {
                let n = start_name(&e);
                out.push(Ev::Start(n.clone(), start_attrs(&e)));
                out.push(Ev::End(n));
            }
            Event::End(e) => out.push(Ev::End(
                local(&String::from_utf8_lossy(e.name().as_ref())).to_string(),
            )),
            Event::Text(t) => out.push(Ev::Text(t.unescape().ok()?.into_owned())),
            Event::CData(c) => out.push(Ev::Text(
                String::from_utf8_lossy(&c.into_inner()).into_owned(),
            )),
            Event::Eof => break,
            _ => {}
        }
    }
    Some(out)
}

fn read_entry(zip: &mut Zip, name: &str) -> Option<String> {
    let file = zip.by_name(name).ok()?;
    let mut buf = Vec::new();
    file.take(MAX_ENTRY_BYTES).read_to_end(&mut buf).ok()?;
    let s = String::from_utf8(buf).ok()?;
    Some(s.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(s))
}

fn entry_names(zip: &Zip) -> Vec<String> {
    zip.file_names().map(|s| s.to_string()).collect()
}

fn md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn md_table(rows: &[Vec<String>]) -> String {
    let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if ncols == 0 {
        return String::new();
    }
    let line = |r: &Vec<String>| {
        let cells: Vec<String> = (0..ncols)
            .map(|i| md_cell(r.get(i).map(|s| s.as_str()).unwrap_or("")))
            .collect();
        format!("| {} |", cells.join(" | "))
    };
    let mut out = vec![line(&rows[0])];
    out.push(format!("| {} |", vec!["---"; ncols].join(" | ")));
    for r in &rows[1..] {
        out.push(line(r));
    }
    out.join("\n")
}

// ---------- docx ----------

fn heading_level(style: &str) -> Option<usize> {
    let lower = style.to_lowercase();
    let rest = lower.strip_prefix("heading")?.trim();
    let n: usize = rest.parse().ok()?;
    if (1..=6).contains(&n) { Some(n) } else { None }
}

#[derive(Default)]
struct Para {
    text: String,
    heading: Option<usize>,
    list: bool,
}

fn docx_text(xml: &str) -> Option<String> {
    let evs = parse_xml(xml)?;
    let mut blocks: Vec<String> = Vec::new();
    let mut tbl_depth = 0usize;
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell: Vec<String> = Vec::new();
    let mut para: Option<Para> = None;
    let mut in_t = false;
    let mut in_run = 0usize;
    for ev in evs {
        match ev {
            Ev::Start(n, attrs) => match n.as_str() {
                "tbl" => {
                    tbl_depth += 1;
                    if tbl_depth == 1 {
                        rows.clear();
                        row.clear();
                        cell.clear();
                    }
                }
                "p" => para = Some(Para::default()),
                "pStyle" => {
                    if let (Some(p), Some(v)) = (para.as_mut(), attr(&attrs, "val")) {
                        p.heading = heading_level(v);
                    }
                }
                "numPr" => {
                    if let Some(p) = para.as_mut() {
                        p.list = true;
                    }
                }
                "r" => in_run += 1,
                "t" => in_t = true,
                "tab" if in_run > 0 => {
                    if let Some(p) = para.as_mut() {
                        p.text.push('\t');
                    }
                }
                "br" | "cr" if in_run > 0 => {
                    if let Some(p) = para.as_mut() {
                        p.text.push('\n');
                    }
                }
                _ => {}
            },
            Ev::Text(t) => {
                if in_t {
                    if let Some(p) = para.as_mut() {
                        p.text.push_str(&t);
                    }
                }
            }
            Ev::End(n) => match n.as_str() {
                "t" => in_t = false,
                "r" => in_run = in_run.saturating_sub(1),
                "p" => {
                    if let Some(p) = para.take() {
                        let t = p.text.trim().to_string();
                        if tbl_depth > 0 {
                            if !t.is_empty() {
                                cell.push(t.replace('\n', " "));
                            }
                        } else if t.is_empty() {
                            if blocks.last().is_some_and(|l| !l.is_empty()) {
                                blocks.push(String::new());
                            }
                        } else if let Some(h) = p.heading {
                            blocks.push(format!("{} {}", "#".repeat(h), t));
                        } else if p.list {
                            blocks.push(format!("- {}", t));
                        } else {
                            blocks.push(t);
                        }
                    }
                }
                "tc" if tbl_depth == 1 => {
                    row.push(cell.join(" "));
                    cell.clear();
                }
                "tr" if tbl_depth == 1 => rows.push(std::mem::take(&mut row)),
                "tbl" => {
                    tbl_depth = tbl_depth.saturating_sub(1);
                    if tbl_depth == 0 {
                        if blocks.last().is_some_and(|l| !l.is_empty()) {
                            blocks.push(String::new());
                        }
                        let t = md_table(&rows);
                        if !t.is_empty() {
                            blocks.push(t);
                            blocks.push(String::new());
                        }
                        rows.clear();
                    }
                }
                _ => {}
            },
        }
    }
    Some(blocks.join("\n").trim().to_string())
}

fn extract_docx(zip: &mut Zip) -> Option<String> {
    docx_text(&read_entry(zip, "word/document.xml")?)
}

// ---------- xlsx ----------

fn shared_strings(xml: &str) -> Option<Vec<String>> {
    let evs = parse_xml(xml)?;
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_si = false;
    let mut in_t = false;
    let mut in_rph = false;
    for ev in evs {
        match ev {
            Ev::Start(n, _) => match n.as_str() {
                "si" => {
                    in_si = true;
                    cur.clear();
                }
                "rPh" => in_rph = true,
                "t" => in_t = true,
                _ => {}
            },
            Ev::Text(t) => {
                if in_si && in_t && !in_rph {
                    cur.push_str(&t);
                }
            }
            Ev::End(n) => match n.as_str() {
                "si" => {
                    in_si = false;
                    out.push(std::mem::take(&mut cur));
                }
                "rPh" => in_rph = false,
                "t" => in_t = false,
                _ => {}
            },
        }
    }
    Some(out)
}

/// "AB12" -> (row 12, col 28), both 1-based.
fn cell_ref(r: &str) -> Option<(usize, usize)> {
    let letters: String = r.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let digits = &r[letters.len()..];
    if letters.is_empty() {
        return None;
    }
    let row: usize = digits.parse().ok()?;
    let mut col = 0usize;
    for c in letters.chars() {
        col = col * 26 + (c.to_ascii_uppercase() as usize - 'A' as usize + 1);
    }
    if row == 0 { None } else { Some((row, col)) }
}

fn sheet_cells(xml: &str, shared: &[String]) -> Option<BTreeMap<(usize, usize), String>> {
    let evs = parse_xml(xml)?;
    let mut cells = BTreeMap::new();
    let mut pos: Option<(usize, usize)> = None;
    let mut ctype = String::new();
    let mut v = String::new();
    let mut inline = String::new();
    let mut in_c = false;
    let mut in_v = false;
    let mut in_is = false;
    let mut in_t = false;
    for ev in evs {
        match ev {
            Ev::Start(n, attrs) => match n.as_str() {
                "c" => {
                    in_c = true;
                    pos = attr(&attrs, "r").and_then(cell_ref);
                    ctype = attr(&attrs, "t").unwrap_or("").to_string();
                    v.clear();
                    inline.clear();
                }
                "v" if in_c => in_v = true,
                "is" if in_c => in_is = true,
                "t" if in_is => in_t = true,
                _ => {}
            },
            Ev::Text(t) => {
                if in_v {
                    v.push_str(&t);
                } else if in_is && in_t {
                    inline.push_str(&t);
                }
            }
            Ev::End(n) => match n.as_str() {
                "v" => in_v = false,
                "is" => in_is = false,
                "t" => in_t = false,
                "c" => {
                    in_c = false;
                    if let Some(p) = pos.take() {
                        let val = match ctype.as_str() {
                            "s" => v
                                .trim()
                                .parse::<usize>()
                                .ok()
                                .and_then(|i| shared.get(i).cloned())
                                .unwrap_or_default(),
                            "inlineStr" => inline.clone(),
                            _ => v.clone(),
                        };
                        if !val.is_empty() {
                            cells.insert(p, val);
                        }
                    }
                }
                _ => {}
            },
        }
    }
    Some(cells)
}

fn sheet_markdown(cells: &BTreeMap<(usize, usize), String>) -> String {
    if cells.is_empty() {
        return String::new();
    }
    let min_row = cells.keys().map(|k| k.0).min().unwrap_or(1);
    let max_row = cells.keys().map(|k| k.0).max().unwrap_or(1);
    let min_col = cells.keys().map(|k| k.1).min().unwrap_or(1);
    let max_col = cells.keys().map(|k| k.1).max().unwrap_or(1);
    let total = max_row - min_row + 1;
    let shown = total.min(MAX_ROWS);
    let rows: Vec<Vec<String>> = (min_row..min_row + shown)
        .map(|r| {
            (min_col..=max_col)
                .map(|c| cells.get(&(r, c)).cloned().unwrap_or_default())
                .collect()
        })
        .collect();
    let mut out = md_table(&rows);
    if total > shown {
        out.push_str(&format!("\n(… 이하 {}행 생략)", total - shown));
    }
    out
}

fn extract_xlsx(zip: &mut Zip) -> Option<String> {
    let wb = parse_xml(&read_entry(zip, "xl/workbook.xml")?)?;
    let mut sheets: Vec<(String, String)> = Vec::new();
    for ev in wb {
        if let Ev::Start(n, attrs) = ev {
            if n == "sheet" {
                sheets.push((
                    attr(&attrs, "name")?.to_string(),
                    attr(&attrs, "id")?.to_string(),
                ));
            }
        }
    }
    let mut rels: BTreeMap<String, String> = BTreeMap::new();
    for ev in parse_xml(&read_entry(zip, "xl/_rels/workbook.xml.rels")?)? {
        if let Ev::Start(n, attrs) = ev {
            if n == "Relationship" {
                if let (Some(id), Some(target)) = (attr(&attrs, "Id"), attr(&attrs, "Target")) {
                    rels.insert(id.to_string(), target.to_string());
                }
            }
        }
    }
    let shared = match read_entry(zip, "xl/sharedStrings.xml") {
        Some(xml) => shared_strings(&xml)?,
        None => Vec::new(),
    };
    let mut parts: Vec<String> = Vec::new();
    for (name, rid) in sheets {
        let target = rels.get(&rid)?;
        let path = match target.strip_prefix('/') {
            Some(p) => p.to_string(),
            None => format!("xl/{}", target),
        };
        let cells = sheet_cells(&read_entry(zip, &path)?, &shared)?;
        let table = sheet_markdown(&cells);
        if table.is_empty() {
            parts.push(format!("## {}", name));
        } else {
            parts.push(format!("## {}\n\n{}", name, table));
        }
    }
    Some(parts.join("\n\n"))
}

// ---------- pptx / hwpx ----------

/// Returns the text of every paragraph (`para` local name) whose text lives in `text` elements.
fn paragraph_lines(xml: &str, para: &str, text: &str) -> Option<Vec<String>> {
    let evs = parse_xml(xml)?;
    let mut stack: Vec<String> = Vec::new();
    let mut lines = Vec::new();
    let mut in_t = false;
    for ev in evs {
        match ev {
            Ev::Start(n, _) => {
                if n == para {
                    stack.push(String::new());
                } else if n == text {
                    in_t = true;
                }
            }
            Ev::Text(t) => {
                if in_t {
                    if let Some(top) = stack.last_mut() {
                        top.push_str(&t);
                    }
                }
            }
            Ev::End(n) => {
                if n == text {
                    in_t = false;
                } else if n == para {
                    if let Some(s) = stack.pop() {
                        let s = s.trim().to_string();
                        if !s.is_empty() {
                            lines.push(s);
                        }
                    }
                }
            }
        }
    }
    Some(lines)
}

/// Entries named `<prefix><N><suffix>` sorted by N numerically.
fn numbered_entries(zip: &Zip, prefix: &str, suffix: &str) -> Vec<(u64, String)> {
    let mut v: Vec<(u64, String)> = entry_names(zip)
        .into_iter()
        .filter_map(|name| {
            let mid = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
            let n: u64 = mid.parse().ok()?;
            Some((n, name))
        })
        .collect();
    v.sort();
    v
}

fn extract_pptx(zip: &mut Zip) -> Option<String> {
    let slides = numbered_entries(zip, "ppt/slides/slide", ".xml");
    if slides.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    for (n, path) in slides {
        let lines = paragraph_lines(&read_entry(zip, &path)?, "p", "t")?;
        if lines.is_empty() {
            parts.push(format!("## 슬라이드 {}", n));
        } else {
            parts.push(format!("## 슬라이드 {}\n{}", n, lines.join("\n")));
        }
    }
    Some(parts.join("\n\n"))
}

fn extract_hwpx(zip: &mut Zip) -> Option<String> {
    let sections = numbered_entries(zip, "Contents/section", ".xml");
    if sections.is_empty() {
        return None;
    }
    let mut lines = Vec::new();
    for (_, path) in sections {
        lines.extend(paragraph_lines(&read_entry(zip, &path)?, "p", "t")?);
    }
    Some(lines.join("\n"))
}

// ---------- entry points ----------

pub fn extract(name: &str, bytes: &[u8]) -> Result<String, String> {
    let err = || format!("파일을 읽을 수 없습니다: {}", name);
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| err())?;
    let body = match ext.as_str() {
        "docx" => extract_docx(&mut zip),
        "xlsx" => extract_xlsx(&mut zip),
        "pptx" => extract_pptx(&mut zip),
        "hwpx" => extract_hwpx(&mut zip),
        _ => None,
    }
    .ok_or_else(err)?;
    Ok(format!("({}에서 추출한 텍스트)\n\n{}", name, body))
}

pub fn decode_base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b'\r' | b'\n' | b' ' => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn make_zip(files: &[(&str, &str)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, body) in files {
            w.start_file(*name, opts).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn docx_heading_list_table() {
        let xml = r#"<?xml version="1.0"?><w:document xmlns:w="x"><w:body>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Title</w:t></w:r></w:p>
<w:p><w:r><w:t>Hello</w:t></w:r><w:r><w:tab/><w:t>there</w:t></w:r></w:p>
<w:p/>
<w:p/>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>item</w:t></w:r></w:p>
<w:tbl>
<w:tr><w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B|x</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:p><w:r><w:t>1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>2</w:t></w:r></w:p><w:p><w:r><w:t>3</w:t></w:r></w:p></w:tc></w:tr>
</w:tbl>
<w:p><w:r><w:t>end</w:t></w:r></w:p>
</w:body></w:document>"#;
        let bytes = make_zip(&[("word/document.xml", xml)]);
        let out = extract("t.docx", &bytes).unwrap();
        assert_eq!(
            out,
            "(t.docx에서 추출한 텍스트)\n\n# Title\nHello\tthere\n\n- item\n\n| A | B\\|x |\n| --- | --- |\n| 1 | 2 3 |\n\nend"
        );
    }

    #[test]
    fn xlsx_two_sheets() {
        let wb = r#"<workbook xmlns:r="r"><sheets><sheet name="First" sheetId="1" r:id="rId1"/><sheet name="Second" sheetId="2" r:id="rId2"/></sheets></workbook>"#;
        let rels = r#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Target="/xl/worksheets/sheet2.xml"/></Relationships>"#;
        let ss = r#"<sst><si><t>Name</t></si><si><r><t>Va</t></r><r><t>lue</t></r></si></sst>"#;
        let s1 = r#"<worksheet><sheetData>
<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row>
<row r="2"><c r="A2" t="inlineStr"><is><t>foo</t></is></c><c r="B2"><v>42</v></c></row>
</sheetData></worksheet>"#;
        let s2 = r#"<worksheet><sheetData><row r="1"><c r="B1" t="str"><v>x</v></c></row><row r="3"><c r="B3" t="b"><v>1</v></c></row></sheetData></worksheet>"#;
        let bytes = make_zip(&[
            ("xl/workbook.xml", wb),
            ("xl/_rels/workbook.xml.rels", rels),
            ("xl/sharedStrings.xml", ss),
            ("xl/worksheets/sheet1.xml", s1),
            ("xl/worksheets/sheet2.xml", s2),
        ]);
        let out = extract("book.xlsx", &bytes).unwrap();
        assert_eq!(
            out,
            "(book.xlsx에서 추출한 텍스트)\n\n## First\n\n| Name | Value |\n| --- | --- |\n| foo | 42 |\n\n## Second\n\n| x |\n| --- |\n|  |\n| 1 |"
        );
    }

    #[test]
    fn pptx_slide_order() {
        let slide = |t: &str| {
            format!(
                r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#,
                t
            )
        };
        let (s1, s2, s10) = (slide("one"), slide("two"), slide("ten"));
        let bytes = make_zip(&[
            ("ppt/slides/slide10.xml", &s10),
            ("ppt/slides/slide2.xml", &s2),
            ("ppt/slides/slide1.xml", &s1),
            ("ppt/slides/_rels/slide1.xml.rels", "<x/>"),
        ]);
        let out = extract("d.pptx", &bytes).unwrap();
        assert_eq!(
            out,
            "(d.pptx에서 추출한 텍스트)\n\n## 슬라이드 1\none\n\n## 슬라이드 2\ntwo\n\n## 슬라이드 10\nten"
        );
    }

    #[test]
    fn hwpx_two_sections() {
        let sec = |t: &str| {
            format!(
                r#"<hs:sec xmlns:hs="s" xmlns:hp="p"><hp:p><hp:run><hp:t>{}</hp:t></hp:run></hp:p></hs:sec>"#,
                t
            )
        };
        let (a, b) = (sec("first"), sec("second"));
        let bytes = make_zip(&[("Contents/section1.xml", &b), ("Contents/section0.xml", &a)]);
        let out = extract("k.HWPX", &bytes).unwrap();
        assert_eq!(out, "(k.HWPX에서 추출한 텍스트)\n\nfirst\nsecond");
    }

    #[test]
    fn non_zip_is_error() {
        assert_eq!(
            extract("bad.docx", b"not a zip").unwrap_err(),
            "파일을 읽을 수 없습니다: bad.docx"
        );
    }

    #[test]
    fn base64_decoding() {
        assert_eq!(decode_base64("TWFu").unwrap(), b"Man".to_vec());
        assert_eq!(decode_base64("TWE=").unwrap(), b"Ma".to_vec());
        assert_eq!(decode_base64("TQ==").unwrap(), b"M".to_vec());
        assert_eq!(decode_base64("TW\nFu").unwrap(), b"Man".to_vec());
        assert!(decode_base64("@@@").is_none());
    }
}
