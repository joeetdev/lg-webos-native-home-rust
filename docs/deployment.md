# Manual deployment and recovery

This document describes the order and the contracts, not a universal installer and not an instruction to run every step. The root commands can leave Home invisible or cause it to be relaunched. Keep independent SSH access, the originals and external backups BEFORE changing the TV. Nothing on any device is modified by cloning or testing this repository.

All device commands below are the single binary `/var/lib/lg-native/lg-native`. It refuses to run unless it is executed as root on Linux from exactly that path, as a root-owned, single-link file that group/other cannot write, inside root-owned directories.

## 1. Prerequisites

- The exact device/firmware stated in the README and the known hashes of the eight originals.
- Root already configured, a compatible Homebrew Channel and `luna-send` available. Python is no longer required on the TV.
- A private inventory of apps, versions, keyFilters, hooks, preloads and mounts. Do not commit it.
- A real external backup of the Home-derived checkpoints, dereferencing linked pages, BEFORE using the checkpoint helper. An empty JSON file is not a backup.
- Directory `/var/lib/lg-native` root-owned and private (0700); regular root-owned files without group/other write; private assets/state 0600. Check ancestors and symlinks too.
- Your own background image (or one you have permission to use) at the exact paths of the rebuilt payload. Do not silently substitute images, hashes or paths in a validated package.
- An independent recovery guard BEFORE every test with side effects; do not rely on `genericTimer` or on an SSH channel that might disappear.

## 2. Build on the host

```sh
cargo xtask test                                    # offline suites
rustup target add aarch64-unknown-linux-musl
cargo build --release -p lg-native --target aarch64-unknown-linux-musl
```

Confirm the TV's userland architecture first; for 32-bit ARM use `armv7-unknown-linux-musleabihf`. Stage the binary as `/var/lib/lg-native/lg-native`, owner `root:root`, mode `0755`, and verify its SHA-256 on the TV against the host build.

## 3. Offline reconstruction

Obtain the originals NOT covered by overrides and run the builder from [home/README.md](../home/README.md). Check that the eight products match: with `--labels original` they match the final hashes in `home/`; with the default English labels the builder first verifies those pinned hashes and then writes the manifest with the hashes of the labelled files. Do not patch by similarity or update the allowlist just to force a different firmware.

The generated manifest describes staging paths in `/var/lib/lg-native` and the native targets. Copy to the TV exactly the files it references, verify SHA-256 and permissions, and keep the originals off the TV. If you change paths, generate and validate a coherent manifest: never copy one from someone else's backup.

## 4. Home runtime

`lg-native home apply` limits the targets, validates ownership/metadata, records intent before each mount and uses immutable backing copies. `lg-native home rollback` first releases the presentation gate, then restores only mounts that are provably owned.

Staging needs the binary, the generated `home-final-manifest.json`, the exact sources and a private `checkpoint-backup.json` documenting the real backup. The tools do not create that backup for you. Checkpoint clearing only considers the two Home-derived roots, never `inputcommon` or global CRIU state; `lg-native checkpoint --dry-run` shows the plan without changing anything.

Do not install the boot hook until the gate recovery is prepared: the published `runtime/55-lg-native` runs `home apply --boot` and then `gate recover-if-unready`. Never install a hook that calls a binary or module that is absent.

Before persisting: apply with a guard, check the eight read-only mounts and hashes, open Home with the remote, test TV/inputs/apps and a real restore. Commit each trial before the next one. Do not touch `startup.sh`/`jumpstart.sh`.

## 5. Early presentation gate

Packaging on the host:

```sh
cargo run --release -p lg-native-host -- pack-gate final
```

This produces three immutable assets and `gate/packed-final/package.json`. The Home runtime's hash/mount proof remains the authority. The JS proof compares the eight sources/targets/backings, the journal and the manifest; each window additionally needs `lgNativeHomeReady=lg-native-home-v1`, published by the customised MainView.

Manual staging: the three assets and the final package under the remote name `gate-package-final.json`, plus the binary. Review the hooks embedded in the installer first (`runtime/55-lg-native` and the trial hook in `crates/lg-native/src/gate/install.rs`): installing the gate changes only the owned hooks `55-lg-native` and adds the temporary recovery hook `54-lg-native-gate-trial`. Foreign hooks are never overwritten.

```sh
/var/lib/lg-native/lg-native gate install /var/lib/lg-native/gate-package-final.json
/var/lib/lg-native/lg-native gate check
/var/lib/lg-native/lg-native gate commit    # only after the final verification
```

`gate install` validates and installs behind a 240-second trial guard that removes the gate again unless `gate commit` runs first; `gate check` reports readiness and the chain; `gate commit` removes the trial token and the temporary hook. `gate disable` removes only the exact gate entries; `gate recover-if-unready` gives priority to a usable Home when integrity fails.

The gate is not allowed without a working payload/marker and recovery. Verify a real boot without opening Homebrew; persistent configuration on its own does not prove the first frame. Do not delete the firmware originals.

## 6. Remote control, independent of Home

On the host, `cargo run --release -p lg-native-host -- pack-remote` generates the trial and final packages. The final package contains no telemetry and no pointer expiry. The current trial package contains observation traces, NOT an installed server and NOT an eight-second timer. Do not install it without building a bounded private observation; use the final package after testing the hardware.

Staging: the binary, the three assets of the final package and `remote-final-package.json`. Confirm the Crunchyroll ID and the remote's codes/device before using this profile. Install with an external recovery already armed:

```sh
/var/lib/lg-native/lg-native remote install /var/lib/lg-native/remote-final-package.json
/var/lib/lg-native/lg-native remote check
```

The state file `remote-controls-state.json` is created locally and stays private. The entry is added before `handleCPHotkeys`, preserving every previous filter including the gate. The total of 19 describes only the reference installation, not a list to be forced.

Physical tests: OFF at start; ON/OFF with Google; arrows/OK/Back/volume; MOVIES→Crunchyroll; wait more than 15 s without reactivation; leave ON, power off/on with the remote and check OFF; enable again. Test phone/USB separately if needed. Read back state/package/configuration and check that no diagnostics remain before removing the external guard.

## 7. Migrating an installation made with the Python implementation

The Rust binary reads the existing state unchanged (`home-mounts.json`, `home-boot.json`, `native-home-gate-state.json`, `remote-controls-state.json`), uses the same boot identity format and produces byte-identical gate/remote assets, so no reinstallation of the overrides, gate or remote entries is needed. The migration only replaces the helpers.

1. With an external recovery armed, stage `/var/lib/lg-native/lg-native` as described in section 2.
2. Run the read-only checks: `lg-native gate check` (proves the live Python-created mounts and journal with the Rust readiness proof) and `lg-native remote check`. Do not continue unless both succeed.
3. Replace the boot hook `/var/lib/webosbrew/init.d/55-lg-native` with `runtime/55-lg-native` (root-owned, mode 0755). This is the smallest change: it does not touch keyFilters. Alternatively, `lg-native gate install` with the same final package also rewrites the hook and re-arms the trial guard, but it writes the keyFilters chain again (unchanged content), which reloads the loaders.
4. If an uncommitted Python trial hook `54-lg-native-gate-trial` exists, `lg-native gate commit` recognises and removes it.
5. Reboot and verify as in sections 4–6. Only after a verified boot remove the old Python helpers from `/var/lib/lg-native` (`native_home.py`, `nativeapply.py`, `rollback.py`, `clear-home-checkpoint.py`, `native_gate_control.py`, `gate_install.py`, `gate_trial_guard.py`, `remote_controls.py`). Until then they remain a working fallback.

## Independent recovery

Remote only:

```sh
/var/lib/lg-native/lg-native remote disable
```

Gate only:

```sh
/var/lib/lg-native/lg-native gate disable
```

Stock Home, keeping everything foreign and releasing its gate first:

```sh
/var/lib/lg-native/lg-native home rollback
```

Do not delete state files to bypass ownership. Do not blindly restore an old filter chain, and do not copy journals or backups from another device. Keep evidence off the TV; remove only enumerated temporary files verified by hash. A successful command does not replace checking the result.
