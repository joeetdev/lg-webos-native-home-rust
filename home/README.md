# Home: reproducible deltas, no redistributed firmware

This directory contains only replacement data, not the eight complete LG files. The builder is the host tool `lg-native-host build-home` (Rust, offline): no downloads, no TV, no SSH, no privileged commands and no installation.

## Firmware boundary

`deltas.json` pins eight exact `sourceSHA256` → `targetSHA256` pairs. Only those firmware bytes are accepted: matching the model, the webOS version or a file name is **not** enough. There is no force mode and no approximate application. A different firmware needs review and new deltas.

Each edit uses `start` (zero-based index of an original line), `delete` (number of original lines to skip) and `lines` (only new/replacement lines, with explicit terminators). It contains no removed original lines, no diff context and no proprietary headers. Offsets always refer to the original, never to the result of earlier edits.

## Local build

From the repository root, put legitimately obtained copies in a **private directory**, keeping the subdirectories under `qml/` given by `stock` in `home/deltas.json`. Do not add those copies or the results to the repository. `private/` and `generated/` are example private/ignored relative paths:

```sh
cargo run --release -p lg-native-host -- build-home --stock-dir private/stock-qml --out private/generated-home
cargo test -p lg-native-host
```

The parent of `--out` must exist; the output directory must be new and outside the stock tree. Outputs are never overwritten. Missing files, non-regular files, symlinks/reparse points and unsafe path components are rejected. All input hashes are checked **before anything is transformed**; all pinned output hashes are checked before any output is created. Use local directories under your control, with no processes modifying them during the operation.

There is no silent normalisation. If a local copy was converted to CRLF, only this explicit mode converts CRLF → LF **in memory** before validating the hashes; it does not modify the originals, does not strip a BOM and does not convert a lone CR:

```sh
cargo run --release -p lg-native-host -- build-home --stock-dir private/stock-qml --out private/generated-home-lf --normalize-crlf
```

Eight `native-*` files and `home-final-manifest.json` are produced. The manifest keeps `source`, `target`, `originalHash` and `newHash`, as consumed by `lg-native home apply`. For **manual** staging, `source` uses `R=/var/lib/lg-native`: `/var/lib/lg-native/native-MainView.qml`, and so on. `target` is the original destination under `/usr/palm/applications/com.webos.app.home/qml/`. These device paths are public constants; private paths of the authoring machine are never written. The builder does not copy anything to the device and does not authorise an installation.

## On-screen labels

The physically verified build used Spanish labels for three custom strings. By default the builder now produces English labels:

| Location | Original | English (default) |
| --- | --- | --- |
| Sidebar search button | Buscar | Search |
| Sidebar settings button | Ajustes | Settings |
| Twelfth tile and its audio-guide text | Todas las apps | All apps |

The English layer runs strictly AFTER the eight pinned output hashes have been verified, replaces exactly one complete line per label (the build fails if a line is not found exactly once) and changes display text only — no identifier, LaunchPoint, action, size or position. The manifest then records the hashes of the labelled files, which the runtime and the gate verify as usual.

To reproduce the byte-exact, physically verified output instead:

```sh
cargo run --release -p lg-native-host -- build-home --stock-dir private/stock-qml --out private/generated-home --labels original
```

## Required background, not distributed

The exact reproduction of `native-MainView.qml` references:

```text
file:///var/lib/lg-native/home-assets/sidebar-landscape-92d63fb128d8b806b6234b2d9a8034f55c4a1fc406f031192de8a5494d21f529.png
```

The image is **not included**, and its licence/redistribution rights are not established here. You must obtain and stage that asset separately with the corresponding rights. The builder neither checks nor installs the image. Without it the background may be missing even if all eight hashes are correct. Changing the reference means a different payload, not a byte-for-byte reproduction.

## What has been verified

`verification.json` records a real run against private stock and the independent final manifest (performed with the earlier Python builder, original labels): **8/8 input hashes, 8/8 final hashes and 8/8 byte-for-byte comparisons with the candidate**. The CRLF input failed in strict mode and passed only with `--normalize-crlf`. The Rust builder implements the same algorithm; it was checked against the Python builder on 3,000 randomised differential cases, but it has not yet been re-run against the private stock files. The unit tests use synthetic fixtures, never stock files. None of this is equivalent to compiling QML with LG's private imports or to validating focus, the remote, boot or standby on a TV. The generated results contain firmware and must stay private; publishing deltas grants no rights over third-party sources or images.
