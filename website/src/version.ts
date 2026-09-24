import cargo from "../../Cargo.toml?raw";

// The workspace version, which is the latest release: release.yml redeploys the
// site once that release is published, so whatever the site names exists by then.
const found = /\[workspace\.package\][^[]*?\bversion = "([^"]+)"/.exec(cargo)?.[1];
if (!found) throw new Error("Cargo.toml has no [workspace.package] version");
export const version = found;
