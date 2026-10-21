//! #5196: Mixer Brush previews are real pixels, with one command and history step on release.

use photocraft_engine::{Session, brush_cmds::LiveMixer};
use photocraft_geom::Rect;
use photocraft_paint::StrokePoint;
use serde_json::{Value, json};

fn session(depth: u8, mode: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 128, "height": 64, "depth": depth, "mode": mode})).unwrap();
    s.execute("edit.fill", json!({"color": "#2040df"})).unwrap();
    s.execute("tools.setBrush", json!({"brush": {"size": 18, "hardness": 0.7, "smoothing": {"amount": 0.0}, "pressureSize": false}})).unwrap();
    s
}

fn pixels(doc: &photocraft_doc::Document) -> Vec<f32> {
    doc.layers[0].surface().unwrap().read_region(Rect::new(0, 0, 128, 64))
}

fn full_params(all: bool) -> Value {
    json!({"points": [[12,32,1],[60,32,0.6],[112,32,1]], "color": "#ef2000", "wet": 80, "mix": 75,
        "load": 30, "flow": 60, "seed": 57, "sampleAllLayers": all, "cleanAfterStroke": false})
}

#[test]
fn live_mixer_matches_command_pixels_and_reservoir_at_every_depth_and_mode() {
    for depth in [8, 16, 32] {
        for mode in ["rgb", "gray", "cmyk", "lab"] {
            for all in [false, true] {
                let mut s = session(depth, mode);
                let p = full_params(all);
                let before = pixels(&s.active().unwrap().doc);
                let initial_state = s.tools.mixer.clone();
                let mut start = p.clone();
                start["points"] = json!([[12, 32, 1]]);
                let mut live = LiveMixer::begin(&s, &start).unwrap();
                assert_ne!(pixels(&live.doc), before, "a press paints before release");
                live.push(&[StrokePoint::new(60.0, 32.0, 0.6)]).unwrap();
                live.push(&[StrokePoint::new(112.0, 32.0, 1.0)]).unwrap();
                assert_eq!(pixels(&s.active().unwrap().doc), before, "preview doesn't edit the session");
                assert_eq!(s.tools.mixer, initial_state, "preview doesn't consume session paint");
                let mut oracle = session(depth, mode);
                oracle.execute("paint.mixerBrush", p.clone()).unwrap();
                s.offer_live_mixer(live).unwrap();
                let result = s.execute("paint.mixerBrush", p).unwrap();
                assert_eq!(result["fromLive"], true);
                assert_eq!(pixels(&s.active().unwrap().doc), pixels(&oracle.active().unwrap().doc), "{depth} {mode} all={all}");
                assert_eq!(s.tools.mixer, oracle.tools.mixer);
                assert_eq!(s.active().unwrap().history.undo_label(), Some("Mixer Brush"));
                s.execute("edit.undo", json!({})).unwrap();
                assert_eq!(pixels(&s.active().unwrap().doc), before);
                s.execute("edit.redo", json!({})).unwrap();
                assert_eq!(pixels(&s.active().unwrap().doc), pixels(&oracle.active().unwrap().doc));
            }
        }
    }
}

#[test]
fn invalid_live_mixer_points_leave_preview_and_session_unchanged() {
    let s = session(8, "rgb");
    let mut p = full_params(false);
    p["points"] = json!([[12, 32, 1]]);
    let mut live = LiveMixer::begin(&s, &p).unwrap();
    let before = pixels(&live.doc);
    assert!(live.push(&[StrokePoint::new(f64::NAN, 32.0, 1.0)]).is_err());
    assert_eq!(pixels(&live.doc), before);
    assert!(LiveMixer::begin(&s, &json!({"points": []})).is_err());
    assert!(LiveMixer::begin(&Session::new(), &p).is_err());
}

#[test]
fn changed_mixer_settings_or_document_refuse_the_offer() {
    let mut s = session(8, "rgb");
    let p = full_params(false);
    let live = LiveMixer::begin(&s, &p).unwrap();
    s.execute("tools.setBrush", json!({"brush": {"size": 30}})).unwrap();
    assert!(s.offer_live_mixer(live).is_err());
    let live = LiveMixer::begin(&s, &p).unwrap();
    s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
    s.offer_live_mixer(live).unwrap();
    let result = s.execute("paint.mixerBrush", p).unwrap();
    assert!(result.get("fromLive").is_none(), "changed revision falls back to the command");
}

#[test]
fn changed_commit_points_cannot_take_stale_preview_pixels() {
    let mut s = session(8, "rgb");
    let p = full_params(false);
    let live = LiveMixer::begin(&s, &p).unwrap();
    s.offer_live_mixer(live).unwrap();
    let mut changed = p;
    changed["points"] = json!([[12, 12, 1], [112, 12, 1]]);
    let result = s.execute("paint.mixerBrush", changed).unwrap();
    assert!(result.get("fromLive").is_none());
}

#[test]
fn live_mixer_tail_matches_smoothing_dynamics_and_selection() {
    for amount in [0.0, 0.6] {
        let setup = || {
            let mut s = session(16, "rgb");
            s.execute(
                "tools.setBrush",
                json!({"brush":{"opacity":0.3,"smoothing":{"amount":amount},
                "shapeDynamics":{"enabled":true,"size":{"jitter":0.2}},
                "transfer":{"enabled":true,"wetness":{"jitter":0.3},"mix":{"jitter":0.2}}}}),
            )
            .unwrap();
            s.execute("select.rect", json!({"x":40,"y":0,"width":40,"height":64})).unwrap();
            s
        };
        let mut s = setup();
        let p = full_params(false);
        let mut start = p.clone();
        start["points"] = json!([[12, 32, 1]]);
        let mut live = LiveMixer::begin(&s, &start).unwrap();
        live.push(&[StrokePoint::new(60.0, 32.0, 0.6), StrokePoint::new(112.0, 32.0, 1.0)]).unwrap();
        let shown = pixels(&live.doc);
        let mut oracle = setup();
        oracle.execute("paint.mixerBrush", p.clone()).unwrap();
        assert_eq!(shown, pixels(&oracle.active().unwrap().doc), "finishing tail is shown without consuming paint");
        s.offer_live_mixer(live).unwrap();
        assert_eq!(s.execute("paint.mixerBrush", p).unwrap()["fromLive"], true);
        assert_eq!(pixels(&s.active().unwrap().doc), shown);
        assert_eq!(s.tools.mixer, oracle.tools.mixer);
    }
}

#[test]
#[ignore = "release benchmark on a 24 MP synthetic canvas"]
fn live_mixer_24mp_benchmark() {
    use std::time::Instant;
    let mut s = Session::new();
    s.execute("file.new", json!({"width":6000,"height":4000,"depth":16})).unwrap();
    s.execute("tools.setBrush", json!({"brush":{"size":60,"hardness":0.7,"smoothing":{"amount":0.0}}})).unwrap();
    let params = json!({"points":[[100,2000,1]],"wet":70,"mix":80,"seed":57});
    let points: Vec<_> = (0..100).map(|i| StrokePoint::new(100.0 + i as f64 * 10.0, 2000.0, 1.0)).collect();
    let mut baseline = Vec::new();
    let mut incremental = Vec::new();
    let mut releases = Vec::new();
    for _ in 0..5 {
        let mut oracle = Session::new();
        oracle.execute("file.new", json!({"width":6000,"height":4000,"depth":16})).unwrap();
        oracle.tools.brush = s.tools.brush.clone();
        let mut p = params.clone();
        p["points"] = json!(points);
        let t = Instant::now();
        oracle.execute("paint.mixerBrush", p.clone()).unwrap();
        baseline.push(t.elapsed().as_secs_f64() * 1000.0);
        let mut live = LiveMixer::begin(&s, &params).unwrap();
        for point in &points[1..] {
            let t = Instant::now();
            live.push(&[*point]).unwrap();
            incremental.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let t = Instant::now();
        s.offer_live_mixer(live).unwrap();
        assert_eq!(s.execute("paint.mixerBrush", p).unwrap()["fromLive"], true);
        releases.push(t.elapsed().as_secs_f64() * 1000.0);
        s.execute("edit.undo", json!({})).unwrap();
    }
    for (label, values) in
        [("full stroke on release (before)", &mut baseline), ("live pointer step (after)", &mut incremental), ("reuse on release (after)", &mut releases)]
    {
        values.sort_by(f64::total_cmp);
        eprintln!("24 MP {label}: median {:.3} ms, p95 {:.3} ms", values[values.len() / 2], values[(values.len() * 95 / 100).min(values.len() - 1)]);
    }
}

/// Refreshing the live path must retain the automatic Load/Clean settings added upstream.
#[test]
fn live_mixer_keeps_automatic_load_clean_and_explicit_paint_across_strokes() {
    for depth in [8, 16, 32] {
        for clean in [false, true] {
            for load in [false, true] {
                let setup = || {
                    let mut s = session(depth, "rgb");
                    s.execute("tools.setColors", json!({"foreground":"#ef2000"})).unwrap();
                    s.execute("tools.setBrush", json!({"brush":{"mixer":{"cleanAfterStroke":clean,"loadAfterStroke":load}}})).unwrap();
                    s.execute("brush.mixer.load", json!({"color":"#00ff00"})).unwrap();
                    s
                };
                let mut live_session = setup();
                let mut oracle = setup();
                let mut p = full_params(false);
                p.as_object_mut().unwrap().remove("cleanAfterStroke");
                for _ in 0..2 {
                    let live = LiveMixer::begin(&live_session, &p).unwrap();
                    live_session.offer_live_mixer(live).unwrap();
                    assert_eq!(live_session.execute("paint.mixerBrush", p.clone()).unwrap()["fromLive"], true);
                    oracle.execute("paint.mixerBrush", p.clone()).unwrap();
                    assert_eq!(pixels(&live_session.active().unwrap().doc), pixels(&oracle.active().unwrap().doc));
                    assert_eq!(live_session.tools.mixer, oracle.tools.mixer);
                    let state = &live_session.tools.mixer;
                    assert_eq!(state.color, Some(live_session.tools.foreground));
                    if load {
                        assert_eq!(state.reservoir, Some(live_session.tools.foreground));
                        assert_eq!(state.amount, 1.0);
                    } else if clean {
                        assert_eq!((state.reservoir, state.pickup, state.amount), (None, None, 0.0));
                    } else {
                        assert_eq!(state.reservoir, Some([0.0, 1.0, 0.0, 1.0]));
                        assert!(state.amount < 1.0);
                    }
                }
            }
        }
    }
}
