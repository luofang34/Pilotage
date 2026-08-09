# Pilotage situation client

This client shows traffic and weather on iPadOS.

The Rust facade links the Surveillance and Airmass domain crates. It builds
three Apple library slices. The build script puts these slices in one
XCFramework. UniFFI generates the Swift binding from the same library.

Run this command:

```sh
sh clients/apple-situation/scripts/ci-ios.sh
```

The command checks the Rust facade. It builds the XCFramework. It then builds
and tests the Swift package.

Generated bindings and binary artifacts are build outputs. Do not commit these
outputs.
