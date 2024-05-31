pub mod buffer;
pub mod manifest;
pub mod parse;
pub mod player;
pub mod range;

use dioxus::prelude::*;
use futures::channel::mpsc;
use futures::channel::oneshot;

pub enum PlayerState {
    Created { 
        id: String,
        manifest: String,
        tx: oneshot::Sender<Vec<()>>,
    },
}

pub struct MediaPlayer {
    task: TaskId,
    tx: mpsc::Sender<PlayerState>,

    cached_track_list: Option<Vec<()>>,
}

impl MediaPlayer {
    pub fn new(cx: Scope<()>) -> Self {
        let mut player = player::Player::new();
        let (tx, rx) = mpsc::channel(2048);

        let task = cx.push_future(async move {
            player
                .listen(rx)
                .await
                .expect("Player Backend died unexpectedly.");
        });

        Self { task, tx, cached_track_list: None }
    }

    pub async fn create(&mut self, id: String, manifest: String) {
        let (tx, rx) = oneshot::channel();

        self.tx
            .try_send(PlayerState::Created { id, manifest, tx })
            .expect("Channel full");

        // TODO: Get a result instead so that we know whether loading the manifest was successful.
        /*
        let tracks = rx.await.unwrap();

        self.cached_track_list = Some(tracks);
        */
    }

    pub fn tracks(&self) -> Vec<()> {
        self.cached_track_list.clone().unwrap_or_default()
    }

    pub fn destroy(self, cx: Scope<()>) {
        // FIXME: We might have to issue a cleanup command to clean up buffers.
        cx.remove_future(self.task);
    }
}
