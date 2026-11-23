# Building and running sshexplorer

sshexplorer is a Rust crate (`cargo build --release`). Most of it is pure Rust. The system libraries it needs are listed below for each OS.

## Audio and video Preview: GStreamer

Audio and video Preview uses GStreamer 1.14 or later (ADR-0005). The build links against it through pkg-config, so building needs its development files and running needs its runtime.

Elements sshexplorer uses:

| Plugin set | Elements | Needed for |
|---|---|---|
| core | `fakesink`, typefinding | always |
| base | `playbin`, `appsrc`, `appsink`, `videoconvert`, `videoscale`, Ogg, Vorbis, Theora, Opus | always |
| good | `autoaudiosink`, Matroska/WebM, VP8/VP9, MP4/QuickTime demux, FLAC, WAV, MP3 | most files |
| libav (or bad, or a platform decoder) | H.264, H.265, AAC and other FFmpeg decoders | most MP4/MOV video and M4A audio |

A file whose demuxer or decoder is missing shows "Cannot preview: unsupported format". The rest of the app works without any plugins. If the GStreamer libraries themselves are missing, the app does not start, because they are linked dynamically.

### Linux

| Distribution | Build | Runtime |
|---|---|---|
| Debian, Ubuntu | `pkg-config libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev` | `gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-libav` |
| Fedora | `pkgconf gstreamer1-devel gstreamer1-plugins-base-devel` | `gstreamer1-plugins-base gstreamer1-plugins-good gstreamer1-libav` (RPM Fusion) or `gstreamer1-plugin-openh264` |
| Arch | `pkgconf gstreamer gst-plugins-base` | `gst-plugins-base gst-plugins-good gst-libav` |

Sound goes through `autoaudiosink`, which picks PipeWire, PulseAudio or ALSA.

### macOS

- Install the official **runtime and development** packages from gstreamer.freedesktop.org (`gstreamer-1.0-<version>-universal.pkg` and `gstreamer-1.0-devel-<version>-universal.pkg`). Then build with
  `PKG_CONFIG_PATH=/Library/Frameworks/GStreamer.framework/Versions/1.0/lib/pkgconfig` and `pkg-config` installed (Homebrew `pkgconf`).
- Or use Homebrew: `brew install gstreamer pkgconf`. The formula includes the plugin sets.
- A bundled `.app` must ship the GStreamer framework, or require the runtime package.

### Windows

- Install the official **MSVC 64-bit runtime and development** installers from gstreamer.freedesktop.org (choose the Complete install so libav is included).
- Install pkg-config, for example `winget install bloodrush.pkg-config-lite` or `choco install pkgconfiglite`.
- Build with `PKG_CONFIG_PATH=C:\gstreamer\1.0\msvc_x86_64\lib\pkgconfig`. At run time, `C:\gstreamer\1.0\msvc_x86_64\bin` must be on `PATH`, or its DLLs must ship next to `sshexplorer.exe`.

### Licence

GStreamer is LGPL-2.1+. sshexplorer links it dynamically. A bundle must ship it unmodified and replaceable, with its licence notices. gst-libav brings FFmpeg, which is LGPL unless it was built with GPL parts. The gstreamer-rs bindings are MIT or Apache-2.0.

## Other system libraries

- **Linux:** a Secret Service provider (GNOME Keyring or KWallet) for Remember, and the usual X11/Wayland libraries eframe opens at run time.
- **macOS and Windows:** nothing beyond GStreamer.
