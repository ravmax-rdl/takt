use std::time::Duration;

use chrono::{TimeZone, Utc};
use serde_json::json;
use takt_google::*;
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
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
    GoogleClient::new(http, endpoints, auth)
}

fn ok(body: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

#[tokio::test]
async fn calendar_list_sends_sync_token_and_shows_hidden() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/users/me/calendarList"))
        .and(query_param("syncToken", "cl1"))
        .and(query_param("showHidden", "true"))
        .and(query_param("showDeleted", "true"))
        .and(query_param("maxResults", "250"))
        .respond_with(ok(
            json!({"items": [{"id": "me@example.com", "deleted": true}], "nextSyncToken": "cl2"}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let page = client_at(&server)
        .calendar_list_page(Some("cl1"), None)
        .await
        .unwrap();
    assert!(page.items[0].fields.deleted);
    assert_eq!(page.next_sync_token.as_deref(), Some("cl2"));
}

#[tokio::test]
async fn events_page_follows_page_tokens() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/calendars/primary/events"))
        .and(query_param_is_missing("pageToken"))
        .and(query_param("showDeleted", "true"))
        .and(query_param("maxResults", "2500"))
        .and(query_param_is_missing("singleEvents"))
        .and(query_param_is_missing("timeMin"))
        .and(query_param("syncToken", "s0"))
        .respond_with(ok(json!({"items": [{"id": "e1"}], "nextPageToken": "p2"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/calendars/primary/events"))
        .and(query_param("pageToken", "p2"))
        .and(query_param("syncToken", "s0"))
        .respond_with(ok(json!({"items": [{"id": "e2"}], "nextSyncToken": "s1"})))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_at(&server);
    let first = client
        .events_page("primary", Some("s0"), None)
        .await
        .unwrap();
    assert_eq!(first.next_page_token.as_deref(), Some("p2"));
    let second = client
        .events_page("primary", Some("s0"), first.next_page_token.as_deref())
        .await
        .unwrap();
    assert_eq!(second.items[0].fields.id, "e2");
    assert_eq!(second.next_sync_token.as_deref(), Some("s1"));
}

#[tokio::test]
async fn events_page_percent_encodes_calendar_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/calendar/v3/calendars/en.usa%23holiday@group.v.calendar.google.com/events",
        ))
        .respond_with(ok(json!({"items": [], "nextSyncToken": "s"})))
        .expect(1)
        .mount(&server)
        .await;
    let page = client_at(&server)
        .events_page("en.usa#holiday@group.v.calendar.google.com", None, None)
        .await
        .unwrap();
    assert!(page.items.is_empty());
}

#[tokio::test]
async fn events_page_encodes_slash_space_and_hash_in_calendar_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/calendars/a%2Fb%20c%23d/events"))
        .respond_with(ok(json!({"items": [], "nextSyncToken": "s"})))
        .expect(1)
        .mount(&server)
        .await;
    client_at(&server)
        .events_page("a/b c#d", None, None)
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests[0].url.path(),
        "/calendar/v3/calendars/a%2Fb%20c%23d/events"
    );
}

#[tokio::test]
async fn settings_collects_all_pages() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/users/me/settings"))
        .and(query_param_is_missing("pageToken"))
        .respond_with(ok(
            json!({"items": [{"id": "timezone", "value": "Europe/London"}], "nextPageToken": "n"}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/users/me/settings"))
        .and(query_param("pageToken", "n"))
        .respond_with(ok(
            json!({"items": [{"id": "format24HourTime", "value": "true"}]}),
        ))
        .mount(&server)
        .await;
    let settings = client_at(&server).settings().await.unwrap();
    let ids: Vec<_> = settings.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["timezone", "format24HourTime"]);
}

#[tokio::test]
async fn colors_reads_palette() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/colors"))
        .respond_with(ok(
            json!({"event": {"11": {"background": "#dc2127", "foreground": "#1d1d1d"}}}),
        ))
        .mount(&server)
        .await;
    let colors = client_at(&server).colors().await.unwrap();
    assert_eq!(colors.event["11"].background, "#dc2127");
}

#[tokio::test]
async fn task_lists_collects_all_pages() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/tasks/v1/users/@me/lists"))
        .and(query_param("maxResults", "100"))
        .and(query_param_is_missing("pageToken"))
        .respond_with(ok(
            json!({"items": [{"id": "L1", "title": "My Tasks"}], "nextPageToken": "n"}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tasks/v1/users/@me/lists"))
        .and(query_param("pageToken", "n"))
        .respond_with(ok(json!({"items": [{"id": "L2", "title": "Errands"}]})))
        .mount(&server)
        .await;
    let lists = client_at(&server).task_lists().await.unwrap();
    assert_eq!(lists.len(), 2);
}

#[tokio::test]
async fn tasks_page_sends_updated_min_and_flags() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/tasks/v1/lists/L1/tasks"))
        .and(query_param("updatedMin", "2026-10-17T10:00:00Z"))
        .and(query_param("showCompleted", "true"))
        .and(query_param("showDeleted", "true"))
        .and(query_param("showHidden", "true"))
        .and(query_param("maxResults", "100"))
        .respond_with(ok(json!({"items": [{"id": "t1", "title": "Pay rent"}]})))
        .expect(1)
        .mount(&server)
        .await;
    let since = Utc.with_ymd_and_hms(2026, 10, 17, 10, 0, 0).unwrap();
    let page = client_at(&server)
        .tasks_page("L1", Some(since), None)
        .await
        .unwrap();
    assert_eq!(page.items[0].fields.title.as_deref(), Some("Pay rent"));
}

#[tokio::test]
async fn instances_collects_all_pages() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/calendars/cal/events/m/instances"))
        .and(query_param("timeMin", "2026-01-01T00:00:00Z"))
        .and(query_param("timeMax", "2027-01-01T00:00:00Z"))
        .and(query_param_is_missing("pageToken"))
        .respond_with(ok(json!({"items": [{"id": "m_1"}], "nextPageToken": "n"})))
        .mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/calendars/cal/events/m/instances"))
        .and(query_param("pageToken", "n"))
        .respond_with(ok(json!({"items": [{"id": "m_2"}]})))
        .mount(&server).await;
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap();
    let got = client_at(&server).instances("cal", "m", from, to).await.unwrap();
    assert_eq!(got.len(), 2);
}
