import cargo from "../../Cargo.toml?raw";
import mobile from "../../mobile/version.xcconfig?raw";

// The latest release's version. website.yml names it, as the latest published release,
// whatever ref the site is built from; a local build reads the workspace version.
const found =
  process.env.CONDR_RELEASE_VERSION ||
  /\[workspace\.package\][^[]*?\bversion = "([^"]+)"/.exec(cargo)?.[1];
if (!found) throw new Error("Cargo.toml has no [workspace.package] version");
export const version = found;

// The Companion's latest release, which website.yml names likewise, since the release tag
// the site is built from may predate it (ADR 0049); a local build reads the file.
const companion =
  process.env.CONDR_COMPANION_VERSION || /^MARKETING_VERSION\s*=\s*(\S+)/m.exec(mobile)?.[1];
if (!companion) throw new Error("mobile/version.xcconfig has no MARKETING_VERSION");
export const companionVersion = companion;
