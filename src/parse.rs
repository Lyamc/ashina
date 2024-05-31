use mp4::box_start;
use mp4::skip_box;
use mp4::skip_bytes_to;
use mp4::BoxHeader;
use mp4::BoxType;
use mp4::MoofBox;
use mp4::Mp4Box;
use mp4::ReadBox;
use mp4::Result;
use mp4::HEADER_EXT_SIZE;
use mp4::HEADER_SIZE;

use std::io::BufReader;
use std::io::Cursor;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::time::Duration;

use byteorder::BigEndian;
use byteorder::ReadBytesExt;

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
struct SidxBox {
    version: u8,
    flags: u32,
    reference_id: u32,
    timescale: u32,
    earliest_presentation_time: u64,
    first_offset: u64,

    subseg_durations: Vec<u32>,
}

impl SidxBox {
    pub fn get_type(&self) -> BoxType {
        unimplemented!()
    }

    pub fn total_duration(&self) -> u32 {
        self.subseg_durations.iter().sum()
    }

    pub fn get_size(&self) -> u64 {
        let sub_hdr_sz = match self.version {
            0 => 8,
            _ => 16,
        };

        HEADER_SIZE
            + HEADER_EXT_SIZE
            + 4
            + 8
            + sub_hdr_sz
            + (self.subseg_durations.len() as u64 * 12)
    }
}

impl Mp4Box for SidxBox {
    fn box_type(&self) -> BoxType {
        self.get_type()
    }

    fn box_size(&self) -> u64 {
        self.get_size()
    }

    fn to_json(&self) -> Result<String> {
        unimplemented!();
    }

    fn summary(&self) -> Result<String> {
        Ok(String::new())
    }
}

impl<R: Read + Seek> ReadBox<&mut R> for SidxBox {
    fn read_box(reader: &mut R, size: u64) -> Result<Self> {
        let start = box_start(reader)?;

        let version = reader.read_u8()?;
        let flags = reader.read_u24::<BigEndian>()?;

        let reference_id = reader.read_u32::<BigEndian>()?;
        let timescale = reader.read_u32::<BigEndian>()?;

        let earliest_presentation_time = match version {
            0 => reader.read_u32::<BigEndian>()? as u64,
            _ => reader.read_u64::<BigEndian>()?,
        };

        let first_offset = match version {
            0 => reader.read_u32::<BigEndian>()? as u64,
            _ => reader.read_u64::<BigEndian>()?,
        };

        let _reserved = reader.read_u16::<BigEndian>()?;
        let ref_count = reader.read_u16::<BigEndian>()?;

        let mut subseg_durations = Vec::new();
        for idx in 1..=ref_count {
            let _ = reader.read_u32::<BigEndian>()?;
            let duration = reader.read_u32::<BigEndian>()?;
            tracing::info!(idx, "got here.");

            let _ = reader.read_u32::<BigEndian>()?;

            subseg_durations.push(duration);
        }

        skip_bytes_to(reader, start + size)?;

        Ok(Self {
            version,
            flags,
            reference_id,
            timescale,
            earliest_presentation_time,
            first_offset,
            subseg_durations,
        })
    }
}

const SIDX_BOX: u32 = 0x73696478;

#[derive(Clone, Copy, Debug)]
pub struct SegmentMetadata {
    pub segment_number: usize,
    pub earliest_presentation_time: f64,
    pub timescale: f64,
    pub total_duration: f64,
}

impl SegmentMetadata {
    #[track_caller]
    pub fn parse(data: &[u8]) -> Result<Self> {
        let cursor = Cursor::new(data);
        let mut rdr = BufReader::new(cursor);
        let mut current = rdr.seek(SeekFrom::Current(0))?;

        let mut sidx = None;
        let mut moof = None;

        while current < data.len() as _ {
            let header = BoxHeader::read(&mut rdr)?;

            match header.name {
                BoxType::UnknownBox(SIDX_BOX) => {
                    tracing::info!("Parsing sidx");
                    sidx = Some(SidxBox::read_box(&mut rdr, header.size)?);
                    tracing::info!("Parsed sidx");
                }
                BoxType::MoofBox => {
                    tracing::info!("Parsing moof");
                    moof = Some(MoofBox::read_box(&mut rdr, header.size)?);
                    tracing::info!("Parsed moof");
                }
                rest => {
                    tracing::info!(?rest, "Unknown box type.");
                    skip_box(&mut rdr, header.size)?;
                }
            }

            current = rdr.seek(SeekFrom::Current(0))?;
        }

        let sidx = sidx.expect("No Sidx box found.");
        let moof = moof.expect("No moof box found.");

        Ok(Self {
            segment_number: moof.mfhd.sequence_number as _,
            earliest_presentation_time: sidx.earliest_presentation_time as _,
            timescale: sidx.timescale as _,
            total_duration: sidx.total_duration() as _,
        })
    }

    pub fn pts(&self) -> f64 {
        self.earliest_presentation_time / self.timescale
    }

    pub fn duration(&self) -> Duration {
        let duration = (self.total_duration / self.timescale) * 1000.0;
        Duration::from_millis(duration as _)
    }

    pub fn segment_number(&self) -> usize {
        self.segment_number
    }
}
