import { version } from "../version";

// install.sh / install.ps1 compare this with the installed condr before downloading, and
// the GUI's stable update check reads it (ADR 0029): keep it a bare `1.2.3` line.
export const GET = () => new Response(`${version}\n`);
