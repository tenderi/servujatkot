//! HTTP server: receives Game State Integration POSTs and serves the overlays
//! (OBS browser sources) and a control page for the operator.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Form, Query, State};
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
use crate::dota::overlay::{self, MatchInfo, TeamInfo, View};
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
    /// Team names typed on /control; `None` = take them from the match data.
    radiant_name: Option<String>,
    dire_name: Option<String>,
}

impl Live {
    fn match_info(&self) -> MatchInfo {
        let league = self.payload.as_ref().and_then(gsi::parse_league);
        let team = |manual: &Option<String>, side: Option<&gsi::LeagueTeam>, fallback: &str| {
            let name = manual
                .clone()
                .or_else(|| side.map(|t| t.name.clone()).filter(|n| !n.is_empty()))
                .unwrap_or_else(|| fallback.to_string());
            TeamInfo {
                name,
                // A tag only makes sense with the league's own name.
                tag: match (manual, side) {
                    (None, Some(t)) => t.tag.clone(),
                    _ => String::new(),
                },
                wins: side.map_or(0, |t| t.series_wins),
            }
        };
        MatchInfo {
            radiant: team(
                &self.radiant_name,
                league.as_ref().map(|l| &l.radiant),
                "Radiant",
            ),
            dire: team(&self.dire_name, league.as_ref().map(|l| &l.dire), "Dire"),
            event: league
                .as_ref()
                .map(|l| l.name.clone())
                .filter(|n| !n.is_empty()),
            wins_needed: league.as_ref().and_then(gsi::League::wins_needed),
        }
    }
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
                radiant_name: None,
                dire_name: None,
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
        .route("/dota/draft", get(|s| draft_page(s, View::Bar)))
        .route("/dota/draft/demo", get(|s| draft_demo(s, View::Bar)))
        .route(
            "/dota/draft/full",
            get(|s, Query(q): Query<FullQuery>| draft_page(s, q.view())),
        )
        .route(
            "/dota/draft/full/demo",
            get(|s, Query(q): Query<FullQuery>| draft_demo(s, q.view())),
        )
        .route("/dota/draft/events", get(draft_events))
        .route("/api/dota/state", get(raw_state))
        .route(
            "/static/overlay.css",
            get(|| static_file("text/css", include_str!("../static/overlay.css"))),
        )
        .route(
            "/static/draft-full.css",
            get(|| static_file("text/css", include_str!("../static/draft-full.css"))),
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

#[derive(Deserialize, Default)]
struct FullQuery {
    /// Any value: PNG stills instead of animated hero renders.
    still: Option<String>,
}

impl FullQuery {
    fn view(&self) -> View {
        View::Full {
            still: self.still.is_some(),
        }
    }
}

#[derive(Deserialize, Default)]
struct EventsQuery {
    view: Option<String>,
    still: Option<String>,
}

impl EventsQuery {
    fn view(&self) -> View {
        match self.view.as_deref() {
            Some("full") => FullQuery {
                still: self.still.clone(),
            }
            .view(),
            _ => View::Bar,
        }
    }
}

fn events_url(view: View) -> &'static str {
    match view {
        View::Bar => "/dota/draft/events",
        View::Full { still: false } => "/dota/draft/events?view=full",
        View::Full { still: true } => "/dota/draft/events?view=full&still=1",
    }
}

async fn render_draft(app: &App, view: View) -> Markup {
    let live = app.live.read().await;
    let draft = live.payload.as_ref().and_then(gsi::parse_draft);
    view.render(draft.as_ref(), &live.match_info())
}

async fn draft_page(State(app): State<Arc<App>>, view: View) -> Markup {
    overlay::page(
        "Draft",
        view,
        Some(events_url(view)),
        render_draft(&app, view).await,
    )
}

/// The draft overlay filled from the bundled sample, for lining up OBS
/// without a live game. Team names typed on /control still apply.
async fn draft_demo(State(app): State<Arc<App>>, view: View) -> Markup {
    let payload: Value = serde_json::from_str(SAMPLE_DRAFT).expect("sample draft is valid JSON");
    let live = app.live.read().await;
    let demo = Live {
        payload: Some(payload),
        received: None,
        packets: 0,
        rejected: 0,
        radiant_name: live.radiant_name.clone(),
        dire_name: live.dire_name.clone(),
    };
    let draft = demo.payload.as_ref().and_then(gsi::parse_draft);
    overlay::page(
        "Draft (demo)",
        view,
        None,
        view.render(draft.as_ref(), &demo.match_info()),
    )
}

async fn draft_events(
    State(app): State<Arc<App>>,
    Query(q): Query<EventsQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let view = q.view();
    // Render once on connect, then on every change. A lagged receiver only
    // means we skipped intermediate states; rendering the latest is enough.
    // GSI posts several times a second; most change nothing visible, so a
    // fragment identical to the last one sent is dropped.
    let changes = BroadcastStream::new(app.changed.subscribe()).map(|_| ());
    let ticks = stream::once(async {}).chain(changes);
    let rendered = ticks.then(move |()| {
        let app = app.clone();
        async move { render_draft(&app, view).await.into_string() }
    });
    let events = rendered
        .scan(String::new(), |last, html| {
            let fresh = *last != html;
            if fresh {
                last.clone_from(&html);
            }
            futures::future::ready(Some(fresh.then_some(html)))
        })
        .filter_map(|html| async move { html.map(|h| Ok(Event::default().data(h))) });
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
    let manual = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    let mut live = app.live.write().await;
    live.radiant_name = manual(&t.radiant);
    live.dire_name = manual(&t.dire);
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
    let league = live.payload.as_ref().and_then(gsi::parse_league);
    let auto = |side: fn(&gsi::League) -> &gsi::LeagueTeam, fallback: &str| {
        let name = league
            .as_ref()
            .map(|l| side(l).name.clone())
            .unwrap_or_default();
        if name.is_empty() {
            fallback.to_string()
        } else {
            format!("{name} (from match data)")
        }
    };
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
                    p { "Leave a name empty to use the one from the match data." }
                    form method="post" action="/control/teams" {
                        label { "Radiant " input name="radiant"
                            value=(live.radiant_name.as_deref().unwrap_or(""))
                            placeholder=(auto(|l| &l.radiant, "Radiant")); }
                        label { "Dire " input name="dire"
                            value=(live.dire_name.as_deref().unwrap_or(""))
                            placeholder=(auto(|l| &l.dire, "Dire")); }
                        button { "Save" }
                    }
                }
                section {
                    h2 { "Overlays" }
                    p { "Add as OBS browser sources at 1920×1080." }
                    ul {
                        li { a href="/dota/draft" { "/dota/draft" } " — draft bar over gameplay" }
                        li { a href="/dota/draft/full" { "/dota/draft/full" } " — fullscreen draft scene (add " code { "?still=1" } " for still hero art on slow machines)" }
                        li { "Sample data, for layout: "
                            a href="/dota/draft/demo" { "bar" } ", "
                            a href="/dota/draft/full/demo" { "fullscreen" } }
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
        assert!(page.contains("Earthshaker"));
        assert!(page.contains(r#"data-events="/dota/draft/events""#));
        assert!(page.contains("PARIVISION"), "team name from league data");

        let (s, full) = send(
            &app,
            Request::get("/dota/draft/full?still=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert!(full.contains(r#"data-events="/dota/draft/events?view=full&amp;still=1""#));
        assert!(full.contains("renders/earthshaker.png"));
        assert!(full.contains("Best of 3"));
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

        // Clearing a name goes back to the match data.
        let req = Request::post("/control/teams")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("radiant=&dire=Servu"))
            .unwrap();
        send(&app, req).await;
        let (_, page) = send(
            &app,
            Request::get("/dota/draft").body(Body::empty()).unwrap(),
        )
        .await;
        assert!(page.contains("PARIVISION"));
    }
}
