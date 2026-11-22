# Which Rust media stack can stream audio and video into an egui panel?

Research for [Which Rust media stack can stream audio and video into an egui panel?](https://github.com/mnj/sshexplorer/issues/16). Researched 2026-11-22. Versions and dates come from the crates.io API (`max_stable_version`, `updated_at`) and from shallow clones of the upstream repositories on that date.

The Preview needs four things from a stack:

- decode common audio and video;
- put video frames into an egui texture;
- read the bytes from a seekable custom reader, here ranged SFTP reads buffered in the Scratch folder, instead of a path or URL;
- support Play/Pause, seek, volume and mute.

## The common egui end

Every candidate except libmpv ends with RGBA bytes in memory. egui takes those bytes through `ColorImage::from_rgba_unmultiplied(size, &rgba)` and `TextureHandle::set(image, options)`. The texture handle is updated in place each frame (`egui/crates/epaint/src/image.rs:120`, `egui/crates/epaint/src/texture_handle.rs:70`, egui 0.37.0, 2026-10-15). Each frame is copied from the CPU to the GPU.

A zero-copy path exists only for a GPU texture the app already owns: `egui_wgpu::Renderer::register_native_texture` and `egui_glow::Painter::register_native_texture` (`egui-wgpu/src/renderer.rs:870`, `egui_glow/src/painter.rs:655`).

## Candidates

### 1. ffmpeg-next (FFmpeg bindings)

- **Crate:** `ffmpeg-next` 9.1.0 and `ffmpeg-sys-next` 9.1.1, both released 2026-11-11. The last commit was 2026-11-17 (<https://github.com/zmwangx/rust-ffmpeg>). The README calls the crate "in maintenance mode". It tracks FFmpeg from 3.4 up to 9.1, judging by the `ffmpeg_9_1` cfg flags in the `rust-ffmpeg-sys` README. The crate is WTFPL.
- **Custom reader:** yes, and safe. `format::context::StreamIo::from_read` / `from_read_seek` wraps a Rust `Read + Seek + Send + 'static` in an FFmpeg `AVIOContext`. It is opened with `format::input_from_stream` (or `input_from_stream_with_interrupt` for cancellation). The I/O is buffered at 32 KiB by default. Per the doc comment in `src/format/context/stream_io.rs`, the stream must be blocking: "the first `WouldBlock`/`TimedOut` error poisons the context". Seeking uses `Input::seek(ts, range)` (`src/format/context/input.rs:166`), which calls the reader's `Seek`.
- **Frames to egui:** decode with `decoder::Video` using `send_packet` / `receive_frame`. Convert to RGBA with `software::scaling` (swscale), then build a `ColorImage` as described above. There is no egui widget in the crate.
- **Audio:** decode with `decoder::Audio` and convert with `software::resampling` (swresample). Then play the PCM through cpal, or through rodio, whose `Source` trait sits on cpal. Volume and mute are the app's job.
- **Install:**
  - **Build:** `ffmpeg-sys-next` runs bindgen (0.73), so the build machine needs libclang. It finds FFmpeg through `FFMPEG_DIR`, then vcpkg (MSVC), then pkg-config (`build.rs` lines 1164-1236).
  - **Linux:** distro `libav*-dev` packages.
  - **macOS:** Homebrew `ffmpeg`, or a source build.
  - **Windows:** vcpkg `ffmpeg`, or a prebuilt tree pointed to by `FFMPEG_DIR`.
  - **Source build:** the `build` feature compiles FFmpeg from source and links it statically. The `build-license-gpl`, `build-license-version3` and `build-license-nonfree` features opt into the stricter licences (`Cargo.toml` `[features]`).
  - **Runtime:** the operator needs the FFmpeg shared libraries, either bundled next to the binary or provided by the system.
- **Licence:** FFmpeg is LGPL-2.1+. Optional parts are GPL, and "if those parts get used the GPL applies to all of FFmpeg". The LGPL checklist says to build without `--enable-gpl` and `--enable-nonfree` and to link dynamically, which means DLLs on Windows (<https://ffmpeg.org/legal.html>). A static `build` therefore creates LGPL relinking obligations. The app can read which licence the linked build has at runtime through `ffmpeg::format::license()`.
- **Formats by default:** FFmpeg's own demuxers and decoders cover the common containers (MP4/MOV, Matroska/WebM, AVI, Ogg, MPEG-TS, WAV, FLAC) and codecs (H.264, HEVC, VP8/VP9, MPEG-2/4, AAC, MP3, Opus, Vorbis, FLAC, PCM). The set actually present depends on how the installed FFmpeg was configured. Software AV1 normally comes from the external libdav1d (`build-lib-dav1d` feature).
- **Related crates on the same libraries:**
  - **ffmpeg-the-third** 6.0.0+ffmpeg-9.0 (2026-08-09): a fork. No `avio_alloc_context` wrapper was found in its source, so custom I/O would need unsafe sys calls.
  - **rsmpeg** 0.18.0+ffmpeg.8.0 (2025-08-24, MIT): a thin binding. `AVIOContextCustom::alloc_context` takes read, write and seek closures (`src/avformat/avio.rs:116`). FFmpeg is linked through rusty_ffmpeg's `link_system_ffmpeg` / `link_vcpkg_ffmpeg`.
  - **video-rs** 0.13.0 (2026-11-20): a high-level wrapper on `ffmpeg-next =9.1.0`. Its input is `Location::File(PathBuf)` or `Location::Network(Url)` only (`src/location.rs`), so it does not take a reader.
  - **egui-video** 0.9.0 (2024-10-28, last commit 2024-10-27): an egui player widget on ffmpeg-the-third 2 with SDL2 audio. It is pinned to egui 0.29 and opens inputs by path (`input(&self.input_path)`, `src/lib.rs:1078`). It is unmaintained for current egui and does not take a reader, but it shows the texture and audio pattern.

### 2. gstreamer-rs (GStreamer bindings)

- **Crate:** `gstreamer` 0.25.4 and `gstreamer-video` 0.25.4 (2026-09-21), and `gstreamer-app` 0.25.2 (2026-05-11). The main branch is at 0.26.0-alpha with MSRV 1.92. The project is official and lives at <https://gitlab.freedesktop.org/gstreamer/gstreamer-rs>. The bindings are MIT/Apache-2.0.
- **Custom reader:** yes, through `appsrc`. Set `stream-type` to `AppStreamType::Seekable` or `RandomAccess` (`gstreamer-app/src/auto/enums.rs`). Answer `need-data` by pushing buffers, and answer the `seek_data(Fn(&AppSrc, u64) -> bool)` callback (`gstreamer-app/src/app_src.rs:113`) by repositioning the reader. Set `size` to the total byte length. Per the upstream docs, `seek-data` is "only called for seekable stream types", and "the next push-buffer should produce buffers from the new offset" (<https://gstreamer.freedesktop.org/documentation/app/appsrc.html>). The alternative is a custom source element written in Rust. A demuxer such as `qtdemux` (MP4) then seeks by pulling byte ranges.
- **Frames to egui:** build `appsrc ! decodebin ! videoconvert ! appsink caps=video/x-raw,format=RGBA`, or use `playbin` with an `appsink` as `video-sink`. Map each `gst::Sample` buffer and copy it into a `ColorImage`. There is no egui sink element.
- **Audio:** GStreamer plays the audio itself through `autoaudiosink`, which picks PulseAudio/ALSA, WASAPI or CoreAudio. `playbin` has `volume` and `mute` properties, and seeking is `seek_simple` on the pipeline, so Play/Pause, seek, volume and mute are all pipeline calls. No cpal is needed.
- **Install** (from the gstreamer-rs README, "Installation"):
  - **All platforms:** GStreamer and gst-plugins-base 1.14 or later must be installed when building and running.
  - **Linux:** distro packages, `libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev` plus the plugin packages `gstreamer1.0-plugins-{base,good,bad,ugly} gstreamer1.0-libav`. On Fedora, the RPMFusion "freeworld" and "ugly" packages carry the patent-encumbered codecs.
  - **macOS:** the two official `.pkg` files (runtime and devel). Homebrew is noted as "currently broken". Put GStreamer's `pkg-config` first on `PATH`.
  - **Windows:** the two official `.msi` files that match the toolchain (MSVC or MinGW), with `bin` on `PATH`. The README prefers these over MSYS2.
  - **Runtime:** the operator needs the GStreamer runtime and plugins, either installed or bundled with the app.
- **Licence:** GStreamer is LGPL. Core, base and good may not contain "plugins with known patent issues", which go to the ugly module. GPL code is not accepted in the plugin modules (<https://gstreamer.freedesktop.org/documentation/frequently-asked-questions/licensing.html>). gst-libav wraps FFmpeg, so FFmpeg's terms also apply.
- **Formats by default:** what decodes depends on which plugin modules are installed. The module locations below come from the GStreamer monorepo tree, `subprojects/*/{gst,ext}`, on 2026-11-22:
  - **base:** `app`, `playback` (decodebin/playbin), `videoconvertscale`, `ogg`, `vorbis`, `opus`, `theora`.
  - **good:** `isomp4` (MP4/MOV), `matroska` (MKV/WebM), `avi`, `flac`, `vpx` (VP8/VP9), `mpg123` (MP3), `wavparse`.
  - **bad:** `openh264`, `faad` (AAC), `libde265` (HEVC), `aom`.
  - **gst-libav:** H.264, HEVC, AAC and most other formats, through FFmpeg.
  - **ugly:** mostly encoders, such as `x264`.
  - **dav1d** (AV1) is not in bad/ext. It ships with the Rust plugins (gst-plugins-rs).
  - **Platform decoders:** hardware decoders vary by platform, for example VA-API, NVCODEC and VideoToolbox.

  base and good alone play Ogg/Vorbis/Opus/Theora, WebM/VP8/VP9, FLAC, MP3 and WAV. H.264 and AAC in MP4 need gst-libav, bad or a platform decoder.

### 3. libmpv2 (mpv as a library)

- **Crate:** `libmpv2` 7.0.1 (2026-11-07, last commit 2026-11-07, <https://github.com/kohsine/libmpv-rs>). It needs libmpv 2.0 (mpv 0.35) or later. The crate is LGPL-2.1. The older `libmpv` crate (2.0.1, 2020-09-29) is unmaintained.
- **Custom reader:** yes. `Protocol::new(name, user_data, open, close, read, seek, size)` followed by `register()` (`src/mpv/protocol.rs`) registers a URL scheme through libmpv's `mpv_stream_cb_add_ro`. `StreamSeek` is `fn(&mut T, i64) -> i64` and `StreamSize` returns the total size (`libmpv-sys/include/stream_cb.h:126-187`). mpv's own demuxer cache then reads ahead through these callbacks.
- **Frames to egui:** libmpv draws video itself through its render API. The crate exposes only `RenderParamApiType::OpenGl` (`src/mpv/render.rs:83`): mpv renders into an FBO that the app supplies. In egui this means the `glow` backend, an offscreen texture passed to `egui_glow::Painter::register_native_texture`, or a `PaintCallback`. libmpv's C header also has a slow `MPV_RENDER_API_TYPE_SW` software renderer that would give CPU RGBA bytes (`libmpv-sys/include/render.h:128`), but the crate has no wrapper for it. There is no wgpu path.
- **Audio:** mpv plays the audio itself. Pause, seek, volume and mute are mpv properties and commands.
- **Install:** the libmpv shared library must be present at build and run time. On Linux this is the distro `libmpv` / `mpv-libs` package, on macOS Homebrew `mpv`, and on Windows a prebuilt `libmpv-2.dll`. The `build_libmpv` feature links against a tree set by `MPV_SOURCE` (crate README). libmpv in turn depends on FFmpeg.
- **Licence:** "mpv as a whole is licensed under the GNU General Public License GPL version 2 or later". The `-Dgpl=false` switch excludes GPL-only files for LGPL use, which is "intended ... with libmpv", but the switch "does not in itself create a LGPLv2.1+ license grant" (<https://github.com/mpv-player/mpv/blob/master/Copyright>). Distro libmpv packages are normally GPL builds.
- **Formats by default:** everything the linked FFmpeg decodes, which is the same as candidate 1.

### 4. Pure-Rust pieces (no system media framework)

- **Audio, Symphonia** 0.6.1 (2026-08-13, MPL-2.0, <https://github.com/pdeljanov/Symphonia>):
  - **Custom reader:** yes. `MediaSource: Read + Seek + Send + Sync` with `is_seekable()` and `byte_len()` (`symphonia-core/src/io/mod.rs:42`).
  - **Default formats:** the default features are "only royalty-free open standard codecs and formats": FLAC, MKV, Ogg, PCM, Vorbis, WAV and ADPCM. AAC, ALAC, MP1/2/3, MP4 (isomp4), AIFF and CAF are opt-in through features or `all`.
  - **Video:** only experimental `exp-video-codecs` flags, and no video decoder.
- **Audio output, rodio** 0.23.0 (2026-10-11) **and cpal** 0.18.2 (2026-08-16), both MIT/Apache-2.0:
  - `rodio::Decoder` is built over any `Read + Seek + Send + Sync`, with `DecoderBuilder::with_data`, `with_byte_len` and `with_seekable` (`src/decoder/builder.rs`). Its default features include flac, mp3, mp4, vorbis and wav.
  - Playback goes through cpal. On Linux the default `pulseaudio` feature is on. cpal also has ALSA, WASAPI and CoreAudio backends.
  - rodio's `Player` handles pause, volume and seeking (`Player::try_seek`, `src/player.rs:246`).
  - This is the audio path for audio-only files, and the output path under candidate 1.
- **Video, codec-only crates:** there is no maintained pure-Rust demuxer-plus-decoder that covers the common formats. Codec crates exist, but each needs a separate demuxer (for example `mp4` or `matroska-demuxer`), sync and colour conversion:
  - `openh264` 0.9.10 (2026-11-10, BSD-2): H.264 only. By default it compiles the bundled Cisco source with `cc`. The optional `libloading` feature uses Cisco's prebuilt library instead (README; the patent reasoning is in openh264-rs issue 43).
  - `dav1d` 0.12.0 (2026-11-17): AV1 only, through libdav1d.

## Summary table

| Stack | Custom seekable reader | Frames to egui | Audio out | Needs on the operator's machine | Licence exposure | Common formats out of the box |
|---|---|---|---|---|---|---|
| ffmpeg-next 9.1 | Yes, safe `StreamIo` over `Read + Seek` | swscale to RGBA, then `TextureHandle::set` | App's own (cpal/rodio) | FFmpeg shared libs, or a static build | LGPL-2.1+ (GPL if the build uses GPL parts); static linking means relinking duties | Broad, depends on the FFmpeg build |
| gstreamer-rs 0.25 | Yes, `appsrc` Seekable/RandomAccess with `seek_data` | `appsink` RGBA, then `TextureHandle::set` | GStreamer (`autoaudiosink`, playbin volume/mute) | GStreamer runtime plus plugin sets (official pkg/msi on macOS/Windows) | LGPL; patent-encumbered codecs sit in ugly/bad/libav | base+good: open formats; H.264/AAC need libav, bad or a platform decoder |
| libmpv2 7.0 | Yes, `Protocol` (stream_cb) with seek and size | OpenGL FBO only (glow backend, native texture) | mpv | libmpv (and its FFmpeg) | mpv GPL-2+ unless built `-Dgpl=false` | Broad (FFmpeg) |
| Symphonia + rodio/cpal | Yes, `MediaSource` / `DecoderBuilder` over `Read + Seek` | No video | cpal | None beyond the OS audio stack | MPL-2.0 / MIT-Apache | Audio only; royalty-free by default, MP3/AAC/MP4 opt-in |

## Conclusion

Three maintained stacks meet every requirement in the ticket for both audio and video: ffmpeg-next, gstreamer-rs and libmpv2. Each one can take a seekable reader over SFTP ranged reads and seek through it.

They differ in four ways:

- **Who owns timing and audio.** With ffmpeg-next the app does, through cpal or rodio. GStreamer and mpv do it themselves.
- **How frames reach egui.** ffmpeg-next and GStreamer give RGBA copies, which work with any egui backend. libmpv renders into OpenGL, which ties the app to the glow backend.
- **What the operator installs.** FFmpeg can be bundled or linked statically. GStreamer needs a runtime and the right plugin sets. libmpv needs its library.
- **Licence.** FFmpeg and GStreamer are LGPL. mpv is GPL by default.

Symphonia with rodio/cpal covers audio alone in pure Rust, with no system libraries. No maintained pure-Rust stack decodes the common video formats.
