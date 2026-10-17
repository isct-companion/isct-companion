# School OTP QR import — 17 October 2026

Desktop keeps the masked manual setup field. A right-hand button group offers
Scan QR and Select image. The question button has a hover hint and opens a
keyboard-accessible help panel for Google, Microsoft/other and first-time setup.
The help distinguishes the setup secret from a changing six-digit code and
allows manual verification at login. It never tells existing users to remove
App Authentication; the real school page showed Already Set and only Remove.

ZXing handles local QR image/camera decoding. Core owns standard otpauth validation
and Google transfer protobuf parsing through prost. Only TOTP, supported hashes
and 6/8-digit Google records are accepted. Malformed, oversized, empty, unsupported
or incomplete multi-page exports fail without a partial import; multi-account
exports ask the user which school account to use. Camera/image recognition only
fills an unsaved draft; the existing Connect action saves credentials to the vault.
No QR data is sent to a website, logged or stored in the workspace.

The reusable CameraPreview surface now serves MatrixCamera and QrCamera. Native
Matrix frame processing and consensus remain untouched; QR decoding uses ZXing's
camera lifecycle and stops on close/background and late camera acquisition.
Production reduction is 12 duplicate Matrix preview CSS lines removed; new QR
logic, tests and generated types are additions, not claimed code reduction.

Validation:

- Frontend full regression: 526 passed, 4 skipped; TypeScript checks and production
  frontend build passed.

- Core with ts: 371 passed, including RFC TOTP equivalence after both import paths,
  multi-account preservation and rejection of unsupported/incomplete payloads.
- Full workspace cargo check passed, including the Tauri command and FFI projection.
- Existing settings coverage now tests import selection, no automatic save,
  Connect forwarding and late camera cleanup. Matrix-disabled builds still offer
  independent QR image import; the Matrix capability assertion remains scoped
  to the Matrix card and retains its original guarantee.
- Temporary local browser preview used the actual input, help and app theme with
  blank example fields; manual field and right-side controls aligned, all help
  choices rendered in Chinese. The temporary page/server/tab were removed.
- Real camera optics and native permission prompts remain a manual check:
  SCHOOL-OTP-QR in manual-checks.yaml. No real credential QR was scanned and no
  account settings were changed. No signed package was built for this change.

The workspace check also caught the previous official-filter addition missing
OfficialSelection in the FFI schema generator. That type is now projected as a
real record rather than an unresolved Core crate path.
