# AGENTS.md

## Scope

Keep the SAME native Home, `com.webos.app.home`. Do not create another launcher, dashboard, app, resident service or compositor. Reference device: LG 50UQ8050PSB, webOS 7.6.0 / webOS 22, firmware 04.60.90. Do not claim universal compatibility.

`home/` holds reproducible deltas, not complete firmware. The Home runtime controls mounts; the gate protects early presentation; the remote subsystem implements the remote control and its independent ownership. All device logic lives in the single static binary built from `crates/lg-native`; host tooling lives in `crates/lg-native-host`. QML/JS stays only where it runs inside the existing compositor (`gate/`, `remote/`).

## Development and verification

Host: Rust 1.74+ (rustup) and Node 20+. TV runtime: the static `lg-native` binary; no interpreter is required on the TV.

```sh
cargo xtask test
```

Use RED→GREEN: a test that fails before the change, the minimal implementation, then the complete affected suite. Do not lower expectations to hide regressions. Ordinary tests are offline: do not connect to or modify a TV without specific authorisation.

Keep these separate: synthetic tests, static analysis, compiled QML, injected events and physical tests. An API that opens an app does not prove playback. Powering off with the remote is not the same as removing power or a software reboot. Do not claim green CI when only local tests ran.

Memory is a design constraint: one short-lived process per command, no threads, async runtimes or daemons, streamed hashing, bounded reads (1 MiB) and bounded child processes. Measure before claiming savings and state where the measurement was taken.

## Home invariants

- Eight exact targets and known hashes; reject unknown firmware. Device originals stay out of Git.
- Preserve LaunchPoints, `doLaunch` objects, source indices, the editor, accessibility, focus and native navigation.
- Flat grid, six columns, eleven apps and "All apps" in position 12. Explicit order from the README, not usage frequency; Apps/Media/Web tools at the end.
- ThinQ only in the sidebar with its native object; Settings last. USB only with real storage, not because a physical port exists.
- Clock without a card, colour `#ADC4CE`, fixed slots per character and seconds raised. Do not depend on proportional widths. Update only while Home is visible/active, outside the editor/Recents.
- On-screen labels: English by default, applied only after the pinned output hashes are verified; `--labels original` reproduces the physically verified bytes.

## Safety and recovery

Do not touch firmware, partitions, the root chain, startup/jumpstart or shared executables. Preserve TV, HDMI, picture/sound, CEC/ARC/eARC, AirPlay, HomeKit/ThinQ, standby networking, accounts, pairings, the store and DRM. Do not fake activation flags or preload support. The reference preference is Netflix ON / YouTube OFF; Crunchyroll was not supported by the manager.

Validate canonical paths, permissions, root ownership, regular files, links, hashes and mount identity. Record intent and ownership BEFORE side effects, and verify the exact destination after writing. A successful SSH command or `setConfigs` does not prove that the QML loaded.

Immutable JS/QML/loader assets and mount copies named by SHA-256: Qt may reuse old code if content changes while the URL stays the same. The gate touches only Home; it requires proof of the eight overrides AND the marker of the specific window, never just correct files.

Preserve foreign filters, their order, and safety/accessibility. Read the LIVE chain and remove only exact entries recorded as owned; never restore an old complete list. Serialise writers: configd offers no compare-and-swap.

A keyFilters change destroys loaders and cancels genericTimer and subscriptions. genericTimer is NOT an independent watchdog. Every test needs a recovery that survives the failure being tested. Do not add VNC, injectors, perpetual polling or resident diagnostic servers.

Only the installed, root-owned binary at `/var/lib/lg-native/lg-native` changes Home, installs the gate or manages the remote; hooks and the trial guard execute exactly that file. Gate-release paths need only root: the safety net must never be blocked by a trust check. Recovery and the preload restore defer termination signals. `unsafe` Rust is limited to individually allowed, documented libc calls.

## Remote control

GoogleVoice Qt 18874398 and USP/MOVIES Qt 18874413 were confirmed physically with deviceId 3 on the reference remote. They are not evdev codes; capture other remotes before changing this profile. Consume press/release; ignore auto-repeat for the action; discard orphaned/expired releases; keep every other button.

`desiredEnabled` is distinct from the applied flag. OFF at start/resume; manual ON without expiry. Keep the choice during the screensaver. Use the native blockPointer/cancelBlockPointer, not invented Luna endpoints. On a late native reset, observe cursorVisibleChanged and reapply OFF only when the cursor becomes visible, the desired policy is OFF and powerState=active. Respect manual activation and the power-off logo.

MOVIES uses checkLaunch with the installed ID, without parameters that force a relaunch. The eight-second timer was only a historical test, NOT the permanent behaviour.

## Publication

Exclude SSH keys, tokens, `.env`, passwords, operational nonces, accounts, private addresses/MAC/SSID, personal paths, live state, full inventories, ROMs/BIOS, IPK/SO, dumps and ZIP/TAR backups. Do not add the private backup directory. Fixtures are small, synthetic and labelled; never present invented data as physical results.

Generated products and originals stay ignored. Do not publish complete LG firmware. The screenshot documents the result; it grants no rights to the background or third-party trademarks. Do not assign licences to files without verifying provenance and the owner's choice.

Before publishing: tests, diff review, privacy/secret analysis and independent review. This is the owner's personal repository, not an organisation's; do not change visibility or ownership without authorisation. README and documentation must distinguish available automation, manual steps and real limits.
