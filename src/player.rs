use crate::buffer::TrackBufferManager;
use crate::manifest::Manifest;
use crate::manifest::Track;
use crate::PlayerState;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use web_sys::HtmlVideoElement;

use futures::channel::mpsc::Receiver;
use futures::future::FutureExt;
use futures::stream::FuturesUnordered;
use futures::StreamExt;

use gloo_timers::future::TimeoutFuture;

use core::future::Future;
use core::pin::Pin;
use core::time::Duration;
use std::collections::HashMap;

use displaydoc::Display;
use thiserror::Error;

pub type BoxError = Box<dyn std::error::Error>;
pub type ScheduledEvent = Pin<Box<dyn Future<Output = InternalEvent>>>;

pub struct Player {
    video_id: Option<String>,
    manifest_url: Option<String>,
    manifest: Option<Manifest>,

    /// Internal event queue is used to react to events such as those coming from event listeners,
    /// without blocking the UI in any way.
    rcvr: flume::Receiver<InternalEvent>,
    sndr: flume::Sender<InternalEvent>,

    video_element: Option<HtmlVideoElement>,
    media_source: web_sys::MediaSource,

    scheduled_events: FuturesUnordered<ScheduledEvent>,
    active_tracks: HashMap<usize, TrackBufferManager>,
    result_tx: Option<futures::channel::oneshot::Sender<Result<(), Box<dyn std::error::Error>>>>,
}

impl Player {
    pub fn new() -> Self {
        let (sndr, rcvr) = flume::unbounded();
        let media_source = web_sys::MediaSource::new().unwrap();

        Self {
            video_id: None,
            manifest_url: None,
            manifest: None,
            scheduled_events: FuturesUnordered::new(),
            video_element: None,
            active_tracks: HashMap::new(),
            sndr,
            rcvr,
            media_source,
            result_tx: None,
        }
    }

    pub async fn listen(&mut self, mut cx: Receiver<PlayerState>) -> Result<(), BoxError> {
        loop {
            futures::select_biased! {
                event = cx.next() => {
                    let Some(event) = event else {
                        tracing::info!("Breaking because events dropped.");
                        break;
                    };

                    match event {
                        PlayerState::Created { manifest, id, tx } => {
                            self.detach();
                            self.manifest_url = Some(manifest);
                            self.video_id = Some(id);
                            self.result_tx = tx;

                            if let Err(e) = self.load_manifest().await {
                                tracing::error!(error = ?e, "Load manifest failed.");
                                if let Some(tx) = self.result_tx.take() { let _ = tx.send(Err(e)); }
                            } else if let Err(e) = self.attach().await {
                                tracing::error!(error = ?e, "Attach failed.");
                                if let Some(tx) = self.result_tx.take() { let _ = tx.send(Err(e)); }
                            } else {
                                // Success
                                if let Some(tx) = self.result_tx.take() { let _ = tx.send(Ok(())); }
                            }
                        }
                        PlayerState::Cleanup => {
                            break;
                        }
                    }
                }
                event = self.rcvr.recv_async() => {
                    let Ok(event) = event else {
                        tracing::info!("Breaking because internal_events dropped.");
                        break;
                    };

                    self.process_internal_event(event).await?;
                }
                // FIXME: FutUnord when polled empty might return None, which
                event = self.scheduled_events.next() => {
                    if let Some(event) = event {
                        self.process_internal_event(event).await?;
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn process_internal_event(&mut self, event: InternalEvent) -> Result<(), BoxError> {
        match event {
            InternalEvent::SourceOpen => self.on_source_open().await?,
            InternalEvent::Seeking => self.on_seeking().await?,
            InternalEvent::TryLoadSegment {
                track,
                next_segment,
            } => self.try_load_segment(track, next_segment).await?,
        }

        Ok(())
    }

    async fn load_manifest(&mut self) -> Result<(), BoxError> {
        let manifest_url = self.manifest_url.as_ref().unwrap();

        tracing::info!(manifest_url, "Loading manifest...");

        let xml = reqwest::get(manifest_url).await?.text().await?;

        self.manifest = Some(xml.parse()?);

        tracing::info!("Manifest parsed...");

        Ok(())
    }

    async fn attach(&mut self) -> Result<(), BoxError> {
        tracing::info!("Attaching to player");

        let video_element = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .get_element_by_id(self.video_id())
            .unwrap()
            .dyn_into::<web_sys::HtmlVideoElement>()
            .unwrap();

        self.video_element = Some(video_element.clone());

        // TODO: Add event handler for current time update.
        let sndr = self.sndr.clone();

        self.add_event_listener("seeking", move || {
            let _ = sndr.send(InternalEvent::Seeking);
        });

        let sndr = self.sndr.clone();

        self.add_event_listener("timeupdate", move || {
            let _ = sndr.send(InternalEvent::Seeking);
        });

        let sndr = self.sndr.clone();

        let event_listener = Closure::once(Box::new(move || {
            tracing::info!("Sending SourceOpen");

            let _ = sndr.send(InternalEvent::SourceOpen);
        }));

        self.media_source
            .add_event_listener_with_callback(
                "sourceopen",
                &event_listener.as_ref().unchecked_ref(),
            )
            .unwrap();

        event_listener.forget();

        let url = web_sys::Url::create_object_url_with_source(&self.media_source).unwrap();
        video_element.set_src(&url);

        Ok(())
    }

    fn detach(&mut self) {
        // First we clear scheduled events and mem-swap the internal receivers.
        self.scheduled_events = FuturesUnordered::new();
        let (sndr, rcvr) = flume::unbounded();

        self.sndr = sndr;
        self.rcvr = rcvr;

        for (_, track) in self.active_tracks.drain() {
            track.cleanup();
        }
    }

    fn schedule(&mut self, event: InternalEvent, deadline: Duration) {
        self.scheduled_events.push(
            async move {
                TimeoutFuture::new(deadline.as_millis() as _).await;
                event
            }
            .boxed_local(),
        );
    }

    fn base_url(&self) -> url::Url {
        let mut url = url::Url::parse(self.manifest_url()).expect("Invalid manifest url.");

        url.path_segments_mut().unwrap().pop();

        url
    }

    fn add_event_listener(&mut self, event: &str, callback: impl Fn() + 'static) {
        let video = self.video();
        let callback: Closure<dyn FnMut()> = Closure::new(Box::new(callback));

        video
            .add_event_listener_with_callback(event, &callback.as_ref().unchecked_ref())
            .unwrap();

        callback.forget();
    }

    async fn on_source_open(&mut self) -> Result<(), BoxError> {
        let duration = self
            .manifest
            .as_ref()
            .unwrap()
            .duration()
            .unwrap()
            .as_secs_f64();

        self.media_source.set_duration(duration);

        // FIXME: Handle multiple video tracks gracefully.
        for (index, track) in self.tracks().into_iter().enumerate() {
            tracing::info!(?track);
            if track.is_video() {
                let manager = TrackBufferManager::new(self.media_source.clone(), track)
                    .with_base_url(self.base_url());

                self.active_tracks.insert(index, manager);

                break;
            }
        }

        // FIXME: Handle multiple audio tracks gracefully.
        for (index, track) in self.tracks().into_iter().enumerate() {
            tracing::info!(?track);
            if track.is_audio() {
                let manager = TrackBufferManager::new(self.media_source.clone(), track)
                    .with_base_url(self.base_url());

                self.active_tracks.insert(index, manager);

                break;
            }
        }

        tracing::info!("Prepared track buffers.");

        self.load_init().await?;

        Ok(())
    }

    async fn load_init(&mut self) -> Result<(), BoxError> {
        for (track_id, track) in self.active_tracks.iter_mut() {
            tracing::info!(track_id, "Loading init segment.");
            // TODO: Spawn on executor so we dont block event processing.
            let init = track.fetch_init_segment().await?;
            track.append_init_segment(init)?;

            self.sndr
                .send_async(InternalEvent::TryLoadSegment {
                    track: *track_id,
                    next_segment: None,
                })
                .await?;
        }

        Ok(())
    }

    async fn try_load_segment(
        &mut self,
        track: usize,
        next_segment: Option<usize>,
    ) -> Result<(), BoxError> {
        let manager = self.active_tracks.get_mut(&track).unwrap();

        let Ok(segment) = manager.fetch_segment(next_segment).await else {
            tracing::info!("Failed to fetch segment");
            return Ok(());
        };

        // TODO: Handle timestamp in segment is out of range error.
        match manager.append_segment(segment).await {
            Err(Error::QuotaExceededError) => {
                tracing::error!("Got a Quota error during append.");
                // Schedule append for later.
                self.schedule(
                    InternalEvent::TryLoadSegment {
                        track,
                        next_segment: None,
                    },
                    Duration::from_millis(1000),
                );
            }
            Err(Error::OutOfRange { next_segment }) => {
                tracing::error!("Guessed segment not within range, fetching next one.");
                self.sndr
                    .send_async(InternalEvent::TryLoadSegment {
                        track,
                        next_segment: Some(next_segment),
                    })
                    .await?;
            }
            Err(error) => return Err(Box::new(error)),
            Ok(()) => {
                self.schedule(
                    InternalEvent::TryLoadSegment {
                        track,
                        next_segment: None,
                    },
                    Duration::from_millis(200),
                );
            }
        }

        Ok(())
    }

    async fn on_seeking(&mut self) -> Result<(), Error> {
        let video = self.video();
        let current_time = video.current_time();

        tracing::info!(timestamp = video.current_time(), "Timeupdate / Seeking...");

        for (id, track) in self.active_tracks.iter_mut() {
            if !track.current_time(current_time) {
                self.sndr
                    .send_async(InternalEvent::TryLoadSegment {
                        track: *id,
                        next_segment: None,
                    })
                    .await
                    .unwrap();
            }
        }

        Ok(())
    }

    fn video(&mut self) -> &HtmlVideoElement {
        self.video_element.as_ref().unwrap()
    }

    fn manifest_url(&self) -> &str {
        self.manifest_url.as_ref().unwrap()
    }
    
    fn video_id(&self) -> &str {
        self.video_id.as_ref().unwrap()
    }

    fn tracks(&self) -> Vec<Track> {
        self.manifest.as_ref().unwrap().tracks()
    }
}

pub enum InternalEvent {
    SourceOpen,
    TryLoadSegment {
        track: usize,
        next_segment: Option<usize>,
    },
    Seeking,
}

#[derive(Clone, Copy, Debug, Display, Error)]
pub enum Error {
    /// Quota error
    QuotaExceededError,
    /// Fetch error
    FetchError,
    /// Data error
    DataError,
    /// Server returned non 200 code
    HttpCode,
    /// The given segment is out of range for our timestamp
    OutOfRange { next_segment: usize },
}
