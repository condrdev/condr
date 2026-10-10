// swift-tools-version:5.9
// condr-mobile's Swift interface (ADR 0039): `script/build-mobile.sh ios` builds the
// xcframework beside this file and generates `Sources/CondrKit/condr_mobile.swift`.

import PackageDescription

let package = Package(
    name: "CondrKit",
    platforms: [.iOS(.v17)],
    products: [
        .library(name: "CondrKit", targets: ["CondrKit"]),
    ],
    targets: [
        .binaryTarget(name: "condr_mobileFFI", path: "condr_mobileFFI.xcframework"),
        .target(name: "CondrKit", dependencies: ["condr_mobileFFI"]),
    ]
)
