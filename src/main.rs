use std::collections::HashMap;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use wry::{http::Response, WebViewBuilder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("MITOS Browser")
        .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 800.0))
        .build(&event_loop)?;

    // Zero-overhead custom protocol handler for "mitos://"
    let webview = WebViewBuilder::new(&window)
        .with_custom_protocol("mitos".to_string(), move |_id, request| {
            let uri = request.uri().path();
            match uri {
                "/home" => Response::builder()
                    .header("Content-Type", "text/html")
                    .body(include_bytes!("../assets/home.html").to_vec().into())
                    .unwrap(),
                _ => Response::builder()
                    .status(404)
                    .body(b"404 - Not Found in MITOS".to_vec().into())
                    .unwrap(),
            }
        })
        // Fast IPC: Receive messages from JS without HTTP overhead
        .with_ipc_handler(|_id, payload| {
            println!("IPC Message received: {}", payload);
        })
        .with_url("mitos://home")?
        .build()?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            let _ = &webview;
            *control_flow = ControlFlow::Exit;
        }
    });
}
