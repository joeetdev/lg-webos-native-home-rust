# Provenance and redistribution

This project brings together presentation and control modifications developed for one particular webOS installation. It is not affiliated with or endorsed by LG, Homebrew Channel or the providers of the applications mentioned.

No firmware images, executables or complete copies of LG files are distributed. The Home deltas require originals from the device and exact hashes; unmodified parts are not included as complete source files. The test fixtures are synthetic and are not dumps of the TV.

Third-party names, logos and applications belong to their respective owners. The Home screenshot documents the result; the background image is not published as a reusable asset.

No general reuse licence has been selected yet for the code in this repository. Do not attribute an MIT, GPL or any other licence to files or dependencies without verifying their provenance and the owner's choice. The Rust dependencies fetched by Cargo (`sha2`, `libc`, `regex` and their transitive crates) keep their own upstream licences.

Technical references, not automatically installed dependencies:

- Homebrew Channel / webOS Homebrew: https://github.com/webosbrew/webos-homebrew-channel
- webOS TV documentation: https://webostv.developer.lge.com/
- Qt QML: https://doc.qt.io/qt-5/qtqml-index.html

The reference installation kept the firmware originals, its root mechanism and recovery. Never copy an image or a complete backup from another TV to reproduce this project.
