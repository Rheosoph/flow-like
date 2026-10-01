//! chromedriver 154 parity on the G2 goldens in `tests/fixtures/parity` (spec §6.4).
//! Run with `cargo test -p flow-like-browser --test e2e_parity -- --include-ignored`.

#[path = "support/e2e.rs"]
mod e2e;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use e2e::E2e;
use flow_like_browser::input::ActionOutcome;
use flow_like_browser::input::keys::{Key, Modifiers, NamedKey};
use flow_like_browser::output::{Clip, ScreenshotOptions};
use flow_like_browser::script::{
    ELEMENT_KEY, SHADOW_KEY, ScriptArg, ScriptOptions, ScriptValue, WINDOW_KEY,
};
use flow_like_browser::{BrowserError, Element, ElementRect, Frame, Page};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const VIEWPORT: (u32, u32) = (1200, 713);
const GOLDEN_ORIGIN: &str = "http://127.0.0.1:57117";
const LAYOUT_ENV: &str = "FLOW_LIKE_BROWSER_E2E_PARITY_LAYOUT";
const RECT_TOLERANCE: f64 = 1e-3;
/// Chrome dithers gradients: two captures of one clip differ by up to 1 per channel.
const PIXEL_TOLERANCE: u64 = 2;
const CHROMEDRIVER: &str = "chromedriver";
const SELECT_RULES: &str = "spec §2.18 (option click rules, not measured against chromedriver)";
const KEY_EXPECTATION: &str = "the expected browser behaviour";

const TEXT_HTML: &str = include_str!("fixtures/parity/text.html");
const INPUT_HTML: &str = include_str!("fixtures/parity/input.html");
const SHOT_HTML: &str = include_str!("fixtures/parity/shot.html");
const FORM2_HTML: &str = include_str!("fixtures/parity/form2.html");
const A_TEXT: &str = include_str!("fixtures/parity/golden/a_text.json");
const B_SCRIPT_PLAIN: &str = include_str!("fixtures/parity/golden/b_script_plain.json");
const B_SCRIPT_CSP: &str = include_str!("fixtures/parity/golden/b_script_csp.json");
const C_INPUT: &str = include_str!("fixtures/parity/golden/c_input.json");
const E_SHOT: &str = include_str!("fixtures/parity/golden/e_shot.json");
const F_MISC: &str = include_str!("fixtures/parity/golden/f_misc.json");
const H_MISC: &str = include_str!("fixtures/parity/golden/h_misc.json");
const GET_LOCATION_ATOM: &str = include_str!("../src/js/atoms/get_location.js");
const GET_SIZE_ATOM: &str = include_str!("../src/js/atoms/get_size.js");

const SELECT_HTML: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>Select fixture</title></head>
<body>
<select id="single"><option value="a" selected>A</option><option value="b">B</option><option value="b">B2</option></select>
<select id="multi" multiple size="4"><option value="x" selected>X</option><option value="y">Y</option><option value="y">Y2</option><option value="z">Z</option></select>
<script>
window.__log = [];
for (const select of document.querySelectorAll('select')) {
  select.addEventListener('change', () => window.__log.push(select.id));
}
window.__state = () => Array.from(document.querySelectorAll('select'), (select) =>
  select.id + '=' + Array.from(select.selectedOptions, (option) => option.text).join(','));
</script>
</body></html>
"#;

const COLLECT_INPUT: &str = "return { log: window.__log, state: window.__state() }";
const COLLECT_FORM2: &str =
    "return { log: window.__log, value: document.getElementById('q').value }";
const COLLECT_SELECT: &str = "return { log: window.__log, state: window.__state() }";
const COMPARE_PNGS: &str = r#"
const decode = (base64) => new Promise((resolve, reject) => {
  const image = new Image();
  image.onload = () => {
    const canvas = document.createElement('canvas');
    canvas.width = image.naturalWidth;
    canvas.height = image.naturalHeight;
    const context = canvas.getContext('2d');
    context.drawImage(image, 0, 0);
    resolve({ width: canvas.width, height: canvas.height,
      data: context.getImageData(0, 0, canvas.width, canvas.height).data });
  };
  image.onerror = () => reject(new Error('a PNG does not decode'));
  image.src = 'data:image/png;base64,' + base64;
});
return Promise.all([decode(arguments[0]), decode(arguments[1])]).then(([ours, reference]) => {
  let delta = null;
  if (ours.width === reference.width && ours.height === reference.height) {
    delta = 0;
    for (let index = 0; index < ours.data.length; index++) {
      delta = Math.max(delta, Math.abs(ours.data[index] - reference.data[index]));
    }
  }
  const hex = (x, y) => Array.from(ours.data.subarray((y * ours.width + x) * 4, (y * ours.width + x) * 4 + 3),
    (channel) => channel.toString(16).padStart(2, '0')).join('');
  const right = ours.width - 1;
  const bottom = ours.height - 1;
  return { delta, corners: { tl: hex(0, 0), tr: hex(right, 0), bl: hex(0, bottom), br: hex(right, bottom),
    center: hex(ours.width >> 1, ours.height >> 1) } };
});
"#;
const SCROLL_POSITION: &str = "return [window.scrollX, window.scrollY]";
const VIEWPORT_SIZE: &str = "return [window.innerWidth, window.innerHeight]";
const ACTIVE_ID: &str = "return document.activeElement ? document.activeElement.id : null";
const ADD_DATE_PROBE: &str =
    "document.body.insertAdjacentHTML('afterbegin', '<input type=\"date\" id=\"date_probe\">')";
const DATE_PROBE_VALUE: &str = "return document.getElementById('date_probe').value";
const IN_SHADOW: &str =
    "return document.querySelector('#t_shadow_host').shadowRoot.querySelector('#in_shadow')";
const DESCRIBE_NODE: &str = "return arguments[0].localName + '#' + arguments[0].id";
const OUTER_HTML: &str = "return document.documentElement.outerHTML";
const MAIN_WORLD_GLOBALS: &str = "return { se: typeof window.se_exportedFunctionSymbol, \
cdc: Object.keys(window).filter(function (key) { return key.indexOf('cdc_') === 0; }).length, \
atoms: typeof globalThis.__flowlike }";
const PREFILLED_SELECTION: &str = "var field = document.getElementById('i_prefilled'); \
return [field.selectionStart, field.selectionEnd, field.value]";
const PREFILLED_VALUE: &str = "return document.getElementById('i_prefilled').value";
const F_MISC_EXTRA: &str = r#"document.body.insertAdjacentHTML('beforeend', '<input type="radio" id="r1" name="g" checked><input type="radio" id="r2" name="g"><div role="checkbox" aria-checked="true" id="aria_cb">x</div><select id="sel2"><option id="o1">A</option><option id="o2" selected>B</option></select><input type="checkbox" id="cb_off">');"#;

const RECT_EXCLUSIONS: &[(&str, &str)] = &[(
    "t_details_body",
    "closed <details> body: content-visibility skips its layout, so chromedriver's y is order dependent (G2 §3.3)",
)];
const NODE_NAMES: &[(&str, &[&str])] = &[
    ("element", &["BODY"]),
    ("nodelist", &["LI", "LI"]),
    ("htmlcollection", &["LI", "LI"]),
    ("document", &["#document"]),
    ("shadow_root", &["#document-fragment"]),
];

macro_rules! parity_test {
    ($name:ident, $body:expr) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "requires a Chromium browser; run with --include-ignored"]
        async fn $name() {
            run_parity(stringify!($name), $body).await;
        }
    };
}

parity_test!(text_and_displayed_match_chromedriver, text_and_displayed);
parity_test!(rect_matches_chromedriver, rects);
parity_test!(attributes_match_chromedriver, attributes);
parity_test!(clear_traces_match_chromedriver, clear_cases);
parity_test!(script_values_match_chromedriver, |parity| {
    script_values(parity, "text.html", B_SCRIPT_PLAIN)
});
parity_test!(script_values_match_chromedriver_under_csp, |parity| {
    script_values(parity, "csp.html", B_SCRIPT_CSP)
});
parity_test!(
    send_keys_values_and_event_types_match_chromedriver,
    typing_cases
);
parity_test!(
    element_screenshots_match_chromedriver_clips,
    element_screenshots
);
parity_test!(
    find_selected_and_source_match_chromedriver,
    find_selected_source
);
parity_test!(enter_submits_the_form_exactly_once, enter_submits_once);
parity_test!(select_all_chord_selects_the_field, select_all);
parity_test!(tab_and_shift_tab_move_focus, tab_focus);
parity_test!(
    select_by_value_follows_the_spec_option_rules,
    select_by_value_cases
);

async fn text_and_displayed(parity: Parity) -> Report {
    let golden = golden(A_TEXT);
    let rows = golden.rows("rows");
    let mut report = Report::against(CHROMEDRIVER);
    report.same("golden", "text cases", rows.len(), 52);
    let frame = parity.open("text.html").await;
    for row in rows {
        let id = row.text("id");
        let element = parity.text_element(&frame, id).await;
        let expected = &row["wd"];
        match element.text().await {
            Ok(text) => report.same(id, "text", text.as_str(), expected.text("text")),
            Err(error) => report.fail(id, format!("text failed: {error}")),
        }
        match element.is_displayed().await {
            Ok(shown) => report.same(
                id,
                "displayed",
                Some(shown),
                expected["displayed"].as_bool(),
            ),
            Err(error) => report.fail(id, format!("is_displayed failed: {error}")),
        }
    }
    let globals = parity.eval(&frame, MAIN_WORLD_GLOBALS, Vec::new()).await;
    report.same_json(
        "main world",
        "globals after the atoms ran (chromedriver leaves se_exportedFunctionSymbol and 7 cdc_ globals; ours run isolated)",
        &globals,
        &json!({"se": "undefined", "cdc": 0, "atoms": "undefined"}),
    );
    report
}

async fn rects(parity: Parity) -> Report {
    let golden = golden(A_TEXT);
    let mut report = Report::against(CHROMEDRIVER);
    let frame = parity.open("text.html").await;
    parity.check_viewport(&frame, &mut report).await;
    let strict = strict_layout();
    if !strict {
        report.skip(format!(
            "layout-relative mode: rect parity with chromedriver is NOT checked, only the mapping of the \
             GET_LOCATION/GET_SIZE atoms run in the page (the goldens carry macOS font metrics) and the \
             golden rects of unrendered elements; set {LAYOUT_ENV}=strict to compare every rect"
        ));
    }
    let rows: Vec<&Value> = golden
        .rows("rows")
        .iter()
        .filter(|row| row["wd"].get("rect").is_some())
        .collect();
    report.same("golden", "rect cases", rows.len(), 51);
    for row in rows {
        let id = row.text("id");
        match RECT_EXCLUSIONS.iter().find(|(skip, _)| *skip == id) {
            Some((_, reason)) => report.skip(format!("{id} excluded from rect: {reason}")),
            None => compare_rect(&parity, &frame, row, strict, &mut report).await,
        }
    }
    report
}

async fn compare_rect(
    parity: &Parity,
    frame: &Frame,
    row: &Value,
    strict: bool,
    report: &mut Report,
) {
    let id = row.text("id");
    let element = parity.find(frame, &format!("#{id}")).await;
    let ours = match element.rect().await {
        Ok(rect) => rect,
        Err(error) => return report.fail(id, format!("rect failed: {error}")),
    };
    let golden = golden_rect(&row["wd"]["rect"]);
    if strict || is_empty_rect(&golden) {
        return report.check(id, rects_match(&ours, &golden), || {
            format!("rect: ours {ours:?}, chromedriver {golden:?}")
        });
    }
    match parity.atom_rect(frame, &element).await {
        Ok(atoms) => report.check(id, rects_match(&ours, &atoms), || {
            format!("rect: ours {ours:?}, the atoms in the page {atoms:?}")
        }),
        Err(error) => report.fail(id, format!("the in-page atom reference failed: {error}")),
    }
}

async fn attributes(parity: Parity) -> Report {
    let golden = golden(A_TEXT);
    let cases = golden.rows("attrs");
    let mut report = Report::against(CHROMEDRIVER);
    report.same("golden", "attribute cases", cases.len(), 14);
    let frame = parity.open("text.html").await;
    for row in cases {
        let (id, name) = (row.text("id"), row.text("name"));
        let case = format!("#{id} {name}");
        let element = parity.find(&frame, &format!("#{id}")).await;
        match element.attribute(name).await {
            Ok(value) => report.same(
                &case,
                "attribute",
                value.as_deref(),
                row["wd_attribute"].as_str(),
            ),
            Err(error) => report.fail(&case, format!("attribute failed: {error}")),
        }
        compare_property(&mut report, &case, &element, row, &parity.origin).await;
    }
    report
}

async fn compare_property(
    report: &mut Report,
    case: &str,
    element: &Element,
    row: &Value,
    origin: &str,
) {
    let name = row.text("name");
    if name == "style" {
        report.note(format!(
            "{case} property not compared (documented): chromedriver serialises the CSSStyleDeclaration \
             as {}, returnByValue gives an index map",
            row["wd_property"]
        ));
        return;
    }
    let expected = with_origin(&row["wd_property"], origin);
    match element.property(name).await {
        Ok(value) => report.same_json(case, "property", &value, &expected),
        Err(error) => report.fail(case, format!("property failed: {error}")),
    }
}

async fn clear_cases(parity: Parity) -> Report {
    let golden = golden(C_INPUT);
    let cases = input_cases(&golden, InputKind::Clear);
    let mut report = Report::against(CHROMEDRIVER);
    report.same("golden", "clear cases", cases.len(), 10);
    for row in cases {
        let name = row["case"].text("name");
        let run = run_input_case(&parity, &row["case"]).await;
        let expected = &row["wd"];
        run.compare_error(&mut report, name, expected);
        report.same_json(name, "event trace", &run.observed["log"], &expected["log"]);
        report.same_json(
            name,
            "field state",
            &run.observed["state"],
            &expected["state"],
        );
    }
    report
}

async fn typing_cases(parity: Parity) -> Report {
    let golden = golden(C_INPUT);
    let cases = input_cases(&golden, InputKind::Type);
    let mut report = Report::against(CHROMEDRIVER);
    report.same("golden", "typing cases", cases.len(), 27);
    let day_first = parity.date_fields_are_day_first().await;
    if strict_layout() && cfg!(target_os = "macos") {
        report.check("date field order", day_first, || {
            format!(
                "{LAYOUT_ENV}=strict (the default on macOS) expects the recording Mac, whose region is Germany: \
                 T13_date's golden was typed into a day-first date field, but this host orders the date fields \
                 differently; set {LAYOUT_ENV}=relative on any other host"
            )
        });
    }
    for row in cases {
        let name = row["case"].text("name");
        if name == "T13_date" && !day_first {
            report.skip(format!(
                "{name} skipped: its golden was typed into a day-first date field and this host orders the \
                 date fields differently (Chrome takes the order from the OS locale)"
            ));
            continue;
        }
        let run = run_input_case(&parity, &row["case"]).await;
        let expected = &row["wd"];
        run.compare_error(&mut report, name, expected);
        report.same(
            name,
            "event types (async select events ignored)",
            event_types(&run.observed["log"]),
            event_types(&expected["log"]),
        );
        report.same_json(
            name,
            "field state",
            &run.observed["state"],
            &expected["state"],
        );
    }
    report
}

struct InputRun {
    outcome: Result<ActionOutcome, BrowserError>,
    observed: Value,
}

impl InputRun {
    fn compare_error(&self, report: &mut Report, name: &str, expected: &Value) {
        if let Ok(ActionOutcome::DialogOpened { message, .. }) = &self.outcome {
            report.fail(name, format!("an unexpected dialog opened: {message}"));
        }
        report.same(
            name,
            "error",
            self.outcome.as_ref().err().map(w3c_error),
            expected["error"].as_str().map(golden_error_kind),
        );
    }
}

async fn run_input_case(parity: &Parity, case: &Value) -> InputRun {
    let frame = parity.open("input.html").await;
    let field = parity.find(&frame, &format!("#{}", case.text("id"))).await;
    let outcome = match case["text"].as_str() {
        Some(text) => field.send_keys(text).await,
        None => field.clear().await.map(|()| ActionOutcome::Completed),
    };
    let observed = parity.eval(&frame, COLLECT_INPUT, Vec::new()).await;
    InputRun { outcome, observed }
}

async fn script_values(parity: Parity, page_name: &str, golden_text: &str) -> Report {
    let golden = golden(golden_text);
    let rows = golden.rows("rows");
    let mut report = Report::against(CHROMEDRIVER);
    report.same("golden", "script cases", rows.len(), 51);
    let frame = parity.open(page_name).await;
    let mut comparison = ScriptComparison::new(&parity.page, &frame, rows);
    for row in rows {
        let name = row.text("name");
        let args = parity.script_args(&frame, name).await;
        let outcome = frame
            .execute_script(row.text("body"), args, ScriptOptions::USER)
            .await;
        if name == "lone_surrogate" {
            report.note(lone_surrogate_note(&outcome));
            continue;
        }
        comparison
            .compare(&mut report, name, &row["wd"], outcome)
            .await;
    }
    let alive = frame
        .execute_script("return 1", Vec::new(), ScriptOptions::PROBE)
        .await;
    report.check(
        "renderer",
        alive.as_ref().is_ok_and(|value| value.json() == &json!(1)),
        || "the page stopped answering after the script cases (renderer crash?)".to_owned(),
    );
    report
}

async fn element_screenshots(parity: Parity) -> Report {
    let golden = golden(E_SHOT);
    let rows = golden.rows("rows");
    let mut report = Report::against(CHROMEDRIVER);
    report.same("golden", "screenshot cases", rows.len(), 6);
    for (index, row) in rows.iter().enumerate() {
        let frame = parity.open("shot.html").await;
        if index == 0 {
            parity.check_viewport(&frame, &mut report).await;
        }
        compare_element_shot(&parity, &frame, row, &mut report).await;
    }
    report
}

async fn compare_element_shot(parity: &Parity, frame: &Frame, row: &Value, report: &mut Report) {
    let id = row.text("id");
    let element = parity.find(frame, &format!("#{id}")).await;
    let png = match element.screenshot_png().await {
        Ok(png) => png,
        Err(error) => return report.fail(id, format!("screenshot failed: {error}")),
    };
    let expected = &row["wd"];
    let shot = &expected["shot"];
    report.same(
        id,
        "PNG size",
        e2e::png_size(&png),
        Some((u32_at(shot, "width"), u32_at(shot, "height"))),
    );
    let scroll = parity.eval(frame, SCROLL_POSITION, Vec::new()).await;
    report.same_json(
        id,
        "scroll position after the capture",
        &scroll,
        &expected["scrollAfter"],
    );
    let clip = golden_clip(&expected["captureScreenshotParams"]["clip"]);
    let options = ScreenshotOptions {
        clip: Some(clip),
        capture_beyond_viewport: false,
    };
    match parity.page.screenshot(options).await {
        Ok(reference) => {
            let pixels = parity.compare_pngs(frame, &png, &reference).await;
            compare_pixels(report, id, &clip, &pixels, &shot["corners"]);
        }
        Err(error) => report.fail(id, format!("capturing chromedriver's clip failed: {error}")),
    }
}

fn compare_pixels(report: &mut Report, id: &str, clip: &Clip, pixels: &Value, golden: &Value) {
    let delta = pixels["delta"].as_u64();
    report.check(
        id,
        delta.is_some_and(|delta| delta <= PIXEL_TOLERANCE),
        || {
            format!(
                "the element capture differs from a capture of chromedriver's clip {clip:?} \
             (largest channel difference {delta:?}, tolerance {PIXEL_TOLERANCE})"
            )
        },
    );
    let corners = &pixels["corners"];
    report.check(id, colors_close(corners, golden), || {
        format!(
            "sampled pixels: ours {corners}, chromedriver {golden} (tolerance {PIXEL_TOLERANCE})"
        )
    });
}

async fn find_selected_source(parity: Parity) -> Report {
    let golden = golden(F_MISC);
    let mut report = Report::against(CHROMEDRIVER);
    let frame = parity.open("text.html").await;
    parity.run(&frame, F_MISC_EXTRA).await;
    compare_finds(&parity, &frame, golden.rows("finds"), &mut report).await;
    compare_selected(&parity, &frame, golden.rows("selected"), &mut report).await;
    compare_source(&parity, &frame, &golden["source"], &mut report).await;
    report
}

async fn compare_finds(parity: &Parity, frame: &Frame, finds: &[Value], report: &mut Report) {
    report.same("golden", "find cases", finds.len(), 16);
    for row in finds {
        let (using, selector) = (row.text("using"), row.text("value"));
        let case = format!("{using} {selector}");
        let ours = parity.find_outcome(frame, using, selector).await;
        let expected = &row["wd"];
        let chromedriver = match expected["found"].as_str() {
            Some(found) => format!("found {found}"),
            None => format!("error {}", expected.text("error")),
        };
        report.same(&case, "outcome", ours, chromedriver);
    }
}

async fn compare_selected(parity: &Parity, frame: &Frame, cases: &[Value], report: &mut Report) {
    report.same("golden", "selected cases", cases.len(), 9);
    for row in cases {
        let id = row.text("id");
        let element = parity.find(frame, &format!("#{id}")).await;
        match element.is_selected().await {
            Ok(value) => report.same(id, "selected", Some(value), row["wd"].as_bool()),
            Err(error) => report.fail(id, format!("is_selected failed: {error}")),
        }
    }
}

async fn compare_source(parity: &Parity, frame: &Frame, source: &Value, report: &mut Report) {
    match frame.source().await {
        Ok(ours) => {
            let outer = parity.eval(frame, OUTER_HTML, Vec::new()).await;
            report.check("source", outer.as_str() == Some(ours.as_str()), || {
                "the page source is not documentElement.outerHTML".to_owned()
            });
            report.same(
                "source",
                "length in UTF-16 units",
                Some(ours.encode_utf16().count() as u64),
                source["wd_len"].as_u64(),
            );
            let start: String = ours.chars().take(40).collect();
            report.same("source", "start", start.as_str(), source.text("starts"));
        }
        Err(error) => report.fail("source", format!("source failed: {error}")),
    }
}

async fn enter_submits_once(parity: Parity) -> Report {
    let golden = golden(H_MISC);
    let expected = &golden["submit_wd"];
    let mut report = Report::against(CHROMEDRIVER);

    let frame = parity.open("form2.html").await;
    let field = parity.find(&frame, "#q").await;
    let typed = field.send_keys("hello\n").await;
    report.check(
        "send_keys hello\\n",
        matches!(typed, Ok(ActionOutcome::Completed)),
        || format!("send_keys returned {typed:?}"),
    );
    let observed = parity.eval(&frame, COLLECT_FORM2, Vec::new()).await;
    report.same_json("send_keys hello\\n", "log and value", &observed, expected);

    let frame = parity.open("form2.html").await;
    let field = parity.find(&frame, "#q").await;
    let typed = field.send_keys("hello").await;
    report.check(
        "key_chord Enter",
        matches!(typed, Ok(ActionOutcome::Completed)),
        || format!("send_keys returned {typed:?}"),
    );
    let chord = parity
        .page
        .key_chord(Key::Named(NamedKey::Enter), Modifiers::NONE)
        .await;
    report.check(
        "key_chord Enter",
        matches!(chord, Ok(ActionOutcome::Completed)),
        || format!("key_chord returned {chord:?}"),
    );
    let observed = parity.eval(&frame, COLLECT_FORM2, Vec::new()).await;
    let submits = observed["log"].as_array().map_or(0, |log| {
        log.iter().filter(|entry| entry["t"] == "submit").count()
    });
    report.same("key_chord Enter", "submit events", submits, 1);
    report.same_json("key_chord Enter", "log and value", &observed, expected);
    report
}

async fn select_all(parity: Parity) -> Report {
    let mut report = Report::against(KEY_EXPECTATION);
    let frame = parity.open("input.html").await;
    let field = parity.find(&frame, "#i_prefilled").await;
    if let Err(error) = field.focus().await {
        report.fail("focus", format!("focus failed: {error}"));
        return report;
    }
    let modifier = if cfg!(target_os = "macos") {
        Modifiers::META
    } else {
        Modifiers::CTRL
    };
    let chord = parity.page.key_chord(Key::Char('a'), modifier).await;
    report.check(
        "select all",
        matches!(chord, Ok(ActionOutcome::Completed)),
        || format!("key_chord returned {chord:?}"),
    );
    let selection = parity.eval(&frame, PREFILLED_SELECTION, Vec::new()).await;
    report.same_json(
        "select all",
        "[selectionStart, selectionEnd, value]",
        &selection,
        &json!([0, 3, "abc"]),
    );
    let typed = parity.page.key_chord(Key::Char('z'), Modifiers::NONE).await;
    report.check(
        "replace selection",
        matches!(typed, Ok(ActionOutcome::Completed)),
        || format!("key_chord returned {typed:?}"),
    );
    let value = parity.eval(&frame, PREFILLED_VALUE, Vec::new()).await;
    report.same_json(
        "replace selection",
        "value (the selection was real and no modifier stayed held)",
        &value,
        &json!("z"),
    );
    report
}

async fn tab_focus(parity: Parity) -> Report {
    let mut report = Report::against(KEY_EXPECTATION);
    let frame = parity.open("input.html").await;
    let field = parity.find(&frame, "#i_text").await;
    if let Err(error) = field.focus().await {
        report.fail("focus", format!("focus failed: {error}"));
        return report;
    }
    let steps = [
        ("Tab", Modifiers::NONE, "i_prefilled"),
        ("Shift+Tab", Modifiers::SHIFT, "i_text"),
    ];
    for (case, modifiers, target) in steps {
        let chord = parity
            .page
            .key_chord(Key::Named(NamedKey::Tab), modifiers)
            .await;
        report.check(case, matches!(chord, Ok(ActionOutcome::Completed)), || {
            format!("key_chord returned {chord:?}")
        });
        let active = parity.eval(&frame, ACTIVE_ID, Vec::new()).await;
        report.same_json(case, "focused element", &active, &json!(target));
    }
    report
}

async fn select_by_value_cases(parity: Parity) -> Report {
    let mut report = Report::against(SELECT_RULES);
    let frame = parity.open("select.html").await;
    let single = parity.find(&frame, "#single").await;
    let multi = parity.find(&frame, "#multi").await;
    let steps: [(&str, &Element, &str, Value); 3] = [
        (
            "single select, first matching option only",
            &single,
            "b",
            json!({"log": ["single"], "state": ["single=B", "multi=X"]}),
        ),
        (
            "multiple select, every matching option is toggled on",
            &multi,
            "y",
            json!({"log": ["single", "multi", "multi"], "state": ["single=B", "multi=X,Y,Y2"]}),
        ),
        (
            "multiple select, an already selected option stays selected",
            &multi,
            "x",
            json!({"log": ["single", "multi", "multi"], "state": ["single=B", "multi=X,Y,Y2"]}),
        ),
    ];
    for (case, select, value, expected) in steps {
        if let Err(error) = select.select_by_value(value).await {
            report.fail(case, format!("select_by_value({value:?}) failed: {error}"));
            continue;
        }
        let observed = parity.eval(&frame, COLLECT_SELECT, Vec::new()).await;
        report.same_json(
            case,
            "change events and selected options",
            &observed,
            &expected,
        );
    }
    check_missing_option(&single, &mut report).await;
    report
}

async fn check_missing_option(select: &Element, report: &mut Report) {
    match select.select_by_value("nope").await {
        Err(BrowserError::NotFound { message }) => report.same(
            "missing value",
            "error",
            message.as_str(),
            "Could not find an option with value 'nope'",
        ),
        other => report.fail(
            "missing value",
            format!(
                "expected NotFound, got {:?}",
                other.map_err(|error| error.to_string())
            ),
        ),
    }
}

struct Parity {
    page: Page,
    origin: String,
}

impl Parity {
    async fn open(&self, name: &str) -> Frame {
        let url = format!("{}/{name}", self.origin);
        if let Err(error) = self.page.goto(&url).await {
            panic!("goto {url} failed: {error}");
        }
        self.page.main_frame()
    }

    async fn find(&self, frame: &Frame, css: &str) -> Element {
        let found = frame
            .find_css(css, None)
            .await
            .unwrap_or_else(|error| panic!("find {css} failed: {error}"));
        found
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("{css} matched no element"))
    }

    async fn text_element(&self, frame: &Frame, id: &str) -> Element {
        if id != "in_shadow" {
            return self.find(frame, &format!("#{id}")).await;
        }
        frame
            .execute_script(IN_SHADOW, Vec::new(), ScriptOptions::PROBE)
            .await
            .and_then(|value| value.element())
            .unwrap_or_else(|error| panic!("resolving #in_shadow failed: {error}"))
    }

    async fn eval(&self, frame: &Frame, body: &str, args: Vec<ScriptArg>) -> Value {
        frame
            .execute_script(body, args, ScriptOptions::PROBE)
            .await
            .unwrap_or_else(|error| panic!("script `{body}` failed: {error}"))
            .into_json()
    }

    async fn run(&self, frame: &Frame, body: &str) {
        if let Err(error) = frame
            .execute_script(body, Vec::new(), ScriptOptions::USER)
            .await
        {
            panic!("script `{body}` failed: {error}");
        }
    }

    async fn describe(&self, frame: &Frame, element: &Element) -> String {
        let args = vec![ScriptArg::Element(element.clone())];
        let described = self.eval(frame, DESCRIBE_NODE, args).await;
        described.as_str().unwrap_or_default().to_owned()
    }

    async fn find_outcome(&self, frame: &Frame, using: &str, selector: &str) -> String {
        let found = if using == "css selector" {
            frame.find_css(selector, None).await
        } else {
            frame.find_xpath(selector, None).await
        };
        match found {
            Ok(elements) => match elements.first() {
                Some(first) => format!("found {}", self.describe(frame, first).await),
                None => "error no such element".to_owned(),
            },
            Err(error) => format!("error {}", w3c_error(&error)),
        }
    }

    async fn script_args(&self, frame: &Frame, case: &str) -> Vec<ScriptArg> {
        if case != "element_argument" {
            return Vec::new();
        }
        let element = self.find(frame, "#t_plain").await;
        vec![ScriptArg::Element(element), ScriptArg::Json(json!(5))]
    }

    /// Typing 31122020 gives 2020-12-31 only when the day field comes first.
    async fn date_fields_are_day_first(&self) -> bool {
        let frame = self.open("input.html").await;
        self.run(&frame, ADD_DATE_PROBE).await;
        let probe = self.find(&frame, "#date_probe").await;
        if let Err(error) = probe.send_keys("31122020").await {
            panic!("typing into the date probe failed: {error}");
        }
        self.eval(&frame, DATE_PROBE_VALUE, Vec::new()).await == json!("2020-12-31")
    }

    async fn compare_pngs(&self, frame: &Frame, ours: &[u8], reference: &[u8]) -> Value {
        let encode = |png: &[u8]| ScriptArg::Json(json!(BASE64.encode(png)));
        let args = vec![encode(ours), encode(reference)];
        frame
            .execute_script(COMPARE_PNGS, args, ScriptOptions::INTERNAL)
            .await
            .unwrap_or_else(|error| panic!("decoding the captures in the page failed: {error}"))
            .into_json()
    }

    async fn check_viewport(&self, frame: &Frame, report: &mut Report) {
        let size = self.eval(frame, VIEWPORT_SIZE, Vec::new()).await;
        report.same_json(
            "viewport",
            "[innerWidth, innerHeight] (the goldens were recorded in a 1200x713 headless viewport)",
            &size,
            &json!([VIEWPORT.0, VIEWPORT.1]),
        );
    }

    async fn atom_rect(
        &self,
        frame: &Frame,
        element: &Element,
    ) -> Result<ElementRect, BrowserError> {
        let body = format!(
            "var location = ({GET_LOCATION_ATOM})(arguments[0]);\n\
             var size = ({GET_SIZE_ATOM})(arguments[0]);\n\
             return {{x: location.x, y: location.y, width: size.width, height: size.height}};"
        );
        let args = vec![ScriptArg::Element(element.clone())];
        let value = frame
            .execute_script(&body, args, ScriptOptions::INTERNAL)
            .await?;
        serde_json::from_value(value.into_json()).map_err(|error| BrowserError::Unsupported {
            message: format!("the atom rect has an unexpected shape: {error}"),
        })
    }
}

async fn run_parity<F, Fut>(test: &str, body: F)
where
    F: FnOnce(Parity) -> Fut,
    Fut: Future<Output = Report>,
{
    e2e::run(test, |ctx: E2e| async move {
        let server = FixtureServer::start().await;
        let mut options = ctx.options();
        options.window_size = VIEWPORT;
        let browser = ctx.launch_with(options).await;
        let parity = Parity {
            page: e2e::first_page(&browser).await,
            origin: server.origin.clone(),
        };
        let report = body(parity).await;
        let product = browser.version().product.clone();
        if let Err(error) = browser.close().await {
            ctx.note(format!("closing the browser failed: {error}"));
        }
        report.finish(&ctx, test, &product);
    })
    .await;
}

/// Strict mode asserts that this host is the Mac the goldens were recorded on (macOS 26 font
/// metrics, region Germany): every rect is compared with chromedriver and the date fields must be
/// day-first. It is the default on macOS only; CI sets relative on every leg.
fn strict_layout() -> bool {
    match std::env::var(LAYOUT_ENV).as_deref() {
        Ok("strict") => true,
        Ok("relative") => false,
        _ => cfg!(target_os = "macos"),
    }
}

struct FixtureServer {
    origin: String,
    task: tokio::task::JoinHandle<()>,
}

impl FixtureServer {
    async fn start() -> FixtureServer {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the fixture server");
        let port = listener
            .local_addr()
            .expect("fixture server address")
            .port();
        FixtureServer {
            origin: format!("http://127.0.0.1:{port}"),
            task: tokio::spawn(accept_requests(listener)),
        }
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn accept_requests(listener: TcpListener) {
    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(respond(stream));
    }
}

async fn respond(mut stream: TcpStream) {
    let Some(path) = request_path(&mut stream).await else {
        return;
    };
    let response = match fixture(&path) {
        Some((body, csp)) => http_response("200 OK", body, csp),
        None => http_response("404 Not Found", "not found", false),
    };
    if stream.write_all(&response).await.is_ok() {
        let _ = stream.shutdown().await;
    }
}

async fn request_path(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut chunk = [0u8; 4096];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream
            .read(&mut chunk)
            .await
            .ok()
            .filter(|read| *read > 0)?;
        head.extend_from_slice(&chunk[..read]);
        if head.len() > 64 * 1024 {
            return None;
        }
    }
    let head = String::from_utf8_lossy(&head);
    let target = head.split_whitespace().nth(1)?;
    Some(target.split(['?', '#']).next().unwrap_or(target).to_owned())
}

fn fixture(path: &str) -> Option<(&str, bool)> {
    Some(match path.trim_start_matches('/') {
        "" | "text.html" => (TEXT_HTML, false),
        "csp.html" => (TEXT_HTML, true),
        "input.html" => (INPUT_HTML, false),
        "shot.html" => (SHOT_HTML, false),
        "form2.html" => (FORM2_HTML, false),
        "select.html" => (SELECT_HTML, false),
        _ => return None,
    })
}

fn http_response(status: &str, body: &str, csp: bool) -> Vec<u8> {
    let policy = if csp {
        "Content-Security-Policy: script-src 'self' 'unsafe-inline'\r\n"
    } else {
        ""
    };
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\n{policy}Connection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

struct Report {
    reference: &'static str,
    checks: usize,
    skipped: usize,
    failures: Vec<String>,
    notes: Vec<String>,
}

impl Report {
    fn against(reference: &'static str) -> Report {
        Report {
            reference,
            checks: 0,
            skipped: 0,
            failures: Vec::new(),
            notes: Vec::new(),
        }
    }

    fn check(&mut self, case: &str, ok: bool, detail: impl FnOnce() -> String) {
        self.checks += 1;
        if !ok {
            self.failures.push(format!("{case}: {}", detail()));
        }
    }

    fn same<T: PartialEq + std::fmt::Debug>(&mut self, case: &str, what: &str, ours: T, golden: T) {
        let reference = self.reference;
        self.check(case, ours == golden, || {
            format!("{what}: ours {ours:?}, {reference} {golden:?}")
        });
    }

    fn same_json(&mut self, case: &str, what: &str, ours: &Value, golden: &Value) {
        let reference = self.reference;
        self.check(case, ours == golden, || {
            format!("{what}: ours {ours}, {reference} {golden}")
        });
    }

    fn fail(&mut self, case: &str, detail: String) {
        self.check(case, false, || detail);
    }

    fn note(&mut self, note: String) {
        self.notes.push(note);
    }

    fn skip(&mut self, note: String) {
        self.skipped += 1;
        self.note(note);
    }

    fn finish(self, ctx: &E2e, test: &str, product: &str) {
        let summary = format!(
            "{test}: {} of {} checks differ from {} ({product}); {} documented, {} skipped",
            self.failures.len(),
            self.checks,
            self.reference,
            self.notes.len(),
            self.skipped
        );
        if self.failures.is_empty() {
            for note in &self.notes {
                ctx.observe(format!("note: {note}"));
            }
            ctx.observe(summary);
            return;
        }
        for line in self.notes.iter().map(|note| format!("note: {note}")) {
            ctx.note(line);
        }
        for failure in &self.failures {
            ctx.note(format!("mismatch: {failure}"));
        }
        panic!("{summary}\n{}", self.failures.join("\n"));
    }
}

fn golden(text: &str) -> Value {
    serde_json::from_str(&replace_lone_surrogates(text)).expect("the golden file is valid JSON")
}

/// The lone_surrogate rows hold lone UTF-16 escapes, which serde_json rejects.
fn replace_lone_surrogates(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('\\') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let unit = unicode_escape(rest);
        let paired = rest.get(6..).and_then(unicode_escape);
        let taken = match (unit, paired) {
            (Some(0xD800..=0xDBFF), Some(0xDC00..=0xDFFF)) => 12,
            (Some(0xD800..=0xDFFF), _) => {
                out.push_str("\\ufffd");
                rest = &rest[6..];
                continue;
            }
            (Some(_), _) => 6,
            (None, _) => 1 + rest[1..].chars().next().map_or(0, char::len_utf8),
        };
        out.push_str(&rest[..taken]);
        rest = &rest[taken..];
    }
    out.push_str(rest);
    out
}

fn unicode_escape(text: &str) -> Option<u16> {
    let hex = text.strip_prefix("\\u")?.get(..4)?;
    u16::from_str_radix(hex, 16).ok()
}

trait Golden {
    fn text(&self, key: &str) -> &str;
    fn rows(&self, key: &str) -> &[Value];
}

impl Golden for Value {
    fn text(&self, key: &str) -> &str {
        self[key]
            .as_str()
            .unwrap_or_else(|| panic!("golden field {key} is not a string in {self}"))
    }

    fn rows(&self, key: &str) -> &[Value] {
        self[key]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_else(|| panic!("the golden has no {key} array"))
    }
}

#[derive(Clone, Copy)]
enum InputKind {
    Clear,
    Type,
}

fn input_cases(golden: &Value, kind: InputKind) -> Vec<&Value> {
    let wire = match kind {
        InputKind::Clear => "clear",
        InputKind::Type => "type",
    };
    golden
        .rows("rows")
        .iter()
        .filter(|row| row["case"]["kind"] == wire)
        .collect()
}

fn f64_at(value: &Value, key: &str) -> f64 {
    value[key]
        .as_f64()
        .unwrap_or_else(|| panic!("golden field {key} is not a number in {value}"))
}

fn u32_at(value: &Value, key: &str) -> u32 {
    value[key]
        .as_u64()
        .and_then(|number| u32::try_from(number).ok())
        .unwrap_or_else(|| panic!("golden field {key} is not a size in {value}"))
}

fn with_origin(value: &Value, origin: &str) -> Value {
    match value {
        Value::String(text) => Value::String(text.replace(GOLDEN_ORIGIN, origin)),
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| with_origin(item, origin)).collect())
        }
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, item)| (key.clone(), with_origin(item, origin)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn w3c_error(error: &BrowserError) -> &str {
    match error {
        BrowserError::Javascript { .. } => "javascript error",
        BrowserError::StaleElement | BrowserError::StaleRef { .. } => "stale element reference",
        BrowserError::NotInteractable { .. } => "element not interactable",
        BrowserError::InvalidElementState { .. } => "invalid element state",
        BrowserError::ClickIntercepted { .. } => "element click intercepted",
        BrowserError::InvalidArgument { message } if message.starts_with("invalid selector") => {
            "invalid selector"
        }
        BrowserError::InvalidArgument { .. } => "invalid argument",
        BrowserError::NotFound { .. } => "no such element",
        BrowserError::ScriptTimeout { .. } => "script timeout",
        BrowserError::DialogOpen { .. } => "unexpected alert open",
        _ => "unknown error",
    }
}

fn golden_error_kind(text: &str) -> &str {
    text.split(':').next().unwrap_or(text).trim()
}

fn event_types(log: &Value) -> Vec<String> {
    log.as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["t"] != "select")
        .map(|entry| {
            let kind = entry["t"].as_str().unwrap_or("?");
            let target = entry["id"].as_str().unwrap_or_default();
            format!("{kind}@{target}")
        })
        .collect()
}

fn golden_rect(value: &Value) -> ElementRect {
    ElementRect {
        x: f64_at(value, "x"),
        y: f64_at(value, "y"),
        width: f64_at(value, "width"),
        height: f64_at(value, "height"),
    }
}

fn is_empty_rect(rect: &ElementRect) -> bool {
    [rect.x, rect.y, rect.width, rect.height] == [0.0; 4]
}

fn rects_match(ours: &ElementRect, reference: &ElementRect) -> bool {
    [
        (ours.x, reference.x),
        (ours.y, reference.y),
        (ours.width, reference.width),
        (ours.height, reference.height),
    ]
    .iter()
    .all(|(left, right)| (left - right).abs() < RECT_TOLERANCE)
}

fn colors_close(ours: &Value, golden: &Value) -> bool {
    let channels = |hex: &str| -> Option<Vec<i64>> {
        (0..hex.len())
            .step_by(2)
            .map(|start| i64::from_str_radix(hex.get(start..start + 2)?, 16).ok())
            .collect()
    };
    let Some(golden) = golden.as_object() else {
        return false;
    };
    golden.iter().all(|(corner, expected)| {
        let ours = ours[corner].as_str().and_then(channels);
        let expected = expected.as_str().and_then(channels);
        match (ours, expected) {
            (Some(ours), Some(expected)) if ours.len() == expected.len() => ours
                .iter()
                .zip(&expected)
                .all(|(left, right)| left.abs_diff(*right) <= PIXEL_TOLERANCE),
            _ => false,
        }
    })
}

fn golden_clip(value: &Value) -> Clip {
    Clip {
        x: f64_at(value, "x"),
        y: f64_at(value, "y"),
        width: f64_at(value, "width"),
        height: f64_at(value, "height"),
        scale: f64_at(value, "scale"),
    }
}

fn lone_surrogate_note(outcome: &Result<ScriptValue, BrowserError>) -> String {
    let ours = match outcome {
        Ok(value) => format!("returns {}", value.json()),
        Err(error) => format!("fails with {error}"),
    };
    format!(
        "lone_surrogate (documented, G2 §3.7): chromedriver fails with 'cannot deserialize the result value'; ours {ours}"
    )
}

async fn check_node_names(report: &mut Report, case: &str, value: &ScriptValue) {
    let Some((_, expected)) = NODE_NAMES.iter().find(|(name, _)| *name == case) else {
        return;
    };
    let elements = if expected.len() == 1 {
        value.element().map(|element| vec![element])
    } else {
        value.elements()
    };
    let names = match elements {
        Ok(elements) => node_names(&elements).await,
        Err(error) => Err(error),
    };
    match names {
        Ok(names) => {
            let expected: Vec<String> = expected.iter().map(|name| (*name).to_owned()).collect();
            report.same(case, "nodeName of the returned nodes", names, expected);
        }
        Err(error) => report.fail(
            case,
            format!("resolving the returned nodes failed: {error}"),
        ),
    }
}

async fn node_names(elements: &[Element]) -> Result<Vec<String>, BrowserError> {
    let mut names = Vec::with_capacity(elements.len());
    for element in elements {
        let name = element.property("nodeName").await?;
        names.push(name.as_str().unwrap_or_default().to_owned());
    }
    Ok(names)
}

struct ScriptComparison {
    ours: DocumentIds,
    golden: DocumentIds,
    ours_nodes: NodeLabels,
    golden_nodes: NodeLabels,
}

impl ScriptComparison {
    fn new(page: &Page, frame: &Frame, rows: &[Value]) -> ScriptComparison {
        ScriptComparison {
            ours: DocumentIds::of(page, frame),
            golden: DocumentIds::from_golden(rows),
            ours_nodes: NodeLabels::default(),
            golden_nodes: NodeLabels::default(),
        }
    }

    async fn compare(
        &mut self,
        report: &mut Report,
        name: &str,
        expected: &Value,
        outcome: Result<ScriptValue, BrowserError>,
    ) {
        match (outcome, expected.get("ok")) {
            (Ok(value), Some(ok)) => {
                let ours = canonical(value.json(), &self.ours, &mut self.ours_nodes);
                let theirs = canonical(ok, &self.golden, &mut self.golden_nodes);
                report.same_json(name, "value", &ours, &theirs);
                check_node_names(report, name, &value).await;
            }
            (Err(error), None) => compare_script_error(report, name, expected, &error),
            (Ok(value), None) => report.fail(
                name,
                format!(
                    "returned {} where chromedriver failed with {}",
                    value.json(),
                    expected["message"]
                ),
            ),
            (Err(error), Some(ok)) => report.fail(
                name,
                format!("failed with {error} where chromedriver returned {ok}"),
            ),
        }
    }
}

fn compare_script_error(report: &mut Report, name: &str, expected: &Value, error: &BrowserError) {
    let kind = expected.text("error");
    report.same(name, "error", w3c_error(error), kind);
    if kind == "javascript error" {
        let message = error.to_string();
        report.same(name, "message", message.as_str(), expected.text("message"));
    }
}

struct DocumentIds {
    frame: String,
    loader: String,
    window: String,
}

impl DocumentIds {
    fn of(page: &Page, frame: &Frame) -> DocumentIds {
        let loader = frame
            .loader_id()
            .expect("the main frame has a committed document");
        DocumentIds {
            frame: frame.id().to_string(),
            loader: loader.to_string(),
            window: page.target_id().to_string(),
        }
    }

    fn from_golden(rows: &[Value]) -> DocumentIds {
        let ok = |name: &str| {
            rows.iter()
                .find(|row| row["name"] == name)
                .map(|row| &row["wd"]["ok"])
                .unwrap_or_else(|| panic!("the script golden has no {name} row"))
        };
        let element = ok("element")[ELEMENT_KEY]
            .as_str()
            .expect("the golden element row holds an element reference");
        let (frame, loader, _) = split_element_id(element)
            .expect("the golden element reference is f.<frame>.d.<loader>.e.<node>");
        let window = ok("window")[WINDOW_KEY]
            .as_str()
            .expect("the golden window row holds a window reference");
        DocumentIds {
            frame: frame.to_owned(),
            loader: loader.to_owned(),
            window: window.to_owned(),
        }
    }
}

#[derive(Default)]
struct NodeLabels(Vec<String>);

impl NodeLabels {
    fn label(&mut self, node: &str) -> usize {
        if let Some(index) = self.0.iter().position(|known| known == node) {
            return index;
        }
        self.0.push(node.to_owned());
        self.0.len() - 1
    }
}

fn split_element_id(id: &str) -> Option<(&str, &str, &str)> {
    let rest = id.strip_prefix("f.")?;
    let (frame, rest) = rest.split_once(".d.")?;
    let (loader, node) = rest.split_once(".e.")?;
    Some((frame, loader, node))
}

fn canonical(value: &Value, ids: &DocumentIds, nodes: &mut NodeLabels) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| canonical(item, ids, nodes))
                .collect(),
        ),
        Value::Object(object) => match single_reference(object) {
            Some((key, id)) => reference_label(key, id, ids, nodes),
            None => Value::Object(
                object
                    .iter()
                    .map(|(key, item)| (key.clone(), canonical(item, ids, nodes)))
                    .collect(),
            ),
        },
        other => other.clone(),
    }
}

fn single_reference(object: &Map<String, Value>) -> Option<(&str, &str)> {
    if object.len() != 1 {
        return None;
    }
    let (key, value) = object.iter().next()?;
    let known = [ELEMENT_KEY, SHADOW_KEY, WINDOW_KEY].contains(&key.as_str());
    known.then_some((key.as_str(), value.as_str()?))
}

fn reference_label(key: &str, id: &str, ids: &DocumentIds, nodes: &mut NodeLabels) -> Value {
    let label = if key == WINDOW_KEY {
        if id == ids.window {
            "window of the main frame".to_owned()
        } else {
            format!("foreign window {id}")
        }
    } else {
        match split_element_id(id) {
            Some((frame, loader, node)) if frame == ids.frame && loader == ids.loader => {
                format!("node #{} of the main document", nodes.label(node))
            }
            _ => format!("foreign node {id}"),
        }
    };
    let mut reference = Map::new();
    reference.insert(key.to_owned(), Value::String(label));
    Value::Object(reference)
}
