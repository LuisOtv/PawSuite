//! Projetos editados à mão ou estragados: números absurdos nos campos não podem derrubar o
//! programa quando o arquivo abre. Ou é recusado, ou todas as contas da timeline funcionam.

mod common;

use common::*;
use pawcut_core::*;
use serde_json::{Value, json};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn numbers<'a>(v: &'a mut Value, out: &mut Vec<&'a mut Value>) {
    match v {
        Value::Number(_) => out.push(v),
        Value::Array(a) => a.iter_mut().for_each(|x| numbers(x, out)),
        Value::Object(o) => o.values_mut().for_each(|x| numbers(x, out)),
        _ => {}
    }
}

fn sample() -> String {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let (vc, _) = place_av(&mut ed, m, 10, 5.0, 100);
    let v1 = v(&ed, 0);
    place(&mut ed, v1, solid(), 200, 0.0, 50);
    ed.apply(Edit::SetKeyframe {
        clip: vc,
        param: Param::Opacity,
        at: 20.0,
        value: Some(0.5),
    })
    .unwrap();
    ed.apply(Edit::SetClip {
        clip: vc,
        change: ClipChange::FadeIn(10),
    })
    .unwrap();
    ed.apply(Edit::SetMarker {
        frame: 30,
        marker: Some(Marker {
            frame: 30,
            color: Rgba::new(1, 2, 3, 255),
            note: "x".into(),
        }),
    })
    .unwrap();
    to_json(ed.project()).unwrap()
}

/// Todas as contas que o app faz com um projeto aberto.
fn exercise(p: &Project) {
    let _ = p.duration();
    let _ = p.edit_points();
    for t in p.all_tracks() {
        for c in &t.clips {
            let mid = c.start.saturating_add(c.duration / 2);
            let _ = (
                c.end(),
                c.source_time_at(mid),
                c.transform_at(mid),
                c.color_at(mid),
            );
            let _ = c.dissolve_window();
            for param in [Param::X, Param::Scale, Param::Opacity, Param::VolumeDb] {
                let _ = c.param(param, mid);
            }
        }
        let _ = t.layers_at(10);
        let _ = t.clip_at(10);
    }
}

#[test]
fn absurd_numbers_in_a_project_file_are_refused_or_harmless() {
    let good = sample();
    let mut rng = Rng(2024);
    let extremes = [
        json!(0),
        json!(-1),
        json!(1),
        json!(1e30),
        json!(-1e30),
        json!(i64::MAX),
        json!(i64::MIN),
        json!(u64::MAX),
        json!(1e-30),
        json!(0.0),
        json!(2),
    ];
    let mut opened = 0;
    for round in 0..4000 {
        let mut v: Value = serde_json::from_str(&good).unwrap();
        let mut nums = Vec::new();
        numbers(&mut v, &mut nums);
        for _ in 0..1 + rng.below(3) {
            let i = rng.below(nums.len());
            *nums[i] = extremes[rng.below(extremes.len())].clone();
        }
        let text = v.to_string();
        let r = std::panic::catch_unwind(|| {
            if let Ok(p) = from_json(&text) {
                exercise(&p);
                true
            } else {
                false
            }
        });
        match r {
            Ok(true) => opened += 1,
            Ok(false) => {}
            Err(_) => panic!("rodada {round}: o projeto abriu/usou e deu pânico: {text}"),
        }
    }
    assert!(
        opened > 100,
        "poucos arquivos abriram ({opened}): o teste não exercita nada"
    );
}
