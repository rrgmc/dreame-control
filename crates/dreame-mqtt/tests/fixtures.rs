//! Decodes real MQTT captures from the research spike.
//!
//! Runs only when `DREAME_FIXTURES` points at the capture directory
//! (`local/out/`), which holds `mqtt-clean.jsonl` and `mqtt-idle.jsonl`.
//! Each line is `{"t","topic_matches","payload"}`. The captures contain real
//! identifiers, so they stay outside the repository and nothing from them is
//! printed.

use dreame_mqtt::{decode_message, PropId, Push};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn fixtures() -> Option<PathBuf> {
    std::env::var_os("DREAME_FIXTURES").map(PathBuf::from)
}

fn decode_file(path: &PathBuf) -> Vec<Push> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
        let rec: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("line {}: {e}", n + 1));
        let payload = serde_json::to_vec(&rec["payload"]).unwrap();
        out.extend(decode_message(&payload).unwrap_or_else(|e| panic!("line {}: {e}", n + 1)));
    }
    out
}

struct Counts {
    props: BTreeMap<PropId, usize>,
    events: BTreeMap<(u32, u32), usize>,
    otc: usize,
    props_msgs: usize,
    other: usize,
}

fn count(pushes: &[Push]) -> Counts {
    let mut c = Counts { props: BTreeMap::new(), events: BTreeMap::new(), otc: 0, props_msgs: 0, other: 0 };
    for p in pushes {
        match p {
            Push::PropertiesChanged(v) => {
                for (id, _) in v {
                    *c.props.entry(*id).or_default() += 1;
                }
            }
            Push::Event { siid, eiid, .. } => *c.events.entry((*siid, *eiid)).or_default() += 1,
            Push::OtcInfo { .. } => c.otc += 1,
            Push::Props(_) => c.props_msgs += 1,
            Push::Other(_) => c.other += 1,
        }
    }
    c
}

#[test]
fn decode_captures() {
    let Some(dir) = fixtures() else {
        eprintln!("skipped: set DREAME_FIXTURES to the spike output directory");
        return;
    };

    let clean = decode_file(&dir.join("mqtt-clean.jsonl"));
    let c = count(&clean);
    eprintln!(
        "mqtt-clean: {} pushes, {} property ids, events {:?}, otc {}, props {}, other {}",
        clean.len(),
        c.props.len(),
        c.events,
        c.otc,
        c.props_msgs,
        c.other
    );
    assert_eq!(c.props.get(&PropId::new(6, 1)), Some(&122), "6.1 map frames");
    for ev in [(2, 3), (4, 4), (4, 3), (4, 1)] {
        assert!(c.events.contains_key(&ev), "event {}.e{} missing", ev.0, ev.1);
    }
    assert_eq!(c.otc, 3);
    assert_eq!(c.other, 0);

    // The cleaning record carries the documented arguments.
    let record = clean
        .iter()
        .find_map(|p| match p {
            Push::Event { siid: 4, eiid: 1, arguments } => Some(arguments),
            _ => None,
        })
        .unwrap();
    let piids: Vec<u32> = record.iter().map(|(p, _)| *p).collect();
    for p in [1, 2, 3, 13, 8, 9, 10] {
        assert!(piids.contains(&p), "4.e1 argument {p} missing");
    }
    assert!(clean.iter().all(|p| match p {
        Push::OtcInfo { fw_ver, rssi, local_ip, .. } => fw_ver.is_some() && rssi.is_some() && local_ip.is_some(),
        _ => true,
    }));

    let idle = decode_file(&dir.join("mqtt-idle.jsonl"));
    let c = count(&idle);
    eprintln!("mqtt-idle: {} pushes, property ids {:?}", idle.len(), c.props.keys().map(|p| p.to_string()).collect::<Vec<_>>());
    assert_eq!(idle.len(), 9);
    assert!(idle.iter().all(|p| matches!(p, Push::PropertiesChanged(v) if !v.is_empty())));
}
