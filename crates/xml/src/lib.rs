//! Lossless XML tree for Kinaute.
//!
//! OOXML parts are parsed into a small owned tree that keeps *everything*:
//! element order, qualified names (prefixes are kept verbatim), attributes in
//! their original order, text (including whitespace), comments, CDATA and
//! processing instructions. Serialising a parsed tree yields an XML document
//! that is semantically identical to the input. Kinaute uses this to patch only
//! the parts of a file it understands and to carry everything else through
//! untouched.

use quick_xml::events::Event;
use quick_xml::Reader;
use std::fmt::Write as _;

/// Errors raised while parsing XML.
#[derive(Debug, thiserror::Error)]
pub enum XmlError {
    #[error("XML syntax error at byte {pos}: {msg}")]
    Syntax { pos: u64, msg: String },
    #[error("XML document has no root element")]
    NoRoot,
    #[error("unbalanced XML: unexpected closing tag `{0}`")]
    Unbalanced(String),
    #[error("XML document is not valid UTF-8")]
    Encoding,
    #[error("XML nesting deeper than {0} levels")]
    TooDeep(usize),
}

/// Maximum nesting depth accepted (protection against malicious files).
pub const MAX_DEPTH: usize = 512;

/// A node of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Element(Element),
    /// Unescaped character data.
    Text(String),
    CData(String),
    Comment(String),
    /// Processing instruction, raw content between `<?` and `?>`.
    Pi(String),
}

/// An XML element with its qualified name kept verbatim (e.g. `p:sp`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Element {
    pub name: String,
    /// Attributes in document order, values unescaped.
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

/// A whole document: optional declaration, nodes before the root, the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// Raw content of the XML declaration (`version="1.0" ...`), if any.
    pub decl: Option<String>,
    pub prolog: Vec<Node>,
    pub root: Element,
}

impl Element {
    pub fn new(name: impl Into<String>) -> Self {
        Element { name: name.into(), attrs: Vec::new(), children: Vec::new() }
    }

    /// Builder: add an attribute.
    #[must_use]
    pub fn with_attr(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.attrs.push((k.into(), v.into()));
        self
    }

    /// Builder: add a child element.
    #[must_use]
    pub fn with_child(mut self, c: Element) -> Self {
        self.children.push(Node::Element(c));
        self
    }

    /// Builder: add a text child.
    #[must_use]
    pub fn with_text(mut self, t: impl Into<String>) -> Self {
        self.children.push(Node::Text(t.into()));
        self
    }

    /// Local part of the name (`sp` for `p:sp`).
    pub fn local(&self) -> &str {
        local_name(&self.name)
    }

    /// Prefix of the name (`p` for `p:sp`, empty when none).
    pub fn prefix(&self) -> &str {
        match self.name.find(':') {
            Some(i) => &self.name[..i],
            None => "",
        }
    }

    /// Attribute lookup by exact qualified name (`r:embed`) or, when the
    /// requested name has no prefix, by unprefixed name.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Attribute lookup on the local part of the name, ignoring the prefix.
    pub fn attr_local(&self, local: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| local_name(k) == local).map(|(_, v)| v.as_str())
    }

    /// Attribute with a namespace prefix and this local name (`r:id` for
    /// `"id"`), skipping an unprefixed attribute of the same local name.
    pub fn attr_ns(&self, local: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k.contains(':') && local_name(k) == local).map(|(_, v)| v.as_str())
    }

    pub fn attr_i64(&self, name: &str) -> Option<i64> {
        self.attr(name).and_then(|v| v.trim().parse().ok())
    }

    pub fn attr_bool(&self, name: &str) -> Option<bool> {
        self.attr(name).map(|v| matches!(v.trim(), "1" | "true" | "on"))
    }

    /// Set (or replace) an attribute, keeping its position when it exists.
    pub fn set_attr(&mut self, name: &str, value: impl Into<String>) {
        let value = value.into();
        if let Some(slot) = self.attrs.iter_mut().find(|(k, _)| k == name) {
            slot.1 = value;
        } else {
            self.attrs.push((name.to_string(), value));
        }
    }

    pub fn remove_attr(&mut self, name: &str) {
        self.attrs.retain(|(k, _)| k != name);
    }

    /// Set the attribute when `value` is `Some`, remove it otherwise.
    pub fn set_opt_attr(&mut self, name: &str, value: Option<String>) {
        match value {
            Some(v) => self.set_attr(name, v),
            None => self.remove_attr(name),
        }
    }

    /// Iterator over child elements.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    pub fn elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    /// First child element with the given local name.
    pub fn child(&self, local: &str) -> Option<&Element> {
        self.elements().find(|e| e.local() == local)
    }

    pub fn child_mut(&mut self, local: &str) -> Option<&mut Element> {
        self.elements_mut().find(|e| e.local() == local)
    }

    /// All child elements with the given local name.
    pub fn children_named<'a>(&'a self, local: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.elements().filter(move |e| e.local() == local)
    }

    /// Follow a path of local names.
    pub fn path(&self, path: &[&str]) -> Option<&Element> {
        let mut cur = self;
        for p in path {
            cur = cur.child(p)?;
        }
        Some(cur)
    }

    /// Concatenated text content of direct text children.
    pub fn text(&self) -> String {
        let mut s = String::new();
        for n in &self.children {
            match n {
                Node::Text(t) | Node::CData(t) => s.push_str(t),
                _ => {}
            }
        }
        s
    }

    /// Remove every child element with this local name.
    pub fn remove_children(&mut self, local: &str) {
        self.children.retain(|n| !matches!(n, Node::Element(e) if e.local() == local));
    }

    /// Replace the first child with the same local name as `new`, or insert it
    /// at the position mandated by `order` (a list of local names in schema
    /// order). Elements not listed in `order` are treated as coming last.
    pub fn put_child_ordered(&mut self, new: Element, order: &[&str]) {
        let local = new.local().to_string();
        if let Some(pos) = self
            .children
            .iter()
            .position(|n| matches!(n, Node::Element(e) if e.local() == local))
        {
            self.children[pos] = Node::Element(new);
            return;
        }
        let rank = |l: &str| order.iter().position(|o| *o == l).unwrap_or(order.len());
        let my_rank = rank(&local);
        let pos = self
            .children
            .iter()
            .position(|n| matches!(n, Node::Element(e) if rank(e.local()) > my_rank))
            .unwrap_or(self.children.len());
        self.children.insert(pos, Node::Element(new));
    }

    /// Get a mutable child with the given qualified name, creating it at its
    /// schema position if missing.
    pub fn ensure_child_ordered(&mut self, qname: &str, order: &[&str]) -> &mut Element {
        let local = local_name(qname).to_string();
        if self.child(&local).is_none() {
            self.put_child_ordered(Element::new(qname), order);
        }
        self.child_mut(&local).unwrap_or_else(|| unreachable_element())
    }

    /// Serialise this element (without XML declaration).
    pub fn to_xml(&self) -> String {
        let mut out = String::new();
        write_element(&mut out, self);
        out
    }
}

#[allow(clippy::panic)]
fn unreachable_element() -> &'static mut Element {
    // Only reachable if the code above is wrong; never on user input.
    panic!("internal error: element slot is not an element")
}

/// Local part of a qualified name.
pub fn local_name(q: &str) -> &str {
    match q.rfind(':') {
        Some(i) => &q[i + 1..],
        None => q,
    }
}

/// Unescape XML entities in an attribute or text value.
pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find(';') {
            Some(j) if j <= 12 => {
                let ent = &after[..j];
                if let Some(c) = resolve_entity(ent) {
                    out.push(c);
                } else {
                    out.push('&');
                    out.push_str(ent);
                    out.push(';');
                }
                rest = &after[j + 1..];
            }
            _ => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn resolve_entity(ent: &str) -> Option<char> {
    match ent {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let num = ent.strip_prefix('#')?;
            let code = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                num.parse().ok()?
            };
            char::from_u32(code).filter(|c| *c != '\0')
        }
    }
}

fn escape_into(out: &mut String, s: &str, attr: bool) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            '\n' if attr => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' if attr => out.push_str("&#9;"),
            _ => out.push(c),
        }
    }
}

fn write_element(out: &mut String, e: &Element) {
    out.push('<');
    out.push_str(&e.name);
    for (k, v) in &e.attrs {
        out.push(' ');
        out.push_str(k);
        out.push_str("=\"");
        escape_into(out, v, true);
        out.push('"');
    }
    if e.children.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    for c in &e.children {
        write_node(out, c);
    }
    out.push_str("</");
    out.push_str(&e.name);
    out.push('>');
}

fn write_node(out: &mut String, n: &Node) {
    match n {
        Node::Element(e) => write_element(out, e),
        Node::Text(t) => escape_into(out, t, false),
        Node::CData(t) => {
            // `]]>` cannot appear inside CDATA; split it if present.
            let _ = write!(out, "<![CDATA[{}]]>", t.replace("]]>", "]]]]><![CDATA[>"));
        }
        Node::Comment(t) => {
            let _ = write!(out, "<!--{t}-->");
        }
        Node::Pi(t) => {
            let _ = write!(out, "<?{t}?>");
        }
    }
}

impl Document {
    /// Parse a document from bytes (UTF-8; a UTF-8 BOM is accepted).
    pub fn parse(bytes: &[u8]) -> Result<Document, XmlError> {
        let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
        let text = match std::str::from_utf8(bytes) {
            Ok(t) => std::borrow::Cow::Borrowed(t),
            Err(_) => {
                // UTF-16 documents are legal OOXML but rare; decode them.
                if bytes.len() >= 2 && (bytes[0] == 0xFF && bytes[1] == 0xFE) {
                    let u16s: Vec<u16> = bytes[2..]
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    std::borrow::Cow::Owned(String::from_utf16(&u16s).map_err(|_| XmlError::Encoding)?)
                } else if bytes.len() >= 2 && (bytes[0] == 0xFE && bytes[1] == 0xFF) {
                    let u16s: Vec<u16> = bytes[2..]
                        .chunks_exact(2)
                        .map(|c| u16::from_be_bytes([c[0], c[1]]))
                        .collect();
                    std::borrow::Cow::Owned(String::from_utf16(&u16s).map_err(|_| XmlError::Encoding)?)
                } else {
                    return Err(XmlError::Encoding);
                }
            }
        };
        Self::parse_str(&text)
    }

    pub fn parse_str(text: &str) -> Result<Document, XmlError> {
        let mut reader = Reader::from_str(text);
        let cfg = reader.config_mut();
        cfg.trim_text(false);
        cfg.check_end_names = false;
        cfg.expand_empty_elements = false;

        let mut decl = None;
        let mut prolog = Vec::new();
        let mut stack: Vec<Element> = Vec::new();
        let mut root: Option<Element> = None;

        let syntax = |reader: &Reader<&[u8]>, e: quick_xml::Error| XmlError::Syntax {
            pos: reader.buffer_position(),
            msg: e.to_string(),
        };

        loop {
            let ev = reader.read_event().map_err(|e| syntax(&reader, e))?;
            match ev {
                Event::Eof => break,
                Event::Decl(d) => {
                    let raw: &str = &d;
                    decl = Some(raw.trim_start_matches("xml").trim().to_string());
                }
                Event::Start(s) => {
                    if stack.len() >= MAX_DEPTH {
                        return Err(XmlError::TooDeep(MAX_DEPTH));
                    }
                    stack.push(start_to_element(&s).map_err(|e| syntax(&reader, e))?);
                }
                Event::Empty(s) => {
                    let el = start_to_element(&s).map_err(|e| syntax(&reader, e))?;
                    push_node(&mut stack, &mut root, &mut prolog, Node::Element(el));
                }
                Event::End(e) => {
                    let name = e.name();
                    let name: &str = name.as_ref();
                    let el = stack.pop().ok_or_else(|| XmlError::Unbalanced(name.to_string()))?;
                    if el.name != name {
                        return Err(XmlError::Unbalanced(name.to_string()));
                    }
                    push_node(&mut stack, &mut root, &mut prolog, Node::Element(el));
                }
                Event::Text(t) => {
                    let s = t.xml10_content().into_owned();
                    if let Some(top) = stack.last_mut() {
                        append_text(top, &s);
                    }
                }
                Event::GeneralRef(r) => {
                    let c = match r.resolve_char_ref() {
                        Ok(Some(c)) => Some(c),
                        _ => {
                            let raw: &str = &r;
                            resolve_entity(raw)
                        }
                    };
                    if let (Some(top), Some(c)) = (stack.last_mut(), c) {
                        let mut buf = [0u8; 4];
                        append_text(top, c.encode_utf8(&mut buf));
                    }
                }
                Event::CData(c) => {
                    let s = c.into_inner().into_owned();
                    if let Some(top) = stack.last_mut() {
                        top.children.push(Node::CData(s));
                    }
                }
                Event::Comment(c) => {
                    let s = c.into_inner().into_owned();
                    push_node(&mut stack, &mut root, &mut prolog, Node::Comment(s));
                }
                Event::PI(p) => {
                    let raw: &str = &p;
                    push_node(&mut stack, &mut root, &mut prolog, Node::Pi(raw.to_string()));
                }
                Event::DocType(_) => {
                    // DTDs are never legitimate in OOXML; ignore them (and
                    // never expand custom entities: no billion-laughs).
                }
            }
        }
        if let Some(open) = stack.pop() {
            return Err(XmlError::Unbalanced(open.name));
        }
        let root = root.ok_or(XmlError::NoRoot)?;
        Ok(Document { decl, prolog, root })
    }

    /// Serialise with an XML declaration (standalone OOXML style).
    pub fn to_xml(&self) -> String {
        let mut out = String::with_capacity(4096);
        match &self.decl {
            Some(d) => {
                let _ = write!(out, "<?xml {d}?>\r\n");
            }
            None => out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n"),
        }
        for n in &self.prolog {
            write_node(&mut out, n);
        }
        write_element(&mut out, &self.root);
        out
    }

    pub fn new(root: Element) -> Self {
        Document {
            decl: Some("version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"".into()),
            prolog: Vec::new(),
            root,
        }
    }
}

fn append_text(top: &mut Element, s: &str) {
    if let Some(Node::Text(prev)) = top.children.last_mut() {
        prev.push_str(s);
    } else {
        top.children.push(Node::Text(s.to_string()));
    }
}

fn push_node(stack: &mut [Element], root: &mut Option<Element>, prolog: &mut Vec<Node>, n: Node) {
    if let Some(top) = stack.last_mut() {
        top.children.push(n);
    } else {
        match n {
            Node::Element(e) => {
                if root.is_none() {
                    *root = Some(e);
                }
            }
            other => {
                if root.is_none() {
                    prolog.push(other);
                }
            }
        }
    }
}

fn start_to_element(s: &quick_xml::events::BytesStart<'_>) -> Result<Element, quick_xml::Error> {
    let name = s.name();
    let name: &str = name.as_ref();
    let mut el = Element::new(name);
    for a in s.attributes().with_checks(false) {
        let a = a.map_err(quick_xml::Error::from)?;
        let k: &str = a.key.as_ref();
        el.attrs.push((k.to_string(), unescape(&a.value)));
    }
    Ok(el)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_preserves_structure() {
        let src = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:p="urn:p" xmlns:a="urn:a"><!-- c --><p:cSld name="A &amp; B"><a:t xml:space="preserve"> x &lt; y &#233; </a:t><a:e/></p:cSld><?pi data?></p:sld>"#;
        let d = Document::parse(src.as_bytes()).expect("parse");
        let out = d.to_xml();
        let d2 = Document::parse(out.as_bytes()).expect("reparse");
        assert_eq!(d, d2);
        let t = d.root.path(&["cSld", "t"]).expect("t");
        assert_eq!(t.text(), " x < y é ");
        assert_eq!(d.root.child("cSld").and_then(|c| c.attr("name")), Some("A & B"));
    }

    #[test]
    fn ordered_insert() {
        let mut e = Element::new("p:spPr").with_child(Element::new("a:xfrm")).with_child(Element::new("a:ln"));
        e.put_child_ordered(Element::new("a:solidFill"), &["xfrm", "prstGeom", "solidFill", "ln"]);
        let names: Vec<_> = e.elements().map(|e| e.local().to_string()).collect();
        assert_eq!(names, ["xfrm", "solidFill", "ln"]);
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        for s in ["", "<", "<a>", "</a>", "<a></b>", "<a b='>", "\u{0}\u{1}"] {
            let _ = Document::parse(s.as_bytes());
        }
    }
}
