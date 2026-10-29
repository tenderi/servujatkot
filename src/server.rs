//! HTTP server: receives Game State Integration POSTs and serves the overlays
//! (OBS browser sources) and a control page for the operator.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Form, State};
use axum::http::{StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json, Redirect, Response};
use axum::routing::{get, post};
use futures::stream::{self, Stream, StreamExt};
use maud::{Markup, html};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{RwLock, broadcast};
use tokio_stream::wrappers::BroadcastStream;

use crate::dota::gsi;
use crate::dota::overlay::{self, Names};
use crate::record::Recorder;

const SAMPLE_DRAFT: &str = include_str!("../data/dota/sample_draft.json");

pub struct App {
    /// Required in `auth.token` of every GSI POST when set.
    token: Option<String>,
    live: RwLock<Live>,
    changed: broadcast::Sender<()>,
    recorder: Option<Recorder>,
}

struct Live {
    payload: Option<Value>,
    received: Option<Instant>,
    packets: u64,
    rejected: u64,
    radiant_name: String,
    dire_name: String,
}

impl App {
    pub fn new(token: Option<String>, recorder: Option<Recorder>) -> Arc<Self> {
        Arc::new(App {
            token,
            recorder,
            live: RwLock::new(Live {
                payload: None,
                received: None,
                packets: 0,
                rejected: 0,
                radiant_name: "Radiant".into(),
                dire_name: "Dire".into(),
            }),
            changed: broadcast::channel(16).0,
        })
    }
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/", get(|| async { Redirect::to("/control") }))
        .route("/gsi/dota", post(gsi_dota))
        .route("/control", get(control))
        .route("/control/teams", post(set_teams))
        .route("/dota/draft", get(draft_page))
        .route("/dota/draft/events", get(draft_events))
        .route("/dota/draft/demo", get(draft_demo))
        .route("/api/dota/state", get(raw_state))
        .route(
            "/static/overlay.css",
            get(|| static_file("text/css", include_str!("../static/overlay.css"))),
        )
        .route(
            "/static/overlay.js",
            get(|| static_file("text/javascript", include_str!("../static/overlay.js"))),
        )
        .with_state(app)
}

async fn static_file(mime: &'static str, body: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
}

async fn gsi_dota(State(app): State<Arc<App>>, Json(payload): Json<Value>) -> StatusCode {
    let mut live = app.live.write().await;
    if let Some(want) = &app.token
        && gsi::auth_token(&payload) != Some(want.as_str())
    {
        live.rejected += 1;
        return StatusCode::UNAUTHORIZED;
    }
    if let Some(r) = &app.recorder {
        r.offer(&payload);
    }
    live.payload = Some(payload);
    live.received = Some(Instant::now());
    live.packets += 1;
    drop(live);
    // No receivers just means no overlay is open.
    let _ = app.changed.send(());
    StatusCode::OK
}

async fn render_draft(app: &App) -> Markup {
    let live = app.live.read().await;
    let draft = live.payload.as_ref().and_then(gsi::parse_draft);
    overlay::draft(
        draft.as_ref(),
        &Names {
            radiant: &live.radiant_name,
            dire: &live.dire_name,
        },
    )
}

async fn draft_page(State(app): State<Arc<App>>) -> Markup {
    overlay::page(
        "Draft",
        Some("/dota/draft/events"),
        render_draft(&app).await,
    )
}

/// The draft overlay filled from the bundled sample, for lining up OBS
/// without a live game.
async fn draft_demo(State(app): State<Arc<App>>) -> Markup {
    let payload: Value = serde_json::from_str(SAMPLE_DRAFT).expect("sample draft is valid JSON");
    let live = app.live.read().await;
    let names = Names {
        radiant: &live.radiant_name,
        dire: &live.dire_name,
    };
    overlay::page(
        "Draft (demo)",
        None,
        overlay::draft(gsi::parse_draft(&payload).as_ref(), &names),
    )
}

async fn draft_events(
    State(app): State<Arc<App>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Render once on connect, then on every change. A lagged receiver only
    // means we skipped intermediate states; rendering the latest is enough.
    let changes = BroadcastStream::new(app.changed.subscribe()).map(|_| ());
    let ticks = stream::once(async {}).chain(changes);
    let events = ticks.then(move |()| {
        let app = app.clone();
        async move { Ok(Event::default().data(render_draft(&app).await.into_string())) }
    });
    Sse::new(events).keep_alive(KeepAlive::default())
}

async fn raw_state(State(app): State<Arc<App>>) -> Json<Value> {
    Json(app.live.read().await.payload.clone().unwrap_or(Value::Null))
}

#[derive(Deserialize)]
struct Teams {
    radiant: String,
    dire: String,
}

async fn set_teams(State(app): State<Arc<App>>, Form(t): Form<Teams>) -> Redirect {
    let mut live = app.live.write().await;
    live.radiant_name = t.radiant.trim().to_string();
    live.dire_name = t.dire.trim().to_string();
    drop(live);
    let _ = app.changed.send(());
    Redirect::to("/control")
}

async fn control(State(app): State<Arc<App>>) -> Markup {
    let live = app.live.read().await;
    let age = live.received.map(|t| t.elapsed());
    let state = live
        .payload
        .as_ref()
        .and_then(gsi::game_state)
        .unwrap_or("-");
    let has_draft = live.payload.as_ref().and_then(gsi::parse_draft).is_some();
    html! {
        (maud::DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta http-equiv="refresh" content="5";
                title { "servujatkot" }
                link rel="stylesheet" href="/static/overlay.css";
            }
            body.control {
                h1 { "servujatkot" }
                section {
                    h2 { "Dota 2 feed" }
                    dl {
                        dt { "Last packet" } dd { (fmt_age(age)) }
                        dt { "Packets" } dd { (live.packets) }
                        @if live.rejected > 0 {
                            dt { "Rejected (bad token)" } dd.warn { (live.rejected) }
                        }
                        dt { "Game state" } dd { code { (state) } }
                        dt { "Draft data" } dd { @if has_draft { "yes" } @else { "no" } }
                    }
                }
                section {
                    h2 { "Teams" }
                    form method="post" action="/control/teams" {
                        label { "Radiant " input name="radiant" value=(live.radiant_name); }
                        label { "Dire " input name="dire" value=(live.dire_name); }
                        button { "Save" }
                    }
                }
                section {
                    h2 { "Overlays" }
                    p { "Add as OBS browser sources at 1920×1080." }
                    ul {
                        li { a href="/dota/draft" { "/dota/draft" } " — live draft" }
                        li { a href="/dota/draft/demo" { "/dota/draft/demo" } " — sample data, for layout" }
                        li { a href="/api/dota/state" { "/api/dota/state" } " — last raw GSI payload" }
                    }
                }
            }
        }
    }
}

fn fmt_age(age: Option<Duration>) -> String {
    match age {
        None => "never: run `servujatkot dota install`, add -gamestateintegration to Dota's launch options, restart Dota, and spectate a game".into(),
        Some(a) => format!("{:.1} s ago", a.as_secs_f32()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn send(app: &Arc<App>, req: Request<Body>) -> (StatusCode, String) {
        let res = router(app.clone()).oneshot(req).await.unwrap();
        let status = res.status();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    fn post_gsi(body: &str) -> Request<Body> {
        Request::post("/gsi/dota")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn gsi_post_feeds_the_draft_overlay() {
        let app = App::new(Some("test-token".into()), None);
        let (s, page) = send(
            &app,
            Request::get("/dota/draft").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert!(page.contains("draft idle"));

        let (s, _) = send(&app, post_gsi(SAMPLE_DRAFT)).await;
        assert_eq!(s, StatusCode::OK);
        let (_, page) = send(
            &app,
            Request::get("/dota/draft").body(Body::empty()).unwrap(),
        )
        .await;
        assert!(page.contains("Anti-Mage"));
        assert!(page.contains(r#"data-events="/dota/draft/events""#));
    }

    #[tokio::test]
    async fn wrong_token_is_rejected() {
        let app = App::new(Some("other".into()), None);
        let (s, _) = send(&app, post_gsi(SAMPLE_DRAFT)).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        assert!(app.live.read().await.payload.is_none());
        assert_eq!(app.live.read().await.rejected, 1);
    }

    #[tokio::test]
    async fn team_names_show_on_overlay() {
        let app = App::new(None, None);
        send(&app, post_gsi(SAMPLE_DRAFT)).await;
        let req = Request::post("/control/teams")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("radiant=Jallu+Esports&dire=Servu"))
            .unwrap();
        let (s, _) = send(&app, req).await;
        assert_eq!(s, StatusCode::SEE_OTHER);
        let (_, page) = send(
            &app,
            Request::get("/dota/draft").body(Body::empty()).unwrap(),
        )
        .await;
        assert!(page.contains("Jallu Esports"));
        assert!(page.contains("Servu pick"));
    }
}
