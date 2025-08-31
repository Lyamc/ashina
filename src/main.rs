// use dioxus_dash::*;
use ashina::*;
use dioxus::prelude::*;
use wasm_bindgen_futures::spawn_local;

fn app() -> Element {
    let mut player = use_signal(|| MediaPlayer::new());
    let mut url = use_signal(|| String::new());
    let mut error_signal = use_signal(|| String::new());

    // Compute error text for display
    let error_text = if error_signal.read().is_empty() {
        String::new()
    } else {
        format!("Error: {}", error_signal.read())
    };

    // Render the UI with improved comments and error display
    rsx! {
        div {
            width: "100%",
            margin: "auto",
            display: "flex",
            flex_direction: "column",

            // Input and button section for video URL
            div {
                display: "flex",
                align_items: "center",
                margin_bottom: "20px",

                // Input field for video URL with basic validation (ensure non-empty and HTTP-based URLs)
                input {
                    flex_grow: "1",
                    margin_right: "1rem",
                    padding: "0.625rem",
                    position: "relative",
                    width: "200px",
                    oninput: move |event| {
                        let value = event.value().clone();
                        if !value.trim().is_empty() && value.starts_with("http") {
                            *url.write() = value;
                        } else {
                            *url.write() = String::new(); // Reset invalid inputs
                        }
                    },
                },
                // Load button triggers async video loading with proper error handling
                button {
                    flex_basis: "15%",
                    font_size: "1rem",
                    padding: "0.625rem",
                    onclick: move |_| {
                        spawn_local(async move {
                            let url_val = url.read().clone();
                            let mut player_guard = player.write();
                            match player_guard.create("video-player".into(), url_val).await {
                                Ok(_) => {
                                    *error_signal.write() = String::new();
                                }
                                Err(e) => {
                                    *error_signal.write() = format!("Failed to load video: {}", e);
                                }
                            }
                        });
                    },
                    "Load"
                },
            }
            // Video element for playback
            video {
                id: "video-player",
                controls: true,
                autoplay: true,
                height: "auto",
                width: "100%",
                background_color: "black",
                "Your video should load here."
            },
            // Error display section for handling failures
            if !error_text.is_empty() {
                div {
                    color: "red",
                    "{error_text}"
                }
            }
        }
    }
}

fn main() {
    tracing_wasm::set_as_global_default();
    dioxus::launch(app)
}
