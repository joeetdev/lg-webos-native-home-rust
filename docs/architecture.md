# Architecture

```text
The TV's own original firmware (not distributed)
  -> Home deltas with exact hashes (lg-native-host build-home)
  -> eight generated sources in private staging
  -> lg-native home apply: ownership + backing copies + read-only bind mounts
  -> the SAME com.webos.app.home / LaunchPoints and LG actions

Existing compositor
  -> LG keyFilters chain, kept in order
     -> non-visual gate before handleSystemKeys
        -> proof of eight overrides + per-window marker -> restores saved opacity
     -> remapping before handleCPHotkeys
        -> Google: native blockPointer / cancelBlockPointer policy
        -> MOVIES: applicationManager.checkLaunch Crunchyroll
```

## One static binary, no resident process

Everything that runs on the TV outside the compositor is the single static binary `/var/lib/lg-native/lg-native`, invoked once per command:

| Command | Replaces | Runs |
| --- | --- | --- |
| `home apply --boot` | `nativeapply.py --boot` | from the boot hook, once per boot |
| `home rollback` | `rollback.py` | manually; releases the gate first |
| `gate recover-if-unready` | `native_gate_control.py --recover-if-unready` | from the boot hook after apply |
| `gate disable` | `native_gate_control.py --disable` | manually or from the hook when Home is disabled |
| `gate install`, `gate check`, `gate commit` | `gate_install.py` | manually |
| `gate guard --spawn` / `--watch` | `gate_trial_guard.py` | trial hook; the watcher lives at most 240 s |
| `remote install`, `remote check`, `remote disable` | `remote_controls.py` | manually |
| `checkpoint --dry-run` / `--apply` | `clear-home-checkpoint.py` | manually; `home apply` runs the same logic in-process |

Each command exits as soon as it is done. There is no daemon, no thread pool and no async runtime. Commands that change Home, install the gate or manage the remote refuse to act unless the binary runs as root on Linux AND is the root-owned, single-link file at the fixed private path, so hooks and the trial guard always execute exactly the trusted file. The gate-release paths are the exception: they are the safety net that keeps Home visible, so they need only root, and an untrusted binary simply cannot prove readiness, which releases the gate.

## Authority boundaries

The Home runtime checks SHA-256, metadata, inodes and real mount ownership; the JS proof inside the compositor does not replace those checks. The gate affects only Home windows. An old Home without the marker stays held back even when the current files are correct.

The gate's readiness proof is written against a read-only trait (`HomeReader`) that has no mutating operation at all, so a health check cannot change Home, mounts, preload policy or files by construction.

Runtime, gate and remote state belong to the device and are NOT part of the repository. Each subsystem removes only its own exact entries/mounts, preserving everything foreign. Its helpers are one-shot; no resident processes are added.

## Transactions

`home apply` journals every intent durably BEFORE the side effect it describes (copy, bind, read-only remount, unmount, checkpoint clearing, kill, launch, preload change). All payload and stock hashes are verified before the first mount. Any failure — including SIGTERM or SIGINT, which unwind at the next checkpoint and kill a pending `luna-send` — disables the overrides, removes owned mounts and relaunches stock Home in the background, and the original preload policy of the current boot is restored. While recovery or the preload restore runs, further signals are deferred, so neither can be cut short. Backing copies are content addressed and written under a temporary name, then renamed into place only when their hash matches, so a failed copy never leaves a corrupt file at the content-addressed path.

The read-only remount keeps the existing per-mount flags (`nosuid`, `nodev`, `noexec`, access-time policy) and adds only `ro`; mount ownership is then re-proved from `/proc/self/mountinfo`, inode identity and hashes.

## Pointer lifecycle

`desiredEnabled=false` initially and after real standby. While active, Google toggles the policy; a screensaver preserves the manual choice. The system can restore native support late when returning from standby: observing `cursorVisibleChanged` corrects only the visible + desired-OFF + active case, without continuous polling and without fighting a manual ON or the power-off logo.

The non-visual controller has a cleanup tied to the loader, which enables the pointer and destroys its instance when the loader is removed. No code is injected into lginput and no fictitious cursor endpoints are used. The global scope of the API means different pointer devices must be tested separately.

## Assets and caching

JS, QML and the loader are named by SHA-256, chaining references before the next hash is computed. Rewriting a file under the same URL can make the Qt engine reuse an earlier compilation. Do not restart the shared compositor just to clear that cache. The Rust packers produce byte-identical assets to the earlier Python packers.

## Memory design

- One process per command; nothing stays resident except the bounded trial watcher.
- Static musl binary (~0.6 MB on disk), size-optimised, `panic = "abort"`, no unwinding tables.
- Streaming SHA-256 through a 16 KiB stack buffer; files are never loaded whole except bounded (≤ 1 MiB) JSON.
- Child output drained with `poll(2)` from one thread, capped at 1 MiB, with hard deadlines.
- `mount(2)`/`umount2(2)` system calls instead of spawning `mount` for each of the eight files.
- A purpose-built JSON parser with a depth limit and duplicate-key rejection, instead of a general-purpose framework.
