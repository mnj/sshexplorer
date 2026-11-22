# GStreamer plays audio and video

The spec leaves the media stack to the implementer: ffmpeg-next, gstreamer-rs or libmpv2, fed by a seekable ranged-SFTP reader, weighing install burden and licence (issue #16). sshexplorer uses gstreamer-rs 0.25 (ticket #38). A `playbin` reads the remote file through an `appsrc` in random-access mode, whose callbacks read a `RangedReader` that asks the host for 256 KiB ranges over the Session's SFTP connection. Video comes out of an `appsink` as RGBA frames, scaled down to the GPU's texture limit, and is drawn as an egui texture. GStreamer keeps time, syncs audio to video, plays the sound and applies volume and mute.

## Considered Options

- **ffmpeg-next:** also takes a `Read + Seek` reader and gives RGBA frames, but the app would have to own the clock, audio/video sync, resampling and a cpal audio output, which is a lot more code. It needs libclang to build, and FFmpeg DLLs to ship on Windows.
- **libmpv2:** mpv draws only into an OpenGL FBO. That works with eframe's glow renderer, which sshexplorer uses, but libmpv is GPL unless it is built with `-Dgpl=false`, and it pulls in FFmpeg as well.
- **Symphonia with rodio:** pure Rust, but it plays audio only, and no maintained pure-Rust stack decodes common video.

## Consequences

- Building needs the GStreamer 1.14+ development files and pkg-config. Running needs the GStreamer runtime with its base and good plugins. H.264, H.265 and AAC also need gst-libav (or bad, or a platform decoder plugin). A format whose plugin is missing shows "Cannot preview: unsupported format". `docs/building.md` lists the packages for each OS.
- GStreamer is LGPL and is linked dynamically. A bundle must ship it unmodified and replaceable, with its notices.
- Sound goes to `autoaudiosink` in the app. In tests it goes to a synced `fakesink` (`App::audio_out`, `media::AudioOut::Silent`), so no sound device is needed.
- Streamed media is never kept in the Scratch folder. The reader holds only the last range it fetched, so memory stays bounded however large the file is (spec-gaps 21).
