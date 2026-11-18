//! Read-only checks against the real Dreamehome cloud.
//!
//! Runs only with `DREAME_LIVE=1` and the credentials in `DREAME_USER_EMAIL`,
//! `DREAME_PASSWORD` and optionally `DREAME_REGION`. It never writes to the
//! robot and never prints tokens, passwords, device ids, uids, MACs or serials.

use dreame_cloud::{CloudClient, CloudConfig, MemoryTokenStore, PropId, Region, TokenStore};
use std::sync::Arc;

fn live() -> bool {
    std::env::var("DREAME_LIVE").is_ok_and(|v| v == "1")
}

#[tokio::test]
async fn live_read_only() {
    if !live() {
        eprintln!("skipped: set DREAME_LIVE=1 to run");
        return;
    }
    let user = std::env::var("DREAME_USER_EMAIL").expect("DREAME_USER_EMAIL not set");
    let pass = std::env::var("DREAME_PASSWORD").expect("DREAME_PASSWORD not set");
    let regions: Vec<Region> = match std::env::var("DREAME_REGION") {
        Ok(r) if !r.trim().is_empty() => vec![Region::parse(&r)],
        _ => Vec::new(),
    };

    let store = Arc::new(MemoryTokenStore::new());
    let cfg = CloudConfig::default().with_token_store(store.clone());
    let client = match CloudClient::login_any_with(cfg, &regions, &user, &pass).await {
        Ok(c) => c,
        Err(e) => panic!("login failed: {e}"),
    };
    eprintln!("login ok: region {}, country {:?}, expires in {:?}", client.region(), client.country(), client.expires_in());
    assert!(!client.uid().is_empty());
    assert!(store.load().is_some());

    let devices = client.list_devices().await.expect("list_devices");
    eprintln!("devices: {}", devices.len());
    let Some(dev) = devices.iter().find(|d| d.is_vacuum()).cloned() else {
        eprintln!("no vacuum on this account; stopping");
        return;
    };
    eprintln!("vacuum: model {}, firmware {}, online {}", dev.model, dev.firmware, dev.online);
    assert!(dev.bind_domain.is_some());

    let cached = client
        .cached_properties(&dev.did, &[PropId::new(2, 1), PropId::new(3, 1), PropId::new(6, 3)])
        .await
        .expect("cached_properties");
    for c in &cached {
        if c.id == PropId::new(6, 3) {
            // The object name contains the uid and did, so print only its shape.
            let ok = c.value.as_str().is_some_and(|s| s.starts_with("ali_dreame/"));
            eprintln!("cached 6.3: object name present: {ok}");
        } else {
            eprintln!("cached {}: {:?}", c.id, c.value);
        }
    }
    assert!(cached.iter().any(|c| c.id == PropId::new(2, 1)));

    match client.get_properties(&dev, &[PropId::new(2, 1), PropId::new(3, 1)]).await {
        Ok(rs) => {
            for r in &rs {
                eprintln!("relayed {}: code {}, value {:?}", r.id, r.code, r.value);
            }
            assert_eq!(rs.len(), 2);
        }
        // The robot may be asleep; that is an outcome, not a client bug.
        Err(dreame_cloud::Error::NoAck) => eprintln!("relayed get_properties: robot did not acknowledge (80001)"),
        Err(e) => panic!("get_properties: {e}"),
    }

    let before = client.access_token();
    let rt_before = store.load().unwrap().refresh_token;
    client.refresh().await.expect("refresh");
    let rotated = store.load().unwrap().refresh_token != rt_before;
    eprintln!("refresh ok: access token changed {}, refresh token rotated {rotated}", client.access_token() != before);
    client.list_devices().await.expect("list_devices after refresh");

    // Resume from the stored session, as an app would after a restart.
    let stored = store.load().unwrap();
    let cfg = CloudConfig::default().with_token_store(store.clone());
    let resumed = CloudClient::from_refresh_token(cfg, stored).await.expect("from_refresh_token");
    let n = resumed.list_devices().await.expect("list_devices after resume").len();
    eprintln!("resumed session ok: region {}, devices {n}", resumed.region());
}

/// Two open protocol questions (l40-ultra-ae.md, "Not yet tested"). Both send
/// only cloud reads and token requests; neither touches the robot. They print
/// the outcome instead of asserting it, since either answer is a finding.
#[tokio::test]
async fn live_unsigned_body_and_old_refresh_token() {
    if !live() {
        eprintln!("skipped: set DREAME_LIVE=1 to run");
        return;
    }
    let user = std::env::var("DREAME_USER_EMAIL").expect("DREAME_USER_EMAIL not set");
    let pass = std::env::var("DREAME_PASSWORD").expect("DREAME_PASSWORD not set");
    let regions: Vec<Region> = match std::env::var("DREAME_REGION") {
        Ok(r) if !r.trim().is_empty() => vec![Region::parse(&r)],
        _ => Vec::new(),
    };
    let store = Arc::new(MemoryTokenStore::new());
    let cfg = CloudConfig::default().with_token_store(store.clone());
    let client = CloudClient::login_any_with(cfg, &regions, &user, &pass).await.expect("login");

    // 1. The device list without `sign` and `timestamp`.
    let body = serde_json::json!({"sharedStatus": 1, "current": 1, "size": 100, "lang": "en"});
    match client.post_api_unsigned("/dreame-user-iot/iotuserbind/device/listV2", body).await {
        Ok(_) => eprintln!("unsigned body: accepted"),
        Err(e) => eprintln!("unsigned body: rejected: {e}"),
    }

    // 2. Refresh, then try the refresh token from before the refresh.
    let old = store.load().unwrap();
    client.refresh().await.expect("refresh");
    let new_rt = store.load().unwrap().refresh_token;
    eprintln!("refresh token rotated: {}", new_rt != old.refresh_token);
    let cfg = CloudConfig::default().with_token_store(Arc::new(MemoryTokenStore::new()));
    match CloudClient::from_refresh_token(cfg, old).await {
        Ok(c) => {
            let ok = c.list_devices().await.is_ok();
            eprintln!("old refresh token: accepted; device list with it works: {ok}");
        }
        Err(e) => eprintln!("old refresh token: rejected: {e}"),
    }
    // Does the newest refresh token still work after the old one was used?
    match client.refresh().await {
        Ok(()) => {
            let ok = client.list_devices().await.is_ok();
            eprintln!("newest refresh token after the probe: accepted; device list works: {ok}");
        }
        Err(e) => eprintln!("newest refresh token after the probe: rejected: {e}"),
    }
}
