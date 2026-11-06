// Which site this build is: condr.dev follows the latest release, built from its tag, and
// next.condr.pages.dev follows main, the nightly's docs. website.yml sets
// CONDR_SITE_CHANNEL=next for the second; a local build is the release site.
export const nightly = process.env.CONDR_SITE_CHANNEL === "next";

export const nightlyBanner = {
  en: 'These docs follow the nightly build and may describe what the latest release does not have yet. For the release, see <a href="https://condr.dev/docs/">condr.dev</a>.',
  "zh-CN": '这里是 nightly 版的文档，可能写到了最新正式版还没有的功能。正式版文档见 <a href="https://condr.dev/zh-cn/docs/">condr.dev</a>。',
};
