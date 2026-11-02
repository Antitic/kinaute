//! OPC package (ZIP) access with safety limits, relationships and paths.

use kinaute_model::Rel;
use kinaute_xml::{Document as Xml, Element};
use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum PptxError {
    #[error("not a valid ZIP/OPC file: {0}")]
    Zip(String),
    #[error("missing part `{0}`")]
    MissingPart(String),
    #[error("invalid XML in `{part}`: {source}")]
    Xml { part: String, source: kinaute_xml::XmlError },
    #[error("file is not a PowerPoint presentation: {0}")]
    NotPresentation(&'static str),
    #[error("file too large or suspicious: {0}")]
    Limit(&'static str),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

const MAX_ENTRIES: usize = 50_000;
const MAX_ENTRY: u64 = 1 << 30;
const MAX_TOTAL: u64 = 3 << 30;

pub const REL_OFFICE_DOC: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
pub const REL_SLIDE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";
pub const REL_LAYOUT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout";
pub const REL_THEME: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";
pub const REL_IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
pub const REL_NOTES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide";

/// All parts of a package in their original order. Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct Package {
    pub parts: Vec<(String, Arc<Vec<u8>>)>,
    index: HashMap<String, usize>,
}

impl Package {
    pub fn read(bytes: &[u8]) -> Result<Package, PptxError> {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| PptxError::Zip(e.to_string()))?;
        if zip.len() > MAX_ENTRIES {
            return Err(PptxError::Limit("too many entries"));
        }
        let mut pkg = Package::default();
        let mut total = 0u64;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).map_err(|e| PptxError::Zip(e.to_string()))?;
            if f.is_dir() {
                continue;
            }
            let raw_name = match f.name() {
                Ok(n) => n.into_owned(),
                Err(_) => continue,
            };
            let name = raw_name.trim_start_matches('/').replace('\\', "/");
            if name.is_empty() || name.split('/').any(|c| c == "..") {
                continue;
            }
            if f.size() > MAX_ENTRY {
                return Err(PptxError::Limit("entry too large"));
            }
            total += f.size();
            if total > MAX_TOTAL {
                return Err(PptxError::Limit("package too large when decompressed"));
            }
            let mut data = Vec::with_capacity(f.size().min(1 << 24) as usize);
            (&mut f).take(MAX_ENTRY + 1).read_to_end(&mut data)?;
            if data.len() as u64 > MAX_ENTRY {
                return Err(PptxError::Limit("entry too large"));
            }
            pkg.set(&name, Arc::new(data));
        }
        Ok(pkg)
    }

    pub fn get(&self, name: &str) -> Option<&Arc<Vec<u8>>> {
        self.index.get(name).map(|i| &self.parts[*i].1)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    pub fn set(&mut self, name: &str, data: Arc<Vec<u8>>) {
        match self.index.get(name) {
            Some(i) => self.parts[*i].1 = data,
            None => {
                self.index.insert(name.to_string(), self.parts.len());
                self.parts.push((name.to_string(), data));
            }
        }
    }

    pub fn remove(&mut self, name: &str) {
        if self.index.remove(name).is_some() {
            self.parts.retain(|(n, _)| n != name);
            self.index = self.parts.iter().enumerate().map(|(i, (n, _))| (n.clone(), i)).collect();
        }
    }

    pub fn xml(&self, name: &str) -> Result<Xml, PptxError> {
        let data = self.get(name).ok_or_else(|| PptxError::MissingPart(name.to_string()))?;
        Xml::parse(data).map_err(|source| PptxError::Xml { part: name.to_string(), source })
    }

    pub fn set_xml(&mut self, name: &str, doc: &Xml) {
        self.set(name, Arc::new(doc.to_xml().into_bytes()));
    }

    /// Relationships of a part (`""` for the package root).
    pub fn rels_of(&self, part: &str) -> Vec<Rel> {
        let rels_name = rels_part_name(part);
        let Ok(doc) = self.xml(&rels_name) else { return Vec::new() };
        let dir = dir_of(part);
        doc.root
            .elements()
            .filter(|e| e.local() == "Relationship")
            .filter_map(|e| {
                let external = e.attr("TargetMode") == Some("External");
                let raw = e.attr("Target")?;
                let target = if external { raw.to_string() } else { resolve(&dir, raw) };
                Some(Rel { id: e.attr("Id")?.to_string(), ty: e.attr("Type")?.to_string(), target, external })
            })
            .collect()
    }

    /// Write relationships for a part (targets relative to its directory).
    pub fn set_rels(&mut self, part: &str, rels: &[Rel]) {
        let dir = dir_of(part);
        let mut root = Element::new("Relationships")
            .with_attr("xmlns", "http://schemas.openxmlformats.org/package/2006/relationships");
        for r in rels {
            let target = if r.external { r.target.clone() } else { relative(&dir, &r.target) };
            let mut e = Element::new("Relationship")
                .with_attr("Id", r.id.clone())
                .with_attr("Type", r.ty.clone())
                .with_attr("Target", target);
            if r.external {
                e.attrs.push(("TargetMode".into(), "External".into()));
            }
            root = root.with_child(e);
        }
        self.set_xml(&rels_part_name(part), &Xml::new(root));
    }

    /// Serialise to a ZIP.
    pub fn write(&self) -> Result<Vec<u8>, PptxError> {
        let mut out = Cursor::new(Vec::with_capacity(1 << 20));
        {
            let mut zw = zip::ZipWriter::new(&mut out);
            let deflate = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            let mut order: Vec<&(String, Arc<Vec<u8>>)> = self.parts.iter().collect();
            // `[Content_Types].xml` first, as Office writes it.
            order.sort_by_key(|(n, _)| n != "[Content_Types].xml");
            for (name, data) in order {
                let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                let already = matches!(
                    ext.as_str(),
                    "png" | "jpg" | "jpeg" | "gif" | "mp4" | "mp3" | "m4a" | "mov" | "zip" | "wmv" | "webp"
                );
                zw.start_file(name.as_str(), if already { stored } else { deflate })
                    .map_err(|e| PptxError::Zip(e.to_string()))?;
                zw.write_all(data)?;
            }
            zw.finish().map_err(|e| PptxError::Zip(e.to_string()))?;
        }
        Ok(out.into_inner())
    }
}

pub fn rels_part_name(part: &str) -> String {
    match part.rfind('/') {
        Some(i) => format!("{}/_rels/{}.rels", &part[..i], &part[i + 1..]),
        None if part.is_empty() => "_rels/.rels".to_string(),
        None => format!("_rels/{part}.rels"),
    }
}

pub fn dir_of(part: &str) -> String {
    part.rfind('/').map(|i| part[..i].to_string()).unwrap_or_default()
}

/// Resolve a relationship target against the directory of its source.
pub fn resolve(dir: &str, target: &str) -> String {
    let mut comps: Vec<&str> =
        if target.starts_with('/') { Vec::new() } else { dir.split('/').filter(|c| !c.is_empty()).collect() };
    for c in target.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                comps.pop();
            }
            c => comps.push(c),
        }
    }
    comps.join("/")
}

/// Relative path from `dir` to `target`.
pub fn relative(dir: &str, target: &str) -> String {
    let d: Vec<&str> = dir.split('/').filter(|c| !c.is_empty()).collect();
    let t: Vec<&str> = target.split('/').filter(|c| !c.is_empty()).collect();
    let common = d.iter().zip(&t).take_while(|(a, b)| a == b).count().min(t.len().saturating_sub(1));
    let mut out: Vec<&str> = std::iter::repeat_n("..", d.len() - common).collect();
    out.extend(&t[common..]);
    out.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(resolve("ppt/slides", "../media/image1.png"), "ppt/media/image1.png");
        assert_eq!(resolve("ppt", "slides/slide1.xml"), "ppt/slides/slide1.xml");
        assert_eq!(resolve("ppt", "/docProps/app.xml"), "docProps/app.xml");
        assert_eq!(relative("ppt/slides", "ppt/media/image1.png"), "../media/image1.png");
        assert_eq!(relative("ppt", "ppt/slides/slide1.xml"), "slides/slide1.xml");
        assert_eq!(rels_part_name("ppt/slides/slide1.xml"), "ppt/slides/_rels/slide1.xml.rels");
        assert_eq!(rels_part_name(""), "_rels/.rels");
    }
}
