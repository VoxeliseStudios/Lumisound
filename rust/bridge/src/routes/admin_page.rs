//! The /admin dashboard, rendered on the server (no browser script). Sign-in
//! posts the admin token (or the operator's session token) once; it's kept
//! in an HttpOnly cookie scoped to /admin, so page JavaScript could never read
//! it (the old page kept it in sessionStorage). The page refreshes itself
//! every 15 s with a meta refresh, and each render fetches its four data
//! sets concurrently.

use super::admin::{cookies_data, download_jobs_data, errors_data, overview_data, perf_data};
use crate::auth::admin_check;
use crate::state::AppState;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::Form;
use serde::Deserialize;
use serde_json::{Value, json};
use std::fmt::Write;

const COOKIE: &str = "lumi_admin";
const REFRESH_SECS: u32 = 15;

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == COOKIE)
        .map(|(_, v)| urlencoding::decode(v).map(|c| c.into_owned()).unwrap_or_default())
        .filter(|v| !v.is_empty())
}

/// Secure only when the visit came over HTTPS (cloudflared sets the
/// forwarded proto); a LAN visit over plain HTTP would drop a Secure cookie.
fn https(headers: &HeaderMap) -> bool {
    headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok()).is_some_and(|v| v.eq_ignore_ascii_case("https"))
        || headers.get("cf-visitor").and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("https"))
}

fn set_cookie(value: &str, max_age: u32, secure: bool) -> HeaderValue {
    let v = format!(
        "{COOKIE}={}; Path=/admin; HttpOnly; SameSite=Strict; Max-Age={max_age}{}",
        urlencoding::encode(value),
        if secure { "; Secure" } else { "" }
    );
    HeaderValue::from_str(&v).unwrap_or_else(|_| HeaderValue::from_static("lumi_admin=; Path=/admin; Max-Age=0"))
}

/// The error's `detail` as plain text.
fn msg(e: &crate::error::AppError) -> String {
    match &e.detail {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn no_store(mut r: Response) -> Response {
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r.headers_mut().insert("x-frame-options", HeaderValue::from_static("DENY"));
    r.headers_mut().insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    // No scripts at all; inline styles only.
    r.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'"),
    );
    r
}

pub async fn page(State(st): State<AppState>, headers: HeaderMap) -> Response {
    let token = cookie_token(&headers);
    if token.is_none() {
        return no_store(Html(login_page(None)).into_response());
    }
    if let Err(e) = admin_check(&st, token.as_deref()).await {
        let mut r = Html(login_page(Some(&msg(&e)))).into_response();
        r.headers_mut().insert(header::SET_COOKIE, set_cookie("", 0, https(&headers)));
        return no_store(r);
    }
    let (o, jobs, errs, perf) = tokio::join!(overview_data(&st), download_jobs_data(&st, 50), errors_data(&st, 50), perf_data(&st, 48));
    let extra = json!({"cookies": cookies_data(&st), "routes": crate::routestats::report(25)});
    let html = match (o, jobs, errs, perf) {
        (Ok(o), Ok(j), Ok(e), Ok(p)) => dashboard(&o, &j, &e, &p, &extra),
        (o, j, e, p) => {
            let err = [o.err(), j.err(), e.err(), p.err()].into_iter().flatten().map(|e| msg(&e)).collect::<Vec<_>>().join("; ");
            shell(&format!("<p class=\"err\">Couldn't load the dashboard: {}</p>", esc(&err)), true)
        }
    };
    no_store(Html(html).into_response())
}

#[derive(Deserialize)]
pub struct LoginForm {
    token: String,
}

pub async fn login(State(st): State<AppState>, headers: HeaderMap, Form(f): Form<LoginForm>) -> Response {
    let t = f.token.trim();
    if t.is_empty() {
        return no_store(Html(login_page(Some("Enter the admin token."))).into_response());
    }
    match admin_check(&st, Some(t)).await {
        Ok(_) => {
            let mut r = Redirect::to("/admin").into_response();
            r.headers_mut().insert(header::SET_COOKIE, set_cookie(t, 12 * 3600, https(&headers)));
            no_store(r)
        }
        Err(e) => {
            let mut r = (StatusCode::UNAUTHORIZED, Html(login_page(Some(&msg(&e))))).into_response();
            r.headers_mut().insert(header::SET_COOKIE, set_cookie("", 0, https(&headers)));
            no_store(r)
        }
    }
}

pub async fn logout(headers: HeaderMap) -> Response {
    let mut r = Redirect::to("/admin").into_response();
    r.headers_mut().insert(header::SET_COOKIE, set_cookie("", 0, https(&headers)));
    no_store(r)
}

// MARK: - Rendering

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A JSON field as display text ("" for null/missing), truncated to `max` chars.
fn txt(v: &Value, key: &str, max: usize) -> String {
    let s = match v.get(key) {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    };
    esc(&s.chars().take(max).collect::<String>())
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

fn fmt_num(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 { format!("{}", x as i64) } else { format!("{x}") }
}

fn fmt_bytes(b: Option<f64>) -> String {
    let Some(mut b) = b else { return "n/a".into() };
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut i = 0;
    while b >= 1024.0 && i < units.len() - 1 {
        b /= 1024.0;
        i += 1;
    }
    format!("{b:.1} {}", units[i])
}

const STYLE: &str = r#"
:root { color-scheme: dark; }
body { font-family: -apple-system, system-ui, sans-serif; background: #0b0d10; color: #e6e8eb; margin: 0; padding: 24px; }
h1 { font-size: 18px; margin: 0 0 20px; display: flex; justify-content: space-between; align-items: center; }
h2 { font-size: 14px; color: #b6bcc4; }
.cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 12px; margin-bottom: 24px; }
.card, .chart { background: #161a1f; border: 1px solid #262b32; border-radius: 10px; padding: 14px 16px; }
.label { font-size: 11px; text-transform: uppercase; letter-spacing: 0.06em; color: #8a929c; margin-bottom: 6px; }
.card .value { font-size: 22px; font-weight: 700; }
.charts { display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 12px; margin-bottom: 24px; }
.chart .label { display: flex; justify-content: space-between; }
.chart svg { width: 100%; height: 90px; display: block; margin-top: 8px; }
.now { color: #e6e8eb; font-weight: 700; }
.scroll { overflow-x: auto; margin-bottom: 28px; }
table { width: 100%; border-collapse: collapse; font-size: 13px; }
th, td { text-align: left; padding: 6px 10px; border-bottom: 1px solid #20242a; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 280px; }
th { color: #8a929c; font-weight: 600; font-size: 11px; text-transform: uppercase; }
.status-completed, .status-success { color: #4ade80; } .status-failed, .status-error { color: #f87171; }
.login { max-width: 360px; margin: 80px auto; }
input { width: 100%; padding: 10px; margin: 8px 0; background: #161a1f; border: 1px solid #262b32; border-radius: 8px; color: #e6e8eb; box-sizing: border-box; }
button { padding: 10px 14px; background: #3b82f6; border: none; border-radius: 8px; color: white; font-weight: 600; cursor: pointer; }
.login button { width: 100%; }
.ghost { background: transparent; border: 1px solid #262b32; color: #b6bcc4; font-size: 12px; padding: 6px 10px; }
.err { color: #f87171; }
.muted { color: #8a929c; font-size: 12px; font-weight: 400; }
"#;

fn shell(body: &str, refresh: bool) -> String {
    let meta = if refresh { format!("<meta http-equiv=\"refresh\" content=\"{REFRESH_SECS}\">") } else { String::new() };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         {meta}<title>ios-bridge Admin</title><style>{STYLE}</style></head><body>{body}</body></html>"
    )
}

fn login_page(error: Option<&str>) -> String {
    let err = error.map(|e| format!("<p class=\"err\">{}</p>", esc(e))).unwrap_or_default();
    shell(
        &format!(
            "<form class=\"login\" method=\"post\" action=\"/admin/login\"><h1>ios-bridge Admin</h1>{err}\
             <input name=\"token\" type=\"password\" placeholder=\"Admin token\" autocomplete=\"off\" autofocus required>\
             <button type=\"submit\">Sign in</button></form>"
        ),
        false,
    )
}

/// One hourly series as an SVG line with its peak and last value.
fn line_chart(points: &[Value], key: &str, label: &str, unit: &str, color: &str) -> String {
    let vals: Vec<f64> = points.iter().filter_map(|p| num(p, key)).collect();
    if vals.is_empty() {
        return format!("<div class=\"chart\"><div class=\"label\"><span>{label}</span><span>no data</span></div></div>");
    }
    let (w, h) = (300.0, 90.0);
    let peak = vals.iter().cloned().fold(f64::MIN, f64::max);
    let max = if peak * 1.1 > 0.0 { peak * 1.1 } else { 1.0 };
    let n = points.len();
    let mut d = String::new();
    for (i, p) in points.iter().enumerate() {
        let Some(v) = num(p, key) else { continue };
        let x = if n == 1 { w / 2.0 } else { i as f64 / (n - 1) as f64 * w };
        let y = h - v / max * (h - 6.0) - 3.0;
        let _ = write!(d, "{}{x:.1} {y:.1} ", if d.is_empty() { "M" } else { "L" });
    }
    let last = vals[vals.len() - 1];
    format!(
        "<div class=\"chart\"><div class=\"label\"><span>{label}</span><span>now <span class=\"now\">{}{unit}</span> · peak {}{unit}</span></div>\
         <svg viewBox=\"0 0 {w} {h}\" preserveAspectRatio=\"none\"><path d=\"{}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"2\" vector-effect=\"non-scaling-stroke\"/></svg></div>",
        fmt_num(last),
        fmt_num(peak),
        d.trim_end()
    )
}

fn table(head: &[&str], rows: impl Iterator<Item = String>) -> String {
    let mut s = String::from("<div class=\"scroll\"><table><thead><tr>");
    for h in head {
        let _ = write!(s, "<th>{h}</th>");
    }
    s.push_str("</tr></thead><tbody>");
    for r in rows {
        s.push_str(&r);
    }
    s.push_str("</tbody></table></div>");
    s
}

fn arr(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn dashboard(o: &Value, jobs: &Value, errors: &Value, perf: &Value, extra: &Value) -> String {
    let mut b = String::with_capacity(32 * 1024);
    b.push_str(
        "<h1><span>ios-bridge Admin <span class=\"muted\">· refreshes every 15 s</span></span>\
         <form method=\"post\" action=\"/admin/logout\"><button class=\"ghost\" type=\"submit\">Sign out</button></form></h1>",
    );

    let j24 = o.get("download_jobs_24h").cloned().unwrap_or(Value::Null);
    let c = o.get("concurrency").cloned().unwrap_or(Value::Null);
    let n = |v: &Value, k: &str| num(v, k).unwrap_or(0.0);
    let cards = [
        ("Users", fmt_num(n(o, "user_count"))),
        ("Music files", fmt_num(n(o, "music_file_count"))),
        ("Storage used", fmt_bytes(num(o, "music_bytes"))),
        ("Disk free", fmt_bytes(o.get("disk").and_then(|d| num(d, "free_bytes")))),
        ("Jobs completed (24h)", fmt_num(n(&j24, "success") + n(&j24, "completed"))),
        ("Jobs failed (24h)", fmt_num(n(&j24, "failed"))),
        ("Errors (24h)", fmt_num(n(o, "recent_error_count_24h"))),
        ("yt-dlp slots", format!("{}/{} free", fmt_num(n(&c, "ytdlp_available")), fmt_num(n(&c, "ytdlp_max")))),
    ];
    b.push_str("<div class=\"cards\">");
    for (label, value) in cards {
        let _ = write!(b, "<div class=\"card\"><div class=\"label\">{label}</div><div class=\"value\">{}</div></div>", esc(&value));
    }
    b.push_str("</div>");

    // Shared YouTube cookies: the first thing to check when streams fail.
    let g = &extra["cookies"]["global"];
    let stale = g.get("stale_flag").and_then(Value::as_str);
    let _ = write!(
        b,
        "<h2>YouTube cookies <span class=\"muted\">· <a href=\"/admin/storage\">storage &amp; trash</a></span></h2><p>{} · jar age {} h · pool {} · {}</p>",
        if g.get("present").and_then(Value::as_bool) == Some(true) { "shared jar present" } else { "<span class=\"err\">no shared jar</span>" },
        txt(g, "age_hours", 10),
        txt(g, "pool_size", 10),
        match stale {
            Some(s) => format!("<span class=\"err\">marked stale: {}</span>", esc(s)),
            None => "not marked stale".into(),
        }
    );
    b.push_str(&table(
        &["Jar", "Last success", "Last bot wall", "Walls since success", "Benched"],
        arr(&extra["cookies"]["jars"]).iter().map(|j| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{} s</td></tr>",
                txt(j, "owner", 40),
                txt(j, "last_ok", 40),
                txt(j, "last_botwall", 40),
                txt(j, "botwalls_since_ok", 10),
                txt(j, "benched_secs", 10)
            )
        }),
    ));

    b.push_str("<h2>Bridge response times (last hour, by total time)</h2>");
    b.push_str(&table(
        &["Route", "Requests", "5xx", "avg", "p50", "p95", "p99", "max"],
        arr(&extra["routes"]["last_hour"]).iter().map(|r| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{} ms</td><td>≤{} ms</td><td>≤{} ms</td><td>≤{} ms</td><td>{} ms</td></tr>",
                txt(r, "route", 80),
                txt(r, "n", 12),
                txt(r, "errors", 12),
                txt(r, "avg_ms", 12),
                txt(r, "p50_ms", 12),
                txt(r, "p95_ms", 12),
                txt(r, "p99_ms", 12),
                txt(r, "max_ms", 12)
            )
        }),
    ));

    let hourly = arr(perf.get("hourly").unwrap_or(&Value::Null));
    b.push_str("<h2>App performance (last 48 h, hourly)</h2><div class=\"charts\">");
    b.push_str(&line_chart(hourly, "cpu", "CPU (avg % of one core)", "%", "#60a5fa"));
    b.push_str(&line_chart(hourly, "mw", "Power", " mW", "#fbbf24"));
    b.push_str(&line_chart(hourly, "hitch", "Hitching", " ms/s", "#f87171"));
    b.push_str(&line_chart(hourly, "mem", "Peak memory", " MB", "#a78bfa"));
    b.push_str("</div>");

    b.push_str("<h2>Screens with the most hitching</h2>");
    b.push_str(&table(
        &["Screen", "Hitch ms/s", "Worst frame", "Time seen", "Windows"],
        arr(perf.get("screens").unwrap_or(&Value::Null)).iter().map(|s| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>{} ms</td><td>{} s</td><td>{}</td></tr>",
                txt(s, "screen", 80),
                txt(s, "hitch", 20),
                txt(s, "worst_ms", 20),
                txt(s, "seconds", 20),
                txt(s, "n", 20)
            )
        }),
    ));

    b.push_str("<h2>By app version</h2>");
    b.push_str(&table(
        &["Version", "CPU avg", "Hitch ms/s", "Peak memory", "Windows"],
        arr(perf.get("versions").unwrap_or(&Value::Null)).iter().map(|v| {
            format!(
                "<tr><td>{}</td><td>{}%</td><td>{}</td><td>{} MB</td><td>{}</td></tr>",
                txt(v, "version", 40),
                txt(v, "cpu", 20),
                txt(v, "hitch", 20),
                txt(v, "mem", 20),
                txt(v, "n", 20)
            )
        }),
    ));

    b.push_str("<h2>Download jobs (last 50)</h2>");
    b.push_str(&table(
        &["Created", "Status", "Source", "Title", "Error"],
        arr(jobs).iter().map(|j| {
            let status = j.get("status").and_then(Value::as_str).unwrap_or_default();
            let class: String = status.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            format!(
                "<tr><td>{}</td><td class=\"status-{class}\">{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                txt(j, "created_at", 40),
                esc(status),
                txt(j, "source", 40),
                txt(j, "title", 60),
                txt(j, "error_message", 80)
            )
        }),
    ));

    b.push_str("<h2>Recent errors (last 50)</h2>");
    b.push_str(&table(
        &["Time", "Category", "Message", "App"],
        arr(errors).iter().map(|e| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                txt(e, "timestamp", 40),
                txt(e, "category", 40),
                txt(e, "message", 100),
                txt(e, "app_version", 30)
            )
        }),
    ));
    shell(&b, true)
}

// MARK: - Storage & trash page (no auto refresh: it has forms)

async fn signed_in(st: &AppState, headers: &HeaderMap) -> Result<(), Response> {
    let token = cookie_token(headers);
    if token.is_none() {
        return Err(Redirect::to("/admin").into_response());
    }
    admin_check(st, token.as_deref()).await.map(|_| ()).map_err(|_| Redirect::to("/admin").into_response())
}

#[derive(Deserialize, Default)]
pub struct StorageQ {
    msg: Option<String>,
}

pub async fn storage_page(State(st): State<AppState>, headers: HeaderMap, axum::extract::Query(q): axum::extract::Query<StorageQ>) -> Response {
    if let Err(r) = signed_in(&st, &headers).await {
        return r;
    }
    let st2 = st.clone();
    let (report, trash) = tokio::task::spawn_blocking(move || (crate::housekeeping::storage_report(&st2), crate::housekeeping::trash_list(&st2, 300)))
        .await
        .unwrap_or((Value::Null, Vec::new()));
    no_store(Html(storage_html(&report, &trash, q.msg.as_deref())).into_response())
}

fn storage_html(report: &Value, trash: &[Value], msg: Option<&str>) -> String {
    let mut b = String::with_capacity(32 * 1024);
    b.push_str("<h1><span>Storage &amp; trash <span class=\"muted\">· <a href=\"/admin\">dashboard</a></span></span></h1>");
    if let Some(m) = msg {
        let _ = write!(b, "<p>{}</p>", esc(m));
    }
    b.push_str("<h2>Caches (trimmed hourly, least recently used first)</h2>");
    b.push_str(&table(
        &["Cache", "Files", "Size", "Cap"],
        arr(&report["caches"]).iter().map(|c| {
            format!("<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>", txt(c, "name", 60), txt(c, "files", 12), esc(&fmt_bytes(num(c, "bytes"))), esc(&fmt_bytes(num(c, "cap_bytes"))))
        }),
    ));
    b.push_str("<h2>LMS3 migration backup</h2>");
    let backups = arr(&report["lms3_backup"]);
    if backups.is_empty() {
        b.push_str("<p class=\"muted\">No backup folder (already deleted).</p>");
    } else {
        b.push_str(&table(
            &["Drive", "Files", "Size"],
            backups.iter().map(|c| format!("<tr><td>{}</td><td>{}</td><td>{}</td></tr>", txt(c, "root", 80), txt(c, "files", 12), esc(&fmt_bytes(num(c, "bytes"))))),
        ));
        b.push_str(
            "<form method=\"post\" action=\"/admin/storage/delete-lms3-backup\"><p>The original files the cloud library was converted from. \
             Type <b>DELETE</b> to erase them for good.</p><input name=\"confirm\" autocomplete=\"off\" placeholder=\"DELETE\">\
             <button type=\"submit\">Delete the LMS3 backup</button></form>",
        );
    }
    let _ = write!(b, "<h2>Trash <span class=\"muted\">· deleted cloud tracks, kept 30 days · {} shown</span></h2>", trash.len());
    b.push_str(&table(
        &["Deleted", "User", "Track", "Size", ""],
        trash.iter().map(|t| {
            let title = t.get("title").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| t["path"].as_str().unwrap_or("").to_string());
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}<div class=\"muted\">{}</div></td><td>{}</td><td><form method=\"post\" action=\"/admin/storage/restore\">\
                 <input type=\"hidden\" name=\"user\" value=\"{}\"><input type=\"hidden\" name=\"day\" value=\"{}\"><input type=\"hidden\" name=\"path\" value=\"{}\">\
                 <button class=\"ghost\" type=\"submit\">Restore</button></form></td></tr>",
                txt(t, "day", 10),
                esc(&t["user"].as_str().unwrap_or("").chars().take(8).collect::<String>()),
                esc(&title),
                txt(t, "path", 120),
                esc(&fmt_bytes(num(t, "bytes"))),
                esc(t["user"].as_str().unwrap_or("")),
                esc(t["day"].as_str().unwrap_or("")),
                esc(t["path"].as_str().unwrap_or(""))
            )
        }),
    ));
    shell(&b, false)
}

#[derive(Deserialize)]
pub struct RestoreForm {
    user: String,
    day: String,
    path: String,
}

pub async fn storage_restore(State(st): State<AppState>, headers: HeaderMap, Form(f): Form<RestoreForm>) -> Response {
    if let Err(r) = signed_in(&st, &headers).await {
        return r;
    }
    let m = match crate::housekeeping::trash_restore(&st, &f.user, &f.day, &f.path).await {
        Ok(m) => format!("{}: {m}", f.path),
        Err(e) => format!("{}: not restored — {e}", f.path),
    };
    no_store(Redirect::to(&format!("/admin/storage?msg={}", urlencoding::encode(&m))).into_response())
}

#[derive(Deserialize)]
pub struct ConfirmForm {
    confirm: String,
}

pub async fn storage_delete_backup(State(st): State<AppState>, headers: HeaderMap, Form(f): Form<ConfirmForm>) -> Response {
    if let Err(r) = signed_in(&st, &headers).await {
        return r;
    }
    let m = if f.confirm.trim() == "DELETE" {
        let st2 = st.clone();
        let (files, bytes) = tokio::task::spawn_blocking(move || crate::housekeeping::delete_lms3_backup(&st2)).await.unwrap_or((0, 0));
        format!("Deleted the LMS3 backup: {files} files, {}", fmt_bytes(Some(bytes as f64)))
    } else {
        "Not deleted: type DELETE to confirm.".to_string()
    };
    no_store(Redirect::to(&format!("/admin/storage?msg={}", urlencoding::encode(&m))).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_without_script_and_escapes() {
        let o = json!({"user_count": 3, "music_file_count": 10, "music_bytes": 2048, "disk": {"free_bytes": 1073741824u64},
                       "download_jobs_24h": {"success": 2, "failed": 1}, "recent_error_count_24h": 4,
                       "concurrency": {"ytdlp_available": 1, "ytdlp_max": 3}});
        let jobs = json!([{"created_at": "2026-11-16T10:00:00", "status": "failed", "source": "youtube", "title": "<script>x</script>", "error_message": null}]);
        let errs = json!([{"timestamp": "t", "category": "c", "message": "m & \"q\"", "app_version": "2.0"}]);
        let perf = json!({"hourly": [{"cpu": 12.5, "mw": null, "hitch": 1.0, "mem": 300}, {"cpu": 20, "hitch": 2.0, "mem": 310}],
                          "screens": [{"screen": "Home", "hitch": 1.2, "worst_ms": 40, "seconds": 60, "n": 2}], "versions": []});
        let html = dashboard(&o, &jobs, &errs, &perf, &json!({}));
        assert!(!html.contains("<script"));
        assert!(html.contains("&lt;script&gt;x&lt;/script&gt;"));
        assert!(html.contains("m &amp; &quot;q&quot;"));
        assert!(html.contains("2.0 KB") && html.contains("1.0 GB") && html.contains("1/3 free"));
        assert!(html.contains("now <span class=\"now\">20%</span> · peak 20%"));
        assert!(html.contains("Power</span><span>no data"));
        assert!(html.contains("class=\"status-failed\""));
        assert!(html.contains("http-equiv=\"refresh\""));
        assert!(!login_page(Some("<b>")).contains("<b>"));
    }

    #[test]
    fn reads_the_cookie() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_static("a=1; lumi_admin=abc%3D%3D; b=2"));
        assert_eq!(cookie_token(&h).as_deref(), Some("abc=="));
        assert!(set_cookie("x", 10, true).to_str().unwrap().contains("HttpOnly; SameSite=Strict; Max-Age=10; Secure"));
    }
}
