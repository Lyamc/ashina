use dioxus_dash::*;
use dioxus::prelude::*;

pub fn app(cx: Scope<()>) -> Element {
    let player = use_ref(cx, || MediaPlayer::new(cx));
    let url = use_state(cx, || String::new());

    let load_video = |_| {
        to_owned![player, url];

        async move {
            player
                .write()
                .create("video-player".into(), url.get().clone())
                .await;
        }
    };

    cx.render(rsx! {
        div {
            width: "100%",
            margin: "auto",
            display: "flex",
            flex_direction: "column",

            div {
                display: "flex",
                align_items: "center",
                margin_bottom: "20px",

                input {
                    flex_grow: "1",
                    margin_right: "1rem",
                    padding: "0.625rem",

                    position: "relative",
                    width: "200px",

                    oninput: |event| url.set(event.data.value.clone()),
                },
                button {
                    flex_basis: "15%",
                    font_size: "1rem",
                    padding: "0.625rem",
                    font_size: "1rem",
                    onclick: load_video,
                    "Load"
                },
            }
            video {
                id: "video-player",
                controls: true,
                autoplay: true,
                height: "auto",
                width: "100%",
                background_color: "black",
                "Fuck your browser."
            }
        }
    })
}

fn main() {
    tracing_wasm::set_as_global_default();
    dioxus_web::launch(app)
}
