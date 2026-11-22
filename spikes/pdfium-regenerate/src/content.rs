//! Reads a page's content stream with lopdf, independently of PDFium, to see what
//! survived regeneration: which operators, and which marked-content spans.

use std::collections::{BTreeMap, BTreeSet};

use lopdf::content::Content;
use lopdf::{Document, Object};

#[derive(Debug, Default, Clone)]
pub struct PageContent {
    /// Operator -> count, page content only (not forms).
    pub operators: BTreeMap<String, usize>,
    /// For each painting operator, the stack of open marked-content spans,
    /// with consecutive repeats collapsed. Tells whether text stays inside the
    /// same spans (structure, optional content, ActualText).
    pub mark_runs: Vec<String>,
    pub mcids: BTreeSet<i64>,
    /// Property-list names used by BDC, and whether the page resources define them.
    pub property_names: BTreeMap<String, bool>,
    pub content_streams: usize,
    pub decoded_bytes: usize,
}

const PAINT: &[&str] =
    &["Tj", "TJ", "'", "\"", "f", "F", "f*", "B", "B*", "b", "b*", "S", "s", "Do", "sh", "BI", "EI"];

pub fn inspect(bytes: &[u8], password: Option<&str>, page_index: usize) -> Result<PageContent, String> {
    let doc = match password {
        Some(p) => Document::load_mem_with_options(bytes, lopdf::LoadOptions::with_password(p)),
        None => Document::load_mem(bytes),
    }
    .map_err(|e| format!("lopdf load: {e}"))?;
    let pages = doc.get_pages();
    let page_id = *pages.values().nth(page_index).ok_or("no such page")?;
    let raw = doc.get_page_content(page_id);
    let mut out = PageContent {
        content_streams: doc.get_page_contents(page_id).len(),
        decoded_bytes: raw.len(),
        ..Default::default()
    };
    let content = Content::decode(&raw).map_err(|e| format!("lopdf decode: {e}"))?;

    let properties: BTreeSet<String> = doc
        .get_page_resources(page_id)
        .ok()
        .map(|(direct, ids)| {
            let dicts: Vec<&lopdf::Dictionary> =
                direct.into_iter().chain(ids.iter().filter_map(|id| doc.get_dictionary(*id).ok())).collect();
            let mut names = BTreeSet::new();
            for dict in dicts {
                if let Ok(Object::Dictionary(props)) = dict.get_deref(b"Properties", &doc) {
                    names.extend(props.iter().map(|(k, _)| String::from_utf8_lossy(k).into_owned()));
                }
            }
            names
        })
        .unwrap_or_default();

    let mut stack: Vec<String> = Vec::new();
    for op in content.operations {
        *out.operators.entry(op.operator.clone()).or_default() += 1;
        match op.operator.as_str() {
            "BMC" => stack.push(name(op.operands.first())),
            "BDC" => {
                let tag = name(op.operands.first());
                let label = match op.operands.get(1) {
                    Some(Object::Dictionary(d)) => {
                        let mut parts = Vec::new();
                        if let Ok(mcid) = d.get(b"MCID").and_then(Object::as_i64) {
                            out.mcids.insert(mcid);
                            parts.push(format!("MCID {mcid}"));
                        }
                        if d.has(b"ActualText") {
                            parts.push("ActualText".into());
                        }
                        format!("{tag}{{{}}}", parts.join(","))
                    }
                    Some(Object::Name(n)) => {
                        let n = String::from_utf8_lossy(n).into_owned();
                        out.property_names.insert(n.clone(), properties.contains(&n));
                        format!("{tag}/{n}")
                    }
                    _ => tag,
                };
                stack.push(label);
            }
            "EMC" => {
                stack.pop();
            }
            op_name if PAINT.contains(&op_name) => {
                let path = if stack.is_empty() { "-".to_string() } else { stack.join(" > ") };
                if out.mark_runs.last() != Some(&path) {
                    out.mark_runs.push(path);
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn name(object: Option<&Object>) -> String {
    match object {
        Some(Object::Name(n)) => String::from_utf8_lossy(n).into_owned(),
        _ => "?".into(),
    }
}
