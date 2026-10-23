# IsctCompanion

A local-first companion for Institute of Science Tokyo schedules, courses and
school services. Source and desktop releases are maintained here by
[@Gliese129](https://github.com/Gliese129) and [@OHNope](https://github.com/OHNope).

## Downloads

Get desktop releases from [Releases](https://github.com/isct-companion/isct-companion/releases).

| System | Download filename contains | Requirements |
| --- | --- | --- |
| Mac with Apple Silicon | `aarch64-apple-darwin.dmg` | macOS 14 or later |
| Mac with Intel processor | `x86_64-apple-darwin.dmg` | macOS 14 or later |
| Windows PC | `x86_64-pc-windows-msvc.exe` | Windows 10/11, x64; WebView2 |

On Mac, open the disk image and drag the app to Applications. On Windows, run
the installer; it installs WebView2 if needed (an internet connection may be
required). Intel Mac builds currently omit campus-card OCR.
Mobile 1.0 for iOS and Android will be published separately when ready.
This release includes desktop installers only.

Each Release states its signing and notarization status; unsigned installers
may display security prompts.
Compatibility listed above is the build target; runtime testing on each system
is ongoing.

## License and data

The app is free software under the GNU GPL v3 or later ([LICENSE](LICENSE)),
with the additional terms in [NOTICE.md](NOTICE.md) ([日本語](NOTICE.ja.md)).
Third-party software keeps its own licenses; notices are bundled in the app.

Workspace data stays on your device. Optional backup uses your own Google
Drive or OneDrive account. School access uses your own school account. Verify
deadlines and academic records in the official university systems.

IsctCompanion is an independent project and is not an official university app.

## Support

Report problems in [Issues](https://github.com/isct-companion/isct-companion/issues).
Include your app version, operating system and steps to reproduce. Keep
passwords, authentication tokens and private school records out of reports.

This repository contains distribution documents and compiled releases. The
application source and build infrastructure are maintained privately.
