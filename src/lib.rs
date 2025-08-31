pub mod buffer;
pub mod manifest;
pub mod parse;
pub mod player;
pub mod range;

use dioxus::prelude::*;
use futures::channel::{mpsc, oneshot};
use wasm_bindgen_futures::spawn_local;


#[derive(Debug)]
pub enum PlayerState {
    Created {
        id: String,
        manifest: String,
        tx: Option<oneshot::Sender<Result<(), Box<dyn std::error::Error>>>>,
    },
    Cleanup,
}

pub struct MediaPlayer {
    tx: mpsc::Sender<PlayerState>,

    cached_track_list: Option<Vec<()>>,
}

impl MediaPlayer {
    pub fn new() -> Self {
        let mut player = player::Player::new();
        let (tx, rx) = mpsc::channel(2048);

        spawn_local(async move {
            if let Err(e) = player.listen(rx).await {
                tracing::error!("Player listen failed: {:?}", e);
            }
        });

        Self { tx, cached_track_list: None }
    }

    pub async fn create(&mut self, id: String, manifest: String) -> Result<(), Box<dyn std::error::Error>> {
        let (tx, rx) = oneshot::channel();

        self.tx
            .try_send(PlayerState::Created { id, manifest, tx: Some(tx) })
            .expect("Channel full");

        let result = rx.await;
        match result {
            Ok(Ok(())) => {
                tracing::info!("Manifest loaded successfully");
                Ok(())
            },
            Ok(Err(e)) => {
                tracing::error!("Failed to load manifest: {:?}", e);
                Err(e)
            },
            Err(_) => {
                tracing::error!("Channel canceled");
                Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "channel canceled")))
            },
        }
    }

    pub fn tracks(&self) -> Vec<()> {
        self.cached_track_list.clone().unwrap_or_default()
    }

    pub fn destroy(mut self) {
        // Send cleanup command to player thread
        let _ = self.tx.try_send(PlayerState::Cleanup);

        // The spawned listen loop will handle cleanup on drop
    }
}
