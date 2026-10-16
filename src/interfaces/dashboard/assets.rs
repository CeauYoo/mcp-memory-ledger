//! Compile-time bundled, allowlisted assets; never serves arbitrary filesystem paths.
pub const MEMORY_CHAN_HERO_PNG: &[u8] = include_bytes!("static/memory_chan_hero.png");
pub const MEMORY_CHAN_SIDEBAR_PNG: &[u8] = include_bytes!("static/memory_chan_sidebar.png");
pub const DASHBOARD_HTML: &str = include_str!("static/index.html");

pub fn frontend_asset(name: &str) -> Option<(&'static str, &'static str)> {
    let body = match name {
        "dashboard.css" => {
            return Some((
                "text/css; charset=utf-8",
                include_str!("static/dashboard.css"),
            ));
        }
        "i18n.mjs" => include_str!("static/i18n.mjs"),
        "state.mjs" => include_str!("static/state.mjs"),
        "transport.mjs" => include_str!("static/transport.mjs"),
        "render.mjs" => include_str!("static/render.mjs"),
        "tasks.mjs" => include_str!("static/tasks.mjs"),
        "app.mjs" => include_str!("static/app.mjs"),
        _ => return None,
    };
    Some(("text/javascript; charset=utf-8", body))
}
