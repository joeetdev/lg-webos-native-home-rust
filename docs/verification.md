# Verification and limits

## Reference device

LG 50UQ8050PSB, webOS 7.6.0 / webOS 22, firmware 04.60.90, aarch64 and Homebrew Channel 0.7.3. The installation was tested during October 2026. Network data, accounts, keys, live journals and dumps are kept out of the repository.

## Observed on the TV (earlier Python implementation)

Everything in this section was observed with the earlier Python implementation of the device helpers. The QML/JS assets that run inside the compositor are unchanged and byte-identical in the Rust packaging.

- Eight read-only overrides with valid hashes/ownership for the same Home, without another app or launcher.
- Compact eleven apps + All apps at 12, expanded with 14 permitted LaunchPoints, editor/Back/ThinQ and tools at the end.
- Free-standing blue-grey clock, seconds raised, half the previous main size. Typographic bounds stayed stable while the seconds changed.
- Final gate tested across three software reboots without opening Homebrew, including two forced early launches of an old Home. The window without the marker was held back; only the verified customised one was released. This is a historical gate test, not an electrical cold boot of the latest remote revision.
- A physical capture confirmed GoogleVoice 18874398 and USP 18874413 with press/release and deviceId 3. The user checked the toggle, basic controls, MOVIES→Crunchyroll, and that the block was kept when opening the app.
- The first test with an eight-second automatic restore was temporary and was replaced by a toggle without expiry.
- The first powerState policy reported "blocked", but the cursor physically reappeared on power-on. After adding observation of native visibility, the user repeated power off/on with the remote, leaving ON beforehand: it came back OFF and the toggle worked.
- Permanent installation of the remote mapping, disabling, reapplication and idempotence verified. Model, handler and final QObject loaded; `desiredEnabled=false`, expiry 0 and the restore Timer inactive. Core/QML identical to the physical test; final loader without traces.
- Final reference chain: 17 LG handlers + Home gate + remapping. Versions of 134 apps, persistent preloads, the Home hook and rollback were kept during the remote phase. Diagnostics/temporary files were removed after saving private evidence.

## Rust port: verified offline only

The Rust implementation has **not been executed on the TV**. What was verified on the development host:

- All behaviour previously covered by the Python unit tests is covered by Rust tests (119 tests: unit, integration and command-line), plus root-only tests against real files for symlinks, hard links, permission bits, duplicate JSON keys and oversize input.
- Transaction parity: the original Python controller and the Rust controller were run against the same fake device boundary; the journals and the full sequences of side effects were identical for apply, rollback, an injected failure at bind with full recovery, and migration of a legacy journal.
- The backing-copy names (`sha256` of the compact JSON identity) are identical to the Python ones, so existing copies are reused.
- The gate and remote packers produce byte-identical assets and `package.json` files to the Python packers, for both trial and final modes.
- The Home delta engine matched the Python engine on 3,000 randomised differential cases, including identical error messages.
- The boot identity string, journal, boot marker, gate state and remote state formats are unchanged, so the Rust binary reads state written by the Python version.
- Builds and tests pass with Rust 1.74 and 1.97; static release builds for `aarch64-unknown-linux-musl` (~0.6 MB) and `armv7-unknown-linux-musleabihf`.
- Peak resident memory per process, measured on an x86_64 host with a ptrace probe that reads each process's own `VmHWM` at exit: 0.5–0.7 MiB for `lg-native` against 10–16 MiB for the equivalent Python commands (Python 3.13). TV measurements may differ.

Deliberate behavioural differences from the Python helpers, all in the fail-closed direction:

- JSON with duplicate keys is rejected everywhere (previously only in some readers); non-finite numbers are rejected.
- Backing copies are written to a temporary file and linked into place only after their hash matches.
- The read-only remount preserves the existing per-mount flags explicitly instead of depending on the behaviour of the TV's `mount` binary.
- The checkpoint helper runs in-process (no separate script to hash-pin), after the same trusted checks on `checkpoint-backup.json`.
- A pending SIGTERM can never skip the restore of the original preload policy.
- Device commands only run as the installed, root-owned binary at `/var/lib/lg-native/lg-native`.
- The gate installer recognises the trial hook written by either implementation as owned, so an uncommitted Python trial can be committed or replaced.

## Offline public tests

`cargo xtask test` runs the included suites without accessing a TV. The gate/runtime fixtures are synthetic; they are not TV logs and do not claim physical acceptance. The exact reconstruction of the eight sources was verified locally against private originals (with the original labels); the repository keeps only hashes and a sanitised summary, not the firmware.

CI and local tests are not the same: this repository includes the local runner and does not claim a GitHub Actions run that does not exist.

## Not demonstrated by this revision

- Any execution of the Rust `lg-native` binary on the TV (apply, rollback, gate, guard, remote, checkpoint).
- The English on-screen labels on the TV. They change only display strings, but the labelled files have not been loaded by the TV's QML engine.
- Compatibility with other firmware/models/remotes.
- Absence of every stock frame under every fault; recovery may remove the gate.
- Removal of power / electrical cold boot, or a new full reboot with the latest remote version. The physical power off/on observed is standby/resume.
- USB mouse or phone touchpad with the global pointer policy.
- New AirPlay/HDMI/DRM playback, signal quality or power-on from iPhone in this last phase. Opening interfaces is not playback.
- Causal RAM/CPU savings on the TV itself. The measurements above were taken on a development host; earlier measurements are not extrapolated to future loads.

Private backups are not published as downloadable evidence; they contain device-specific state and material that does not belong in a public distribution.
