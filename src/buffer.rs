use crate::manifest::Track;
use crate::parse::SegmentMetadata;
use crate::player::BoxError;
use crate::player::Error;
use crate::range::NRangeInclusive;

use wasm_bindgen::JsCast;
use web_sys::MediaSource;
use web_sys::SourceBuffer;

use core::future::Future;
use core::ops::RangeInclusive;

use url::Url;

// default segment duration in case the dash template has no segment duration defined.
const SEGMENT_DURATION: f64 = 10.;

pub struct TrackBufferManager {
    /// The base URL for this track
    base_url: Url,
    /// Copy of the video track from the manifest
    track: Track,
    /// The source buffer for which we are responsible
    source_buffer: SourceBuffer,
    /// The last fetched segment
    current_segment: usize,
    /// Reference to the media source
    media_source: MediaSource,
    /// The target render timestamp for the current video.
    current_time: f64,
}

impl TrackBufferManager {
    pub fn new(media_source: MediaSource, track: Track) -> Self {
        let codec = format!("{}; codecs=\"{}\"", track.mime(), track.codecs());
        let source_buffer = media_source.add_source_buffer(&codec).unwrap();

        Self {
            current_segment: 0,
            base_url: Url::parse("http://127.0.0.1/").unwrap(),
            current_time: 0.,
            track,
            source_buffer,
            media_source,
        }
    }

    pub fn with_base_url(mut self, base_url: url::Url) -> Self {
        self.base_url = base_url;
        self
    }

    pub fn id(&self) -> String {
        self.track.id()
    }

    pub fn cleanup(self) {
        self.media_source
            .remove_source_buffer(&self.source_buffer)
            .unwrap();
    }

    pub fn fetch_init_segment(&self) -> impl Future<Output = Result<Vec<u8>, BoxError>> {
        let mut init_segment = self.track.initialization();
        init_segment.set_id(self.id());

        let path = self.segment_path(&init_segment);

        async move { Ok(reqwest::get(path).await?.bytes().await?.to_vec()) }
    }

    pub fn append_init_segment(&mut self, mut data: Vec<u8>) -> Result<(), BoxError> {
        self.source_buffer
            .append_buffer_with_u8_array(&mut data)
            .unwrap();
        Ok(())
    }

    /// Method sets the current time of seek to `time` and returns a boolean indicating whether the
    /// timestamp is within the buffered range of time or not. This is meant to be used as an
    /// indication of whether we need to ask the player to fetch the next segment or not for the
    /// timestamp.
    #[inline]
    pub fn current_time(&mut self, time: f64) -> bool {
        self.current_time = time;
        self.buffered().contains(&time)
    }

    #[track_caller]
    pub fn fetch_segment(
        &mut self,
        segment_id: Option<usize>,
    ) -> impl Future<Output = Result<Vec<u8>, Error>> {
        let segment = if !self.buffered().contains(&self.current_time) {
            // We are buffering, so we fetch the current_time segment or the segment id passed in.
            let target = segment_id.unwrap_or_else(|| self.segment_for_ts(self.current_time));
            tracing::info!(
                target,
                current = self.current_time,
                "Guessing segment because of hard seek."
            );
            target
        } else {
            // We are not buffering so we can continue fetching the next segment
            let target = self.current_segment + 1;
            tracing::info!(target, "Asking for segment.");
            target
        };

        let mut path = self.track.media();
        path.set_id(self.id());
        path.set_number(segment);

        let path = self.segment_path(&path);

        async move {
            tracing::info!(?path, "Fetching segment.");
            let request = reqwest::get(path).await.map_err(|_| Error::FetchError)?;

            if request.status() != reqwest::StatusCode::OK {
                return Err(Error::HttpCode);
            }

            let data = request
                .bytes()
                .await
                .map_err(|_| Error::DataError)?
                .to_vec();

            Ok(data)
        }
    }

    pub fn buffered(&self) -> NRangeInclusive<f64> {
        let mut range = NRangeInclusive::new();

        let ranges = self.source_buffer.buffered().unwrap();

        for idx in 0..ranges.length() {
            let start = ranges.start(idx).unwrap();
            let end = ranges.end(idx).unwrap();

            range.push(start..=end);
        }

        range
    }

    pub fn is_buffering(&self) -> bool {
        !self.buffered().contains(&self.current_time)
    }

    pub async fn append_segment(&mut self, mut segment: Vec<u8>) -> Result<(), Error> {
        let metadata = SegmentMetadata::parse(&segment).expect("Failed to parse segment.");

        tracing::info!(?metadata, "New segment...");

        if self.is_buffering() {
            let segment_range = RangeInclusive::new(
                metadata.pts(),
                metadata.pts() + metadata.duration().as_secs_f64(),
            );

            tracing::info!(
                start = segment_range.start(),
                end = segment_range.end(),
                "Segment range."
            );
            if !segment_range.contains(&self.current_time) {
                // The segment we are attempting to append does not contain our requested timestamp
                let next_segment = if self.current_time < metadata.pts() {
                    metadata.segment_number - 1
                } else {
                    metadata.segment_number + 1
                };

                return Err(Error::OutOfRange { next_segment });
            }
        }

        // NOTE: Don't be tempted to use append_buffer_async_* as no browsers support this.
        if let Err(error) = self.source_buffer.append_buffer_with_u8_array(&mut segment) {
            let Ok(error) = error.dyn_into::<js_sys::Error>() else {
                panic!("Weird error mhmmm.");
            };

            let name = error.name().as_string().unwrap();

            match name.as_str() {
                "QuotaExceededError" => return Err(Error::QuotaExceededError),
                error => {
                    tracing::error!(?error, "Weird error");
                    // TODO: Handle InvalidStateError
                    return Err(Error::QuotaExceededError);
                }
            }
        }

        self.current_segment = metadata.segment_number;

        Ok(())
    }

    /// Method attempts to guess the segment index for the segment to fetch during a seek. This
    /// needs to be somewhat accurate, but it doesnt have to be as we can bruteforce search
    /// forwards or backwards depending on the real ts that the returned segment has.
    fn segment_for_ts(&self, ts: f64) -> usize {
        let segment_length = self.track.segment_duration().unwrap();
        ((ts / segment_length) + 1.0) as _
    }

    fn segment_path(&self, path: &impl AsRef<str>) -> String {
        let base = self.base_url.as_str().to_string();
        format!("{base}/{}", path.as_ref())
    }
}
