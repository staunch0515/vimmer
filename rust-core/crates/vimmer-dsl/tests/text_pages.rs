//! Shared vectors for page splitting and timing: `text_pages_vectors.json` is also checked by
//! `remotion-renderer/test/text-pages.test.ts`, so the editor preview pages exactly like the render.
//! Regenerate the expected values with `UPDATE_TEXT_PAGES_VECTORS=1 cargo test -p vimmer-dsl --test text_pages`.

use serde_json::{json, Value};
use vimmer_dsl::text_layout::{
    page_cap_em, page_starts, proportional_starts, split_pages, top_inset_px, PageClock, Paging,
    TextPosition,
};
use vimmer_dsl::{Brand, Resolution};

const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/text_pages_vectors.json");

fn res(v: &Value) -> Resolution {
    Resolution {
        width: v["width"].as_u64().unwrap() as u32,
        height: v["height"].as_u64().unwrap() as u32,
    }
}

fn paging(v: &Value) -> Paging {
    serde_json::from_value(v.clone()).unwrap()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

fn compute(case: &Value) -> Value {
    match case["kind"].as_str().unwrap() {
        "split" => {
            let r = res(&case["resolution"]);
            let size = case["size"].as_f64().unwrap();
            json!({
                "cap_em": (page_cap_em(size, r) * 1000.0).round() / 1000.0,
                "pages": split_pages(case["text"].as_str().unwrap(), paging(&case["paging"]), size, r),
            })
        }
        "proportional" => json!(proportional_starts(
            &strings(&case["pages"]),
            case["from_ms"].as_u64().unwrap(),
            case["span_ms"].as_u64().unwrap(),
            case["min_ms"].as_u64().unwrap(),
        )),
        "scene" => json!(page_starts(
            &strings(&case["pages"]),
            &PageClock::Scene {
                duration_ms: case["duration_ms"].as_u64().unwrap()
            }
        )),
        "top_inset" => {
            let brand: Option<Brand> = serde_json::from_value(case["brand"].clone()).unwrap();
            let pos: TextPosition = serde_json::from_value(case["position"].clone()).unwrap();
            let px = top_inset_px(pos, brand.as_ref(), case["scene"].as_str().unwrap(), res(&case["resolution"]));
            json!((px * 1000.0).round() / 1000.0)
        }
        k => panic!("unknown kind {k}"),
    }
}

#[test]
fn rust_matches_the_shared_vectors() {
    let raw = std::fs::read_to_string(PATH).unwrap();
    let mut doc: Value = serde_json::from_str(&raw).unwrap();
    let update = std::env::var_os("UPDATE_TEXT_PAGES_VECTORS").is_some();
    for case in doc["cases"].as_array_mut().unwrap() {
        let got = compute(case);
        if update {
            case["expected"] = got;
        } else {
            assert_eq!(got, case["expected"], "{}", case["name"]);
        }
    }
    if update {
        std::fs::write(PATH, serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap();
    }
}
