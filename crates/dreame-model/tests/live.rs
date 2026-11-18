//! Read-only discovery against the real robot.
//!
//! Runs only with `DREAME_LIVE=1`. Credentials come from `DREAME_USER_EMAIL`,
//! `DREAME_PASSWORD` and optionally `DREAME_REGION`. If `DREAME_ENV_FILE`
//! names a `KEY=VALUE` file (such as `local/env`), missing variables are
//! loaded from it. Values are never printed.
//!
//! The test only calls `get_properties` (relayed) and the cloud cache. It
//! prints a redacted summary: property ids, numbers and booleans, and the
//! length of any string, never strings themselves, since they can hold
//! device ids, account ids, serials, MACs, IPs, SSIDs or room names.

use dreame_cloud::{CloudClient, Region};
use dreame_core::{PropId, RawValue};
use dreame_model::discovery::CloudReader;
use dreame_model::{discover, DiscoverOptions, Probe, Profile, Status};
use std::time::Duration;

fn load_env_file() {
    let Ok(path) = std::env::var("DREAME_ENV_FILE") else { return };
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("DREAME_ENV_FILE is set but unreadable");
        return;
    };
    for line in text.lines() {
        let line = line.trim_end_matches('\r').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim().trim_start_matches("export ").trim();
            if std::env::var_os(k).is_none() {
                std::env::set_var(k, v.trim().trim_matches('"'));
            }
        }
    }
}

fn redact(v: &RawValue) -> String {
    match v {
        RawValue::Int(i) => i.to_string(),
        RawValue::Bool(b) => b.to_string(),
        RawValue::Float(f) => f.to_string(),
        RawValue::Str(s) => format!("<text, {} chars>", s.chars().count()),
        RawValue::Json(j) => format!("<json {}>", if j.is_array() { "list" } else { "object" }),
    }
}

fn ids(list: &[PropId]) -> String {
    list.iter().map(PropId::to_string).collect::<Vec<_>>().join(" ")
}

#[tokio::test]
async fn live_discovery_read_only() {
    if std::env::var("DREAME_LIVE").map_or(true, |v| v != "1") {
        eprintln!("skipped: set DREAME_LIVE=1 to run");
        return;
    }
    load_env_file();
    let user = std::env::var("DREAME_USER_EMAIL").expect("DREAME_USER_EMAIL not set");
    let pass = std::env::var("DREAME_PASSWORD").expect("DREAME_PASSWORD not set");
    let regions: Vec<Region> = match std::env::var("DREAME_REGION") {
        Ok(r) if !r.trim().is_empty() => vec![Region::parse(r.trim())],
        _ => Vec::new(),
    };
    let client = CloudClient::login_any(&regions, &user, &pass).await.unwrap_or_else(|e| panic!("login failed: {e}"));
    let devices = client.list_devices().await.expect("list_devices");
    let dev = devices.into_iter().find(|d| d.is_vacuum()).expect("no vacuum on the account");
    eprintln!("model {}, firmware {}, online {}", dev.model, dev.firmware, dev.online);

    let profile = Profile::for_model(&dev.model);
    let opts = DiscoverOptions { delay: Duration::from_millis(400), ..Default::default() };
    let started = std::time::Instant::now();
    let reader = CloudReader::new(&client, &dev);
    let mut caps = discover(&reader, &profile, &dev.model, &dev.firmware, &opts).await.expect("discover");
    eprintln!("discovery: {} ids probed in {:?}", caps.probes.len(), started.elapsed());
    for i in reader.issues() {
        eprintln!("reply item without siid/piid: index {}, assumed {:?}, keys {:?}, code {:?}", i.index, i.assumed.map(|p| p.to_string()), i.keys, i.code);
    }

    // The cloud cache, for "exists but not readable" evidence.
    let cache = client.cached_properties(&dev.did, &profile.prop_ids()).await.expect("cached_properties");
    caps.merge_cache(cache.iter().map(|c| c.id));

    let (ok, minus1, other, none) = caps.counts();
    eprintln!("codes: 0 = {ok}, -1 = {minus1}, other = {other}, no answer = {none}; cached = {}", caps.cached.len());
    let by = |pred: &dyn Fn(&Probe) -> bool| -> Vec<PropId> {
        caps.probes.iter().filter(|(_, p)| pred(p)).map(|(id, _)| *id).collect()
    };
    let readable = by(&|p| matches!(p, Probe::Value(_) | Probe::Empty));
    let not_readable = by(&|p| matches!(p, Probe::NotReadable));
    eprintln!("code 0: {}", ids(&readable));
    eprintln!("code -1: {}", ids(&not_readable));
    for (id, p) in &caps.probes {
        match p {
            Probe::Code(c) => eprintln!("code {c}: {id}"),
            Probe::NoAnswer(e) => eprintln!("no answer: {id} ({e})"),
            _ => {}
        }
    }
    let cached_unread: Vec<PropId> = caps.cached.iter().copied().filter(|id| !readable.contains(id)).collect();
    eprintln!("in cache but not code 0: {}", ids(&cached_unread));
    let read_uncached: Vec<PropId> = readable.iter().copied().filter(|id| !caps.cached.contains(id)).collect();
    eprintln!("code 0 but not in cache: {}", ids(&read_uncached));
    for (id, v) in caps.values() {
        let name = profile.prop(id).map(|d| d.name).unwrap_or("?");
        eprintln!("  {id} {name} = {}", redact(&v));
    }
    let on: Vec<String> = caps.features.iter().filter(|(_, v)| **v).map(|(f, _)| format!("{f:?}")).collect();
    let off: Vec<String> = caps.features.iter().filter(|(_, v)| !**v).map(|(f, _)| format!("{f:?}")).collect();
    eprintln!("features on: {}", on.join(", "));
    eprintln!("features off: {}", off.join(", "));
    eprintln!("model flags: {}", caps.model_flags.as_ref().map_or(0, |f| f.len()));

    let values = caps.values();
    let s = Status::from_values(&values, caps.decode_context());
    eprintln!("status:");
    eprintln!("  state {:?}, status {:?}", s.state.map(|x| x.to_string()), s.status.map(|x| x.to_string()));
    eprintln!("  task {:?} / {:?}", s.task_status.map(|x| x.to_string()), s.task_type.map(|x| x.to_string()));
    eprintln!("  battery {:?}, charging {:?}", s.battery, s.charging.map(|x| x.to_string()));
    eprintln!("  error {:?}, faults {:?}, active {:?}", s.error.map(|x| x.to_string()), s.faults.as_ref().map(|f| &f.codes), s.active_faults());
    eprintln!("  last job {:?} min, {:?} m2", s.cleaning_time_min, s.cleaned_area_m2);
    eprintln!("  suction {:?}, wetness {:?}, mode {:?} (4.23 {:?})", s.suction.map(|x| x.to_string()), s.wetness, s.logical_cleaning_mode().map(|x| x.to_string()), s.cleaning_mode.map(|g| g.raw));
    eprintln!("  dnd {:?}", s.dnd);
    eprintln!("  consumables {:?}", s.consumables);
    eprintln!("  station {:?}", s.station);
    eprintln!("  settings keys {}", s.settings.as_ref().map_or(0, |x| x.values.len() + x.other.len()));
    // Only ids: error messages can quote the value.
    let errs: Vec<PropId> = s.decode_errors.iter().map(|(id, _)| *id).collect();
    eprintln!("  decode errors at {}", ids(&errs));
    assert!(ok > 0, "no property answered");
}
