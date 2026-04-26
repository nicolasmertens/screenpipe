//! secondbrain menubar app.
//!
//! Single process: hosts the capture loop in a tokio runtime + a
//! macOS tray icon driven by the AppKit main run loop. The tray icon
//! shows recording state, exposes Pause/Resume/Reveal/Quit, and lets
//! you open the secondbrain data folder.
//!
//! Built as a menubar-only app (LSUIElement = true in Info.plist),
//! so no Dock icon and no application window.

#![cfg(target_os = "macos")]

use anyhow::{Context, Result};
use secondbrain_capture::{run as capture_run, CaptureConfig};
use secondbrain_store::Store;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::{Icon, TrayIconBuilder, TrayIconEvent};

const APP_NAME: &str = "secondbrain";

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let recording = Arc::new(AtomicBool::new(true));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;

    // Spawn the capture task. It will read the recording flag each tick.
    {
        let recording = Arc::clone(&recording);
        runtime.spawn(async move {
            if let Err(e) = capture_main(recording).await {
                tracing::error!(error = ?e, "capture task exited with error");
            }
        });
    }

    let event_loop = EventLoopBuilder::new().build();

    let icon = build_status_icon(true);
    let menu = Menu::new();
    let pause_item = MenuItem::new("Pause recording", true, None);
    let resume_item = MenuItem::new("Resume recording", true, None);
    let open_data_item = MenuItem::new("Open data folder", true, None);
    let about_item = MenuItem::new(format!("{APP_NAME} — recording"), false, None);
    let quit_item = MenuItem::new("Quit", true, None);

    menu.append(&about_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&pause_item)?;
    menu.append(&resume_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&open_data_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&quit_item)?;

    // Capture IDs by value so we can match in the event loop.
    let pause_id = pause_item.id().clone();
    let resume_id = resume_item.id().clone();
    let open_data_id = open_data_item.id().clone();
    let quit_id = quit_item.id().clone();

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(format!("{APP_NAME} — recording"))
        .with_icon(icon)
        .with_menu_on_left_click(true)
        .build()
        .context("building tray icon")?;

    let menu_channel = MenuEvent::receiver();
    let tray_channel = TrayIconEvent::receiver();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(
            std::time::Instant::now() + Duration::from_millis(200),
        );

        if let Event::NewEvents(_) = event {
            // Drain menu events.
            while let Ok(ev) = menu_channel.try_recv() {
                if ev.id == pause_id {
                    recording.store(false, Ordering::SeqCst);
                    if let Ok(icon) = build_status_icon_result(false) {
                        let _ = tray.set_icon(Some(icon));
                    }
                    let _ = tray.set_tooltip(Some(format!("{APP_NAME} — paused")));
                    about_item.set_text(format!("{APP_NAME} — paused"));
                } else if ev.id == resume_id {
                    recording.store(true, Ordering::SeqCst);
                    if let Ok(icon) = build_status_icon_result(true) {
                        let _ = tray.set_icon(Some(icon));
                    }
                    let _ = tray.set_tooltip(Some(format!("{APP_NAME} — recording")));
                    about_item.set_text(format!("{APP_NAME} — recording"));
                } else if ev.id == open_data_id {
                    if let Ok(path) = secondbrain_store::default_db_path() {
                        if let Some(parent) = path.parent() {
                            let _ = std::process::Command::new("open").arg(parent).spawn();
                        }
                    }
                } else if ev.id == quit_id {
                    *control_flow = ControlFlow::Exit;
                }
            }
            // Drain tray events (left click etc.) — currently unused.
            while let Ok(_) = tray_channel.try_recv() {}
        }
    });
}

async fn capture_main(recording: Arc<AtomicBool>) -> Result<()> {
    let store = Store::open_default().await?;
    let cfg = CaptureConfig::default();
    // The capture loop in `secondbrain-capture` runs forever; for
    // pause-on-click we wrap it in a polling task that holds the loop
    // until the flag flips back. The capture loop itself does not need
    // to know about pause/resume — we simply gate calling it.
    loop {
        // Wait until recording is enabled.
        while !recording.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        // Run the capture loop. It returns only on fatal error.
        if let Err(e) = capture_run(store.clone(), cfg.clone()).await {
            tracing::error!(error = ?e, "capture loop exited; restarting in 5s");
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
}

/// Build a simple solid-color icon for the menubar.
/// 16×16 RGBA: green dot on transparent when recording, gray when paused.
fn build_status_icon(recording: bool) -> Icon {
    build_status_icon_result(recording).expect("static icon should always build")
}

fn build_status_icon_result(recording: bool) -> Result<Icon> {
    let size = 18u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let (r, g, b) = if recording {
        (76, 175, 80) // green
    } else {
        (158, 158, 158) // gray
    };
    let cx = size as f32 / 2.0;
    let radius = (size as f32 / 2.0) - 2.0;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cx;
            let dist = (dx * dx + dy * dy).sqrt();
            let i = ((y * size + x) * 4) as usize;
            if dist <= radius {
                rgba[i] = r;
                rgba[i + 1] = g;
                rgba[i + 2] = b;
                rgba[i + 3] = 255;
            }
        }
    }
    Ok(Icon::from_rgba(rgba, size, size).context("Icon::from_rgba")?)
}
