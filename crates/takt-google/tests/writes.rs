use std::time::Duration;

use serde_json::json;
use takt_google::*;
use wiremock::matchers::{body_json, header, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client_at(server: &MockServer) -> GoogleClient {
    let endpoints = Endpoints::rooted_at(&server.uri());
    let http = reqwest::Client::new();
    let oauth = OAuthClient {
        client_id: "cid".into(),
        client_secret: "secret".into(),
    };
    let auth = Auth::new(http.clone(), endpoints.token.clone(), oauth, "rt")
        .with_access_token("t1", Duration::from_secs(3600));
    GoogleClient::new(http, endpoints, auth).with_retry(RetryPolicy {
        max_attempts: 3,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(4),
    })
}

fn ok(body: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

fn event(id: &str, etag: &str) -> serde_json::Value {
    json!({"id": id, "etag": etag, "summary": "Lunch", "unknownField": {"kept": true}})
}

#[tokio::test]
async fn insert_posts_the_body_and_returns_the_whole_event() {
    let server = MockServer::start().await;
    let body = json!({"id": "abc12", "summary": "Lunch"});
    Mock::given(method("POST"))
        .and(path("/calendar/v3/calendars/primary/events"))
        .and(query_param("sendUpdates", "all"))
        .and(query_param("conferenceDataVersion", "1"))
        .and(query_param("supportsAttachments", "true"))
        .and(body_json(&body))
        .respond_with(ok(event("abc12", "\"1\"")))
        .expect(1)
        .mount(&server)
        .await;
    let created = client_at(&server)
        .insert_event("primary", &body, SendUpdates::All)
        .await
        .unwrap();
    assert_eq!(created.fields.id, "abc12");
    assert_eq!(created.raw["unknownField"], json!({"kept": true}));
}

#[tokio::test]
async fn patch_sends_only_the_patch_with_if_match() {
    let server = MockServer::start().await;
    let patch = json!({"summary": "Late lunch", "location": null});
    Mock::given(method("PATCH"))
        .and(path("/calendar/v3/calendars/primary/events/e1"))
        .and(header("if-match", "\"1\""))
        .and(query_param("sendUpdates", "none"))
        .and(query_param("conferenceDataVersion", "1"))
        .and(body_json(&patch))
        .respond_with(ok(event("e1", "\"2\"")))
        .expect(1)
        .mount(&server)
        .await;
    let updated = client_at(&server)
        .patch_event("primary", "e1", &patch, Some("\"1\""), SendUpdates::None)
        .await
        .unwrap();
    assert_eq!(updated.fields.etag.as_deref(), Some("\"2\""));
}

#[tokio::test]
async fn patch_without_an_etag_sends_no_if_match() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(header_exists("if-match"))
        .respond_with(ResponseTemplate::new(400))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(
            "/calendar/v3/calendars/primary/events/e1_20261112T090000Z",
        ))
        .respond_with(ok(event("e1_20261112T090000Z", "\"1\"")))
        .expect(1)
        .mount(&server)
        .await;
    client_at(&server)
        .patch_event(
            "primary",
            "e1_20261112T090000Z",
            &json!({"summary": "x"}),
            None,
            SendUpdates::None,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn a_stale_etag_is_a_precondition_failure() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .respond_with(
            ResponseTemplate::new(412)
                .set_body_json(json!({"error": {"code": 412, "message": "Precondition Failed"}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let result = client_at(&server)
        .patch_event(
            "primary",
            "e1",
            &json!({}),
            Some("\"old\""),
            SendUpdates::None,
        )
        .await;
    assert!(matches!(result, Err(GoogleError::PreconditionFailed)));
}

#[tokio::test]
async fn inserting_an_existing_id_is_a_conflict() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(409).set_body_json(
            json!({"error": {"code": 409, "message": "The requested identifier already exists."}}),
        ))
        .mount(&server)
        .await;
    let result = client_at(&server)
        .insert_event("primary", &json!({"id": "abc12"}), SendUpdates::None)
        .await;
    assert!(matches!(result, Err(GoogleError::Conflict)));
}

#[tokio::test]
async fn a_write_is_sent_again_after_a_503() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .respond_with(ok(event("e1", "\"2\"")))
        .expect(1)
        .mount(&server)
        .await;
    client_at(&server)
        .patch_event(
            "primary",
            "e1",
            &json!({"summary": "x"}),
            None,
            SendUpdates::None,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn delete_accepts_an_empty_204() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/calendar/v3/calendars/primary/events/e1"))
        .and(header("if-match", "\"1\""))
        .and(query_param("sendUpdates", "all"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    client_at(&server)
        .delete_event("primary", "e1", Some("\"1\""), SendUpdates::All)
        .await
        .unwrap();
}

#[tokio::test]
async fn deleting_a_deleted_event_is_gone() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(410))
        .mount(&server)
        .await;
    let result = client_at(&server)
        .delete_event("primary", "e1", None, SendUpdates::None)
        .await;
    assert!(matches!(result, Err(GoogleError::Gone)));
}

#[tokio::test]
async fn move_names_the_destination() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/calendar/v3/calendars/primary/events/e1/move"))
        .and(query_param("destination", "work@group.calendar.google.com"))
        .respond_with(ok(event("e1", "\"3\"")))
        .expect(1)
        .mount(&server)
        .await;
    let moved = client_at(&server)
        .move_event(
            "primary",
            "e1",
            "work@group.calendar.google.com",
            SendUpdates::None,
        )
        .await
        .unwrap();
    assert_eq!(moved.fields.id, "e1");
}

#[tokio::test]
async fn get_event_reads_one_event() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/calendars/primary/events/e1"))
        .respond_with(ok(event("e1", "\"1\"")))
        .expect(1)
        .mount(&server)
        .await;
    let got = client_at(&server).get_event("primary", "e1").await.unwrap();
    assert_eq!(got.fields.summary.as_deref(), Some("Lunch"));
}

#[test]
fn new_event_ids_are_base32hex_and_differ() {
    let (a, b) = (new_event_id(), new_event_id());
    assert_eq!(a.len(), 26);
    assert!(a.chars().all(|c| matches!(c, '0'..='9' | 'a'..='v')));
    assert_ne!(a, b);
}
