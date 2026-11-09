//! Tray icon (Windows, macOS): status, open the page, pause/resume, and the privacy
//! disclaimer repeated word for word.

use crate::web::{App, DISCLAIMER};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIconBuilder};

fn icon(rgb: [u8; 3]) -> Icon {
    let n = 32u32;
    let mut px = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
            let d = (dx * dx + dy * dy).sqrt();
            let a = if d < 14.0 { 255 } else if d < 15.5 { ((15.5 - d) / 1.5 * 255.0) as u8 } else { 0 };
            // a small white "note" dot keeps it readable on dark and light bars
            let white = ((x as f32 - 19.0).powi(2) + (y as f32 - 12.0).powi(2)).sqrt() < 4.0;
            let c = if white { [255, 255, 255] } else { rgb };
            px.extend_from_slice(&[c[0], c[1], c[2], a]);
        }
    }
    Icon::from_rgba(px, n, n).expect("icon")
}

/// Splits the disclaimer into menu-sized lines without changing a word.
fn lines(s: &str, width: usize) -> Vec<String> {
    let mut out = vec![String::new()];
    for w in s.split(' ') {
        let cur = out.last_mut().unwrap();
        if !cur.is_empty() && cur.len() + w.len() + 1 > width {
            out.push(w.to_string());
        } else {
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(w);
        }
    }
    out
}

pub fn run(app: Arc<App>) -> ! {
    let event_loop = EventLoopBuilder::new().build();
    let status = MenuItem::new("Starting…", false, None);
    let open = MenuItem::new("Open Ornigo", true, None);
    let toggle = MenuItem::new("Pause listening", true, None);
    let phone = MenuItem::new("See your birds on your phone (honemesh.net/me)", true, None);
    let quit = MenuItem::new("Quit", true, None);
    let menu = Menu::new();
    let _ = menu.append(&status);
    let _ = menu.append(&open);
    let _ = menu.append(&toggle);
    let _ = menu.append(&phone);
    let _ = menu.append(&PredefinedMenuItem::separator());
    for l in lines(DISCLAIMER, 48) {
        let _ = menu.append(&MenuItem::new(l, false, None));
    }
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&quit);
    let mut tray = None;
    let mut next = Instant::now();
    let url = crate::web::url(app.port, &app.token, "127.0.0.1");
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(500));
        if let Event::NewEvents(StartCause::Init) = event {
            tray = TrayIconBuilder::new().with_menu(Box::new(menu.clone())).with_tooltip("Ornigo").with_icon(icon([120, 120, 120])).build().ok();
        }
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            if ev.id == open.id() {
                let _ = open::that(&url);
            } else if ev.id == toggle.id() {
                let on = app.cfg.lock().unwrap().listening;
                if on || app.cfg.lock().unwrap().ready() {
                    app.update_cfg(|c| c.listening = !on);
                } else {
                    let _ = open::that(&url);
                }
                next = Instant::now();
            } else if ev.id == phone.id() {
                let _ = open::that(crate::web::PHONE_URL);
            } else if ev.id == quit.id() {
                crate::voiceprint::save_now();
                std::process::exit(0);
            }
        }
        if Instant::now() >= next {
            next = Instant::now() + Duration::from_secs(5);
            let (listening, ready) = {
                let c = app.cfg.lock().unwrap();
                (c.listening, c.ready())
            };
            let s = if ready { crate::live::summary(listening) } else { "Finish setup to start listening".into() };
            let l = crate::live::snapshot();
            let color = if l.model_error.is_some() { [200, 60, 50] } else if !listening { [120, 120, 120] } else if l.stations.values().any(|x| x.running) { [46, 139, 87] } else { [215, 150, 40] };
            status.set_text(s.chars().take(90).collect::<String>());
            toggle.set_text(if listening { "Pause listening" } else { "Start listening" });
            if let Some(t) = tray.as_ref() {
                let _ = t.set_tooltip(Some(s.chars().take(120).collect::<String>()));
                let _ = t.set_icon(Some(icon(color)));
            }
        }
    })
}
