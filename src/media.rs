//! Audio and video playback for Preview, apart from egui's widgets: GStreamer plays a
//! file from any seekable reader (ADR-0005).
//!
//! A [`Player`] opens paused. Once it is [`Phase::Ready`] a video's first frame is
//! waiting in [`Player::take_frame`] and an audio file has none. Play, pause, seek,
//! volume and mute go straight to GStreamer, which keeps time and sends the sound to the
//! output [`AudioOut`] picks: the system's, or nowhere for tests and machines without a
//! sound device.
//!
//! The file reaches GStreamer through an `appsrc` in random-access mode, so demuxers ask
//! for the ranges they need (an MP4's index at the end, a seek target) and nothing is
//! read whole. Video frames come out of an `appsink` as RGBA no larger than
//! [`Options::max_side`] on either side, the GPU's texture limit.

use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui::ColorImage;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;

/// Where the sound goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioOut {
    /// The system's default output.
    Device,
    /// Nowhere, played at the same pace: for tests and machines without sound.
    Silent,
}

/// How a [`Player`] plays.
#[derive(Clone)]
pub struct Options {
    pub audio: AudioOut,
    /// The longest side a video frame may have: the GPU's texture limit.
    pub max_side: usize,
    /// Called from GStreamer's threads when there is something new: a frame, the end, a
    /// failure. The window passes its repaint request.
    pub wake: Arc<dyn Fn() + Send + Sync>,
}

/// Where a [`Player`] is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Finding out what the file is and decoding its first frame.
    Opening,
    /// Paused or playing.
    Ready,
    /// The line Preview shows instead: "Cannot preview: unsupported format", or a read
    /// failure such as "Cannot read: connection lost".
    Failed(String),
}

/// The line for a file GStreamer has no demuxer or decoder for.
pub const UNSUPPORTED_FORMAT: &str = "Cannot preview: unsupported format";

/// One file's playback. Dropping it stops playback and closes the reader.
pub struct Player {
    pipeline: Option<gst::Element>,
    phase: Phase,
    playing: bool,
    ended: bool,
    frame: Arc<Mutex<Option<ColorImage>>>,
}

impl Player {
    /// Opens the file `reader` gives, `size` bytes long when known, paused on the first
    /// frame.
    pub fn open<R>(reader: R, size: Option<u64>, options: Options) -> Player
    where
        R: Read + Seek + Send + 'static,
    {
        let frame = Arc::new(Mutex::new(None));
        let mut player = Player {
            pipeline: None,
            phase: Phase::Opening,
            playing: false,
            ended: false,
            frame: frame.clone(),
        };
        match build(reader, size, &options, frame) {
            Ok(pipeline) => {
                if pipeline.set_state(gst::State::Paused).is_err() {
                    // The bus says why.
                    player.pipeline = Some(pipeline);
                    player.poll();
                    if player.phase == Phase::Opening {
                        player.phase = Phase::Failed(UNSUPPORTED_FORMAT.to_owned());
                    }
                } else {
                    player.pipeline = Some(pipeline);
                }
            }
            Err(line) => player.phase = Phase::Failed(line),
        }
        player
    }

    /// Takes in what GStreamer has said since the last call. Call it every frame.
    pub fn poll(&mut self) {
        let Some(pipeline) = &self.pipeline else {
            return;
        };
        let Some(bus) = pipeline.bus() else {
            return;
        };
        while let Some(message) = bus.pop() {
            use gst::MessageView;
            match message.view() {
                MessageView::AsyncDone(_) if self.phase == Phase::Opening => {
                    self.phase = Phase::Ready;
                }
                MessageView::Eos(_) => {
                    self.ended = true;
                    self.playing = false;
                    let _ = pipeline.set_state(gst::State::Paused);
                }
                // The first error is the cause; the rest follow from it.
                MessageView::Error(error) if !matches!(self.phase, Phase::Failed(_)) => {
                    self.phase = Phase::Failed(failure_line(&error.error()));
                    self.playing = false;
                }
                _ => {}
            }
        }
        if matches!(self.phase, Phase::Failed(_)) {
            self.stop();
        }
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// The newest video frame not taken yet.
    pub fn take_frame(&self) -> Option<ColorImage> {
        self.frame.lock().ok()?.take()
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Whether playback reached the end.
    pub fn has_ended(&self) -> bool {
        self.ended
    }

    /// Whether the file has a video stream.
    pub fn has_video(&self) -> bool {
        self.pipeline
            .as_ref()
            .is_some_and(|p| p.property::<i32>("n-video") > 0)
    }

    /// How far playback is.
    pub fn position(&self) -> Option<Duration> {
        let at = self.pipeline.as_ref()?.query_position::<gst::ClockTime>()?;
        Some(Duration::from_nanos(at.nseconds()))
    }

    /// How long the file plays, when its container says.
    pub fn duration(&self) -> Option<Duration> {
        let total = self.pipeline.as_ref()?.query_duration::<gst::ClockTime>()?;
        Some(Duration::from_nanos(total.nseconds()))
    }

    /// Plays from where it is, or from the start once it has ended.
    pub fn play(&mut self) {
        if self.phase != Phase::Ready {
            return;
        }
        let Some(pipeline) = &self.pipeline else {
            return;
        };
        if self.ended {
            self.ended = false;
            let _ = pipeline.seek_simple(
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::ClockTime::ZERO,
            );
        }
        if pipeline.set_state(gst::State::Playing).is_ok() {
            self.playing = true;
        }
    }

    pub fn pause(&mut self) {
        if let Some(pipeline) = &self.pipeline
            && self.playing
        {
            let _ = pipeline.set_state(gst::State::Paused);
        }
        self.playing = false;
    }

    /// Play when paused, pause when playing.
    pub fn toggle(&mut self) {
        if self.playing {
            self.pause();
        } else {
            self.play();
        }
    }

    /// Moves playback to `at`, keeping it playing or paused.
    pub fn seek(&mut self, at: Duration) {
        if self.phase != Phase::Ready {
            return;
        }
        let Some(pipeline) = &self.pipeline else {
            return;
        };
        self.ended = false;
        let at = gst::ClockTime::from_nseconds(at.as_nanos() as u64);
        let _ = pipeline.seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE, at);
    }

    /// The volume, 0 (silent) to 1 (the file's own level).
    pub fn volume(&self) -> f64 {
        self.pipeline
            .as_ref()
            .map_or(1.0, |p| p.property::<f64>("volume"))
    }

    pub fn set_volume(&mut self, volume: f64) {
        if let Some(pipeline) = &self.pipeline {
            pipeline.set_property("volume", volume.clamp(0.0, 1.0));
        }
    }

    pub fn is_muted(&self) -> bool {
        self.pipeline
            .as_ref()
            .is_some_and(|p| p.property::<bool>("mute"))
    }

    pub fn set_muted(&mut self, muted: bool) {
        if let Some(pipeline) = &self.pipeline {
            pipeline.set_property("mute", muted);
        }
    }

    /// Stops GStreamer off this thread: a streaming thread may be waiting on a read.
    fn stop(&mut self) {
        if let Some(pipeline) = self.pipeline.take() {
            let _ = std::thread::Builder::new()
                .name("media-stop".into())
                .spawn(move || {
                    let _ = pipeline.set_state(gst::State::Null);
                });
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A failure as Preview's one line.
fn failure_line(error: &gst::glib::Error) -> String {
    if error.kind::<gst::ResourceError>().is_some() {
        return crate::preview::failure_line(error.message());
    }
    let unsupported = matches!(
        error.kind::<gst::StreamError>(),
        Some(
            gst::StreamError::TypeNotFound
                | gst::StreamError::CodecNotFound
                | gst::StreamError::WrongType
                | gst::StreamError::Format
                | gst::StreamError::Demux
                | gst::StreamError::Decode
        )
    ) || matches!(
        error.kind::<gst::CoreError>(),
        Some(gst::CoreError::MissingPlugin)
    );
    if unsupported {
        return UNSUPPORTED_FORMAT.to_owned();
    }
    crate::image_view::failure_line(error.message())
}

/// The file as `appsrc` reads it: the reader and where it is.
struct Source<R> {
    reader: R,
    at: u64,
    size: Option<u64>,
}

fn build<R>(
    reader: R,
    size: Option<u64>,
    options: &Options,
    frame: Arc<Mutex<Option<ColorImage>>>,
) -> Result<gst::Element, String>
where
    R: Read + Seek + Send + 'static,
{
    let missing = |what: &str| format!("Cannot preview: GStreamer has no {what}");
    gst::init().map_err(|e| format!("Cannot preview: GStreamer is not available ({e})"))?;
    let playbin = gst::ElementFactory::make("playbin")
        .property("uri", "appsrc://")
        .build()
        .map_err(|_| missing("playbin"))?;
    // Video and audio only: no subtitles or visualisations.
    playbin.set_property_from_str("flags", "video+audio+soft-volume");

    let source = Arc::new(Mutex::new(Source {
        reader,
        at: 0,
        size,
    }));
    playbin.connect("source-setup", false, move |values| {
        let element = values[1].get::<gst::Element>().ok()?;
        let appsrc = element.downcast::<gst_app::AppSrc>().ok()?;
        setup_source(&appsrc, source.clone());
        None
    });

    let video =
        gst::parse::bin_from_description("videoconvert ! videoscale ! appsink name=frames", true)
            .map_err(|_| missing("videoconvert or videoscale"))?;
    let sink = video
        .by_name("frames")
        .and_then(|e| e.downcast::<gst_app::AppSink>().ok())
        .ok_or_else(|| missing("appsink"))?;
    let max = options.max_side.clamp(1, i32::MAX as usize) as i32;
    let caps = gst::Caps::builder("video/x-raw")
        .field("format", "RGBA")
        .field("width", gst::IntRange::new(1, max))
        .field("height", gst::IntRange::new(1, max))
        .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
        .build();
    sink.set_caps(Some(&caps));
    sink.set_max_buffers(1);
    sink.set_drop(true);
    let keep = move |sample: gst::Sample, frame: &Mutex<Option<ColorImage>>| {
        if let Some(image) = to_image(&sample)
            && let Ok(mut slot) = frame.lock()
        {
            *slot = Some(image);
        }
    };
    let (preroll_frame, preroll_wake) = (frame.clone(), options.wake.clone());
    let (sample_frame, sample_wake) = (frame, options.wake.clone());
    let keep_preroll = keep;
    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_preroll(move |sink| {
                let sample = sink.pull_preroll().map_err(|_| gst::FlowError::Eos)?;
                keep_preroll(sample, &preroll_frame);
                preroll_wake();
                Ok(gst::FlowSuccess::Ok)
            })
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                keep(sample, &sample_frame);
                sample_wake();
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );
    playbin.set_property("video-sink", &video);

    let audio = match options.audio {
        AudioOut::Device => gst::ElementFactory::make("autoaudiosink").build(),
        AudioOut::Silent => gst::ElementFactory::make("fakesink")
            .property("sync", true)
            .build(),
    }
    .map_err(|_| missing("audio sink"))?;
    playbin.set_property("audio-sink", &audio);

    if let Some(bus) = playbin.bus() {
        let wake = options.wake.clone();
        bus.set_sync_handler(move |_, _| {
            wake();
            gst::BusSyncReply::Pass
        });
    }
    Ok(playbin)
}

/// Feeds `appsrc` from `source`, in whatever ranges it asks for.
fn setup_source<R>(appsrc: &gst_app::AppSrc, source: Arc<Mutex<Source<R>>>)
where
    R: Read + Seek + Send + 'static,
{
    let size = source.lock().ok().and_then(|s| s.size);
    appsrc.set_stream_type(gst_app::AppStreamType::RandomAccess);
    appsrc.set_format(gst::Format::Bytes);
    // -1: unknown, read until the reader ends.
    appsrc.set_size(size.map_or(-1, |s| s.min(i64::MAX as u64) as i64));
    let seeking = source.clone();
    appsrc.set_callbacks(
        gst_app::AppSrcCallbacks::builder()
            .need_data(move |appsrc, length| {
                let Ok(mut source) = source.lock() else {
                    let _ = appsrc.end_of_stream();
                    return;
                };
                let left = source
                    .size
                    .map_or(u64::MAX, |s| s.saturating_sub(source.at));
                let want = (length as u64).min(left) as usize;
                if want == 0 {
                    let _ = appsrc.end_of_stream();
                    return;
                }
                let mut data = vec![0; want];
                let mut got = 0;
                while got < want {
                    match source.reader.read(&mut data[got..]) {
                        Ok(0) => break,
                        Ok(n) => got += n,
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(e) => {
                            appsrc.post_error_message(gst::error_msg!(
                                gst::ResourceError::Read,
                                ("{e}")
                            ));
                            let _ = appsrc.end_of_stream();
                            return;
                        }
                    }
                }
                if got == 0 {
                    let _ = appsrc.end_of_stream();
                    return;
                }
                data.truncate(got);
                let mut buffer = gst::Buffer::from_mut_slice(data);
                if let Some(buffer) = buffer.get_mut() {
                    buffer.set_offset(source.at);
                }
                source.at += got as u64;
                let _ = appsrc.push_buffer(buffer);
            })
            .seek_data(move |_, offset| {
                let Ok(mut source) = seeking.lock() else {
                    return false;
                };
                match source.reader.seek(SeekFrom::Start(offset)) {
                    Ok(_) => {
                        source.at = offset;
                        true
                    }
                    Err(_) => false,
                }
            })
            .build(),
    );
}

/// An RGBA sample as an image, row by row past any padding.
fn to_image(sample: &gst::Sample) -> Option<ColorImage> {
    let info = gst_video::VideoInfo::from_caps(sample.caps()?).ok()?;
    let buffer = sample.buffer()?;
    let map = buffer.map_readable().ok()?;
    let (width, height) = (info.width() as usize, info.height() as usize);
    let stride = *info.stride().first()? as usize;
    let row = width * 4;
    let mut pixels = Vec::with_capacity(row * height);
    for y in 0..height {
        pixels.extend_from_slice(map.get(y * stride..y * stride + row)?);
    }
    Some(ColorImage::from_rgba_unmultiplied([width, height], &pixels))
}

/// How much a [`RangedReader`] asks the host for at a time.
pub const CHUNK: usize = 256 * 1024;

/// A remote file as a blocking, seekable reader, for a [`Player`] on GStreamer's
/// threads. Each read that leaves the last range fetched asks the host for the next
/// [`CHUNK`] from there, so seeking costs one round trip and nothing is read whole.
/// The file opens on the first read and closes when the reader is dropped. Errors are
/// the reason as the operator reads it ("No such file", "Permission denied").
/// A read the connection's drop stopped.
const LOST: &str = "connection lost";

pub struct RangedReader {
    asks: tokio::sync::mpsc::UnboundedSender<(Ask, std::sync::mpsc::Sender<Answer>)>,
    at: u64,
    /// The last range fetched, and where it starts.
    range: (u64, Vec<u8>),
}

enum Ask {
    Read { offset: u64, len: usize },
    Size,
}

enum Answer {
    Data(Result<Vec<u8>, String>),
    Size(Result<u64, String>),
}

impl RangedReader {
    pub fn new(session: &crate::ssh::Session, path: String) -> RangedReader {
        let (asks, mut asked) =
            tokio::sync::mpsc::unbounded_channel::<(Ask, std::sync::mpsc::Sender<Answer>)>();
        let remote = session.remote();
        session.spawn(async move {
            use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};
            let mut file = None;
            while let Some((ask, answer)) = asked.recv().await {
                if file.is_none() {
                    match remote.open(path.clone()).await {
                        Ok(opened) => file = Some(opened),
                        Err(e) => {
                            let e = if remote.closed().await {
                                LOST.to_owned()
                            } else {
                                e
                            };
                            let _ = answer.send(match ask {
                                Ask::Read { .. } => Answer::Data(Err(e)),
                                Ask::Size => Answer::Size(Err(e)),
                            });
                            continue;
                        }
                    }
                }
                let Some(file) = file.as_mut() else {
                    continue;
                };
                let reply = match ask {
                    Ask::Read { offset, len } => {
                        let read = async {
                            file.seek(SeekFrom::Start(offset))
                                .await
                                .map_err(|e| e.to_string())?;
                            let mut data = Vec::with_capacity(len);
                            (&mut *file)
                                .take(len as u64)
                                .read_to_end(&mut data)
                                .await
                                .map_err(|e| e.to_string())?;
                            Ok(data)
                        };
                        Answer::Data(read.await)
                    }
                    Ask::Size => Answer::Size(
                        file.metadata()
                            .await
                            .map(|m| m.len())
                            .map_err(|e| e.to_string()),
                    ),
                };
                // A read the drop cut short reads as the connection lost, not as
                // whatever russh says about its channel.
                let reply = match reply {
                    Answer::Data(Err(_)) if remote.closed().await => {
                        Answer::Data(Err(LOST.to_owned()))
                    }
                    Answer::Size(Err(_)) if remote.closed().await => {
                        Answer::Size(Err(LOST.to_owned()))
                    }
                    reply => reply,
                };
                let _ = answer.send(reply);
            }
        });
        RangedReader {
            asks,
            at: 0,
            range: (0, Vec::new()),
        }
    }

    fn ask(&self, ask: Ask) -> std::io::Result<Answer> {
        let (answer, answered) = std::sync::mpsc::channel();
        let gone = || std::io::Error::other(LOST);
        self.asks.send((ask, answer)).map_err(|_| gone())?;
        answered.recv().map_err(|_| gone())
    }
}

impl Read for RangedReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let (start, data) = &self.range;
        let inside = self.at >= *start && self.at < start + data.len() as u64;
        if !inside {
            let Answer::Data(data) = self.ask(Ask::Read {
                offset: self.at,
                len: CHUNK,
            })?
            else {
                unreachable!("a read is answered with data");
            };
            let data = data.map_err(std::io::Error::other)?;
            if data.is_empty() {
                return Ok(0);
            }
            self.range = (self.at, data);
        }
        let (start, data) = &self.range;
        let from = (self.at - start) as usize;
        let n = buf.len().min(data.len() - from);
        buf[..n].copy_from_slice(&data[from..from + n]);
        self.at += n as u64;
        Ok(n)
    }
}

impl Seek for RangedReader {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let at = match to {
            SeekFrom::Start(at) => Some(at),
            SeekFrom::Current(by) => self.at.checked_add_signed(by),
            SeekFrom::End(by) => {
                let Answer::Size(size) = self.ask(Ask::Size)? else {
                    unreachable!("a size is answered with a size");
                };
                size.map_err(std::io::Error::other)?.checked_add_signed(by)
            }
        };
        self.at = at.ok_or_else(|| std::io::Error::other("seek before the start"))?;
        Ok(self.at)
    }
}
