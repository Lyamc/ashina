use std::str::FromStr;
use std::time::Duration;

use dash_mpd::AdaptationSet;
use dash_mpd::Representation;
use dash_mpd::SegmentTemplate;

use regex::Regex;

pub struct Manifest {
    inner: dash_mpd::MPD,
}

impl FromStr for Manifest {
    type Err = dash_mpd::DashMpdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mpd = dash_mpd::parse(s)?;

        Ok(Self { inner: mpd })
    }
}

impl Manifest {
    pub fn duration(&self) -> Option<Duration> {
        self.inner.mediaPresentationDuration
    }

    pub fn tracks(&self) -> Vec<Track> {
        let mut tracks = vec![];

        for period in &self.inner.periods {
            for adaptation in &period.adaptations {
                for representation in &adaptation.representations {
                    let mut track = Track::new(representation.clone(), adaptation.clone());
                    track.adaptation_segment_template(adaptation.SegmentTemplate.clone());

                    tracks.push(track);
                }
            }
        }

        tracks
    }
}

#[derive(Clone, Debug)]
pub struct Track {
    /// Sometimes manifests dont have a segment template in the representation, but in the
    /// adaptation set.
    adaptation_segment_template: Option<SegmentTemplate>,
    representation: Representation,
    adaptation: AdaptationSet,
}

impl Track {
    fn new(rep: Representation, adaptation: AdaptationSet) -> Self {
        Self {
            representation: rep,
            adaptation_segment_template: None,
            adaptation,
        }
    }

    fn adaptation_segment_template(&mut self, template: Option<SegmentTemplate>) {
        self.adaptation_segment_template = template;
    }

    pub fn id(&self) -> String {
        self.representation.id.clone().unwrap_or_default()
    }

    pub fn segment_template(&self) -> Option<&SegmentTemplate> {
        self.adaptation_segment_template
            .as_ref()
            .or(self.representation.SegmentTemplate.as_ref())
    }

    pub fn is_video(&self) -> bool {
        let mime = self.mime();
        let content_type = self.content_type();

        mime.contains("video") || content_type.contains("video")
    }

    pub fn is_audio(&self) -> bool {
        let mime = self.mime();
        let content_type = self.content_type();

        mime.contains("audio") || content_type.contains("audio")
    }

    pub fn mime(&self) -> String {
        self.representation
            .mimeType
            .as_ref()
            .or(self.adaptation.mimeType.as_ref())
            .cloned()
            .expect("Mime type not set on representation.")
    }

    pub fn codecs(&self) -> String {
        self.representation
            .codecs
            .as_ref()
            .or(self.adaptation.codecs.as_ref())
            .cloned()
            .expect("Codecs not set on representation.")
    }

    pub fn content_type(&self) -> String {
        self.representation
            .contentType
            .as_ref()
            .or(self.adaptation.contentType.as_ref())
            .cloned()
            .expect("Content-Type not set on representation.")
    }

    pub fn initialization(&self) -> ChunkTemplate {
        self.segment_template()
            .expect("Only segment templates are supported.")
            .initialization
            .clone()
            .expect("Initialization segment not listed in template.")
            .into()
    }

    pub fn media(&self) -> ChunkTemplate {
        self.segment_template()
            .expect("Only segment templates are supported.")
            .media
            .clone()
            .expect("Media segment not listed in template.")
            .into()
    }

    pub fn start_number(&self) -> usize {
        self.segment_template()
            .as_ref()
            .unwrap()
            .startNumber
            .unwrap() as _
    }

    pub fn segment_duration(&self) -> Option<f64> {
        // Optional timescale
        let timescale = self
            .segment_template()
            .and_then(|x| x.timescale)
            .unwrap_or(1);

        self.segment_template()
            .and_then(|x| x.duration)
            .map(|duration| duration / timescale as f64)
    }

    pub fn bitrate(&self) -> Option<u64> {
        self.representation.bandwidth
    }

    pub fn width(&self) -> Option<u64> {
        self.representation.width
    }

    pub fn height(&self) -> Option<u64> {
        self.representation.height
    }
}

pub struct ChunkTemplate {
    template: String,
}

impl ChunkTemplate {
    pub fn set_id(&mut self, id: String) {
        self.template = resolve_url_template(&self.template, ("RepresentationID", id));
    }

    pub fn set_number(&mut self, number: usize) {
        self.template = resolve_url_template(&self.template, ("Number", number.to_string()));
    }
}

impl From<String> for ChunkTemplate {
    fn from(template: String) -> ChunkTemplate {
        Self { template }
    }
}

impl AsRef<str> for ChunkTemplate {
    fn as_ref(&self) -> &str {
        self.template.as_str()
    }
}

impl std::fmt::Display for ChunkTemplate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", &self.template)
    }
}

lazy_static::lazy_static! {
    static ref URL_TEMPLATE_IDS: Vec<(&'static str, String, Regex)> = {
        vec!["RepresentationID", "Number", "Time", "Bandwidth"].into_iter()
            .map(|k| (k, format!("${k}$"), Regex::new(&format!("\\${k}%0([\\d])d\\$")).unwrap()))
            .collect()
    };
}

fn resolve_url_template(template: &str, params: (&str, String)) -> String {
    let mut result = template.to_string();

    for (k, ident, rx) in URL_TEMPLATE_IDS.iter() {
        if k != &params.0 {
            continue;
        }

        // first check for simple cases such as $Number$
        if result.contains(ident) {
            result = result.replace(ident, params.1.as_str());
        }
        // now check for complex cases such as $Number%06d$
        if let Some(cap) = rx.captures(&result) {
            let value = params.1.clone();
            let width: usize = cap[1].parse::<usize>().unwrap();
            let count = format!("{value:0>width$}");
            let m = rx.find(&result).unwrap();
            result = result[..m.start()].to_owned() + &count + &result[m.end()..];
        }
    }
    tracing::info!(result);
    dbg!(result)
}
