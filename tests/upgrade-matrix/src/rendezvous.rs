//! 本地 rendezvous：邀请码直接映射为邀请方 ticket，避免访问外部配对服务。

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, Request, Respond, ResponseTemplate,
    matchers::{method, path},
};

const EXPIRES_AT_MS: i64 = 2_000_000_000_000;

type Tickets = Arc<Mutex<HashMap<String, String>>>;

struct CreatePairing(Tickets);

impl Respond for CreatePairing {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let Some(ticket) = body["sponsorTicket"].as_str() else {
            return ResponseTemplate::new(400);
        };
        if let Ok(mut tickets) = self.0.lock() {
            tickets.insert(ticket.to_owned(), ticket.to_owned());
        }
        ResponseTemplate::new(200).set_body_json(json!({
            "code": ticket,
            "expiresAtMs": EXPIRES_AT_MS,
        }))
    }
}

struct ResolvePairing(Tickets);

impl Respond for ResolvePairing {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let ticket = body["code"]
            .as_str()
            .and_then(|code| self.0.lock().ok()?.get(code).cloned());
        match ticket {
            Some(ticket) => ResponseTemplate::new(200).set_body_json(json!({
                "sponsorTicket": ticket,
                "sponsorEndpointId": "local-upgrade-matrix",
                "expiresAtMs": EXPIRES_AT_MS,
            })),
            None => ResponseTemplate::new(404),
        }
    }
}

pub(crate) async fn start() -> MockServer {
    let server = MockServer::start().await;
    let tickets: Tickets = Arc::default();
    Mock::given(method("POST"))
        .and(path("/v1/pairings"))
        .respond_with(CreatePairing(Arc::clone(&tickets)))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/pairings/resolve"))
        .respond_with(ResolvePairing(tickets))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/pairings/consume"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    server
}
