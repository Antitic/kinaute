//! Constructors for objects created in the editor.

use crate::color::Color;
use crate::types::*;

fn base(id: u32, name: String, xfrm: Xfrm, kind: ShapeKind) -> Shape {
    Shape { id, name, descr: None, hidden: false, ph: None, xfrm: Some(xfrm), kind, source: None }
}

/// A text box (no fill, no outline, text grows the shape).
pub fn text_box(id: u32, xfrm: Xfrm, text: &str, size: i32) -> Shape {
    let mut body = TextBody::default();
    body.body.wrap = Some(true);
    body.body.autofit = Some(AutoFit::Shape);
    let props = RunProps { size: Some(size), lang: Some("fr-FR".into()), ..Default::default() };
    body.paragraphs = text
        .split('\n')
        .map(|l| Paragraph {
            runs: if l.is_empty() { vec![] } else { vec![Run { props: props.clone(), ..Run::text(l) }] },
            end_props: Some(props.clone()),
            ..Default::default()
        })
        .collect();
    let sp = SpProps {
        geom: Some(Geometry::preset("rect")),
        fill: Some(Fill::None),
        text: Some(body),
        text_box: true,
        ..Default::default()
    };
    base(id, format!("ZoneTexte {id}"), xfrm, ShapeKind::Shape(Box::new(sp)))
}

/// A preset shape styled with the theme's accent colour, like PowerPoint's
/// default shape style.
pub fn shape(id: u32, xfrm: Xfrm, preset: &str) -> Shape {
    let mut body = TextBody::default();
    body.body.anchor = Some(Anchor::Middle);
    body.paragraphs =
        vec![Paragraph { props: ParaProps { align: Some(Align::Center), ..Default::default() }, ..Default::default() }];
    let sp = SpProps {
        geom: Some(Geometry::preset(preset)),
        style: Some(default_style()),
        text: Some(body),
        ..Default::default()
    };
    base(id, format!("Forme {id}"), xfrm, ShapeKind::Shape(Box::new(sp)))
}

/// A straight line (connector).
pub fn line(id: u32, xfrm: Xfrm) -> Shape {
    let sp = SpProps {
        geom: Some(Geometry::preset("line")),
        line: Some(Line { width: Some(28575), ..Default::default() }),
        style: Some(StyleRefs {
            line: (1, Some(Color::scheme("accent1"))),
            fill: (0, Some(Color::scheme("accent1"))),
            effect: (0, Some(Color::scheme("accent1"))),
            font: ("minor".into(), Some(Color::scheme("tx1"))),
        }),
        ..Default::default()
    };
    base(id, format!("Connecteur {id}"), xfrm, ShapeKind::Connector(Box::new(sp)))
}

/// A picture referencing a media part.
pub fn picture(id: u32, xfrm: Xfrm, media: &str) -> Shape {
    let pic = PicProps {
        image: ImageFill { media: Some(media.to_string()), crop: (0, 0, 0, 0), stretch: true, alpha: 100_000 },
        geom: Some(Geometry::preset("rect")),
        ..Default::default()
    };
    base(id, format!("Image {id}"), xfrm, ShapeKind::Picture(Box::new(pic)))
}

pub fn default_style() -> StyleRefs {
    StyleRefs {
        line: (
            2,
            Some(Color { base: crate::ColorBase::Scheme("accent1".into()), mods: vec![("shade".into(), 15000)] }),
        ),
        fill: (1, Some(Color::scheme("accent1"))),
        effect: (0, Some(Color::scheme("accent1"))),
        font: ("minor".into(), Some(Color::scheme("lt1"))),
    }
}
