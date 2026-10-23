import cargo from "../../Cargo.toml?raw";
import mobile from "../../mobile/version.xcconfig?raw";

// The workspace version, which is the latest release: release.yml redeploys the
// site once that release is published, so whatever the site names exists by then.
const found = /\[workspace\.package\][^[]*?\bversion = "([^"]+)"/.exec(cargo)?.[1];
if (!found) throw new Error("Cargo.toml has no [workspace.package] version");
export const version = found;

// The Companion's version, which is its latest release: companion.yml redeploys the site
// once that release is published (ADR 0049).
const companion = /^MARKETING_VERSION\s*=\s*(\S+)/m.exec(mobile)?.[1];
if (!companion) throw new Error("mobile/version.xcconfig has no MARKETING_VERSION");
export const companionVersion = companion;
