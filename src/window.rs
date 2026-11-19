//! Native desktop window: a frameless WebView2 window that shows the console UI.

use std::path::PathBuf;

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{Fullscreen, Icon, WindowBuilder};
use wry::{WebContext, WebViewBuilder};

#[derive(Debug)]
enum UiEvent {
    Drag,
    Minimize,
    Maximize,
    Fullscreen,
    Close,
}

/// Opens the window and runs the event loop. Only returns if the window could not be created.
pub fn run(url: String, data_dir: PathBuf, fullscreen: bool, on_exit: Box<dyn FnOnce()>) -> Result<(), String> {
    let event_loop = EventLoopBuilder::<UiEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let icon = Icon::from_rgba(crate::logo::logo_rgba(64), 64, 64).ok();
    let window = WindowBuilder::new()
        .with_title("FahrenHack")
        .with_decorations(false)
        .with_inner_size(LogicalSize::new(1280.0, 800.0))
        .with_min_inner_size(LogicalSize::new(960.0, 600.0))
        .with_window_icon(icon)
        .with_visible(false)
        .build(&event_loop)
        .map_err(|e| e.to_string())?;
    if fullscreen {
        window.set_fullscreen(Some(Fullscreen::Borderless(None)));
    }

    let mut context = WebContext::new(Some(data_dir.join("webview")));
    let webview = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(url)
        .with_background_color((6, 12, 21, 255))
        .with_devtools(cfg!(debug_assertions))
        .with_ipc_handler(move |req| {
            let ev = match req.body().as_str() {
                "drag" => UiEvent::Drag,
                "min" => UiEvent::Minimize,
                "max" => UiEvent::Maximize,
                "fullscreen" => UiEvent::Fullscreen,
                "close" => UiEvent::Close,
                _ => return,
            };
            let _ = proxy.send_event(ev);
        })
        .build(&window)
        .map_err(|e| format!("WebView2: {e}"))?;
    window.set_visible(true);

    let mut on_exit = Some(on_exit);
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let _keep_alive = (&webview, &context);
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } | Event::UserEvent(UiEvent::Close) => {
                if let Some(f) = on_exit.take() {
                    f();
                }
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(UiEvent::Drag) => {
                let _ = window.drag_window();
            }
            Event::UserEvent(UiEvent::Minimize) => window.set_minimized(true),
            Event::UserEvent(UiEvent::Maximize) => window.set_maximized(!window.is_maximized()),
            Event::UserEvent(UiEvent::Fullscreen) => {
                if window.fullscreen().is_some() {
                    window.set_fullscreen(None);
                } else {
                    window.set_fullscreen(Some(Fullscreen::Borderless(None)));
                }
            }
            _ => {}
        }
    });
}
