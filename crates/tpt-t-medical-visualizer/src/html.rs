//! Self-contained HTML export of the operator scene (spec.txt §6.5).
//!
//! [`render`] writes one HTML file with no external resources. The scene is
//! embedded as data, and a short script draws it as SVG and lets the operator
//! drag to rotate. The script's projection is the same formula as
//! [`View::project`], so the page and the Rust model show the same geometry.
//!
//! Every number is written in a finite, fixed form, and every string is escaped
//! for both HTML and JavaScript, because event text comes from the running
//! system and must not be able to break out of the page.

use std::fmt::Write as _;

use crate::scene::{Copy3, Scene, View};

/// Renders the scene as an HTML page of the given size in pixels.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn render(scene: &Scene, view: &View, width_px: u32, height_px: u32) -> String {
    let mut out = String::new();
    out.push_str(HEAD);
    out.push_str("<h1>tpt-teleop-medical operator view</h1>\n");
    let _ = writeln!(
        out,
        "<svg id=\"scene\" width=\"{width_px}\" height=\"{height_px}\" \
         viewBox=\"0 0 {width_px} {height_px}\" role=\"img\" aria-label=\"tool trajectory\"></svg>"
    );
    out.push_str("<aside>\n<h2>Safety copies</h2>\n<ul id=\"tmr\">\n");
    for (i, state) in scene.tmr.iter().enumerate() {
        let class = match state {
            Copy3::Agree => "agree",
            Copy3::Disagree => "disagree",
            Copy3::Unknown => "unknown",
        };
        let _ = writeln!(
            out,
            "<li class=\"{class}\">copy {} : {}</li>",
            i + 1,
            html_text(&format!("{state:?}"))
        );
    }
    out.push_str("</ul>\n<h2>Audit timeline</h2>\n<ol id=\"timeline\">\n");
    for event in &scene.events {
        let _ = writeln!(
            out,
            "<li><span class=\"t\">{:.3} s</span> {}</li>",
            event.at_us as f64 / 1e6,
            html_text(&event.text)
        );
    }
    out.push_str("</ol>\n</aside>\n");
    out.push_str("<script>\nconst DATA = ");
    out.push_str(&data_json(scene, view));
    out.push_str(";\n");
    out.push_str(SCRIPT);
    out.push_str("</script>\n</body>\n</html>\n");
    out
}

/// Renders the scene as a live page. The page reloads itself every
/// `refresh_secs` seconds, so a feed that is regenerated shows up without a
/// window toolkit. The page is [`render`] with a refresh tag added to its head.
#[must_use]
pub fn render_live(
    scene: &Scene,
    view: &View,
    width_px: u32,
    height_px: u32,
    refresh_secs: u32,
) -> String {
    const CHARSET: &str = "<meta charset=\"utf-8\">\n";
    let refresh = format!("<meta http-equiv=\"refresh\" content=\"{refresh_secs}\">\n");
    render(scene, view, width_px, height_px).replacen(CHARSET, &format!("{CHARSET}{refresh}"), 1)
}

/// The scene and view as one JavaScript object literal.
fn data_json(scene: &Scene, view: &View) -> String {
    let mut s = String::from("{");
    let _ = write!(
        s,
        "\"view\":{{\"yaw\":{},\"pitch\":{},\"pxPerM\":{},\"centre\":[{},{}],\"target\":[{},{},{}]}},",
        num(view.yaw),
        num(view.pitch),
        num(view.px_per_m),
        num(view.centre_px[0]),
        num(view.centre_px[1]),
        num(view.target_m[0]),
        num(view.target_m[1]),
        num(view.target_m[2]),
    );
    s.push_str("\"trajectory\":");
    s.push_str(&points_json(&scene.trajectory_m));
    s.push_str(",\"fixture\":");
    s.push_str(&points_json(&scene.fixture_m));
    s.push_str(",\"envelope\":");
    match scene.envelope_m {
        Some((min, max)) => {
            let _ = write!(s, "[{},{}]", point_json(min), point_json(max));
        }
        None => s.push_str("null"),
    }
    s.push_str(",\"forces\":[");
    for (i, f) in scene.forces.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(
            s,
            "{{\"at\":{},\"n\":{}}}",
            point_json(f.at_m),
            point_json(f.newtons)
        );
    }
    s.push_str("],\"events\":[");
    for (i, e) in scene.events.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(s, "{{\"t\":{},\"text\":{}}}", e.at_us, js_string(&e.text));
    }
    s.push_str("]}");
    s
}

fn points_json(points: &[[f64; 3]]) -> String {
    let body: Vec<String> = points.iter().map(|p| point_json(*p)).collect();
    format!("[{}]", body.join(","))
}

fn point_json(p: [f64; 3]) -> String {
    format!("[{},{},{}]", num(p[0]), num(p[1]), num(p[2]))
}

/// A finite number in plain decimal form. Anything that is not finite becomes
/// zero, so a bad value cannot break the page.
fn num(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.9}")
    } else {
        "0".to_string()
    }
}

/// A JavaScript string literal. Angle brackets and ampersands are written as
/// unicode escapes, so the text cannot close the script element.
fn js_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Escapes text for an HTML element body.
fn html_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

const HEAD: &str = "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<title>tpt-teleop-medical visualizer</title>\n\
<style>\n\
body { font-family: sans-serif; margin: 0; display: flex; flex-wrap: wrap; background: #fafafa; color: #222; }\n\
h1 { font-size: 1.1rem; width: 100%; margin: 0.6rem 1rem; }\n\
#scene { background: #fff; border: 1px solid #ccc; margin: 0 1rem 1rem; touch-action: none; cursor: grab; }\n\
aside { margin: 0 1rem 1rem; min-width: 14rem; }\n\
ul, ol { padding-left: 1.2rem; }\n\
li.agree { color: #2e7d32; } li.disagree { color: #c62828; } li.unknown { color: #757575; }\n\
.t { font-variant-numeric: tabular-nums; color: #555; }\n\
</style>\n</head>\n<body>\n";

const SCRIPT: &str = r##"
const NS = "http://www.w3.org/2000/svg";
const svg = document.getElementById("scene");
const view = Object.assign({}, DATA.view);
const viewBox = svg.viewBox.baseVal;

// The same orthographic projection as View::project in the Rust model.
function project(p) {
  const px = p[0] - view.target[0], py = p[1] - view.target[1], pz = p[2] - view.target[2];
  const sy = Math.sin(view.yaw), cy = Math.cos(view.yaw);
  const sp = Math.sin(view.pitch), cp = Math.cos(view.pitch);
  const x1 = cy * px - sy * py;
  const y1 = sy * px + cy * py;
  const up = cp * pz + sp * y1;
  return [view.centre[0] + view.pxPerM * x1, view.centre[1] - view.pxPerM * up];
}

function el(name, attrs) {
  const node = document.createElementNS(NS, name);
  for (const k in attrs) node.setAttribute(k, attrs[k]);
  return node;
}

function polyline(points, colour, width) {
  return el("polyline", {
    points: points.map(p => project(p).map(v => v.toFixed(2)).join(",")).join(" "),
    fill: "none", stroke: colour, "stroke-width": width,
  });
}

function draw() {
  while (svg.firstChild) svg.removeChild(svg.firstChild);
  const env = DATA.envelope;
  if (env) {
    const [lo, hi] = env;
    for (let i = 0; i < 8; i++) {
      for (let axis = 0; axis < 3; axis++) {
        const j = i | (1 << axis);
        if (j === i) continue;
        const corner = k => [0, 1, 2].map(a => ((k >> a) & 1) ? hi[a] : lo[a]);
        const a = project(corner(i)), b = project(corner(j));
        svg.appendChild(el("line", {
          x1: a[0].toFixed(2), y1: a[1].toFixed(2), x2: b[0].toFixed(2), y2: b[1].toFixed(2),
          stroke: "#9e9e9e", "stroke-width": 1,
        }));
      }
    }
  }
  if (DATA.trajectory.length > 1) svg.appendChild(polyline(DATA.trajectory, "#3d7fd0", 2));
  for (const p of DATA.fixture) {
    const q = project(p);
    svg.appendChild(el("circle", { cx: q[0].toFixed(2), cy: q[1].toFixed(2), r: 2, fill: "#e05050" }));
  }
  for (const f of DATA.forces) {
    // Arrows are drawn at 2 cm per newton, as in the window model.
    const end = [0, 1, 2].map(a => f.at[a] + f.n[a] * 0.02);
    const s = project(f.at), e = project(end);
    svg.appendChild(el("line", {
      x1: s[0].toFixed(2), y1: s[1].toFixed(2), x2: e[0].toFixed(2), y2: e[1].toFixed(2),
      stroke: "#f0a028", "stroke-width": 2,
    }));
  }
}

let dragging = false, last = [0, 0];
svg.addEventListener("pointerdown", ev => { dragging = true; last = [ev.clientX, ev.clientY]; });
window.addEventListener("pointerup", () => { dragging = false; });
window.addEventListener("pointermove", ev => {
  if (!dragging) return;
  view.yaw += (ev.clientX - last[0]) * 0.01;
  view.pitch = Math.max(-1.5, Math.min(1.5, view.pitch + (ev.clientY - last[1]) * 0.01));
  last = [ev.clientX, ev.clientY];
  draw();
});
draw();
"##;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene;

    #[test]
    fn the_page_is_self_contained() {
        let s = scene::demo();
        let v = scene::fit(&s.trajectory_m, 800.0, 600.0);
        let html = render(&s, &v, 800, 600);
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<svg id=\"scene\""));
        assert!(!html.contains("src=\"http"), "no external scripts");
        assert!(!html.contains("href=\"http"), "no external styles");
        assert!(!html.contains("NaN") && !html.contains("inf"));
    }

    #[test]
    fn event_text_cannot_break_out_of_the_page() {
        let mut s = scene::demo();
        s.events[0].text = "</script><script>alert(1)</script>".into();
        let v = scene::fit(&s.trajectory_m, 800.0, 600.0);
        let html = render(&s, &v, 800, 600);
        assert!(!html.contains("</script><script>"));
        assert!(html.contains("&lt;/script&gt;"));
        assert!(html.contains("\\u003c/script\\u003e"));
    }

    #[test]
    fn quotes_and_backslashes_are_escaped_for_javascript() {
        assert_eq!(js_string("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(js_string("line\nbreak"), "\"line\\u000abreak\"");
    }

    #[test]
    fn non_finite_numbers_become_zero() {
        assert_eq!(num(f64::NAN), "0");
        assert_eq!(num(f64::INFINITY), "0");
        assert_eq!(num(1.5), "1.500000000");
    }

    #[test]
    fn every_trajectory_point_is_embedded() {
        let s = scene::demo();
        let v = scene::fit(&s.trajectory_m, 800.0, 600.0);
        let html = render(&s, &v, 800, 600);
        let first = point_json(s.trajectory_m[0]);
        assert!(html.contains(&first), "first point {first} missing");
        let embedded = html.matches('[').count();
        assert!(embedded >= s.trajectory_m.len(), "{embedded}");
    }
}
