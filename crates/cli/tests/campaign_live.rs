//! Real CLI process through four protocol endpoints; local HTTP fixture only.
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn campaign_cli_submits_accepted_official_plans() {
    let mut server = mockito::Server::new_async().await;
    let mut cells = vec![vec![0; 8]; 8];
    cells[1][1] = 1;
    cells[1][2] = 2;
    let raw = json!({"startsAt":1,"daySeconds":[30,30,30],"daySteps":[16,20,18],
        "map":{"height":8,"width":8,"cells":cells},
        "spots":[{"brand":0,"pos":9,"stocks":4},{"brand":1,"pos":10,"stocks":4},{"brand":2,"pos":13,"stocks":4}],
        "agents":[8,8,16],"fuelLimits":6,"players":2,"busyThreshold":1,"jammedThreshold":2});
    let setting: protocol::match_config::MatchSetting =
        serde_json::from_value(raw.clone()).unwrap();
    let base = engine::from_wire::protocol_setting_to_engine(
        &setting,
        domain::PlayerId::new("player-a").unwrap(),
    )
    .unwrap();
    let m_setting = server
        .mock("GET", "/setting")
        .with_header("content-type", "application/json")
        .with_body(raw.to_string())
        .expect_at_least(1)
        .create_async()
        .await;
    let m_kinds = server
        .mock("POST", "/agent")
        .match_body(mockito::Matcher::Json(json!([0, 1, 0])))
        .with_status(200)
        .expect_at_least(1)
        .create_async()
        .await;
    let day = Arc::new(AtomicUsize::new(0));
    let served = Arc::new(AtomicUsize::new(0));
    let d = day.clone();
    let sd = served.clone();
    let m_state = server.mock("GET", "/").with_header("content-type","application/json").with_body_from_request(move |_| {
        let i = d.fetch_add(1, Ordering::SeqCst).min(2);
        sd.store(i, Ordering::SeqCst);
        json!({"endsAt":0,"day":i,"agents":[{"kind":0,"pos":8,"fuel":6},{"kind":1,"pos":8,"fuel":6},{"kind":0,"pos":16,"fuel":6}],"others":[],"traffics":[]}).to_string().into_bytes()
    }).expect(3).create_async().await;
    let accepted = Arc::new(AtomicUsize::new(0));
    let movement = Arc::new(Mutex::new(Vec::new()));
    let ac = accepted.clone();
    let mv = movement.clone();
    let m_post = server
        .mock("POST", "/")
        .with_header("content-type", "application/json")
        .with_body_from_request(move |req| {
            let i = served.load(Ordering::SeqCst);
            let body: Vec<Vec<i32>> = serde_json::from_slice(req.body().unwrap()).unwrap();
            let normalized = engine::wire_plan::normalize_day_plan(
                &base.map,
                &base.agents,
                &base.agent_ids(),
                [16, 20, 18][i],
                &body,
            )
            .unwrap();
            let valid = normalized == body;
            mv.lock().unwrap().push(body.iter().flatten().filter(|&&x| x >= 0).count());
            if valid {
                ac.fetch_add(1, Ordering::SeqCst);
            }
            json!({"revision":if valid { 1 } else { -1 }}).to_string().into_bytes()
        })
        .expect(3)
        .create_async()
        .await;
    let output = tokio::time::timeout(
        Duration::from_secs(20),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_procon"))
            .args([
                "coordinator",
                "--server",
                &server.url(),
                "--candidate",
                "coverage:budget=official",
                "--kinds",
                "P,S",
                "--exit-after-match",
            ])
            .env("PROCON_TOKEN", "local-fixture-token")
            .output(),
    )
    .await
    .expect("bounded CLI")
    .expect("CLI process");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(accepted.load(Ordering::SeqCst), 3);
    assert!(movement.lock().unwrap().iter().all(|&n| n > 0));
    m_setting.assert_async().await;
    m_kinds.assert_async().await;
    m_state.assert_async().await;
    m_post.assert_async().await;
}
