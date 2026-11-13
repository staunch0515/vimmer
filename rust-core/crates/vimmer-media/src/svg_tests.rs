//! Sanitizer tests with hostile SVG samples. No Chrome needed except in `check_png` tests,
//! which write their own PNGs.

use super::*;

const W: u32 = 1920;
const H: u32 = 1080;

fn wrap(body: &str) -> String {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1920 1080"><rect width="1920" height="1080" fill="#123"/>{body}</svg>"##
    )
}

fn clean(body: &str) -> SanitizedSvg {
    sanitize_svg(&wrap(body), W, H).expect("sanitizes")
}

/// The sanitized output must itself parse again, and must not contain any of `needles`
/// (case-insensitive).
fn assert_free_of(out: &SanitizedSvg, needles: &[&str]) {
    let again = sanitize_svg(&out.svg, W, H).expect("output parses again");
    assert_eq!(again.svg, out.svg, "sanitizing is idempotent");
    let lower = out.svg.to_ascii_lowercase();
    for n in needles {
        assert!(
            !lower.contains(&n.to_ascii_lowercase()),
            "`{n}` survived in {}",
            out.svg
        );
    }
}

#[test]
fn benign_svg_is_kept_and_canvas_attributes_are_set() {
    let out = clean(
        r##"<defs><linearGradient id="g"><stop offset="0" stop-color="#fff"/></linearGradient></defs><circle cx="5" cy="5" r="3" fill="url(#g)"/><use href="#g"/>"##,
    );
    assert!(out.removed.is_empty(), "{:?}", out.removed);
    assert!(out.svg.starts_with("<?xml"));
    assert!(out.svg.contains(r#"fill="url(#g)""#));
    assert!(out.svg.contains(r##"<use href="#g"/>"##));
    assert!(out.svg.contains(r#"width="1920" height="1080""#));
    assert!(out.svg.contains(r#"preserveAspectRatio="xMidYMid slice""#));
    assert_free_of(&out, &[]);
}

#[test]
fn script_elements_are_removed_with_content() {
    let out = clean(
        "<script>alert(1)</script><SCRIPT>alert(2)</SCRIPT><g><script type=\"text/ecmascript\"><![CDATA[fetch('//x')]]></script></g>",
    );
    assert!(out.removed.contains(&"element <script>".to_string()));
    assert!(out.removed.contains(&"element <SCRIPT>".to_string()));
    assert_free_of(&out, &["script", "alert", "fetch"]);
}

#[test]
fn foreign_object_iframe_image_and_links_are_removed() {
    let out = clean(
        r#"<foreignObject width="100" height="100"><body xmlns="http://www.w3.org/1999/xhtml"><iframe src="https://evil.example"/></body></foreignObject><image href="https://evil.example/x.png" width="10" height="10"/><image xlink:href="file:///etc/passwd"/><a href="javascript:alert(1)"><rect width="1" height="1"/></a><iframe src="x"/>"#,
    );
    assert_free_of(
        &out,
        &[
            "foreignobject",
            "iframe",
            "<image",
            "evil",
            "passwd",
            "javascript",
            "<a ",
        ],
    );
}

#[test]
fn event_attributes_are_removed() {
    let out = clean(
        r#"<rect width="1" height="1" onclick="alert(1)" ONLOAD="x()" onmouseover='y()'/><g onfocusin="z()"><circle r="1"/></g>"#,
    );
    assert!(out.removed.iter().any(|r| r.contains("onclick")));
    assert_free_of(
        &out,
        &["onclick", "onload", "onmouseover", "onfocusin", "alert"],
    );
}

#[test]
fn root_onload_is_removed() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1920 1080" onload="alert(1)"><rect width="10" height="10"/></svg>"#;
    let out = sanitize_svg(svg, W, H).unwrap();
    assert_free_of(&out, &["onload", "alert"]);
}

#[test]
fn external_hrefs_are_removed_and_local_ones_kept() {
    let out = clean(
        r##"<use href="https://evil.example/sprite.svg#a"/><use xlink:href="other.svg#b"/><use xlink:href="#ok"/><use href=" #fine "/><use href="data:image/svg+xml;base64,AAAA"/><use href="#x&quot; onload=&quot;y"/>"##,
    );
    assert!(out.svg.contains(r##"href="#ok""##));
    assert!(out.svg.contains(r##"href="#fine""##));
    assert_free_of(&out, &["evil", "other.svg", "data:", "xlink", "onload"]);
}

#[test]
fn entity_encoded_javascript_is_caught_after_decoding() {
    let out = clean(
        r#"<a href="&#106;avascript:alert(1)"><rect width="1" height="1"/></a><use href="&#x6A;avascript:alert(2)"/><rect width="1" height="1" fill="java&#x09;script:x"/>"#,
    );
    assert_free_of(&out, &["javascript", "alert"]);
}

#[test]
fn css_import_and_external_urls_are_removed() {
    let out = clean(
        r#"<style>@import url("https://evil.example/a.css"); @IMPORT 'b.css'; .a{fill:url(#g)} .b{background:url(http://evil.example/x.png)} .c{fill:URL( 'https://evil.example/y' )}</style><rect class="a" width="1" height="1" style="fill:url(https://evil.example/z);stroke:url(#g)"/><rect width="1" height="1" fill="url(http://evil.example/p)" filter="url(#f)"/>"#,
    );
    assert!(out.svg.contains(".a{fill:url(#g)}"), "{}", out.svg);
    assert!(out.svg.contains("stroke:url(#g)"));
    assert!(out.svg.contains(r##"filter="url(#f)""##));
    assert_free_of(&out, &["@import", "evil", "b.css", "url(http"]);
}

#[test]
fn css_escapes_and_comment_tricks_drop_the_css() {
    let out = clean(
        r#"<style>.a{fill:u\72l(https://evil.example/x)}</style><rect width="1" height="1" style="fill:\75rl(//evil.example)"/><style>@im/**/port 'x.css'; .b{fill:url(/**/https://evil.example/y)}</style>"#,
    );
    assert_free_of(&out, &["evil", "x.css", "\\"]);
}

#[test]
fn script_like_css_is_dropped() {
    let out = clean(
        r#"<style>.a{background-image:image-set("https://evil.example/x.png" 1x)}</style><rect width="1" height="1" style="behavior:url(#x);-moz-binding:url(#y)"/>"#,
    );
    assert_free_of(&out, &["evil", "image-set", "behavior", "binding"]);
}

#[test]
fn animation_elements_are_removed() {
    let out = clean(
        r##"<a><animate attributeName="href" to="javascript:alert(1)"/></a><rect width="1" height="1"><set attributeName="onclick" to="alert(1)"/><animateTransform attributeName="transform" type="rotate" from="0" to="360" dur="1s"/></rect>"##,
    );
    assert_free_of(&out, &["animate", "<set", "javascript"]);
}

#[test]
fn doctype_and_unknown_entities_are_rejected() {
    let xxe = r#"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY xxe SYSTEM "file:///etc/passwd">]><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1920 1080"><text>&xxe;</text></svg>"#;
    assert!(matches!(sanitize_svg(xxe, W, H), Err(SvgError::Parse(_))));
    let laughs = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1920 1080"><text>&lol9;</text></svg>"#;
    assert!(matches!(
        sanitize_svg(laughs, W, H),
        Err(SvgError::Parse(_))
    ));
}

#[test]
fn malformed_xml_is_rejected() {
    for bad in [
        "",
        "not xml at all",
        r#"<svg viewBox="0 0 1920 1080"><rect></svg>"#,
        r#"<svg viewBox="0 0 1920 1080"><rect width=1/></svg>"#,
        r#"<svg viewBox="0 0 1920 1080" viewBox="0 0 1 1"/>"#,
        r#"<svg viewBox="0 0 1920 1080"><rect/></svg><svg/>"#,
        r#"<svg viewBox="0 0 1920 1080"><rect fill="a<b"/></svg>"#,
        r#"<svg viewBox="0 0 1920 1080">a & b</svg>"#,
        r#"<svg viewBox="0 0 1920 1080"><!-- open"#,
    ] {
        let r = sanitize_svg(bad, W, H);
        assert!(matches!(r, Err(SvgError::Parse(_))), "{bad:?} gave {r:?}");
    }
}

#[test]
fn view_box_is_required_and_checked() {
    let none = r#"<svg xmlns="http://www.w3.org/2000/svg" width="1920" height="1080"><rect width="1" height="1"/></svg>"#;
    assert_eq!(sanitize_svg(none, W, H).unwrap_err(), SvgError::NoViewBox);
    let bad = r#"<svg viewBox="0 0 -5 10"><rect width="1" height="1"/></svg>"#;
    assert!(matches!(
        sanitize_svg(bad, W, H),
        Err(SvgError::BadViewBox(_))
    ));
    let nan = r#"<svg viewBox="0 0 NaN 10"><rect width="1" height="1"/></svg>"#;
    assert!(matches!(
        sanitize_svg(nan, W, H),
        Err(SvgError::BadViewBox(_))
    ));
    let square = r#"<svg viewBox="0 0 1000 1000"><rect width="1" height="1"/></svg>"#;
    assert!(matches!(
        sanitize_svg(square, W, H),
        Err(SvgError::AspectMismatch { .. })
    ));
    // Same aspect ratio at another scale is fine; portrait canvas needs a portrait viewBox.
    let scaled = r#"<svg viewBox="0,0,960,540"><rect width="1" height="1"/></svg>"#;
    assert!(sanitize_svg(scaled, W, H).is_ok());
    assert!(matches!(
        sanitize_svg(scaled, 1080, 1920),
        Err(SvgError::AspectMismatch { .. })
    ));
}

#[test]
fn root_must_be_svg() {
    let html = r#"<html><body><svg viewBox="0 0 1920 1080"/></body></html>"#;
    assert_eq!(
        sanitize_svg(html, W, H).unwrap_err(),
        SvgError::NotSvg("html".into())
    );
}

#[test]
fn size_depth_and_element_limits() {
    let big = wrap(&"<rect width=\"1\" height=\"1\"/>".repeat(MAX_SVG_BYTES / 20));
    assert!(matches!(
        sanitize_svg(&big, W, H),
        Err(SvgError::TooLarge { .. })
    ));
    let deep = wrap(&("<g>".repeat(MAX_DEPTH + 1) + &"</g>".repeat(MAX_DEPTH + 1)));
    assert!(matches!(
        sanitize_svg(&deep, W, H),
        Err(SvgError::TooComplex(_))
    ));
    let many = wrap(&"<g/>".repeat(MAX_ELEMENTS + 1));
    assert!(matches!(
        sanitize_svg(&many, W, H),
        Err(SvgError::TooComplex(_))
    ));
}

#[test]
fn nothing_drawable_left_is_empty() {
    let only_script =
        r#"<svg viewBox="0 0 1920 1080"><script>alert(1)</script><title>x</title></svg>"#;
    assert_eq!(
        sanitize_svg(only_script, W, H).unwrap_err(),
        SvgError::Empty
    );
}

#[test]
fn foreign_namespaces_and_xml_base_are_removed() {
    let out = clean(
        r#"<rect width="1" height="1" xml:base="https://evil.example/" ev:event="click" xml:space="preserve" xmlns:ev="http://www.w3.org/2001/xml-events"/>"#,
    );
    assert!(out.svg.contains(r#"xml:space="preserve""#));
    assert_free_of(&out, &["xml:base", "evil", "ev:event", "xmlns:ev"]);
}

#[test]
fn text_is_escaped_on_output() {
    let out = clean(r#"<text x="1" y="1">a &lt;script&gt; &amp; b</text>"#);
    assert!(out.svg.contains("a &lt;script&gt; &amp; b"));
    assert_free_of(&out, &["<script"]);
}

#[test]
fn processing_instructions_and_comments_are_dropped() {
    let svg = r#"<?xml version="1.0"?><?xml-stylesheet href="https://evil.example/a.css"?><!-- hi --><svg viewBox="0 0 1920 1080"><!-- <script>x</script> --><rect width="1" height="1"/></svg>"#;
    let out = sanitize_svg(svg, W, H).unwrap();
    assert_free_of(&out, &["evil", "stylesheet", "script", "hi"]);
}

#[test]
fn check_png_rejects_flat_and_wrong_size_pictures() {
    let dir = std::env::temp_dir().join(format!("vimmer-svg-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let flat = dir.join("flat.png");
    image::RgbImage::from_pixel(64, 36, image::Rgb([10, 20, 30]))
        .save(&flat)
        .unwrap();
    assert!(matches!(check_png(&flat, 64, 36), Err(SvgError::Raster(_))));
    let mut varied = image::RgbImage::from_pixel(64, 36, image::Rgb([10, 20, 30]));
    for x in 0..64 {
        for y in 0..18 {
            varied.put_pixel(x, y, image::Rgb([200, 120, 40]));
        }
    }
    let ok = dir.join("ok.png");
    varied.save(&ok).unwrap();
    assert!(check_png(&ok, 64, 36).is_ok());
    assert!(matches!(check_png(&ok, 64, 64), Err(SvgError::Raster(_))));
    let _ = std::fs::remove_dir_all(&dir);
}
