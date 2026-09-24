import { version } from "../version";

// install.sh / install.ps1 compare this with the installed condr before downloading.
export const GET = () => new Response(`${version}\n`);
