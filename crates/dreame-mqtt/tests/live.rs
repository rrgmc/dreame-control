//! Read-only check against the real Dreame MQTT broker.
//!
//! Runs only with `DREAME_LIVE=1` and the credentials in `DREAME_USER_EMAIL`,
//! `DREAME_PASSWORD` and optionally `DREAME_REGION`. It logs in, connects,
//! waits for CONNACK and SUBACK, listens for 20 s and disconnects. It never
//! publishes and never prints tokens, uids, device ids or payload values.

use dreame_cloud::{CloudClient, Region};
use dreame_mqtt::{ConnectionState, MqttClient, MqttConfig, Push};
use std::sync::Arc;
use std::time::Duration;

fn live() -> bool {
    std::env::var("DREAME_LIVE").is_ok_and(|v| v == "1")
}

#[tokio::test]
async fn live_listen() {
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
    let cloud = match CloudClient::login_any(&regions, &user, &pass).await {
        Ok(c) => Arc::new(c),
        Err(e) => panic!("login failed: {e}"),
    };
    let devices = cloud.list_devices().await.expect("list_devices");
    let Some(dev) = devices.into_iter().find(|d| d.is_vacuum()) else {
        eprintln!("no vacuum on this account; stopping");
        return;
    };
    let cfg = MqttConfig::from_cloud(&dev, &cloud).expect("config");
    eprintln!("broker port {}, model {}, region {}", cfg.port, cfg.model, cfg.region);

    let (handle, mut pushes) = MqttClient::connect(cfg, cloud.clone());
    let mut state = handle.state();
    let connected = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let s = state.borrow_and_update().clone();
            match s {
                ConnectionState::Connected => return Ok(()),
                ConnectionState::Waiting { reason, .. } => return Err(reason),
                ConnectionState::Stopped => return Err("stopped".to_string()),
                ConnectionState::Connecting { .. } => {}
            }
            if state.changed().await.is_err() {
                return Err("task ended".to_string());
            }
        }
    })
    .await;
    match connected {
        Ok(Ok(())) => eprintln!("CONNACK 0 and SUBACK granted"),
        Ok(Err(reason)) => panic!("connection failed: {reason}"),
        Err(_) => panic!("not connected within 30 s"),
    }

    let mut n = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while let Ok(Some(p)) = tokio::time::timeout_at(deadline, pushes.recv()).await {
        n += 1;
        let kind = match &p {
            Push::PropertiesChanged(v) => format!("properties {:?}", v.iter().map(|(id, _)| id.to_string()).collect::<Vec<_>>()),
            Push::Event { siid, eiid, .. } => format!("event {siid}.e{eiid}"),
            Push::Props(m) => format!("props {:?}", m.keys().collect::<Vec<_>>()),
            Push::OtcInfo { .. } => "otc info".to_string(),
            Push::Other(_) => "other".to_string(),
        };
        eprintln!("push: {kind}");
    }
    eprintln!("{n} pushes in 20 s");
    assert_eq!(*handle.state().borrow(), ConnectionState::Connected, "connection dropped while listening");

    let mut state = handle.state();
    handle.stop().await;
    assert_eq!(*state.borrow_and_update(), ConnectionState::Stopped);
    assert!(pushes.recv().await.is_none(), "stream should end after stop");
    eprintln!("disconnected");
}
